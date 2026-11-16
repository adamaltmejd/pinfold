//! Project ownership, admission and connection leases.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, IsTerminal};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nix::fcntl::{Flock, FlockArg};
use nix::libc;
use serde::{Deserialize, Serialize};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, timeout_at};

use crate::cli;
use crate::core::r#box::{Box, Shutdown, Signals};
use crate::core::process::CommandGuard;
use crate::core::runtime::{ExecContext, runtime, sockets};
use crate::core::{artifacts, clean, ownership, runs};
use crate::formatting::terminal_text;
use crate::pi::context::Context;
use crate::pi::session_snapshot::{Resolved, Roots, Snapshot};
use crate::pi::session_wire::{self as wire, Intent, Reply, Request};
use crate::pi::{launch, state};

const CONTROL: Duration = Duration::from_secs(10);

fn unavailable() -> io::Error {
    io::Error::other("owner-unavailable")
}
fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "owner-timeout")
}
#[derive(Debug)]
struct OutputFailure;
impl std::fmt::Display for OutputFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("diagnostic output failed")
    }
}
impl std::error::Error for OutputFailure {}
fn output_failure() -> io::Error {
    io::Error::other(OutputFailure)
}
fn is_output_failure(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<OutputFailure>())
}
fn protocol() -> io::Error {
    io::Error::other("session-protocol")
}
fn changed(fields: Vec<String>) -> io::Error {
    io::Error::other(format!("config-changed: {}", fields.join(", ")))
}
fn reply_error(reply: Reply) -> io::Error {
    match reply {
        Reply::Error { reason, fields } if reason == "config-changed" => changed(fields),
        Reply::Error { reason, .. } if reason == "session-protocol" => protocol(),
        _ => unavailable(),
    }
}
fn key(root: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut key = b"project-owner:".to_vec();
    key.extend(root.as_os_str().as_bytes());
    crate::core::sha256_hex(key)
}
fn launch_key(root: &Path) -> String {
    format!("project-launch:{}", key(root))
}
fn locator_path(root: &Path) -> io::Result<PathBuf> {
    Ok(sockets::host()?.0.join(format!("{}.project", key(root))))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Locator {
    version: u8,
    root: PathBuf,
    name: String,
    generation: Option<String>,
}
impl Locator {
    fn validate(&self, root: &Path) -> io::Result<()> {
        if self.version != 1
            || self.root != root
            || !crate::core::plan::valid_name(&self.name)
            || self
                .generation
                .as_ref()
                .is_some_and(|g| !valid_generation(g))
        {
            return Err(unavailable());
        }
        Ok(())
    }
    fn publish(&self) -> io::Result<()> {
        crate::core::write_atomic(&locator_path(&self.root)?, &serde_json::to_vec(self)?)?;
        sockets::host()?.1.sync_all()
    }
    fn remove(&self) -> io::Result<()> {
        let Some(current) = read_locator(&self.root)? else {
            return Ok(());
        };
        if current.name != self.name || current.generation != self.generation {
            return Err(unavailable());
        }
        fs::remove_file(locator_path(&self.root)?)?;
        sockets::host()?.1.sync_all()
    }
}
fn valid_generation(g: &str) -> bool {
    g.len() == 64
        && g.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn read_locator(root: &Path) -> io::Result<Option<Locator>> {
    let mut file = match File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(locator_path(root)?)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unavailable()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() > 65536
    {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes)?;
    let locator: Locator = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    locator.validate(root)?;
    Ok(Some(locator))
}

// Flock's destructor explicitly unlocks the shared open description. These
// files instead release ownership only when the last inherited copy closes.
fn close_only(lock: Flock<File>) -> File {
    let lock = std::mem::ManuallyDrop::new(lock);
    unsafe { File::from_raw_fd(lock.as_raw_fd()) }
}
fn try_launch(root: &Path) -> io::Result<Option<File>> {
    match Flock::lock(
        ownership::open_lock(&launch_key(root))?,
        FlockArg::LockExclusiveNonblock,
    ) {
        Ok(lock) => Ok(Some(close_only(lock))),
        Err((_, nix::errno::Errno::EAGAIN)) => Ok(None),
        Err((_, error)) => Err(error.into()),
    }
}
async fn launch_lock(root: &Path, signals: &mut Signals, deadline: Instant) -> io::Result<File> {
    let mut poll = tokio::time::interval(Duration::from_millis(20));
    loop {
        if let Some(lock) = try_launch(root)? {
            return Ok(lock);
        }
        tokio::select! {
            _ = signals.recv() => return Err(io::ErrorKind::Interrupted.into()),
            _ = timeout_at(deadline, poll.tick()) => if Instant::now() >= deadline { return Err(timed_out()); },
        }
    }
}

/// Only relevant readable claims take part in project discovery. Core still
/// refuses existing-disk admission if any claim's attachments are unreadable.
fn matching_claims(root: &Path) -> io::Result<Vec<ownership::Record>> {
    let id = state::project_id(root);
    let disks = state::disk_names(&id);
    let mut claims = Vec::new();
    for result in ownership::record_results()? {
        match result {
            Ok(record)
                if record.disks.iter().any(|a| disks.contains(&a.name))
                    || record
                        .described
                        .as_ref()
                        .is_some_and(|d| d.labels.get(clean::PROJECT_LABEL) == Some(&id)) =>
            {
                claims.push(record)
            }
            Ok(_) => {}
            Err(error) => eprintln!("pinfold: {}", terminal_text(&error.to_string())),
        }
    }
    Ok(claims)
}

/// Under launch ownership, retire dead claims before allowing a replacement.
fn discover(root: &Path) -> io::Result<Option<Locator>> {
    let locator = read_locator(root)?;
    let claims = matching_claims(root)?;
    let mut live = Vec::new();
    for record in claims {
        if ownership::name_alive(&record.name)? {
            live.push(record);
            continue;
        }
        let Some(lock) = ownership::try_name_lock(&record.name)? else {
            return Err(unavailable());
        };
        // Strict targeted removal checks liveness and resumes durable retirement.
        clean::remove_orphan(runtime(), &record.name, Some(&record.generation), &lock)?;
    }
    if live.len() > 1 {
        return Err(unavailable());
    }
    match (locator, live.pop()) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(unavailable()),
        (Some(mut locator), Some(record)) => {
            if locator.name != record.name
                || locator
                    .generation
                    .as_ref()
                    .is_some_and(|g| *g != record.generation)
            {
                return Err(unavailable());
            }
            if let Some(actual) = runtime().inspect(&record.name)?
                && (actual.labels.get(ownership::GENERATION_LABEL) != Some(&record.generation)
                    || actual.labels.get(clean::PROJECT_LABEL) != Some(&state::project_id(root)))
            {
                return Err(unavailable());
            }
            if locator.generation.is_none() {
                locator.generation = Some(record.generation);
                locator.publish()?;
            }
            Ok(Some(locator))
        }
        (Some(locator), None) => {
            let Some(lock) = ownership::try_name_lock(&locator.name)? else {
                return Err(io::ErrorKind::WouldBlock.into());
            };
            let record = ownership::record(&locator.name)?;
            if record.as_ref().is_some_and(|r| {
                locator
                    .generation
                    .as_ref()
                    .is_some_and(|g| g != &r.generation)
            }) {
                return Err(unavailable());
            }
            clean::remove_orphan(
                runtime(),
                &locator.name,
                record.as_ref().map(|r| r.generation.as_str()),
                &lock,
            )?;
            locator.remove()?;
            Ok(None)
        }
    }
}

async fn connect(locator: &Locator) -> io::Result<UnixStream> {
    let generation = locator.generation.as_deref().ok_or_else(unavailable)?;
    let socket = sockets::Sockets::open(generation)?
        .connect_control()
        .map_err(|_| unavailable())?;
    wire::peer(&socket)?;
    socket.set_nonblocking(true)?;
    UnixStream::from_std(socket)
}
async fn exchange(locator: &Locator, request: Request) -> io::Result<(UnixStream, Reply)> {
    let mut stream = connect(locator).await?;
    wire::write_request(&mut stream, request).await?;
    let reply = wire::read_reply(&mut stream, false).await?;
    Ok((stream, reply))
}
async fn probe(root: &Path, locator: &Locator) -> io::Result<bool> {
    let (_, reply) = exchange(
        locator,
        Request::Probe {
            root: root.into(),
            generation: locator.generation.clone().ok_or_else(unavailable)?,
            roots: Roots::resolve()?,
        },
    )
    .await?;
    match reply {
        Reply::Compatible {} => Ok(true),
        Reply::Draining { generation } if Some(&generation) == locator.generation.as_ref() => {
            Ok(false)
        }
        other => Err(reply_error(other)),
    }
}

struct Lease {
    name: String,
    generation: String,
    writer: OwnedWriteHalf,
    replies: mpsc::Receiver<io::Result<Reply>>,
    reader: tokio::task::JoinHandle<()>,
    child: Option<Child>,
    ended: Option<io::Result<()>>,
}
impl Lease {
    fn new(stream: UnixStream, name: String, generation: String, child: Option<Child>) -> Self {
        let (mut read, writer) = stream.into_split();
        let (send, replies) = mpsc::channel(2);
        let reader = tokio::spawn(async move {
            loop {
                let reply = wire::read_reply(&mut read, true).await;
                let ended =
                    reply.is_err() || matches!(reply, Ok(Reply::Left {} | Reply::Ended { .. }));
                if send.send(reply).await.is_err() || ended {
                    break;
                }
            }
        });
        Self {
            name,
            generation,
            writer,
            replies,
            reader,
            child,
            ended: None,
        }
    }
    fn remember(&mut self, reply: Option<io::Result<Reply>>, leaving: bool) {
        self.ended = Some(match reply {
            Some(Ok(Reply::Ended {
                generation,
                teardown_ok: true,
                ..
            })) if generation == self.generation => Ok(()),
            Some(Ok(Reply::Left {})) if leaving => Ok(()),
            Some(Ok(other)) => Err(reply_error(other)),
            Some(Err(error)) => Err(error),
            None => Err(unavailable()),
        });
    }
    fn end_result(&self) -> io::Result<()> {
        match self.ended.as_ref().expect("observed lease departure") {
            Ok(()) => Ok(()),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
    async fn ended(&mut self) -> io::Result<()> {
        if self.ended.is_none() {
            let reply = self.replies.recv().await;
            self.remember(reply, false);
        }
        self.end_result()
    }
    async fn leave(mut self) -> io::Result<()> {
        let result = timeout_at(Instant::now() + CONTROL, async {
            if self.ended.is_none() {
                match self.replies.try_recv() {
                    Ok(reply) => self.remember(Some(reply), false),
                    Err(mpsc::error::TryRecvError::Disconnected) => self.remember(None, false),
                    Err(mpsc::error::TryRecvError::Empty) => {
                        // A successful Ended already in flight still decides teardown
                        // after a concurrent owner close makes Leave fail to write.
                        let _ = wire::write_request(&mut self.writer, Request::Leave {}).await;
                        let reply = self.replies.recv().await;
                        self.remember(reply, true);
                    }
                }
            }
            self.end_result()
        })
        .await
        .map_err(|_| timed_out())
        .and_then(|r| r);
        self.reader.abort();
        let _ = self.reader.await;
        if let Some(child) = &mut self.child {
            let _ = child.try_wait();
        }
        result
    }
}

async fn admit(
    root: &Path,
    intent: Intent,
    signals: &mut Signals,
    executable: &File,
) -> io::Result<Option<Lease>> {
    let _maintenance = close_only(ownership::project_lock(&state::project_id(root))?);
    let deadline = Instant::now() + CONTROL;
    loop {
        let launch = launch_lock(root, signals, deadline).await?;
        let discovered = match discover(root) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_tick(signals, deadline).await?;
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some(locator) = discovered {
            match probe(root, &locator).await {
                Ok(true) => {}
                Ok(false) => {
                    wait_name(&locator.name, signals, deadline).await?;
                    continue;
                }
                Err(_error) if retired_or_removing(&locator)? => {
                    wait_name(&locator.name, signals, deadline).await?;
                    continue;
                }
                Err(error) => return Err(error),
            }
            let Resolved { snapshot, .. } = resolve_spec(root, false, signals).await?;
            let generation = locator.generation.clone().ok_or_else(unavailable)?;
            let request = match intent {
                Intent::Join => Request::Join {
                    root: root.into(),
                    generation,
                    snapshot,
                },
                Intent::Keep => Request::Keep {
                    root: root.into(),
                    generation,
                    snapshot,
                },
            };
            let (stream, reply) = match exchange(&locator, request).await {
                Ok(value) => value,
                Err(_error) if retired_or_removing(&locator)? => {
                    wait_name(&locator.name, signals, deadline).await?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            match reply {
                Reply::Admitted { name, generation }
                    if name == locator.name && Some(&generation) == locator.generation.as_ref() =>
                {
                    return Ok(Some(Lease::new(stream, name, generation, None)));
                }
                Reply::Kept { name, generation }
                    if name == locator.name && Some(&generation) == locator.generation.as_ref() =>
                {
                    return Ok(None);
                }
                Reply::Draining { generation }
                    if Some(&generation) == locator.generation.as_ref() =>
                {
                    // A confirmed draining owner cannot need launch during teardown.
                    let mut poll = tokio::time::interval(Duration::from_millis(20));
                    while ownership::name_alive(&locator.name)? {
                        tokio::select! {
                            _ = signals.recv() => return Err(io::ErrorKind::Interrupted.into()),
                            _ = timeout_at(deadline, poll.tick()) => if Instant::now() >= deadline { return Err(timed_out()); },
                        }
                    }
                    drop(launch);
                    continue;
                }
                other => return Err(reply_error(other)),
            }
        }
        let resolved = resolve_spec(root, true, signals).await?;
        let mut child = spawn_owner(root, &launch, executable, &_maintenance)?;
        let mut stream = child.0;
        let sent = wire::write_request(
            &mut stream,
            Request::Bootstrap {
                root: root.into(),
                intent,
                snapshot: resolved.snapshot,
            },
        )
        .await;
        if let Err(error) = sent {
            drop(stream);
            bootstrap_diagnostics(&mut child.2, signals).await?;
            return Err(error);
        }
        let reply = tokio::select! {
            biased;
            reply = wire::read_reply(&mut stream, true) => reply,
            _ = signals.recv() => Err(io::ErrorKind::Interrupted.into()),
        };
        // EOF cancels uncommitted bootstrap; wait for diagnostic drain without
        // keeping launch open after the child has completed failed cleanup.
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                drop(stream);
                bootstrap_diagnostics(&mut child.2, signals).await?;
                return Err(error);
            }
        };
        bootstrap_diagnostics(&mut child.2, signals).await?;
        match reply {
            Reply::Admitted { name, generation } => {
                return Ok(Some(Lease::new(stream, name, generation, Some(child.1))));
            }
            Reply::Kept { .. } => {
                let _ = child.1.try_wait();
                return Ok(None);
            }
            other => return Err(reply_error(other)),
        }
    }
}

async fn resolve_spec(root: &Path, startup: bool, signals: &mut Signals) -> io::Result<Resolved> {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let root = root.to_path_buf();
    let mut worker = tokio::task::spawn_blocking(move || {
        crate::core::process::with_cancellation(worker_cancel, || {
            let resolved = Snapshot::resolve(&root)?;
            let warnings = if startup {
                launch::image_warnings(
                    &resolved.config,
                    resolved.plan.image.as_deref().ok_or_else(unavailable)?,
                )?
            } else {
                Vec::new()
            };
            Ok::<_, io::Error>((resolved, warnings))
        })
    });
    let (resolved, warnings) = tokio::select! {
        biased;
        _ = signals.recv() => { cancel.store(true, Ordering::Release); let _ = worker.await; Err(io::ErrorKind::Interrupted.into()) },
        result = &mut worker => result.map_err(|_| io::Error::other("configuration worker failed"))?,
    }?;
    for warning in warnings {
        forward_diagnostics(warning.into_bytes(), signals).await?;
    }
    Ok(resolved)
}

type Diagnostics = tokio::task::JoinHandle<io::Result<Vec<u8>>>;

fn spawn_owner(
    root: &Path,
    launch: &File,
    executable: &File,
    maintenance: &File,
) -> io::Result<(UnixStream, Child, Diagnostics)> {
    let (client, owner) = {
        let _guard = crate::core::process::fork_guard();
        std::os::unix::net::UnixStream::pair()?
    };
    wire::peer(&client)?;
    client.set_nonblocking(true)?;
    let client = UnixStream::from_std(client)?;
    let descriptors = [
        owner.as_raw_fd(),
        launch.as_raw_fd(),
        executable.as_raw_fd(),
        maintenance.as_raw_fd(),
    ];
    let descriptor_limit = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) }.max(1024) as i32;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("__project-owner")
        .args(descriptors.map(|fd| fd.to_string()))
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            for fd in 3..descriptor_limit {
                // Mark unrelated inherited descriptors close-on-exec; keep the
                // stdlib spawn-error pipe alive until exec itself succeeds.
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            for fd in descriptors {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn_guarded()?;
    drop((owner, command));
    let stderr = child.stderr.take().ok_or_else(unavailable)?;
    use std::os::fd::IntoRawFd;
    let stderr = unsafe { File::from_raw_fd(stderr.into_raw_fd()) };
    // ChildStderr is a pipe, not a UnixStream; AsyncFd reads the retained fd.
    crate::core::pipe::nonblocking(&stderr, "owner diagnostics")?;
    let diagnostic = tokio::spawn(async move {
        let mut kept = Vec::new();
        // SAFETY: the owned pipe keeps the registered descriptor stable.
        let reader = unsafe { tokio::io::unix::AsyncFd::register(stderr) }?;
        loop {
            let mut ready = reader.readable().await?;
            let mut bytes = [0u8; 4096];
            let read = ready.try_io(|fd| {
                use std::io::Read;
                fd.get_ref().read(&mut bytes)
            });
            match read {
                Ok(Ok(0)) => break,
                Ok(Err(error)) => return Err(error),
                Ok(Ok(n)) => {
                    kept.extend_from_slice(&bytes[..n.min(65536usize.saturating_sub(kept.len()))])
                }
                Err(_) => {}
            }
        }
        Ok(kept)
    });
    Ok((client, child, diagnostic))
}

async fn bootstrap_diagnostics(drain: &mut Diagnostics, signals: &mut Signals) -> io::Result<()> {
    if signals.last().is_some() {
        drain.abort();
        let _ = drain.await;
        return Err(io::ErrorKind::Interrupted.into());
    }
    let bytes = tokio::select! {
        biased;
        result = &mut *drain => result.map_err(|_| unavailable())??,
        _ = signals.recv() => {
            drain.abort();
            let _ = drain.await;
            return Err(io::ErrorKind::Interrupted.into());
        }
        _ = tokio::time::sleep(CONTROL) => {
            drain.abort();
            let _ = drain.await;
            return Err(timed_out());
        }
    };
    forward_diagnostics(bytes, signals).await
}

struct DiagnosticFlags(Option<libc::c_int>);
impl DiagnosticFlags {
    fn restore(&mut self) -> io::Result<()> {
        if let Some(flags) = self.0.take()
            && unsafe { libc::fcntl(2, libc::F_SETFL, flags) } < 0
        {
            return Err(output_failure());
        }
        Ok(())
    }
}
impl Drop for DiagnosticFlags {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

async fn forward_diagnostics(bytes: Vec<u8>, signals: &mut Signals) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut escaped = terminal_text(&String::from_utf8_lossy(&bytes)).into_bytes();
    escaped.push(b'\n');
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        let saved = unsafe { libc::fcntl(2, libc::F_GETFL) };
        if saved < 0 {
            return Err(output_failure());
        }
        let mut flags = DiagnosticFlags(Some(saved));
        let mut stderr = io::stderr();
        let written = crate::core::pipe::nonblocking(&stderr, "owner diagnostics").and_then(|()| {
            crate::core::pipe::write_all(
                &mut stderr,
                &escaped,
                std::time::Instant::now() + Duration::from_secs(1),
                Some(&worker_cancel),
                "owner diagnostics",
            )
        });
        let restored = flags.restore();
        written.and(restored).map_err(|_| output_failure())
    });
    tokio::select! {
        biased;
        result = &mut worker => result.map_err(|_| output_failure())?,
        _ = signals.recv() => {
            cancel.store(true, Ordering::Release);
            let _ = worker.await;
            Err(io::ErrorKind::Interrupted.into())
        }
    }
}

pub(crate) fn run(args: &[String]) -> io::Result<i32> {
    if args.first().is_none_or(String::is_empty) {
        return Err(cli::usage("--", "expected a nonempty command after --"));
    }
    client(Some(args))
}
pub(crate) fn start(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(cli::usage("start", "start takes no arguments"));
    }
    client(None)
}
fn client(args: Option<&[String]>) -> io::Result<i32> {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    executor.block_on(async {
        let mut signals = Signals::with_hangup()?;
        let result = async {
        let executable = close_only(crate::update::working()?);
        let cwd = state::canonical(&std::env::current_dir()?)?;
        let context = Context::discover(&cwd)?;
        launch::utf8(&context.root, "project root")?;
        let admitted = admit(&context.root, if args.is_some() { Intent::Join } else { Intent::Keep }, &mut signals, &executable).await;
        let mut lease = match admitted {
            Ok(Some(lease)) => lease,
            Ok(None) => { crate::update::notice("start"); clean::maintain(); return Ok(0); },
            Err(_error) if signals.last().is_some() => return Ok(128 + signals.last().expect("received signal") as i32),
            Err(error) => {
                if !is_output_failure(&error) {
                    let verb = if args.is_some() { "--" } else { "start" };
                    let report = format!("pinfold {verb}: {error}");
                    let _ = forward_diagnostics(report.into_bytes(), &mut signals).await;
                    if let Some(signal) = signals.last() { return Ok(128 + signal as i32); }
                }
                return Ok(125);
            },
        };
        crate::update::notice("--");
        clean::maintain();
        let argv = args.expect("join has a command").to_vec();
        let name = lease.name.clone();
        let root = context.root;
        let init = artifacts::init()?;
        let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let mut exec = tokio::task::spawn_blocking(move || {
            runtime().exec(&name, &init, &ExecContext {
                cwd: Some(&cwd), project_root: Some(&root), tty, agent: Some(&argv[0]), cancel: Some(&worker_cancel),
            }, &argv)
        });
        let (result, completed) = tokio::select! {
            biased;
            result = &mut exec => (result.map_err(|_| io::Error::other("command execution worker failed")).and_then(|r| r).map(cli::exit_code), true),
            _ = lease.ended() => (Ok(125), false),
            signal = signals.recv() => (Ok(128 + signal as i32), false),
        };
        cancel.store(true, Ordering::SeqCst);
        crate::core::runtime::terminal::cancel();
        if !completed {
            // The transport itself can already have observed native completion.
            if let Ok(Ok(status)) = exec.await { return finish_client(lease, Ok(cli::exit_code(status)), &mut signals).await; }
        }
        finish_client(lease, result, &mut signals).await
        }.await;
        match result {
            Ok(code) => Ok(code),
            Err(error) => {
                let verb = if args.is_some() { "--" } else { "start" };
                let _ = forward_diagnostics(format!("pinfold {verb}: {error}").into_bytes(), &mut signals).await;
                Ok(signals.last().map_or(125, |signal| 128 + signal as i32))
            }
        }
    })
}
async fn finish_client(
    lease: Lease,
    result: io::Result<i32>,
    signals: &mut Signals,
) -> io::Result<i32> {
    let leave = lease.leave().await;
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            let _ = forward_diagnostics(format!("pinfold: command: {error}").into_bytes(), signals)
                .await;
            125
        }
    };
    if let Err(error) = leave {
        let _ = forward_diagnostics(format!("pinfold: leave: {error}").into_bytes(), signals).await;
        return Ok(if code == 0 { 125 } else { code });
    }
    Ok(code)
}

