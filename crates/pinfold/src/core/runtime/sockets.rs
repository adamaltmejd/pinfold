//! Checked generation socket directories, independent of the state root.

use std::collections::BTreeSet;
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nix::libc;

#[derive(Clone)]
pub struct Sockets {
    host: Arc<File>,
    directory: Arc<File>,
    vsock: Arc<File>,
    path: PathBuf,
    generation: String,
}

#[derive(Debug)]
struct PathError(String);

impl std::fmt::Display for PathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "runtime-path: {}", self.0)
    }
}

impl std::error::Error for PathError {}

pub(crate) fn is_path_error(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|error| error.is::<PathError>())
}

fn failure(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(PathError(error.to_string()))
}

fn name(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(failure)
}

fn syscall(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn open_at(parent: &File, value: &OsStr, flags: i32, mode: u32) -> io::Result<File> {
    let value = name(value)?;
    // SAFETY: a live directory descriptor and a terminated component name.
    let fd = syscall(unsafe {
        libc::openat(
            parent.as_raw_fd(),
            value.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    })?;
    // SAFETY: openat returned a new descriptor owned here.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn directory_at(parent: &File, value: &OsStr) -> io::Result<File> {
    open_at(parent, value, libc::O_RDONLY | libc::O_DIRECTORY, 0)
}

fn mkdir_at(parent: &File, value: &OsStr) -> io::Result<()> {
    let value = name(value)?;
    // SAFETY: a live directory descriptor and a terminated component name.
    syscall(unsafe { libc::mkdirat(parent.as_raw_fd(), value.as_ptr(), 0o700) }).map(drop)
}

fn check(file: &File, directory: bool) -> io::Result<()> {
    let metadata = file.metadata().map_err(failure)?;
    let kind = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    let mode = if directory { 0o700 } else { 0o600 };
    if !kind || metadata.uid() != nix::unistd::getuid().as_raw() || metadata.mode() & 0o7777 != mode
    {
        return Err(failure("unsafe socket directory or generation marker"));
    }
    Ok(())
}

/// Open every component without following a link. Ancestors may be public;
/// the final directory must be private and owned by this user.
pub(crate) fn checked_directory(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(failure("socket directory is not absolute"));
    }
    let mut current = File::open("/")?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => current = directory_at(&current, value).map_err(failure)?,
            _ => return Err(failure("socket directory is not an absolute plain path")),
        }
    }
    check(&current, true)?;
    Ok(current)
}

pub(crate) fn host() -> io::Result<(PathBuf, File)> {
    open_host(true)?.ok_or_else(|| failure("host socket directory disappeared"))
}

pub(crate) fn existing_host() -> io::Result<Option<(PathBuf, File)>> {
    open_host(false)
}

fn open_host(create: bool) -> io::Result<Option<(PathBuf, File)>> {
    let path = crate::core::ownership::host_dir()?;
    let parent = path.parent().expect("host directory has a parent");
    let mut base = File::open("/")?;
    for component in parent.components() {
        if let Component::Normal(value) = component {
            base = directory_at(&base, value).map_err(failure)?;
        }
    }
    let metadata = base.metadata().map_err(failure)?;
    if metadata.uid() != 0 || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0) {
        return Err(failure("unsafe host socket parent"));
    }
    let leaf = path.file_name().expect("host directory has a name");
    if create
        && let Err(error) = mkdir_at(&base, leaf)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(failure(error));
    }
    let directory = match directory_at(&base, leaf) {
        Err(error) if !create && error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failure(error)),
        Ok(directory) => directory,
    };
    check(&directory, true)?;
    Ok(Some((path, directory)))
}

fn generation_name(generation: &str) -> io::Result<String> {
    if generation.len() != 64
        || !generation
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(failure("invalid generation"));
    }
    Ok(format!("r-{}", &generation[..32]))
}

