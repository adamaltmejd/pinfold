//! Host-only locks and generation-targeted box shutdown.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream as BlockingStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::core::pipe::remaining;
use crate::core::runtime::sockets::{self, Sockets};
use crate::core::{absent_ok, sha256_hex};

pub const GENERATION_LABEL: &str = "dev.pinfold.generation";
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

pub fn host_dir() -> io::Result<PathBuf> {
    // TMPDIR, HOME and XDG variables do not define the runtime's owner.
    Ok(fs::canonicalize("/tmp")?.join(format!("pinfold-{}", nix::unistd::getuid())))
}

fn root() -> io::Result<PathBuf> {
    sockets::host().map(|(path, _)| path)
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
    root()?;
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
fn try_flock(file: File, mode: FlockArg) -> io::Result<Option<Flock<File>>> {
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
    let metadata = match fs::symlink_metadata(host_dir()?) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    check_private(&metadata, metadata.is_dir(), "directory")?;
    let file = match File::options()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path(&format!("box:{name}"), "lock")?)
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
    /// Runtime removal was confirmed before any generation tree was deleted.
    #[serde(default)]
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

pub fn record(name: &str) -> io::Result<Option<Record>> {
    match fs::read(path(&format!("box:{name}"), "owner")?) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
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

/// Name ownership precedes state creation. Publication follows the runtime
/// name check, so a refused claim preserves an orphan's record.
pub struct Claim {
    name: String,
    _lock: Flock<File>,
    record: Option<Record>,
    listener: Option<UnixListener>,
    sockets: Option<Sockets>,
    request: Option<UnixStream>,
    running: bool,
}

impl Claim {
    pub async fn take(name: &str) -> io::Result<Option<Claim>> {
        // Liveness probes and orphan pruning hold the same lock briefly.
        // A conflict proves only contention; never trust a pid to bypass it.
        let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
        let lock = loop {
            if let Some(lock) = try_name_lock(name)? {
                break lock;
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        Ok(Some(Claim {
            name: name.to_string(),
            _lock: lock,
            record: None,
            listener: None,
            sockets: None,
            request: None,
            running: false,
        }))
    }

    pub fn prepare(&mut self, state_dir: &Path) -> io::Result<()> {
        if let Some(old) = record(&self.name)? {
            #[cfg(target_os = "linux")]
            crate::core::runtime::runtime().down(&self.name)?;
            if let Err(error) = remove_record(&self.name, &old.generation) {
                if record(&self.name)?.is_some() {
                    return Err(error);
                }
                eprintln!("pinfold: socket cleanup: {error}");
            }
        }
        absent_ok(fs::remove_dir_all(state_dir))?;
        let mut random = [0; 16];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        self.record = Some(Record {
            name: self.name.clone(),
            generation: sha256_hex(random),
            retiring: false,
            state_dir: state_dir.to_path_buf(),
            disks: Vec::new(),
            described: None,
        });
        fs::create_dir_all(state_dir)?;
        let endpoint = socket(&self.name)?;
        sockets::remove_socket(&endpoint)?;
        let (_, host) = sockets::host()?;
        sockets::check_length(&endpoint)?;
        let listener =
            sockets::bind_at(&host, endpoint.file_name().expect("shutdown socket name"))?;
        listener.set_nonblocking(true)?;
        self.listener = Some(UnixListener::from_std(listener)?);
        #[cfg(target_os = "linux")]
        crate::core::runtime::firecracker::prepare_vmm(state_dir)?;
        self.publish()?;
        self.sockets = Some(Sockets::create(self.generation())?);
        Ok(())
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

    pub async fn stop_requested(&mut self) -> io::Result<()> {
        loop {
            let (mut stream, _) = self
                .listener
                .as_ref()
                .expect("prepared ownership")
                .accept()
                .await?;
            let mut generation = [0; 64];
            if !matches!(
                tokio::time::timeout(REQUEST_TIMEOUT, stream.read_exact(&mut generation)).await,
                Ok(Ok(_))
            ) {
                continue;
            }
            if generation == self.generation().as_bytes() {
                self.request = Some(stream);
                return Ok(());
            }
            let _ = tokio::time::timeout(REQUEST_TIMEOUT, stream.write_all(b"gone\n")).await;
        }
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
        self.listener.take();
        if let Some(mut request) = self.request.take() {
            let answer: &[u8] = if completed { b"done\n" } else { b"failed\n" };
            let _ = tokio::time::timeout(REQUEST_TIMEOUT, request.write_all(answer)).await;
        }
        cleanup
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
pub fn request_stop(name: &str, observed: Option<&Record>) -> io::Result<bool> {
    // An unpublished claim is absent to this request. Adopting a record
    // later could target a replacement after an old owner removed its record.
    let Some(observed) = observed else {
        return Ok(false);
    };
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        if let Some(lock) = try_name_lock(name)? {
            drop(lock);
            return Ok(false);
        }
        if record(name)?.is_none_or(|current| current.generation != observed.generation) {
            return Ok(false);
        }
        match BlockingStream::connect(socket(name)?) {
            Ok(mut stream) => {
                let exchange: io::Result<Vec<u8>> = (|| {
                    stream.set_write_timeout(Some(remaining(deadline, None, "box teardown")?))?;
                    stream.write_all(observed.generation.as_bytes())?;
                    // The owner closes after one reply line, at most "failed\n".
                    stream.set_read_timeout(Some(remaining(deadline, None, "box teardown")?))?;
                    let mut answer = Vec::new();
                    (&mut stream).take(7).read_to_end(&mut answer)?;
                    if !answer.ends_with(b"\n") {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                    Ok(answer)
                })();
                match exchange {
                    Ok(answer) => {
                        return match &answer[..] {
                            b"done\n" => Ok(true),
                            b"gone\n" => Ok(false),
                            b"failed\n" => Err(io::Error::other("box teardown failed")),
                            _ => Err(io::Error::other("invalid box teardown reply")),
                        };
                    }
                    // Startup can finish while its select is reading this
                    // request. Reconnect only to the observed generation.
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::UnexpectedEof
                                | io::ErrorKind::ConnectionReset
                                | io::ErrorKind::BrokenPipe
                        ) => {}
                    Err(error) => {
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
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "box owner did not complete teardown",
    ))
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
        let decoded = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            // A record removed since the listing belongs to a box that ended.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => Err(error),
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
