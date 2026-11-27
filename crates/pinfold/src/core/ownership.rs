//! Host-only locks and generation-targeted box shutdown.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::core::process::Listener as UnixListener;

use crate::core::pipe::remaining;
use crate::core::runtime::sockets::{self, Sockets};
use crate::core::{absent_ok, sha256_hex};

pub const GENERATION_LABEL: &str = "dev.pinfold.generation";
pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

pub fn host_dir() -> io::Result<PathBuf> {
    // TMPDIR, HOME and XDG variables do not define the runtime's owner.
    Ok(fs::canonicalize("/tmp")?.join(format!("pinfold-{}", nix::unistd::getuid())))
}

/// Refuse a host ownership `what` of the wrong kind, another user's, or
/// open to group or others.
fn check_private(metadata: &fs::Metadata, kind: bool, what: &str) -> io::Result<()> {
    if kind && metadata.uid() == nix::unistd::getuid().as_raw() && metadata.mode() & 0o077 == 0 {
        return Ok(());
    }
    Err(io::Error::other(format!("unsafe host ownership {what}")))
}

fn path(key: &str, suffix: &str) -> io::Result<PathBuf> {
    // Even with macOS's /private/tmp prefix this stays below 104 bytes.
    Ok(host_dir()?.join(format!("{}.{suffix}", sha256_hex(key))))
}

/// Return an unlocked stable file. Callers may impose their own deadline.
pub fn open_lock(key: &str) -> io::Result<File> {
    sockets::host()?;
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path(key, "lock")?)?;
    let metadata = file.metadata()?;
    check_private(&metadata, metadata.is_file(), "lock")?;
    Ok(file)
}

/// The lock, or `None` while another holds it.
pub(crate) fn try_flock(file: File, mode: FlockArg) -> io::Result<Option<Flock<File>>> {
    match Flock::lock(file, mode) {
        Ok(lock) => Ok(Some(lock)),
        Err((_, nix::errno::Errno::EAGAIN)) => Ok(None),
        Err((_, error)) => Err(error.into()),
    }
}

fn try_lock(key: &str, mode: FlockArg) -> io::Result<Option<Flock<File>>> {
    try_flock(open_lock(key)?, mode)
}

pub fn try_name_lock(name: &str) -> io::Result<Option<Flock<File>>> {
    try_lock(&format!("box:{name}"), FlockArg::LockExclusiveNonblock)
}

/// Observe an existing lock without creating any host state.
pub fn name_alive(name: &str) -> io::Result<bool> {
    lock_alive(&format!("box:{name}"))
}

pub(crate) fn lock_alive(key: &str) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(host_dir()?) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    check_private(&metadata, metadata.is_dir(), "directory")?;
    let file = match File::options()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path(key, "lock")?)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    check_private(&metadata, metadata.is_file(), "lock")?;
    Ok(try_flock(file, FlockArg::LockExclusiveNonblock)?.is_none())
}

pub fn project_lock(id: &str) -> io::Result<Flock<File>> {
    Flock::lock(open_lock(&format!("project:{id}"))?, FlockArg::LockShared)
        .map_err(|(_, error)| error.into())
}