impl Sockets {
    pub fn create(generation: &str) -> io::Result<Self> {
        let leaf = generation_name(generation)?;
        let (base, host) = host()?;
        let staging = format!(".r-{generation}");
        mkdir_at(&host, OsStr::new(&staging)).map_err(failure)?;
        let directory = directory_at(&host, OsStr::new(&staging)).map_err(failure)?;
        check(&directory, true)?;
        let mut marker = open_at(
            &directory,
            OsStr::new("generation"),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
        .map_err(failure)?;
        marker.write_all(generation.as_bytes()).map_err(failure)?;
        marker.sync_all().map_err(failure)?;
        check(&marker, false)?;
        mkdir_at(&directory, OsStr::new("v")).map_err(failure)?;
        let vsock = directory_at(&directory, OsStr::new("v")).map_err(failure)?;
        check(&vsock, true)?;
        // A shortened-generation collision is never reused or replaced.
        let renamed = rename_at(&host, OsStr::new(&staging), OsStr::new(&leaf));
        let mut sockets = Self {
            host: Arc::new(host),
            directory: Arc::new(directory),
            vsock: Arc::new(vsock),
            path: base.join(&staging),
            generation: generation.to_owned(),
        };
        if let Err(error) = renamed {
            let _ = sockets.remove_directory();
            return Err(error);
        }
        sockets.path = base.join(leaf);
        sockets.validate()?;
        sockets.check_lengths()?;
        Ok(sockets)
    }

    pub fn open(generation: &str) -> io::Result<Self> {
        let leaf = generation_name(generation)?;
        let (base, host) =
            existing_host()?.ok_or_else(|| failure("missing host socket directory"))?;
        let directory = directory_at(&host, OsStr::new(&leaf)).map_err(failure)?;
        check(&directory, true)?;
        let vsock = directory_at(&directory, OsStr::new("v")).map_err(failure)?;
        check(&vsock, true)?;
        let sockets = Self {
            host: Arc::new(host),
            directory: Arc::new(directory),
            vsock: Arc::new(vsock),
            path: base.join(leaf),
            generation: generation.to_owned(),
        };
        sockets.validate()?;
        sockets.check_lengths()?;
        Ok(sockets)
    }

    pub fn validate(&self) -> io::Result<()> {
        check(&self.host, true)?;
        check(&self.directory, true)?;
        check(&self.vsock, true)?;
        check_identity(&self.host, self.path.file_name().unwrap(), &self.directory)?;
        check_identity(&self.directory, OsStr::new("v"), &self.vsock)?;
        check_generation(&self.directory, &self.generation)?;
        check_entries(
            &self.directory,
            &[b"generation", b"c.sock", b"p.sock", b"v"],
        )?;
        check_entries(&self.vsock, &[b"v.sock", b"v.sock_1000", b"v.sock_3128"])?;
        for (directory, leaves) in [
            (&self.directory, &["c.sock", "p.sock"][..]),
            (&self.vsock, &["v.sock", "v.sock_1000", "v.sock_3128"][..]),
        ] {
            for leaf in leaves {
                check_socket(directory, OsStr::new(leaf), true, true)?;
            }
        }
        Ok(())
    }

    fn check_lengths(&self) -> io::Result<()> {
        for leaf in [
            "c.sock",
            "p.sock",
            "v/v.sock",
            "v/v.sock_1000",
            "v/v.sock_3128",
        ] {
            check_length(&self.path.join(leaf))?;
        }
        Ok(())
    }

    pub fn proxy(&self) -> io::Result<PathBuf> {
        self.validate()?;
        #[cfg(target_os = "linux")]
        return Ok(self.path.join("v/v.sock_3128"));
        #[cfg(not(target_os = "linux"))]
        return Ok(self.path.join("p.sock"));
    }

    pub fn bind_proxy(&self) -> io::Result<std::os::unix::net::UnixListener> {
        self.validate()?;
        #[cfg(target_os = "linux")]
        return bind_at(&self.vsock, OsStr::new("v.sock_3128"));
        #[cfg(not(target_os = "linux"))]
        return bind_at(&self.directory, OsStr::new("p.sock"));
    }

    pub fn bind_control(&self) -> io::Result<std::os::unix::net::UnixListener> {
        self.validate()?;
        bind_at(&self.directory, OsStr::new("c.sock"))
    }

    pub fn connect_control_until(
        &self,
        deadline: Instant,
    ) -> io::Result<std::os::unix::net::UnixStream> {
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.validate()?;
        // Linux's connect checks the socket through its own descriptor.
        #[cfg(target_os = "macos")]
        check_socket(&self.directory, OsStr::new("c.sock"), false, true)?;
        connect_at(&self.directory, OsStr::new("c.sock"), Some(deadline))
    }

    #[cfg(target_os = "linux")]
    pub fn bind_boot(&self) -> io::Result<std::os::unix::net::UnixListener> {
        self.validate()?;
        bind_at(&self.vsock, OsStr::new("v.sock_1000"))
    }

    #[cfg(target_os = "linux")]
    pub fn vsock_directory(&self) -> io::Result<&File> {
        self.validate()?;
        Ok(&self.vsock)
    }

    #[cfg(target_os = "linux")]
    pub fn connect(&self) -> io::Result<std::os::unix::net::UnixStream> {
        self.validate()?;
        connect_at(&self.vsock, OsStr::new("v.sock"), None)
    }

    #[cfg(target_os = "linux")]
    pub fn connect_until(&self, deadline: Instant) -> io::Result<std::os::unix::net::UnixStream> {
        self.validate()?;
        connect_at(&self.vsock, OsStr::new("v.sock"), Some(deadline))
    }

    pub fn remove(generation: &str) -> io::Result<()> {
        let leaf = generation_name(generation)?;
        let (_, host) = host()?;
        let retiring = format!(".d-{generation}");
        let mut result = remove_directory(&host, OsStr::new(&retiring), generation, true);
        let published = (|| {
            let Some(directory) = recoverable(&host, OsStr::new(&leaf), generation, false)? else {
                return Ok(());
            };
            check_identity(&host, OsStr::new(&leaf), &directory)?;
            rename_at(&host, OsStr::new(&leaf), OsStr::new(&retiring))?;
            remove_directory(&host, OsStr::new(&retiring), generation, true)
        })();
        result = result.and(published);
        let staged = remove_directory(
            &host,
            OsStr::new(&format!(".r-{generation}")),
            generation,
            true,
        );
        result.and(staged)
    }

    fn remove_directory(&self) -> io::Result<()> {
        remove_directory(
            &self.host,
            self.path.file_name().unwrap(),
            &self.generation,
            true,
        )
    }

    pub fn leftovers(protected: &BTreeSet<String>) -> io::Result<Vec<(String, PathBuf)>> {
        let Some((path, host)) = open_host(false)? else {
            return Ok(Vec::new());
        };
        let mut leftovers = Vec::new();
        for leaf in entries(&host)? {
            let bytes = leaf.as_bytes();
            let full = bytes
                .strip_prefix(b".r-")
                .or_else(|| bytes.strip_prefix(b".d-"));
            let generation = if let Some(full) = full {
                let Ok(generation) = std::str::from_utf8(full) else {
                    continue;
                };
                if generation_name(generation).is_err() {
                    continue;
                }
                generation.to_owned()
            } else if bytes.starts_with(b"r-") && bytes.len() == 34 {
                let directory = match directory_at(&host, &leaf) {
                    Ok(directory) => directory,
                    Err(_) => continue,
                };
                match read_generation(&directory) {
                    Ok(Some(marker)) => match String::from_utf8(marker) {
                        Ok(generation)
                            if generation_name(&generation).is_ok()
                                && bytes[2..] == generation.as_bytes()[..32] =>
                        {
                            generation
                        }
                        _ => continue,
                    },
                    _ => continue,
                }
            } else {
                continue;
            };
            if !protected.contains(&generation) {
                match recoverable(&host, &leaf, &generation, full.is_some()) {
                    Ok(Some(_)) => leftovers.push((generation, path.join(leaf))),
                    Ok(None) => {}
                    Err(error) => eprintln!("pinfold: socket cleanup: {error}"),
                }
            }
        }
        Ok(leftovers)
    }
}

/// One bind at a fixed leaf of the host directory, outside any generation.
pub(crate) struct HostSocket {
    leaf: String,
    identity: (u64, u64),
}

impl HostSocket {
    /// Bind `leaf`, replacing a socket left there. The caller holds the
    /// exclusion that makes it the only live user of `leaf`.
    pub(crate) fn bind(leaf: &str) -> io::Result<(Self, std::os::unix::net::UnixListener)> {
        let (base, host) = host()?;
        check_length(&base.join(leaf))?;
        Self::remove_left(leaf)?;
        let listener = bind_at(&host, OsStr::new(leaf))?;
        let identity = socket_identity(&host, OsStr::new(leaf))?;
        let leaf = leaf.to_owned();
        Ok((Self { leaf, identity }, listener))
    }