pub(crate) fn owner(args: &[String]) -> io::Result<i32> {
    let descriptors: Vec<i32> = args
        .iter()
        .map(|arg| arg.parse().map_err(|_| protocol()))
        .collect::<io::Result<_>>()?;
    if descriptors.len() != 4
        || descriptors.iter().any(|fd| *fd < 3)
        || descriptors
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != 4
    {
        return Err(protocol());
    }
    // Rearm before constructing a runtime or spawning any worker/helper.
    for fd in &descriptors {
        if unsafe { libc::fcntl(*fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(protocol());
        }
    }
    if unsafe { libc::getsid(0) != libc::getpid() } {
        return Err(protocol());
    }
    let mut files = descriptors
        .into_iter()
        .map(|fd| unsafe { File::from_raw_fd(fd) });
    use std::os::fd::IntoRawFd;
    let socket = unsafe {
        std::os::unix::net::UnixStream::from_raw_fd(files.next().expect("socket").into_raw_fd())
    };
    wire::peer(&socket)?;
    socket.set_nonblocking(true)?;
    let launch = files.next().expect("launch");
    let executable = files.next().expect("executable");
    let maintenance = files.next().expect("maintenance");
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?.block_on(async move {
        let mut signals = Signals::with_hangup()?;
        let mut stream = UnixStream::from_std(socket)?;
        let Request::Bootstrap { root, intent, snapshot } = wire::read_request(&mut stream, false).await? else { return Err(protocol()) };
        if state::canonical(&root)? != root { return Err(protocol()); }
        let own_executable = crate::update::working()?;
        let own_maintenance = ownership::project_lock(&state::project_id(&root))?;
        same_file(&executable, &own_executable)?;
        same_file(&maintenance, &own_maintenance)?;
        same_file(&launch, &ownership::open_lock(&launch_key(&root))?)?;
        if unsafe { libc::flock(launch.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 { return Err(protocol()); }
        drop((executable, maintenance));
        let (events, mut received) = mpsc::channel(64);
        let (mut reader, mut writer) = stream.into_split();
        let event_sender = events.clone();
        let initial = tokio::spawn(async move {
            let request = wire::read_request(&mut reader, true).await;
            let leave = matches!(request, Ok(Request::Leave {}));
            let bad = match &request { Ok(_) => !leave, Err(error) => error.kind() != io::ErrorKind::ConnectionAborted };
            let _ = event_sender.send(Event::Depart { id: 0, leave, bad }).await;
        });
        let setup_cancel = Arc::new(AtomicBool::new(false));
        let setup_flag = setup_cancel.clone();
        let setup_root = root.clone();
        let mut resolving = tokio::task::spawn_blocking(move || {
            crate::core::process::with_cancellation(setup_flag, || Snapshot::resolve(&setup_root))
        });
        let setup = tokio::select! {
            biased;
            _ = received.recv() => { setup_cancel.store(true, Ordering::Release); let _ = resolving.await; Err(io::Error::from(io::ErrorKind::Interrupted)) },
            _ = signals.recv() => { setup_cancel.store(true, Ordering::Release); let _ = resolving.await; Err(io::Error::from(io::ErrorKind::Interrupted)) },
            resolved = &mut resolving => resolved.map_err(|_| io::Error::other("configuration worker failed")).and_then(|r| r),
        };
        let resolved = match setup {
            Ok(resolved) => resolved,
            Err(error) => { initial.abort(); let _ = initial.await; return Err(error); },
        };
        let fields = snapshot.differences(&resolved.snapshot);
        if !fields.is_empty() {
            initial.abort(); let _ = initial.await;
            wire::write_reply(&mut writer, Reply::Error { reason: "config-changed".into(), fields }).await?;
            return Ok(125);
        }
        let mut plan = resolved.plan;
        let mut random = [0u8; 6];
        use ring::rand::SecureRandom;
        ring::rand::SystemRandom::new().fill(&mut random).map_err(|_| io::Error::other("owner name generation failed"))?;
        plan.name = format!("pi-{}-{}", state::project_id(&root), crate::core::hex(&random));
        let mut locator = Locator { version: 1, root: root.clone(), name: plan.name.clone(), generation: None };
        locator.publish()?;
        state::record_run(&root)?;
        let (cancel, cancelled) = watch::channel(false);
        let init = artifacts::init()?;
        let image_id = resolved.snapshot.image_id().ok_or_else(unavailable)?.to_string();
        let started = {
        let startup = Box::up_project(&plan, resolved.config.profile, &image_id, &init, &mut signals, cancelled);
        tokio::pin!(startup);
        tokio::select! {
            biased;
            _ = received.recv() => { let _ = cancel.send(true); startup.await },
            result = &mut startup => result,
        }
        };
        let mut box_ = match started {
            Ok(box_) => box_,
            Err(error) => {
                initial.abort(); let _ = initial.await;
                // The locator is bootstrap evidence only; failed core cleanup
                // keeps the durable claim for checked recovery.
                if ownership::record(&plan.name)?.is_none() { locator.remove()?; }
                let reason = "owner-unavailable";
                let _ = wire::write_reply(&mut writer, Reply::Error { reason: reason.into(), fields: vec![] }).await;
                return Err(error.into());
            }
        };
        let result = async {
        locator.generation = Some(box_.run_id.clone());
        locator.publish()?;
        let sync_cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = sync_cancel.clone();
        let (name, sync_root, sync_init, repository) = (plan.name.clone(), root.clone(), init.clone(), resolved.config.repository);
        let mut syncing = tokio::task::spawn_blocking(move || {
            if repository { launch::sync(&name, &sync_init, &sync_root, &worker_cancel); }
        });
        let stopped = tokio::select! {
            biased;
            _ = received.recv() => Some(Shutdown::Signal),
            reason = box_.ended() => Some(reason?),
            _ = signals.recv() => Some(Shutdown::Signal),
            result = &mut syncing => { result.map_err(|_| io::Error::other("project synchronization worker failed"))?; None },
        };
        if let Some(reason) = stopped {
            sync_cancel.store(true, Ordering::SeqCst);
            let _ = syncing.await;
            let down = box_.down(Some(reason.terminal())).await;
            if down.is_ok() { locator.remove()?; }
            down?;
            return Ok(125);
        }
        if received.try_recv().is_ok() {
            box_.down(Some(Shutdown::Signal.terminal())).await?;
            locator.remove()?;
            return Ok(125);
        }
        let listener = crate::core::process::Listener::from_std(sockets::Sockets::open(&box_.run_id)?.bind_control()?)?;
        let mut leases = BTreeMap::new();
        let keep = matches!(intent, Intent::Keep);
        // Commit before acknowledgement, including when that acknowledgement is lost.
        let reply = if keep { Reply::Kept { name: plan.name.clone(), generation: box_.run_id.clone() } }
            else { Reply::Admitted { name: plan.name.clone(), generation: box_.run_id.clone() } };
        redirect_stderr()?;
        let acknowledged = if keep {
            wire::write_reply(&mut writer, reply).await
        } else {
            leases.insert(0, writer);
            wire::write_reply(leases.get_mut(&0).expect("initial lease"), reply).await
        };
        drop(launch);
        if !keep && acknowledged.is_err() { leases.remove(&0); }
        serve(&mut box_, &locator, &root, resolved.snapshot, listener, events, &mut received, leases, keep, &mut signals).await
        }.await;
        initial.abort(); let _ = initial.await;
        if result.is_err() {
            let down = box_.down(None).await;
            if down.is_ok() { let _ = locator.remove(); }
            if let Err(error) = down { eprintln!("pinfold: owner cleanup: {}", terminal_text(&error.to_string())); }
        }
        drop((own_executable, own_maintenance));
        result
    })
}
fn same_file(a: &File, b: &File) -> io::Result<()> {
    let (a, b) = (a.metadata()?, b.metadata()?);
    if a.dev() != b.dev() || a.ino() != b.ino() {
        return Err(protocol());
    }
    Ok(())
}
fn redirect_stderr() -> io::Result<()> {
    let null = File::options().write(true).open("/dev/null")?;
    if unsafe { libc::dup2(null.as_raw_fd(), 2) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

enum Event {
    Request {
        stream: UnixStream,
        request: std::boxed::Box<Request>,
    },
    Depart {
        id: u64,
        leave: bool,
        bad: bool,
    },
}

#[allow(clippy::too_many_arguments)]
async fn serve(
    box_: &mut Box,
    locator: &Locator,
    root: &Path,
    snapshot: Snapshot,
    listener: crate::core::process::Listener,
    events: mpsc::Sender<Event>,
    received: &mut mpsc::Receiver<Event>,
    mut leases: BTreeMap<u64, OwnedWriteHalf>,
    mut keep: bool,
    signals: &mut Signals,
) -> io::Result<i32> {
    let generation = box_.run_id.clone();
    let mut tasks = tokio::task::JoinSet::new();
    let mut next = 1u64;
    let mut final_leave = None;
    let terminal = loop {
        if leases.is_empty() && !keep {
            break runs::Terminal {
                reason: "sessions-ended",
                detail: None,
            };
        }
        tokio::select! {
            biased;
            reason = box_.ended() => break reason?.terminal(),
            _ = signals.recv() => break Shutdown::Signal.terminal(),
            Some(event) = received.recv() => match event {
                Event::Depart { id, leave, bad } => {
                    if let Some(mut writer) = leases.remove(&id) {
                        if bad { let _ = wire::write_reply(&mut writer, Reply::Error { reason: "session-protocol".into(), fields: vec![] }).await; }
                        if leave {
                            if leases.is_empty() && !keep { final_leave = Some(writer); }
                            else { let _ = wire::write_reply(&mut writer, Reply::Left {}).await; }
                        }
                    }
                }
                Event::Request { mut stream, request } => {
                    let request = *request;
                    let (candidate_root, candidate_generation) = match &request {
                        Request::Probe { root, generation, .. } | Request::Join { root, generation, .. }
                        | Request::Keep { root, generation, .. } | Request::Borrow { root, generation, .. } => (root, generation),
                        _ => { let _ = wire::write_reply(&mut stream, Reply::Error { reason: "session-protocol".into(), fields: vec![] }).await; continue; },
                    };
                    if candidate_root != root || candidate_generation != &generation {
                        let _ = wire::write_reply(&mut stream, Reply::Error { reason: "session-protocol".into(), fields: vec![] }).await; continue;
                    }
                    let fields = match &request {
                        Request::Join { snapshot: candidate, .. } | Request::Keep { snapshot: candidate, .. } => snapshot.differences(candidate),
                        Request::Probe { roots, .. } | Request::Borrow { roots, .. } => snapshot.roots.differences(roots),
                        _ => unreachable!(),
                    };
                    if !fields.is_empty() { let _ = wire::write_reply(&mut stream, Reply::Error { reason: "config-changed".into(), fields }).await; continue; }
                    match request {
                        Request::Probe { .. } => { let _ = wire::write_reply(&mut stream, Reply::Compatible {}).await; },
                        Request::Keep { .. } => { keep = true; let _ = wire::write_reply(&mut stream, Reply::Kept { name: locator.name.clone(), generation: generation.clone() }).await; },
                        Request::Join { .. } | Request::Borrow { .. } => {
                            let id = next; next += 1;
                            let (mut reader, writer) = stream.into_split();
                            leases.insert(id, writer);
                            let reply = wire::write_reply(leases.get_mut(&id).expect("registered lease"), Reply::Admitted { name: locator.name.clone(), generation: generation.clone() }).await;
                            if reply.is_err() { leases.remove(&id); continue; }
                            let events = events.clone();
                            tasks.spawn(async move {
                                let request = wire::read_request(&mut reader, true).await;
                                let leave = matches!(request, Ok(Request::Leave {}));
                                let bad = match &request { Ok(_) => !leave, Err(error) => error.kind() != io::ErrorKind::ConnectionAborted };
                                let _ = events.send(Event::Depart { id, leave, bad }).await;
                            });
                        }
                        _ => unreachable!(),
                    }
                }
            },
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                wire::peer(&stream)?;
                let events = events.clone();
                tasks.spawn(async move {
                    let mut stream = stream;
                    match wire::read_request(&mut stream, false).await {
                        Ok(request) => { let _ = events.send(Event::Request { stream, request: std::boxed::Box::new(request) }).await; },
                        Err(_) => { let _ = wire::write_reply(&mut stream, Reply::Error { reason: "session-protocol".into(), fields: vec![] }).await; },
                    }
                });
            },
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
        }
    };
    let (drain_done, mut draining) = watch::channel(false);
    let drain_generation = generation.clone();
    let drain_root = root.to_path_buf();
    let drain = tokio::spawn(async move {
        let mut parsers = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = draining.changed() => break,
                accepted = listener.accept() => {
                    let Ok((mut stream, _)) = accepted else { break };
                    if wire::peer(&stream).is_err() { continue; }
                    let (generation, root) = (drain_generation.clone(), drain_root.clone());
                    parsers.spawn(async move {
                        let correct = match wire::read_request(&mut stream, false).await {
                            Ok(Request::Probe { root:r, generation:g, .. } | Request::Join { root:r, generation:g, .. }
                                | Request::Keep { root:r, generation:g, .. } | Request::Borrow { root:r, generation:g, .. }) => r == root && g == generation,
                            _ => false,
                        };
                        let reply = if correct { Reply::Draining { generation } }
                            else { Reply::Error { reason: "session-protocol".into(), fields: vec![] } };
                        let _ = wire::write_reply(&mut stream, reply).await;
                    });
                }
                Some(_) = parsers.join_next(), if !parsers.is_empty() => {},
            }
        }
        parsers.abort_all();
        while parsers.join_next().await.is_some() {}
    });
    // Admission is now closed, before any resource teardown.
    let reason = terminal.reason.to_string();
    let teardown = box_.down(Some(terminal)).await;
    let teardown = teardown.and_then(|()| locator.remove());
    if let Some(mut writer) = final_leave {
        let reply = if teardown.is_ok() {
            Reply::Left {}
        } else {
            Reply::Ended {
                generation: generation.clone(),
                reason: reason.clone(),
                teardown_ok: false,
            }
        };
        let _ = wire::write_reply(&mut writer, reply).await;
    }
    for (_, mut writer) in leases {
        let _ = wire::write_reply(
            &mut writer,
            Reply::Ended {
                generation: generation.clone(),
                reason: reason.clone(),
                teardown_ok: teardown.is_ok(),
            },
        )
        .await;
    }
    let _ = drain_done.send(true);
    let _ = drain.await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    while let Ok(event) = received.try_recv() {
        if let Event::Request { mut stream, .. } = event {
            let _ = wire::write_reply(
                &mut stream,
                Reply::Draining {
                    generation: generation.clone(),
                },
            )
            .await;
        }
    }
    teardown.map(|()| 0)
}

pub(crate) fn stop(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(cli::usage("stop", "stop takes no arguments"));
    }
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    executor.block_on(async {
        let mut signals = Signals::with_hangup()?;
        let _executable = crate::update::working()?;
        let result = async {
            let root = Context::discover(&std::env::current_dir()?)?.root;
            let _maintenance = ownership::project_lock(&state::project_id(&root))?;
            let deadline = Instant::now() + CONTROL;
            let mut target: Option<Locator> = None;
            loop {
                if target.is_none() {
                    target = read_locator(&root)?;
                }
                if let Some(locator) = &target {
                    if let Some(record) = ownership::record(&locator.name)? {
                        if locator
                            .generation
                            .as_ref()
                            .is_some_and(|g| g != &record.generation)
                        {
                            return Ok(0);
                        }
                        let root = root.clone();
                        stop_work(&mut signals, deadline, move || {
                            stop_observed(&root, &record, deadline.into_std())
                        })
                        .await?;
                        return Ok(0);
                    }
                    if locator.generation.is_some() {
                        // A completed observation remains the target across replacement.
                        wait_name(&locator.name, &mut signals, deadline).await?;
                        let Some(lock) = ownership::try_name_lock(&locator.name)? else {
                            continue;
                        };
                        if ownership::record(&locator.name)?.is_some() {
                            continue;
                        }
                        let locator = locator.clone();
                        stop_work(&mut signals, deadline, move || {
                            clean::remove_orphan(
                                runtime(),
                                &locator.name,
                                locator.generation.as_deref(),
                                &lock,
                            )?;
                            remove_matching_locator(&locator)
                        })
                        .await?;
                        return Ok(0);
                    }
                }
                if let Some(_launch) = try_launch(&root)? {
                    if let Some(locator) = &target {
                        if ownership::record(&locator.name)?.is_some() {
                            continue;
                        }
                        let Some(lock) = ownership::try_name_lock(&locator.name)? else {
                            return Err(unavailable());
                        };
                        let locator = locator.clone();
                        stop_work(&mut signals, deadline, move || {
                            clean::remove_orphan(runtime(), &locator.name, None, &lock)?;
                            remove_matching_locator(&locator)
                        })
                        .await?;
                        return Ok(0);
                    }
                    let discovery_root = root.clone();
                    match stop_work(&mut signals, deadline, move || discover(&discovery_root))
                        .await?
                    {
                        Some(locator) => {
                            target = Some(locator);
                            continue;
                        }
                        None => return Ok(0),
                    }
                }
                wait_tick(&mut signals, deadline).await?;
            }
        }
        .await;
        match signals.last() {
            Some(signal) => Ok(128 + signal as i32),
            None => result,
        }
    })
}

async fn stop_work<T: Send + 'static>(
    signals: &mut Signals,
    deadline: Instant,
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        crate::core::process::with_cancellation(worker_cancel, work)
    });
    tokio::select! {
        biased;
        result = &mut worker => result.map_err(|_| io::Error::other("project stop worker failed"))?,
        _ = signals.recv() => {
            cancel.store(true, Ordering::Release);
            let _ = worker.await;
            Err(io::ErrorKind::Interrupted.into())
        }
        _ = tokio::time::sleep_until(deadline) => {
            cancel.store(true, Ordering::Release);
            let _ = worker.await;
            Err(timed_out())
        }
    }
}
fn remove_matching_locator(locator: &Locator) -> io::Result<()> {
    if let Some(current) = read_locator(&locator.root)?
        && current.name == locator.name
        && current.generation == locator.generation
    {
        current.remove()?;
    }
    Ok(())
}
fn retired_or_removing(locator: &Locator) -> io::Result<bool> {
    Ok(match ownership::record(&locator.name)? {
        Some(record) => Some(&record.generation) == locator.generation.as_ref() && record.retiring,
        None => true,
    })
}
async fn wait_tick(signals: &mut Signals, deadline: Instant) -> io::Result<()> {
    tokio::select! {
        _ = signals.recv() => Err(io::ErrorKind::Interrupted.into()),
        _ = tokio::time::sleep_until((Instant::now() + Duration::from_millis(20)).min(deadline)) => {
            if Instant::now() >= deadline { Err(timed_out()) } else { Ok(()) }
        },
    }
}
async fn wait_name(name: &str, signals: &mut Signals, deadline: Instant) -> io::Result<()> {
    while ownership::name_alive(name)? {
        wait_tick(signals, deadline).await?;
    }
    Ok(())
}
fn stop_observed(
    root: &Path,
    observed: &ownership::Record,
    deadline: std::time::Instant,
) -> io::Result<()> {
    let current = ownership::record(&observed.name)?;
    if current
        .as_ref()
        .is_some_and(|r| r.generation != observed.generation)
    {
        return Ok(());
    }
    let answered = if current.is_none() {
        false
    } else {
        ownership::request_stop_until(&observed.name, Some(observed), deadline)?
    };
    let after = ownership::record(&observed.name)?;
    if after
        .as_ref()
        .is_some_and(|r| r.generation != observed.generation)
    {
        return Ok(());
    }
    let acknowledged = answered || after.is_none();
    if acknowledged {
        while ownership::name_alive(&observed.name)? {
            if std::time::Instant::now() >= deadline {
                return Err(timed_out());
            }
            nix::poll::poll(&mut [], 10u16)?;
        }
    }
    let Some(lock) = ownership::try_name_lock(&observed.name)? else {
        return Err(unavailable());
    };
    match ownership::record(&observed.name)? {
        Some(record) if record.generation == observed.generation => {
            clean::remove_orphan(runtime(), &record.name, Some(&record.generation), &lock)?;
            if ownership::record(&record.name)?.is_some() {
                return Err(unavailable());
            }
        }
        _ => {}
    }
    if let Some(locator) = read_locator(root)?
        && locator.name == observed.name
        && locator
            .generation
            .as_deref()
            .is_none_or(|g| g == observed.generation)
    {
        locator.remove()?;
    }
    Ok(())
}