pub fn try_project_lock(id: &str) -> io::Result<Option<Flock<File>>> {
    try_lock(&format!("project:{id}"), FlockArg::LockExclusiveNonblock)
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Record {
    pub name: String,
    pub generation: String,
    pub ready: bool,
    /// Runtime removal was confirmed before any generation tree was deleted.
    pub retiring: bool,
    pub state_dir: PathBuf,
    /// The disks this generation attaches. They are released once the
    /// owner is dead and the runtime lists the box stopped or not at all,
    /// and `prune` removes the record.
    pub disks: Vec<Attachment>,
    /// The box as `list` reports it, once `up` knows it. Linux's inventory.
    pub described: Option<Described>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Described {
    pub labels: std::collections::BTreeMap<String, String>,
    pub image_id: String,
    pub image_ref: String,
    /// RFC 3339, UTC.
    pub created: String,
    /// The spec's `memory`.
    pub memory: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Attachment {
    pub name: String,
    pub readonly: bool,
}

/// Hold a disk's lock while deciding and recording its attachments. A
/// disk belongs to one state root, and so does its lock.
pub fn disk_lock(name: &str) -> io::Result<Flock<File>> {
    let key = format!("disk:{}:{name}", crate::dirs::state_root_key()?);
    Flock::lock(open_lock(&key)?, FlockArg::LockExclusive).map_err(|(_, error)| error.into())
}

pub(crate) fn try_disk_lock(name: &str) -> io::Result<Option<Flock<File>>> {
    let key = format!("disk:{}:{name}", crate::dirs::state_root_key()?);
    try_lock(&key, FlockArg::LockExclusiveNonblock)
}

pub fn record(name: &str) -> io::Result<Option<Record>> {
    read_owner(&path(&format!("box:{name}"), "owner")?)
}

/// An owner file's record; `None` when there is no such file.
fn read_owner(path: &Path) -> io::Result<Option<Record>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Try `attempt` every 10 ms until it takes its lock; `None` past `deadline`.
async fn poll_lock(
    deadline: tokio::time::Instant,
    attempt: impl Fn() -> io::Result<Option<Flock<File>>>,
) -> io::Result<Option<Flock<File>>> {
    loop {
        if let Some(lock) = attempt()? {
            return Ok(Some(lock));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Hold public dispatch against name reuse; startup publishes ready last.
pub(crate) fn exec_lock(name: &str, generation: &str) -> io::Result<Option<Flock<File>>> {
    let ready = |record: Option<Record>| {
        record.is_some_and(|record| {
            record.generation == generation && record.ready && !record.retiring
        })
    };
    if !ready(record(name)?) {
        return Ok(None);
    }
    let Some(lock) = try_lock(&format!("exec:{name}"), FlockArg::LockSharedNonblock)? else {
        return Ok(None);
    };
    if !ready(record(name)?) {
        return Ok(None);
    }
    Ok(Some(lock))
}

pub fn socket(name: &str) -> io::Result<PathBuf> {
    path(&format!("box:{name}"), "sock")
}

fn publish(record: &Record) -> io::Result<()> {
    crate::core::write_atomic(
        &path(&format!("box:{}", record.name), "owner")?,
        &serde_json::to_vec(record)?,
    )?;
    File::open(host_dir()?)?.sync_all()
}

fn mark_retiring(record: &mut Record) -> io::Result<()> {
    record.retiring = true;
    publish(record)
}

/// Authorize orphan mutations while the caller holds this name's lock.
#[cfg(target_os = "linux")]
pub fn retire_orphan(name: &str, _lock: &Flock<File>) -> io::Result<Option<Record>> {
    let Some(mut record) = record(name)? else {
        return Ok(None);
    };
    crate::core::runtime::runtime().down(name)?;
    mark_retiring(&mut record)?;
    Ok(Some(record))
}

/// Remove an orphan's host files while the caller owns its name lock.
pub fn remove_record(name: &str, generation: &str) -> io::Result<()> {
    if let Some(mut record) = record(name)?
        && record.generation == generation
    {
        if !record.retiring {
            #[cfg(target_os = "linux")]
            crate::core::runtime::runtime().down(name)?;
        }
        mark_retiring(&mut record)?;
        let cleanup = Sockets::remove(generation);
        absent_ok(fs::remove_dir_all(&record.state_dir))?;
        sockets::remove_socket(&socket(name)?)?;
        absent_ok(fs::remove_file(path(&format!("box:{name}"), "owner")?))?;
        File::open(host_dir()?)?.sync_all()?;
        cleanup?;
    }
    Ok(())
}

/// Name ownership precedes state creation. Public startup checks the runtime
/// name before publication; fresh project startup refuses prior state.
pub struct Claim {
    name: String,
    _lock: Flock<File>,
    startup_gate: Option<Flock<File>>,
    record: Option<Record>,
    sockets: Option<Sockets>,
    control: Option<StartupControl>,
    running: bool,
}

impl Claim {
    pub async fn take(name: &str) -> io::Result<Option<Claim>> {
        // Liveness probes and orphan pruning hold the same lock briefly.
        // A conflict proves only contention; never trust a pid to bypass it.
        let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
        let Some(lock) = poll_lock(deadline, || try_name_lock(name)).await? else {
            return Ok(None);
        };
        let gate = || try_lock(&format!("exec:{name}"), FlockArg::LockExclusiveNonblock);
        let Some(startup_gate) = poll_lock(deadline, gate).await? else {
            return Ok(None);
        };
        Ok(Some(Claim {
            name: name.to_string(),
            _lock: lock,
            startup_gate: Some(startup_gate),
            record: None,
            sockets: None,
            control: None,
            running: false,
        }))
    }

    /// With `fresh`: a random project name never authorizes replacing prior
    /// evidence.
    pub(crate) fn prepare(&mut self, state_dir: &Path, fresh: bool) -> io::Result<()> {
        let endpoint = socket(&self.name)?;
        // The caller refused a recorded name while holding this claim.
        if fresh {
            for path in [state_dir, endpoint.as_path()] {
                match fs::symlink_metadata(path) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                    Ok(_) => return Err(io::Error::other("fresh project name has existing state")),
                }
            }
        } else if let Some(old) = record(&self.name)? {
            #[cfg(target_os = "linux")]
            crate::core::runtime::runtime().down(&self.name)?;
            if let Err(error) = remove_record(&self.name, &old.generation) {
                if record(&self.name)?.is_some() {
                    return Err(error);
                }
                eprintln!("pinfold: socket cleanup: {error}");
            }
        }
        if !fresh {
            absent_ok(fs::remove_dir_all(state_dir))?;
        }
        let generation = super::random_hex()?;
        self.record = Some(Record {
            name: self.name.clone(),
            generation,
            ready: false,
            retiring: false,
            state_dir: state_dir.to_path_buf(),
            disks: Vec::new(),
            described: None,
        });
        if fresh {
            if let Some(parent) = state_dir.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::create_dir(state_dir)?;
        } else {
            fs::create_dir_all(state_dir)?;
            sockets::remove_socket(&endpoint)?;
        }
        let (_, host) = sockets::host()?;
        sockets::check_length(&endpoint)?;
        let listener =
            sockets::bind_at(&host, endpoint.file_name().expect("shutdown socket name"))?;
        self.control = Some(StartupControl {
            listener: UnixListener::from_std(listener)?,
            request: None,
            reading: None,
            generation: self.generation().to_string(),
        });
        self.publish()?;
        self.startup_gate.take();
        self.sockets = Some(Sockets::create(self.generation())?);
        Ok(())
    }

    /// Admit public work only after every startup operation completed.
    pub fn ready(&mut self) -> io::Result<()> {
        self.record.as_mut().expect("prepared ownership").ready = true;
        self.publish()
    }

    /// Record the disks this generation attaches. The caller holds each
    /// disk's lock.
    pub fn attach(&mut self, disks: Vec<Attachment>) -> io::Result<()> {
        self.record.as_mut().expect("prepared ownership").disks = disks;
        self.publish()
    }

    /// Record the box this generation starts.
    pub fn describe(&mut self, described: Described) -> io::Result<()> {
        self.record.as_mut().expect("prepared ownership").described = Some(described);
        self.publish()
    }

    fn publish(&self) -> io::Result<()> {
        publish(self.record.as_ref().unwrap())
    }

    /// The caller has removed and reaped this claim's runtime and workers.
    pub fn retire(&mut self) -> io::Result<()> {
        let Some(held) = self.record.as_mut() else {
            return Ok(());
        };
        let mut current = record(&self.name)?
            .filter(|current| current.generation == held.generation)
            .ok_or_else(|| io::Error::other("the retiring claim no longer matches"))?;
        mark_retiring(&mut current)?;
        held.retiring = true;
        Ok(())
    }

    /// Builder recovery must authorize mutations before restoring old state.
    #[cfg(target_os = "linux")]
    pub fn retire_previous(&mut self) -> io::Result<()> {
        let old = record(&self.name)?;
        if self.record.as_ref().is_some_and(|held| {
            old.as_ref()
                .is_none_or(|old| old.generation != held.generation)
        }) {
            return Err(io::Error::other("the retiring claim no longer matches"));
        }
        if let Some(mut old) = old {
            crate::core::runtime::runtime().down(&self.name)?;
            mark_retiring(&mut old)?;
            if let Some(held) = self.record.as_mut()
                && held.generation == old.generation
            {
                held.retiring = true;
            }
        }
        Ok(())
    }

    pub fn sockets(&self) -> &Sockets {
        self.sockets.as_ref().expect("prepared sockets")
    }

    pub fn generation(&self) -> &str {
        &self.record.as_ref().expect("prepared ownership").generation
    }

    pub(crate) fn take_startup_control(&mut self) -> StartupControl {
        self.control.take().expect("prepared ownership")
    }

    pub(crate) fn restore_startup_control(&mut self, control: StartupControl) {
        self.control = Some(control);
    }

    pub async fn stop_requested(&mut self) -> io::Result<()> {
        self.control
            .as_mut()
            .expect("prepared ownership")
            .stop_requested()
            .await
    }

    pub fn starting(&mut self) {
        self.running = true;
    }

    #[cfg(not(target_os = "linux"))]
    pub fn preparation_done(&mut self) {
        self.running = false;
    }

    /// Acknowledge only completed teardown. Ownership stays held until Drop.
    pub async fn finish(&mut self, success: bool) -> io::Result<()> {
        if success {
            self.retire()?;
        }
        let cleanup = match (success, self.record.as_ref()) {
            (true, Some(record)) => remove_record(&self.name, &record.generation),
            _ => Ok(()),
        };
        let completed = success && cleanup.is_ok();
        if completed {
            self.record = None;
        }
        let request = self.control.take().and_then(|control| control.request);
        if !completed && let Some(record) = self.record.as_ref() {
            crate::core::runs::write_teardown_failure(&record.generation);
        }
        if let Some(mut request) = request {
            let answer: &[u8] = if completed { b"done\n" } else { b"failed\n" };
            let _ = tokio::time::timeout(REQUEST_TIMEOUT, request.write_all(answer)).await;
        }
        cleanup
    }
}

/// Shutdown stays responsive while disk preparation owns the claim on a worker.
pub(crate) struct StartupControl {
    listener: UnixListener,
    request: Option<UnixStream>,
    reading: Option<Reading>,
    generation: String,
}

impl StartupControl {
    /// Cancel safe: a request read in part continues on the next call.
    pub(crate) async fn stop_requested(&mut self) -> io::Result<()> {
        self.request =
            Some(receive_stop(&self.listener, &self.generation, &mut self.reading).await?);
        Ok(())
    }
}

/// A stop connection and the request bytes read from it so far.
/// `acknowledged` counts the bytes of `stopping\n` written once the request
/// matched.
struct Reading {
    stream: UnixStream,
    bytes: Vec<u8>,
    deadline: tokio::time::Instant,
    acknowledged: Option<usize>,
}

const STOPPING: &[u8] = b"stopping\n";

/// Every await is cancel safe: the accepted connection, the bytes read and
/// the acknowledgement written live in `reading`, so a dropped wait loses
/// no request.
async fn receive_stop(
    listener: &UnixListener,
    generation: &str,
    reading: &mut Option<Reading>,
) -> io::Result<UnixStream> {
    loop {
        let current = match reading {
            Some(current) => current,
            None => {
                let (stream, _) = listener.accept().await?;
                reading.insert(Reading {
                    stream,
                    bytes: Vec::with_capacity(65),
                    deadline: tokio::time::Instant::now() + REQUEST_TIMEOUT,
                    acknowledged: None,
                })
            }
        };
        if let Some(written) = current.acknowledged {
            if written < STOPPING.len()
                && let Ok(Ok(sent @ 1..)) = tokio::time::timeout_at(
                    current.deadline,
                    current.stream.write(&STOPPING[written..]),
                )
                .await
            {
                current.acknowledged = Some(written + sent);
                continue;
            }
            // Written, or failed within the bound: stopping proceeds either way.
            return Ok(reading.take().expect("matched request").stream);
        }
        // The private project prefix asks for acceptance before the final
        // reply. Public box down keeps its existing 64-byte request.
        let project = current.bytes.first() == Some(&b'P');
        let wanted = if project { 65 } else { 64 };
        if current.bytes.len() < wanted {
            let mut chunk = [0; 65];
            let free = wanted - current.bytes.len();
            match tokio::time::timeout_at(current.deadline, current.stream.read(&mut chunk[..free]))
                .await
            {
                Ok(Ok(read)) if read > 0 => current.bytes.extend_from_slice(&chunk[..read]),
                _ => *reading = None,
            }
            continue;
        }
        if &current.bytes[usize::from(project)..] == generation.as_bytes() {
            if project {
                current.acknowledged = Some(0);
                current.deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
                continue;
            }
            return Ok(reading.take().expect("matched request").stream);
        }
        let mut stream = reading.take().expect("complete request").stream;
        let _ = tokio::time::timeout(REQUEST_TIMEOUT, stream.write_all(b"gone\n")).await;
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if !self.running
            && let Some(record) = self.record.as_ref()
        {
            let _ = remove_record(&self.name, &record.generation);
        }
        if self.record.is_some() {
            let _ = socket(&self.name).and_then(|socket| sockets::remove_socket(&socket));
        }
    }
}

/// Ask only the generation observed before connecting. A replacement's
/// listener refuses the old token instead of stopping its own box.
/// Return true only when the owner acknowledges completed teardown.
/// `project` is a project stop's cancel and acceptance flags.
pub(crate) fn request_stop(
    name: &str,
    observed: Option<&Record>,
    deadline: Instant,
    project: Option<(&AtomicBool, &AtomicBool)>,
) -> io::Result<bool> {
    // An unpublished claim is absent to this request. Adopting a record
    // later could target a replacement after an old owner removed its record.
    let Some(observed) = observed else {
        return Ok(false);
    };
    let cancel = project.map(|(cancel, _)| cancel);
    let accepted = || project.is_some_and(|(_, accepted)| accepted.load(Ordering::Acquire));
    loop {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if try_name_lock(name)?.is_some() {
            return Ok(false);
        }
        if record(name)?.is_none_or(|current| current.generation != observed.generation) {
            return Ok(false);
        }
        let connected = sockets::connect_shutdown(&socket(name)?, deadline);
        match connected {
            Ok(mut stream) => {
                let exchange: io::Result<bool> = (|| {
                    stream.set_nonblocking(true)?;
                    let mut request = Vec::with_capacity(65);
                    if project.is_some() {
                        request.push(b'P');
                    }
                    request.extend(observed.generation.as_bytes());
                    crate::core::pipe::write_all(
                        &mut stream,
                        &request,
                        deadline,
                        cancel,
                        "box teardown",
                    )?;
                    loop {
                        let answer = stop_reply(&mut stream, deadline, cancel, accepted())?;
                        match answer.as_slice() {
                            b"stopping\n" if project.is_some() && !accepted() => {
                                project
                                    .expect("project stop")
                                    .1
                                    .store(true, Ordering::Release);
                            }
                            b"done\n" => return Ok(true),
                            b"gone\n" => return Ok(false),
                            b"failed\n" => return Err(io::Error::other("box teardown failed")),
                            _ => return Err(io::Error::other("invalid box teardown reply")),
                        }
                    }
                })();
                match exchange {
                    Ok(answer) => return Ok(answer),
                    // Startup can finish while its select is reading this
                    // request. Reconnect only before acceptance, and only to
                    // the generation already observed.
                    Err(error)
                        if !accepted()
                            && matches!(
                                error.kind(),
                                io::ErrorKind::UnexpectedEof
                                    | io::ErrorKind::ConnectionReset
                                    | io::ErrorKind::BrokenPipe
                            ) => {}
                    Err(error) => {
                        // Removal evidence does not erase an acknowledged cleanup failure.
                        if accepted() {
                            return Err(error);
                        }
                        if record(name)?
                            .is_none_or(|current| current.generation != observed.generation)
                        {
                            return Ok(false);
                        }
                        return Err(error);
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                        | io::ErrorKind::TimedOut
                ) || error.raw_os_error() == Some(nix::libc::EINPROGRESS) => {}
            Err(error) => return Err(error),
        }
        remaining(deadline, cancel, "box teardown")?;
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn stop_reply(
    stream: &mut std::os::unix::net::UnixStream,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
    accepted: bool,
) -> io::Result<Vec<u8>> {
    let mut answer = Vec::with_capacity(9);
    let mut byte = [0];
    while answer.len() < 9 {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if !accepted {
            remaining(deadline, cancel, "box teardown")?;
        }
        match stream.read(&mut byte) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(_) => {
                answer.push(byte[0]);
                if byte[0] == b'\n' {
                    return Ok(answer);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let until = if accepted {
                    Instant::now() + Duration::from_millis(100)
                } else {
                    deadline
                };
                match crate::core::pipe::wait_ready(
                    Some(stream.as_fd()),
                    nix::poll::PollFlags::POLLIN,
                    until,
                    cancel,
                    "box teardown",
                ) {
                    Err(error) if accepted && error.kind() == io::ErrorKind::TimedOut => {}
                    result => result?,
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("invalid box teardown reply"))
}

/// Published generations across every Pinfold state root. Consumers that
/// decide ownership fail closed when any record cannot be read.
pub fn records() -> io::Result<Vec<Record>> {
    record_results()?.into_iter().collect()
}

/// Maintenance can report a bad record and still examine independent claims.
pub fn record_results() -> io::Result<Vec<io::Result<Record>>> {
    let mut records = Vec::new();
    for entry in crate::dirs::entries(&host_dir()?)? {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != "owner")
        {
            continue;
        }
        // A record removed since the listing belongs to a box that ended.
        let Some(decoded) = read_owner(&path).transpose() else {
            continue;
        };
        records.push(decoded.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("ownership record {:?}: {error}", path.file_name()),
            )
        }));
    }
    Ok(records)
}