    /// Unlink a socket at `leaf`, under the same exclusion as [`Self::bind`].
    pub(crate) fn remove_left(leaf: &str) -> io::Result<()> {
        let Some((_, host)) = existing_host()? else {
            return Ok(());
        };
        check_socket(&host, OsStr::new(leaf), true, false)?;
        unlink_at(&host, OsStr::new(leaf), 0)
    }

    /// Unlink the socket only while it is still this bind's.
    pub(crate) fn remove(&self) -> io::Result<()> {
        let (_, host) = host()?;
        let leaf = OsStr::new(&self.leaf);
        match socket_identity(&host, leaf) {
            Ok(identity) if identity == self.identity => unlink_at(&host, leaf, 0),
            _ => Ok(()),
        }
    }
}

/// `leaf`'s own metadata, never a link's target. A failed call keeps its
/// OS error.
fn stat_at(parent: &File, leaf: &CStr) -> io::Result<libc::stat> {
    let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: a live directory, terminated leaf and initialized output.
    syscall(unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    Ok(metadata)
}

fn socket_identity(parent: &File, leaf: &OsStr) -> io::Result<(u64, u64)> {
    let metadata = stat_at(parent, &name(leaf)?).map_err(failure)?;
    if metadata.st_mode & libc::S_IFMT != libc::S_IFSOCK {
        return Err(failure("unsafe socket"));
    }
    #[allow(clippy::unnecessary_cast)]
    Ok((metadata.st_dev as u64, metadata.st_ino as u64))
}

fn check_identity(parent: &File, leaf: &OsStr, expected: &File) -> io::Result<()> {
    let metadata = stat_at(parent, &name(leaf)?).map_err(failure)?;
    let expected = expected.metadata().map_err(failure)?;
    #[cfg(target_os = "macos")]
    let device = metadata.st_dev as u64;
    #[cfg(target_os = "linux")]
    let device = metadata.st_dev;
    if metadata.st_mode & libc::S_IFMT != libc::S_IFDIR
        || device != expected.dev()
        || metadata.st_ino != expected.ino()
    {
        return Err(failure("socket directory changed"));
    }
    Ok(())
}

fn read_generation(directory: &File) -> io::Result<Option<Vec<u8>>> {
    let mut marker = match open_at(
        directory,
        OsStr::new("generation"),
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
    ) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failure(error)),
        Ok(marker) => marker,
    };
    check(&marker, false)?;
    let mut bytes = Vec::new();
    (&mut marker)
        .take(65)
        .read_to_end(&mut bytes)
        .map_err(failure)?;
    Ok(Some(bytes))
}