/// Project crossing borrows a project-ready owner. A transient crossing
/// instead retains launch ownership through its own complete teardown.
pub(crate) struct CrossingLease {
    _maintenance: File,
    launch: Option<File>,
    lease: Option<Lease>,
}
impl CrossingLease {
    pub(crate) fn name(&self) -> Option<&str> {
        self.lease.as_ref().map(|lease| lease.name.as_str())
    }
    pub(crate) async fn ended(&mut self) -> io::Result<()> {
        match self.lease.as_mut() {
            Some(lease) => lease.ended().await,
            None => std::future::pending().await,
        }
    }
    pub(crate) async fn leave(self) -> io::Result<()> {
        if let Some(lease) = self.lease {
            lease.leave().await?;
        }
        Ok(())
    }
    pub(crate) fn release_launch(&mut self) {
        self.launch.take();
    }
}
pub(crate) async fn borrow(root: &Path, signals: &mut Signals) -> io::Result<CrossingLease> {
    let maintenance = close_only(ownership::project_lock(&state::project_id(root))?);
    let deadline = Instant::now() + CONTROL;
    loop {
        let launch = launch_lock(root, signals, deadline).await?;
        let discovered = match discover(root) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_tick(signals, deadline).await?;
                continue;
            }
            Err(error) => return Err(error),
        };
        let lease = if let Some(locator) = discovered {
            match probe(root, &locator).await {
                Ok(true) => {}
                Ok(false) => {
                    wait_name(&locator.name, signals, deadline).await?;
                    continue;
                }
                Err(_error) if retired_or_removing(&locator)? => {
                    wait_name(&locator.name, signals, deadline).await?;
                    continue;
                }
                Err(error) => return Err(error),
            }
            let generation = locator.generation.clone().ok_or_else(unavailable)?;
            let (stream, reply) = match exchange(
                &locator,
                Request::Borrow {
                    root: root.into(),
                    generation,
                    roots: Roots::resolve()?,
                },
            )
            .await
            {
                Ok(value) => value,
                Err(_error) if retired_or_removing(&locator)? => {
                    wait_name(&locator.name, signals, deadline).await?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            match reply {
                Reply::Admitted { name, generation }
                    if name == locator.name && Some(&generation) == locator.generation.as_ref() =>
                {
                    Some(Lease::new(stream, name, generation, None))
                }
                Reply::Draining { generation }
                    if Some(&generation) == locator.generation.as_ref() =>
                {
                    wait_name(&locator.name, signals, deadline).await?;
                    continue;
                }
                other => return Err(reply_error(other)),
            }
        } else {
            None
        };
        return Ok(CrossingLease {
            _maintenance: maintenance,
            launch: Some(launch),
            lease,
        });
    }
}
