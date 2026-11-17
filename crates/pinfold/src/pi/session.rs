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
use crate::core::runtime::{ExecContext, runtime, sockets};
use crate::core::{artifacts, clean, ownership, runs};
use crate::formatting::terminal_text;
use crate::pi::context::Context;
use crate::pi::session_snapshot::{Resolved, Roots, Snapshot};
use crate::pi::session_wire::{self as wire, Intent, Reply, Request};
use crate::pi::{launch, state};

const CONTROL: Duration = Duration::from_secs(10);

fn unavailable_at(source: &str) -> io::Error {
    io::Error::other(format!("owner-unavailable: {source}"))
}
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
        Reply::Error { reason, .. } if reason == "stopped" => {
            io::Error::other("stopped during startup")
        }
        Reply::Ended {
            generation,
            reason,
            teardown_ok,
        } => io::Error::other(format!(
            "{} ({generation}); pinfold box inspect {generation}{}",
            terminal_text(&reason),
            if teardown_ok { "" } else { "; teardown-failed" }
        )),
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
async fn maintenance(root: &Path, signals: &mut Signals) -> io::Result<File> {
    let deadline = Instant::now() + CONTROL;
    loop {
        match Flock::lock(
            ownership::open_lock(&format!("project:{}", state::project_id(root)))?,
            FlockArg::LockSharedNonblock,
        ) {
            Ok(lock) => return Ok(close_only(lock)),
            Err((_, nix::errno::Errno::EAGAIN)) => wait_tick(signals, deadline).await?,
            Err((_, error)) => return Err(error.into()),
        }
    }
}
async fn launch_lock(root: &Path, signals: &mut Signals, noticed: &mut bool) -> io::Result<File> {
    loop {
        if let Some(lock) = try_launch(root)? {
            return Ok(lock);
        }
        if !*noticed {
            *noticed = true;
            forward_diagnostics(
                b"pinfold: waiting for the project box to start".to_vec(),
                signals,
            )
            .await?;
        }
        tokio::select! {
            _ = signals.recv() => return Err(io::ErrorKind::Interrupted.into()),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {},
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
            Err(error) => crate::core::process::diagnostic(terminal_text(&error.to_string())),
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
            if let Some(actual) = runtime()
                .inspect(&record.name)
                .map_err(|_| unavailable_at("inspect"))?
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

async fn connect(
    locator: &Locator,
    signals: &mut Signals,
    deadline: Instant,
) -> io::Result<UnixStream> {
    let generation = locator.generation.clone().ok_or_else(unavailable)?;
    let socket = control_work(signals, deadline, move || {
        let socket = sockets::Sockets::open(&generation)?
            .connect_control_until(deadline.into_std())
            .map_err(|_| unavailable_at("connect"))?;
        wire::peer(&socket)?;
        socket.set_nonblocking(true)?;
        Ok(socket)
    })
    .await?;
    UnixStream::from_std(socket)
}
async fn exchange(
    locator: &Locator,
    request: Request,
    signals: &mut Signals,
    deadline: Instant,
) -> io::Result<(UnixStream, Reply)> {
    let mut stream = connect(locator, signals, deadline).await?;
    wire::write_request(&mut stream, request).await?;
    let reply = wire::read_reply(&mut stream, false)
        .await
        .map_err(|_| unavailable_at("reply"))?;
    Ok((stream, reply))
}
async fn probe(
    root: &Path,
    locator: &Locator,
    signals: &mut Signals,
    deadline: Instant,
) -> io::Result<bool> {
    let (_, reply) = exchange(
        locator,
        Request::Probe {
            root: root.into(),
            generation: locator.generation.clone().ok_or_else(unavailable)?,
            roots: Roots::resolve()?,
        },
        signals,
        deadline,
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
    reason: Option<String>,
    draining: bool,
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
            reason: None,
            draining: false,
        }
    }
    fn remember(&mut self, reply: Option<io::Result<Reply>>, leaving: bool) {
        self.ended = Some(match reply {
            Some(Ok(Reply::Ended {
                generation,
                reason,
                teardown_ok,
            })) if generation == self.generation => {
                self.reason = Some(reason);
                if teardown_ok {
                    Ok(())
                } else {
                    Err(io::Error::other("teardown-failed"))
                }
            }
            Some(Ok(Reply::Left {})) if leaving => Ok(()),
            Some(Ok(other)) => Err(reply_error(other)),
            Some(Err(error)) => {
                self.reason = Some(error.to_string());
                Err(error)
            }
            None => {
                self.reason = Some("owner-unavailable".into());
                Err(unavailable())
            }
        });
    }
    async fn report_end(&mut self, signals: &mut Signals) -> io::Result<()> {
        if let Some(reason) = self.reason.take() {
            let message = format!(
                "pinfold: {} ({}); pinfold box inspect {}",
                terminal_text(&reason),
                self.generation,
                self.generation
            );
            forward_diagnostics(message.into_bytes(), signals).await?;
        }
        Ok(())
    }
    fn end_result(&self) -> io::Result<()> {
        match self.ended.as_ref().expect("observed lease departure") {
            Ok(()) => Ok(()),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
    async fn ended(&mut self) -> io::Result<()> {
        while self.ended.is_none() {
            let reply = self.replies.recv().await;
            if matches!(&reply, Some(Ok(Reply::Draining { generation })) if generation == &self.generation)
            {
                self.draining = true;
                return Ok(());
            } else {
                self.remember(reply, false);
            }
        }
        self.end_result()
    }
    async fn leave(mut self, signals: &mut Signals) -> io::Result<()> {
        let deadline = Instant::now() + CONTROL;
        let mut acknowledged = self.draining;
        if self.ended.is_none() {
            let _ = wire::write_request(&mut self.writer, Request::Leave {}).await;
            loop {
                let reply = tokio::select! {
                    biased;
                    reply = self.replies.recv() => reply,
                    _ = signals.recv() => break,
                    _ = tokio::time::sleep_until(deadline), if !acknowledged => {
                        self.ended = Some(Err(timed_out())); break;
                    },
                };
                if matches!(&reply, Some(Ok(Reply::Draining { generation })) if generation == &self.generation)
                {
                    acknowledged = true;
                    continue;
                }
                self.remember(reply, true);
                break;
            }
        }
        let reported = self.report_end(signals).await;
        self.reader.abort();
        let _ = (&mut self.reader).await;
        if let Some(child) = &mut self.child {
            let _ = child.try_wait();
        }
        if signals.last().is_some() && self.ended.is_none() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        self.end_result().and(reported)
    }
}

async fn admit(
    root: &Path,
    intent: Intent,
    signals: &mut Signals,
    executable: &File,
) -> io::Result<Option<Lease>> {
    let _maintenance = maintenance(root, signals).await?;
    let mut noticed = false;
    loop {
        let launch = launch_lock(root, signals, &mut noticed).await?;
        let deadline = Instant::now() + CONTROL;
        let discovery_root = root.to_path_buf();
        let discovered =
            match control_work(signals, deadline, move || discover(&discovery_root)).await {
                Ok(value) => value,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait_tick(signals, deadline).await?;
                    continue;
                }
                Err(error) => return Err(error),
            };
        if let Some(locator) = discovered {
            match probe(root, &locator, signals, deadline).await {
                Ok(true) => {}
                Ok(false) => {
                    wait_name_owned(&locator.name, signals).await?;
                    continue;
                }
                Err(_error) if retired_or_removing(&locator)? => {
                    wait_name_owned(&locator.name, signals).await?;
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
            let (stream, reply) = match exchange(&locator, request, signals, deadline).await {
                Ok(value) => value,
                Err(_error) if retired_or_removing(&locator)? => {
                    wait_name_owned(&locator.name, signals).await?;
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
                    wait_name_owned(&locator.name, signals).await?;
                    drop(launch);
                    continue;
                }
                other => return Err(reply_error(other)),
            }
        }
        let resolved = resolve_spec(root, true, signals).await?;
        let mut child = spawn_owner(&resolved.snapshot, &launch, executable, &_maintenance)?;
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
        let mut draining = None;
        let reply = loop {
            let reply = tokio::select! {
                biased;
                reply = async {
                    if draining.is_some() {
                        wire::read_reply(&mut stream, true).await
                    } else {
                        timeout_at(Instant::now() + CONTROL, wire::read_reply(&mut stream, true))
                            .await.map_err(|_| timed_out()).and_then(|reply| reply)
                    }
                } => reply,
                _ = signals.recv() => break Err(io::ErrorKind::Interrupted.into()),
            };
            match reply {
                Ok(Reply::Starting {}) => continue,
                Ok(Reply::Draining { generation })
                    if valid_generation(&generation)
                        && draining
                            .as_ref()
                            .is_none_or(|observed| observed == &generation) =>
                {
                    draining = Some(generation);
                }
                Ok(Reply::Ended { ref generation, .. })
                    if draining
                        .as_ref()
                        .is_some_and(|observed| observed != generation) =>
                {
                    break Err(protocol());
                }
                reply => break reply,
            }
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

struct Diagnostics {
    stop: watch::Sender<bool>,
    worker: tokio::task::JoinHandle<io::Result<Vec<u8>>>,
}

fn spawn_owner(
    snapshot: &Snapshot,
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
    let guard = crate::core::process::fork_guard();
    let inherited = crate::core::process::open_descriptors(&guard)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("__project-owner")
        .args(descriptors.map(|fd| fd.to_string()))
        .env_clear()
        .envs(snapshot.owner_environment())
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            for &fd in inherited.iter().filter(|fd| **fd >= 3) {
                // Mark unrelated inherited descriptors close-on-exec; keep the
                // stdlib spawn-error pipe alive until exec itself succeeds.
                if libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) < 0
                    && io::Error::last_os_error().raw_os_error() != Some(libc::EBADF)
                {
                    return Err(io::Error::last_os_error());
                }
            }
            for fd in descriptors {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    drop(guard);
    drop((owner, command));
    let stderr = child.stderr.take().ok_or_else(unavailable)?;
    use std::os::fd::IntoRawFd;
    let stderr = unsafe { File::from_raw_fd(stderr.into_raw_fd()) };
    // ChildStderr is a pipe, not a UnixStream; AsyncFd reads the retained fd.
    crate::core::pipe::nonblocking(&stderr, "owner diagnostics")?;
    let (stop, mut stopping) = watch::channel(false);
    let worker = tokio::spawn(async move {
        let mut kept = Vec::new();
        // SAFETY: the owned pipe keeps the registered descriptor stable.
        let reader = unsafe { tokio::io::unix::AsyncFd::register(stderr) }?;
        loop {
            let mut ready = tokio::select! {
                ready = reader.readable() => ready?,
                _ = stopping.changed() => {
                    use std::io::Read;
                    let mut bytes = [0u8; 8192];
                    for _ in 0..8 {
                        match reader.get_ref().read(&mut bytes) {
                            Ok(0) => break,
                            Ok(n) => kept.extend_from_slice(&bytes[..n.min(65536usize.saturating_sub(kept.len()))]),
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                            Err(error) => return Err(error),
                        }
                    }
                    break;
                },
            };
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
    Ok((client, child, Diagnostics { stop, worker }))
}

async fn bootstrap_diagnostics(drain: &mut Diagnostics, signals: &mut Signals) -> io::Result<()> {
    let _ = drain.stop.send(true);
    let bytes = (&mut drain.worker)
        .await
        .map_err(|_| unavailable_at("diagnostics"))??;
    forward_diagnostics(bytes, signals).await
}

pub(crate) async fn forward_diagnostics(bytes: Vec<u8>, signals: &mut Signals) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut escaped = terminal_text(&String::from_utf8_lossy(&bytes)).into_bytes();
    escaped.push(b'\n');
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        let mut stderr =
            crate::core::process::StdioRelay::new(2, false).map_err(|_| output_failure())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        crate::core::pipe::write_all(
            &mut stderr,
            &escaped,
            deadline,
            Some(&worker_cancel),
            "owner diagnostics",
        )
        .map_err(|_| output_failure())?;
        stderr.close();
        while !stderr.finished().map_err(|_| output_failure())? {
            if worker_cancel.load(Ordering::Acquire) || std::time::Instant::now() >= deadline {
                return Err(output_failure());
            }
            nix::poll::poll(&mut [], 10u16).map_err(|_| output_failure())?;
        }
        Ok(())
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
        let (result, exec_finished, completed) = tokio::select! {
            biased;
            result = &mut exec => {
                let result = result.map_err(|_| io::Error::other("command execution worker failed")).and_then(|r| r).map(cli::exit_code);
                let completed = result.as_ref().ok().copied();
                (result, true, completed)
            },
            _ = lease.ended() => (Ok(125), false, None),
            signal = signals.recv() => (Ok(128 + signal as i32), false, None),
        };
        cancel.store(true, Ordering::SeqCst);
        crate::core::runtime::terminal::cancel();
        if !exec_finished {
            // The transport itself can already have observed native completion.
            if let Ok(Ok(status)) = exec.await {
                let code = cli::exit_code(status);
                return finish_client(lease, Ok(code), Some(code), &mut signals).await;
            }
        }
        finish_client(lease, result, completed, &mut signals).await
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
    completed: Option<i32>,
    signals: &mut Signals,
) -> io::Result<i32> {
    let leave = lease.leave(signals).await;
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            let _ = forward_diagnostics(format!("pinfold: command: {error}").into_bytes(), signals)
                .await;
            125
        }
    };
    match leave {
        Err(error) => {
            if error.kind() != io::ErrorKind::Interrupted || signals.last().is_none() {
                let _ =
                    forward_diagnostics(format!("pinfold: leave: {error}").into_bytes(), signals)
                        .await;
            }
        }
        Ok(()) if signals.last().is_none() => clean::maintain(),
        Ok(()) => {}
    }
    Ok(completed.unwrap_or_else(|| signals.last().map_or(code, |signal| 128 + signal as i32)))
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
        let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
        let mut launch = Some(launch);
        let mut owned_box = None;
        let mut owned_name = None;
        let result = async {
        let setup_cancel = Arc::new(AtomicBool::new(false));
        let setup_flag = setup_cancel.clone();
        let setup_root = root.clone();
        let mut resolving = tokio::task::spawn_blocking(move || {
            crate::core::process::with_cancellation(setup_flag, || Snapshot::resolve(&setup_root))
        });
        let setup = loop { tokio::select! {
            biased;
            _ = received.recv() => { setup_cancel.store(true, Ordering::Release); let _ = resolving.await; break Err(io::Error::from(io::ErrorKind::Interrupted)) },
            _ = signals.recv() => { setup_cancel.store(true, Ordering::Release); let _ = resolving.await; break Err(io::Error::from(io::ErrorKind::Interrupted)) },
            _ = heartbeat.tick() => { let _ = wire::write_reply(&mut writer, Reply::Starting {}).await; },
            resolved = &mut resolving => break resolved.map_err(|_| io::Error::other("configuration worker failed")).and_then(|r| r),
        }};
        let resolved = match setup {
            Ok(resolved) => resolved,
            Err(error) => return Err(error),
        };
        let fields = snapshot.differences(&resolved.snapshot);
        if !fields.is_empty() {
            wire::write_reply(&mut writer, Reply::Error { reason: "config-changed".into(), fields }).await?;
            return Ok(125);
        }
        let plan = resolved.plan;
        let mut locator = Locator { version: 1, root: root.clone(), name: plan.name.clone(), generation: None };
        owned_name = Some(locator.name.clone());
        locator.publish()?;
        state::record_run(&root)?;
        let (cancel, cancelled) = watch::channel(false);
        let init = artifacts::init_path()?;
        let image_id = resolved.snapshot.image_id().ok_or_else(unavailable)?.to_string();
        let started = {
        let startup = Box::up_project(&plan, resolved.config.profile, &image_id, &mut signals, cancelled);
        tokio::pin!(startup);
        let mut disconnected = false;
        loop { tokio::select! {
            biased;
            _ = received.recv(), if !disconnected => { let _ = cancel.send(true); disconnected = true; },
            _ = heartbeat.tick() => { let _ = wire::write_reply(&mut writer, Reply::Starting {}).await; },
            result = &mut startup => break result,
        }}
        };
        let box_ = match started {
            Ok(box_) => box_,
            Err(error) => {
                    // The locator is bootstrap evidence only; failed core cleanup
                // keeps the durable claim for checked recovery.
                if ownership::record(&plan.name)?.is_none() { locator.remove()?; }
                let reason = if matches!(error, crate::core::r#box::UpError::Control | crate::core::r#box::UpError::Signal) { "stopped" } else { "owner-unavailable" };
                let _ = wire::write_reply(&mut writer, Reply::Error { reason: reason.into(), fields: vec![] }).await;
                return Err(error.into());
            }
        };
        let box_ = owned_box.insert(box_);
        locator.generation = Some(box_.run_id.clone());
        locator.publish()?;
        let sync_cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = sync_cancel.clone();
        let (name, sync_root, sync_init, repository) = (plan.name.clone(), root.clone(), init.clone(), resolved.config.repository);
        let mut syncing = tokio::task::spawn_blocking(move || {
            if repository { launch::sync(&name, &sync_init, &sync_root, &worker_cancel); }
        });
        let stopped = loop { tokio::select! {
            biased;
            _ = received.recv() => break Some(Ok(Shutdown::Signal)),
            reason = box_.ended() => break Some(reason),
            _ = signals.recv() => break Some(Ok(Shutdown::Signal)),
            _ = heartbeat.tick() => { let _ = wire::write_reply(&mut writer, Reply::Starting {}).await; },
            result = &mut syncing => { result.map_err(|_| io::Error::other("project synchronization worker failed"))?; break None },
        }};
        let sync_pending = stopped.is_some();
        let stopped = match stopped {
            Some(reason) => Some(reason),
            None => {
                let checking = timeout_at(Instant::now() + CONTROL, box_.ended_if_ready());
                tokio::pin!(checking);
                loop { tokio::select! {
                    biased;
                    _ = received.recv() => break Some(Ok(Shutdown::Signal)),
                    _ = signals.recv() => break Some(Ok(Shutdown::Signal)),
                    _ = heartbeat.tick() => { let _ = wire::write_reply(&mut writer, Reply::Starting {}).await; },
                    result = &mut checking => break result.map_err(|_| timed_out()).and_then(|result| result).transpose(),
                }}
            }
        };
        if let Some(reason) = stopped {
            let _ = wire::write_reply(&mut writer, Reply::Draining { generation: box_.run_id.clone() }).await;
            sync_cancel.store(true, Ordering::SeqCst);
            if sync_pending { let _ = syncing.await; }
            let reason_name = reason.as_ref().map_or("owner-unavailable", |reason| {
                if *reason == Shutdown::Signal { "stopped" } else { reason.as_str() }
            });
            let down = box_.down(reason.as_ref().ok().map(Shutdown::terminal)).await
                .and_then(|()| locator.remove());
            if down.is_err() { runs::write_teardown_failure(&box_.run_id); }
            let reason_name = if box_.audit_failed() { "audit-log" } else { reason_name };
            let _ = wire::write_reply(&mut writer, Reply::Ended {
                generation: box_.run_id.clone(), reason: reason_name.into(), teardown_ok: down.is_ok(),
            }).await;
            reason?;
            down?;
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
        drop(launch.take());
        if !keep && acknowledged.is_err() { leases.remove(&0); }
        serve(box_, &locator, &root, resolved.snapshot, listener, events, &mut received, leases, keep, &mut signals).await
        }.await;
        initial.abort(); let _ = initial.await;
        if result.is_err() {
            let down = match owned_box.as_mut() { Some(box_) => box_.down(None).await, None => Ok(()) };
            if down.is_ok() && let Some(name) = owned_name.as_ref()
                && ownership::record(name)?.is_none()
                && let Some(locator) = read_locator(&root)?
                && locator.name == *name { let _ = locator.remove(); }
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
            break Ok(runs::Terminal {
                reason: "sessions-ended",
                detail: None,
            });
        }
        tokio::select! {
            biased;
            reason = box_.ended() => break reason.map(|reason| reason.terminal()),
            _ = signals.recv() => break Ok(Shutdown::Signal.terminal()),
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
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(error) => { if let Err(error) = report_accept(error) { break Err(error); } tokio::task::yield_now().await; continue; },
                };
                if let Err(error) = wire::peer(&stream) {
                    eprintln!("pinfold: session peer: {}", terminal_text(&error.to_string()));
                    continue;
                }
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
    let (_, placeholder) = mpsc::channel(1);
    let mut draining_received = std::mem::replace(received, placeholder);
    let drain = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = draining.changed() => break,
                Some(event) = draining_received.recv() => {
                    draining_reply(event, &drain_root, &drain_generation).await;
                },
                accepted = listener.accept() => {
                    let stream = match accepted {
                        Ok((stream, _)) => stream,
                        Err(error) => { if report_accept(error).is_err() { break; } continue; },
                    };
                    if wire::peer(&stream).is_err() { continue; }
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
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        while let Ok(event) = draining_received.try_recv() {
            draining_reply(event, &drain_root, &drain_generation).await;
        }
    });
    let finish = async {
        if let Some(writer) = final_leave.as_mut() {
            let _ = wire::write_reply(
                writer,
                Reply::Draining {
                    generation: generation.clone(),
                },
            )
            .await;
        }
        for writer in leases.values_mut() {
            let _ = wire::write_reply(
                writer,
                Reply::Draining {
                    generation: generation.clone(),
                },
            )
            .await;
        }
        // Admission is now closed, before any resource teardown.
        let mut reason = terminal
            .as_ref()
            .map_or("owner-unavailable", |terminal| terminal.reason)
            .to_string();
        let (terminal, serving_error) = match terminal {
            Ok(terminal) => (Some(terminal), None),
            Err(error) => (None, Some(error)),
        };
        let teardown = box_.down(terminal).await;
        if box_.audit_failed() {
            reason = "audit-log".into();
        }
        let teardown = teardown.and_then(|()| locator.remove());
        if teardown.is_err() {
            runs::write_teardown_failure(&generation);
        }
        if let Some(mut writer) = final_leave {
            let reply = if teardown.is_ok() && reason == "sessions-ended" {
                Reply::Left {}
            } else {
                Reply::Ended {
                    generation: generation.clone(),
                    reason: reason.clone(),
                    teardown_ok: teardown.is_ok(),
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
        teardown.and_then(|()| serving_error.map_or(Ok(0), Err))
    };
    let result = finish.await;
    let _ = drain.await;
    result
}

fn report_accept(error: io::Error) -> io::Result<()> {
    eprintln!(
        "pinfold: session accept: {}",
        terminal_text(&error.to_string())
    );
    if matches!(
        error.raw_os_error(),
        Some(libc::EBADF | libc::EINVAL | libc::ENOTSOCK)
    ) {
        Err(error)
    } else {
        Ok(())
    }
}

async fn draining_reply(event: Event, root: &Path, generation: &str) {
    let Event::Request {
        mut stream,
        request,
    } = event
    else {
        return;
    };
    let correct = match *request {
        Request::Probe {
            root: r,
            generation: g,
            ..
        }
        | Request::Join {
            root: r,
            generation: g,
            ..
        }
        | Request::Keep {
            root: r,
            generation: g,
            ..
        }
        | Request::Borrow {
            root: r,
            generation: g,
            ..
        } => r == root && g == generation,
        _ => false,
    };
    let reply = if correct {
        Reply::Draining {
            generation: generation.into(),
        }
    } else {
        Reply::Error {
            reason: "session-protocol".into(),
            fields: vec![],
        }
    };
    let _ = wire::write_reply(&mut stream, reply).await;
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
            let _maintenance = maintenance(&root, &mut signals).await?;
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
                        stop_target(root.clone(), record, &mut signals, deadline).await?;
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
                        control_work(&mut signals, deadline, move || {
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
                        control_work(&mut signals, deadline, move || {
                            clean::remove_orphan(runtime(), &locator.name, None, &lock)?;
                            remove_matching_locator(&locator)
                        })
                        .await?;
                        return Ok(0);
                    }
                    let discovery_root = root.clone();
                    match control_work(&mut signals, deadline, move || discover(&discovery_root))
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

async fn stop_target(
    root: PathBuf,
    record: ownership::Record,
    signals: &mut Signals,
    deadline: Instant,
) -> io::Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    let accepted = Arc::new(AtomicBool::new(false));
    let (worker_cancel, worker_accepted) = (cancel.clone(), accepted.clone());
    let mut worker = tokio::task::spawn_blocking(move || {
        crate::core::process::with_cancellation(worker_cancel.clone(), || {
            stop_observed(
                &root,
                &record,
                deadline.into_std(),
                &worker_cancel,
                &worker_accepted,
            )
        })
    });
    loop {
        tokio::select! {
            biased;
            result = &mut worker => {
                let result = result.map_err(|_| io::Error::other("project control worker failed"))?;
                return result.map_err(|error| if accepted.load(Ordering::Acquire) { io::Error::new(error.kind(), format!("teardown-failed: {error}")) } else { error });
            },
            _ = signals.recv() => { cancel.store(true, Ordering::Release); let _ = worker.await; return Err(io::ErrorKind::Interrupted.into()); },
            _ = tokio::time::sleep_until(deadline), if !accepted.load(Ordering::Acquire) => {
                if accepted.load(Ordering::Acquire) { continue; }
                cancel.store(true, Ordering::Release); let _ = worker.await; return Err(timed_out());
            },
        }
    }
}

async fn control_work<T: Send + 'static>(
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
        result = &mut worker => result.map_err(|_| io::Error::other("project control worker failed"))?,
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
async fn wait_name_owned(name: &str, signals: &mut Signals) -> io::Result<()> {
    while ownership::name_alive(name)? {
        tokio::select! {
            _ = signals.recv() => return Err(io::ErrorKind::Interrupted.into()),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {},
        }
    }
    Ok(())
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
    cancel: &AtomicBool,
    accepted: &AtomicBool,
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
        ownership::request_stop_project(&observed.name, observed, deadline, cancel, accepted)?
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
            if cancel.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            if !accepted.load(Ordering::Acquire) && std::time::Instant::now() >= deadline {
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
    pub(crate) async fn leave(self, signals: &mut Signals) -> io::Result<()> {
        if let Some(lease) = self.lease {
            lease.leave(signals).await?;
        }
        Ok(())
    }
    pub(crate) fn release_launch(&mut self) {
        self.launch.take();
    }
}
pub(crate) async fn borrow(root: &Path, signals: &mut Signals) -> io::Result<CrossingLease> {
    let maintenance = maintenance(root, signals).await?;
    let mut noticed = false;
    loop {
        let launch = launch_lock(root, signals, &mut noticed).await?;
        let deadline = Instant::now() + CONTROL;
        let discovery_root = root.to_path_buf();
        let discovered =
            match control_work(signals, deadline, move || discover(&discovery_root)).await {
                Ok(value) => value,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait_tick(signals, deadline).await?;
                    continue;
                }
                Err(error) => return Err(error),
            };
        let lease = if let Some(locator) = discovered {
            let generation = locator.generation.clone().ok_or_else(unavailable)?;
            let (stream, reply) = match exchange(
                &locator,
                Request::Borrow {
                    root: root.into(),
                    generation,
                    roots: Roots::resolve()?,
                },
                signals,
                deadline,
            )
            .await
            {
                Ok(value) => value,
                Err(_error) if retired_or_removing(&locator)? => {
                    wait_name_owned(&locator.name, signals).await?;
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
                    wait_name_owned(&locator.name, signals).await?;
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