fn check_generation(directory: &File, generation: &str) -> io::Result<()> {
    if read_generation(directory)?.as_deref() != Some(generation.as_bytes()) {
        return Err(failure("socket generation differs"));
    }
    Ok(())
}

fn recoverable(
    host: &File,
    leaf: &OsStr,
    generation: &str,
    full_name: bool,
) -> io::Result<Option<File>> {
    check(host, true)?;
    let directory = match directory_at(host, leaf) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failure(error)),
        Ok(directory) => directory,
    };
    check(&directory, true)?;
    match read_generation(&directory)? {
        Some(marker) if marker == generation.as_bytes() => {}
        None if full_name => {}
        Some(marker) if full_name && marker.len() < 64 => {}
        _ => return Err(failure("socket generation is unverifiable")),
    }
    check_entries(&directory, &[b"generation", b"c.sock", b"p.sock", b"v"])?;
    check_socket(&directory, OsStr::new("c.sock"), true, false)?;
    check_socket(&directory, OsStr::new("p.sock"), true, false)?;
    if let Some(vsock) = recovery_vsock(&directory)? {
        check_entries(&vsock, &[b"v.sock", b"v.sock_1000", b"v.sock_3128"])?;
        for socket in ["v.sock", "v.sock_1000", "v.sock_3128"] {
            check_socket(&vsock, OsStr::new(socket), true, false)?;
        }
    }
    Ok(Some(directory))
}

fn recovery_vsock(directory: &File) -> io::Result<Option<File>> {
    match directory_at(directory, OsStr::new("v")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(failure(error)),
        Ok(vsock) => {
            check(&vsock, true)?;
            Ok(Some(vsock))
        }
    }
}

