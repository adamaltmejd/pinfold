//! Host-only locks and generation-targeted box shutdown.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream as BlockingStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::core::sha256_hex;

pub const GENERATION_LABEL: &str = "dev.pinfold.generation";
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

pub fn host_dir() -> io::Result<PathBuf> {
    // TMPDIR, HOME and XDG variables do not define the runtime's owner.
    Ok(fs::canonicalize("/tmp")?.join(format!("pinfold-{}", nix::unistd::getuid())))
}

fn root() -> io::Result<PathBuf> {
    let path = host_dir()?;
    match fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other("unsafe host ownership directory"));
    }
    Ok(path)
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
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other("unsafe host ownership lock"));
    }
    Ok(file)
}

fn try_lock(key: &str, mode: FlockArg) -> io::Result<Option<Flock<File>>> {
    match Flock::lock(open_lock(key)?, mode) {
        Ok(lock) => Ok(Some(lock)),
        Err((_, nix::errno::Errno::EAGAIN)) => Ok(None),
        Err((_, error)) => Err(error.into()),
    }
}

pub fn try_name_lock(name: &str) -> io::Result<Option<Flock<File>>> {
    try_lock(&format!("box:{name}"), FlockArg::LockExclusiveNonblock)
}

/// Observe an existing lock without creating any host state.
pub fn name_alive(name: &str) -> io::Result<bool> {
    let directory = host_dir()?;
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other("unsafe host ownership directory"));
    }
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
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other("unsafe host ownership lock"));
    }
    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(_) => Ok(false),
        Err((_, nix::errno::Errno::EAGAIN)) => Ok(true),
        Err((_, error)) => Err(error.into()),
    }
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
    pub pid: i32,
    pub state_dir: PathBuf,
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

/// Remove an orphan's host files while the caller owns its name lock.
pub fn remove_record(name: &str, generation: &str) -> io::Result<()> {
    if let Some(record) = record(name)?
        && record.generation == generation
    {
        match fs::remove_dir_all(&record.state_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        remove_file(&socket(name)?)?;
        remove_file(&path(&format!("box:{name}"), "owner")?)?;
    }
    Ok(())
}

fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Name ownership precedes state creation. Publication follows the runtime
/// name check, so a refused claim preserves an orphan's record.
pub struct Claim {
    name: String,
    _lock: Flock<File>,
    record: Option<Record>,
    listener: Option<UnixListener>,
    request: Option<UnixStream>,
    state_dir: Option<PathBuf>,
    running: bool,
}

impl Claim {
    pub fn take(name: &str) -> io::Result<Option<Claim>> {
        Ok(try_name_lock(name)?.map(|lock| Claim {
            name: name.to_string(),
            _lock: lock,
            record: None,
            listener: None,
            request: None,
            state_dir: None,
            running: false,
        }))
    }

    pub fn prepare(&mut self, state_dir: &Path) -> io::Result<()> {
        if let Some(old) = record(&self.name)? {
            remove_record(&self.name, &old.generation)?;
        }
        match fs::remove_dir_all(state_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::create_dir_all(state_dir)?;
        self.state_dir = Some(state_dir.to_path_buf());
        let mut random = [0; 16];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        let record = Record {
            name: self.name.clone(),
            generation: sha256_hex(random),
            pid: std::process::id() as i32,
            state_dir: state_dir.to_path_buf(),
        };
        self.record = Some(record);
        // Retain the local pid as observable state; it is never signalled.
        fs::write(
            state_dir.join("pid"),
            self.record.as_ref().unwrap().pid.to_string(),
        )?;
        let endpoint = socket(&self.name)?;
        remove_file(&endpoint)?;
        self.listener = Some(UnixListener::bind(endpoint)?);
        let destination = path(&format!("box:{}", self.name), "owner")?;
        let staging = destination.with_extension(format!("tmp-{}", std::process::id()));
        let published = fs::write(&staging, serde_json::to_vec(self.record.as_ref().unwrap())?)
            .and_then(|()| fs::rename(&staging, destination));
        if published.is_err() {
            let _ = fs::remove_file(staging);
        }
        published
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

    /// Acknowledge only completed teardown. Ownership stays held until Drop.
    pub async fn finish(&mut self, success: bool) -> io::Result<()> {
        let cleanup = if success {
            if let Some(record) = self.record.as_ref() {
                remove_record(&self.name, &record.generation)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        };
        let completed = success && cleanup.is_ok();
        if completed {
            self.record = None;
            self.state_dir = None;
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
        if !self.running {
            if let Some(record) = self.record.as_ref() {
                let _ = remove_record(&self.name, &record.generation);
            }
            if let Some(state) = self.state_dir.as_ref() {
                let _ = fs::remove_dir_all(state);
            }
        }
        if self.record.is_some() {
            let _ = socket(&self.name).and_then(|socket| remove_file(&socket));
        }
    }
}

/// Ask only the generation observed before connecting. A replacement's
/// listener refuses the old token instead of stopping its own box.
pub fn request_stop(name: &str, observed: Option<&Record>) -> io::Result<()> {
    // An unpublished claim is absent to this request. Adopting a record
    // later could target a replacement after an old owner removed its record.
    let Some(observed) = observed else {
        return Ok(());
    };
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        if let Some(lock) = try_name_lock(name)? {
            drop(lock);
            return Ok(());
        }
        if record(name)?.is_none_or(|current| current.generation != observed.generation) {
            return Ok(());
        }
        match BlockingStream::connect(socket(name)?) {
            Ok(mut stream) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                stream.set_read_timeout(Some(remaining))?;
                stream.set_write_timeout(Some(remaining))?;
                let exchange: io::Result<[u8; 5]> = (|| {
                    stream.write_all(observed.generation.as_bytes())?;
                    let mut answer = [0; 5];
                    stream.read_exact(&mut answer)?;
                    Ok(answer)
                })();
                match exchange {
                    Ok(answer) => {
                        return match &answer {
                            b"done\n" | b"gone\n" => Ok(()),
                            _ => Err(io::Error::other("box teardown failed")),
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
                            return Ok(());
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

/// Published generations across every Pinfold state root.
pub fn records() -> io::Result<Vec<Record>> {
    let entries = match fs::read_dir(host_dir()?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    entries
        .filter_map(|entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => return Some(Err(error)),
            };
            if entry
                .path()
                .extension()
                .is_none_or(|extension| extension != "owner")
            {
                return None;
            }
            Some(
                fs::read(entry.path())
                    .and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other)),
            )
        })
        .collect()
}