fn remove_directory(
    host: &File,
    leaf: &OsStr,
    generation: &str,
    full_name: bool,
) -> io::Result<()> {
    let Some(directory) = recoverable(host, leaf, generation, full_name)? else {
        return Ok(());
    };
    if let Some(vsock) = recovery_vsock(&directory)? {
        for socket in ["v.sock", "v.sock_1000", "v.sock_3128"] {
            unlink_at(&vsock, OsStr::new(socket), 0)?;
        }
        unlink_at(&directory, OsStr::new("v"), libc::AT_REMOVEDIR)?;
    }
    unlink_at(&directory, OsStr::new("c.sock"), 0)?;
    unlink_at(&directory, OsStr::new("p.sock"), 0)?;
    unlink_at(&directory, OsStr::new("generation"), 0)?;
    unlink_at(host, leaf, libc::AT_REMOVEDIR)
}

fn rename_at(host: &File, old: &OsStr, new: &OsStr) -> io::Result<()> {
    let old = name(old)?;
    let new = name(new)?;
    #[cfg(target_os = "linux")]
    let renamed = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            host.as_raw_fd(),
            old.as_ptr(),
            host.as_raw_fd(),
            new.as_ptr(),
            libc::RENAME_NOREPLACE,
        ) as libc::c_int
    };
    #[cfg(target_os = "macos")]
    let renamed = unsafe {
        libc::renameatx_np(
            host.as_raw_fd(),
            old.as_ptr(),
            host.as_raw_fd(),
            new.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    syscall(renamed).map(drop).map_err(failure)
}

fn unlink_at(parent: &File, leaf: &OsStr, flags: i32) -> io::Result<()> {
    let leaf = name(leaf)?;
    // SAFETY: a live directory descriptor and a terminated component name.
    let result = syscall(unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), flags) });
    match result {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failure(error)),
    }
}

fn check_socket(directory: &File, leaf: &OsStr, absent: bool, mode: bool) -> io::Result<()> {
    let metadata = match stat_at(directory, &name(leaf)?) {
        Err(error) if absent && error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(error)),
        Ok(metadata) => metadata,
    };
    if metadata.st_mode & libc::S_IFMT != libc::S_IFSOCK
        || metadata.st_uid != nix::unistd::getuid().as_raw()
        || (mode && metadata.st_mode & 0o7777 != 0o600)
    {
        return Err(failure("unsafe socket"));
    }
    Ok(())
}

pub(crate) fn check_length(path: &Path) -> io::Result<()> {
    // SAFETY: only the fixed-size array's capacity is read.
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.as_os_str().as_bytes().len() + 1 > address.sun_path.len() {
        return Err(failure("socket path exceeds sockaddr_un capacity"));
    }
    Ok(())
}

/// `path`'s checked directory and its leaf.
fn parent_leaf(path: &Path) -> io::Result<(File, &OsStr)> {
    let directory = checked_directory(
        path.parent()
            .ok_or_else(|| failure("socket has no directory"))?,
    )?;
    let leaf = path
        .file_name()
        .ok_or_else(|| failure("socket has no name"))?;
    Ok((directory, leaf))
}

#[cfg(target_os = "linux")]
pub(crate) fn bind(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    check_length(path)?;
    let (directory, leaf) = parent_leaf(path)?;
    bind_at(&directory, leaf)
}

pub(crate) fn bind_at(
    directory: &File,
    leaf: &OsStr,
) -> io::Result<std::os::unix::net::UnixListener> {
    let socket = socket_at_until(directory, leaf, false, None)?;
    check_socket(directory, leaf, false, true)?;
    syscall(unsafe { libc::listen(socket.as_raw_fd(), 128) })?;
    use std::os::fd::IntoRawFd;
    Ok(unsafe { std::os::unix::net::UnixListener::from_raw_fd(socket.into_raw_fd()) })
}

/// Connect to one checked host-only shutdown socket without a blocking backlog wait.
pub(crate) fn connect_shutdown(
    path: &Path,
    deadline: Instant,
) -> io::Result<std::os::unix::net::UnixStream> {
    crate::core::pipe::remaining(deadline, None, "box teardown")?;
    check_length(path)?;
    let (directory, leaf) = parent_leaf(path)?;
    check_socket(&directory, leaf, true, true)?;
    connect_at(&directory, leaf, Some(deadline))
}

#[cfg(target_os = "macos")]
fn connect_at(
    directory: &File,
    leaf: &OsStr,
    deadline: Option<Instant>,
) -> io::Result<std::os::unix::net::UnixStream> {
    use std::os::fd::IntoRawFd;
    let socket = socket_at_until(directory, leaf, true, deadline)?;
    Ok(unsafe { std::os::unix::net::UnixStream::from_raw_fd(socket.into_raw_fd()) })
}

#[cfg(target_os = "linux")]
fn connect_at(
    directory: &File,
    leaf: &OsStr,
    deadline: Option<Instant>,
) -> io::Result<std::os::unix::net::UnixStream> {
    let socket = open_at(directory, leaf, libc::O_PATH, 0).map_err(|error| {
        if deadline.is_some() && error.kind() == io::ErrorKind::NotFound {
            error
        } else {
            failure(error)
        }
    })?;
    let metadata = socket.metadata().map_err(failure)?;
    if metadata.mode() & libc::S_IFMT != libc::S_IFSOCK
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err(failure("unsafe socket"));
    }
    let path = format!("/proc/self/fd/{}", socket.as_raw_fd());
    let Some(deadline) = deadline else {
        return std::os::unix::net::UnixStream::connect(path);
    };
    if Instant::now() >= deadline {
        return Err(io::ErrorKind::TimedOut.into());
    }
    use nix::sys::socket::{
        AddressFamily, SockFlag, SockType, UnixAddr, connect, socket as create_socket,
    };
    let connection = create_socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        None,
    )?;
    match connect(connection.as_raw_fd(), &UnixAddr::new(path.as_str())?) {
        Ok(()) => {}
        Err(nix::errno::Errno::EINPROGRESS) => wait_connected(connection.as_raw_fd(), deadline)?,
        Err(error) => return Err(error.into()),
    }
    syscall(unsafe { libc::fcntl(connection.as_raw_fd(), libc::F_SETFL, 0) })?;
    Ok(std::os::unix::net::UnixStream::from(connection))
}

fn socket_at_until(
    directory: &File,
    leaf: &OsStr,
    connect: bool,
    deadline: Option<Instant>,
) -> io::Result<File> {
    let helper_deadline = deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(10));
    check(directory, true)?;
    if leaf.as_bytes().contains(&b'/') || leaf.as_bytes() == b"." || leaf.as_bytes() == b".." {
        return Err(failure("socket leaf is not a component"));
    }
    check_length(Path::new(leaf))?;
    let leaf = name(leaf)?;
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    let bytes = leaf.as_bytes_with_nul();
    // SAFETY: the address capacity was checked above, including the terminator.
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            address.sun_path.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    let length =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len()) as libc::socklen_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = length as _;
    }
    let fork_guard = crate::core::process::fork_guard();
    #[cfg(target_os = "linux")]
    let socket_type = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
    #[cfg(target_os = "macos")]
    let socket_type = libc::SOCK_STREAM;
    let fd = syscall(unsafe { libc::socket(libc::AF_UNIX, socket_type, 0) })?;
    // SAFETY: socket returned a new descriptor owned here.
    let socket = unsafe { File::from_raw_fd(fd) };
    #[cfg(target_os = "macos")]
    syscall(unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) })?;
    if connect {
        syscall(unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) })?;
    }
    #[cfg(target_os = "linux")]
    let (read, write) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    #[cfg(target_os = "macos")]
    let (read, write) = {
        let pipes = nix::unistd::pipe()?;
        for pipe in [&pipes.0, &pipes.1] {
            syscall(unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
        }
        pipes
    };
    crate::core::process::prepare_fork()?;
    let pid = syscall(unsafe { libc::fork() })?;
    if pid == 0 {
        // Only async-signal-safe syscalls run between fork and _exit. The
        // child isolates cwd and umask from the multithreaded owner.
        unsafe {
            let inherited = [fd, directory.as_raw_fd(), write.as_raw_fd()];
            let floor = inherited.iter().copied().max().unwrap().max(5) + 1;
            let mut retained = [-1; 3];
            for index in 0..3 {
                retained[index] = libc::fcntl(inherited[index], libc::F_DUPFD_CLOEXEC, floor);
                if retained[index] < 0 {
                    libc::_exit(1);
                }
            }
            for (index, descriptor) in retained.into_iter().enumerate() {
                if libc::dup2(descriptor, index as i32 + 3) < 0 {
                    libc::_exit(1);
                }
            }
            libc::close(0);
            libc::close(1);
            libc::close(2);
            if !crate::core::process::close_fds_from(6) {
                libc::_exit(1);
            }
            libc::umask(0o177);
            let (stage, result) = if libc::fchdir(4) < 0 {
                (1i32, -1)
            } else if connect {
                (
                    2i32,
                    libc::connect(3, (&address as *const libc::sockaddr_un).cast(), length),
                )
            } else {
                (
                    2i32,
                    libc::bind(3, (&address as *const libc::sockaddr_un).cast(), length),
                )
            };
            let error = if result < 0 {
                nix::errno::Errno::last_raw()
            } else {
                0
            };
            let report = [stage, error];
            libc::write(5, report.as_ptr().cast(), std::mem::size_of_val(&report));
            libc::_exit(0);
        }
    }
    drop(fork_guard);
    drop(write);
    let mut report = [0u8; 8];
    let mut reader = File::from(read);
    let received: io::Result<()> = (|| {
        crate::core::pipe::nonblocking(&reader, "socket helper")?;
        let mut filled = 0;
        while filled < report.len() {
            let count = crate::core::pipe::read(
                &mut reader,
                &mut report[filled..],
                helper_deadline,
                None,
                "socket helper",
            )?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            filled += count;
        }
        Ok(())
    })();
    if received.is_err() {
        // This direct child is still unreaped, so its pid remains ours.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    let mut status = 0;
    let mut wait_deadline = Some(helper_deadline);
    let mut wait_error = None;
    loop {
        let waited = unsafe {
            libc::waitpid(
                pid,
                &mut status,
                if wait_deadline.is_some() {
                    libc::WNOHANG
                } else {
                    0
                },
            )
        };
        if waited == pid {
            break;
        }
        if waited < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(io::Error::last_os_error());
        }
        if let Some(deadline) = wait_deadline
            && let Err(error) = crate::core::pipe::wait_ready(
                None,
                nix::poll::PollFlags::empty(),
                deadline,
                None,
                "socket helper",
            )
        {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            wait_error = Some(error);
            wait_deadline = None;
        }
    }
    received?;
    if let Some(error) = wait_error {
        return Err(error);
    }
    let stage = i32::from_ne_bytes(report[..4].try_into().unwrap());
    let error = i32::from_ne_bytes(report[4..].try_into().unwrap());
    if connect && error == libc::EINPROGRESS {
        wait_connected(fd, helper_deadline)?;
    } else if error != 0 {
        let operation = if stage == 1 {
            "enter socket directory"
        } else if connect {
            "connect socket"
        } else {
            "bind socket"
        };
        return Err(failure(format!(
            "{operation}: {}",
            io::Error::from_raw_os_error(error)
        )));
    }
    if connect {
        syscall(unsafe { libc::fcntl(fd, libc::F_SETFL, 0) })?;
    }
    Ok(socket)
}

fn wait_connected(fd: libc::c_int, deadline: Instant) -> io::Result<()> {
    let mut readiness = libc::pollfd {
        fd,
        events: libc::POLLOUT,
        revents: 0,
    };
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "connect socket timed out",
            ));
        }
        let result = unsafe { libc::poll(&mut readiness, 1, remaining.as_millis().max(1) as _) };
        if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        syscall(result)?;
        if result == 0 {
            continue;
        }
        let mut error: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&error) as libc::socklen_t;
        syscall(unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut libc::c_int).cast(),
                &mut length,
            )
        })?;
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        break;
    }
    Ok(())
}

/// Remove an existing checked socket, never a link or another file type.
pub(crate) fn remove_socket(path: &Path) -> io::Result<()> {
    let (directory, leaf) = parent_leaf(path)?;
    check_socket(&directory, leaf, true, false)?;
    unlink_at(&directory, leaf, 0)
}

fn entries(directory: &File) -> io::Result<Vec<std::ffi::OsString>> {
    let mut names = Vec::new();
    let copy = directory_at(directory, OsStr::new(".")).map_err(failure)?;
    crate::core::directory_entries(copy.into(), |leaf| {
        names.push(leaf.to_owned());
        Ok(())
    })
    .map_err(failure)?;
    Ok(names)
}

fn check_entries(directory: &File, allowed: &[&[u8]]) -> io::Result<()> {
    if entries(directory)?
        .iter()
        .any(|leaf| !allowed.contains(&leaf.as_bytes()))
    {
        return Err(failure("unexpected socket-directory entry"));
    }
    Ok(())
}
