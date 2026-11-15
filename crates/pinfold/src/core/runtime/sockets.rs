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
    if let Err(error) = mkdir_at(&base, leaf)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(failure(error));
    }
    let directory = directory_at(&base, leaf).map_err(failure)?;
    check(&directory, true)?;
    Ok((path, directory))
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

    #[cfg(target_os = "linux")]
    pub fn open(generation: &str) -> io::Result<Self> {
        let leaf = generation_name(generation)?;
        let (base, host) = host()?;
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
        check_entries(&self.directory, &[b"generation", b"p.sock", b"v"])?;
        check_entries(&self.vsock, &[b"v.sock", b"v.sock_1000", b"v.sock_3128"])?;
        for (directory, leaves) in [
            (&self.directory, &["p.sock"][..]),
            (&self.vsock, &["v.sock", "v.sock_1000", "v.sock_3128"][..]),
        ] {
            for leaf in leaves {
                check_socket(directory, OsStr::new(leaf), true, true)?;
            }
        }
        Ok(())
    }

    fn check_lengths(&self) -> io::Result<()> {
        for leaf in ["p.sock", "v/v.sock", "v/v.sock_1000", "v/v.sock_3128"] {
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
        let socket =
            open_at(&self.vsock, OsStr::new("v.sock"), libc::O_PATH, 0).map_err(failure)?;
        let metadata = socket.metadata().map_err(failure)?;
        if metadata.mode() & libc::S_IFMT != libc::S_IFSOCK
            || metadata.uid() != nix::unistd::getuid().as_raw()
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(failure("unsafe vsock socket"));
        }
        std::os::unix::net::UnixStream::connect(format!("/proc/self/fd/{}", socket.as_raw_fd()))
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
        let (path, host) = host()?;
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

fn check_identity(parent: &File, leaf: &OsStr, expected: &File) -> io::Result<()> {
    let leaf = name(leaf)?;
    let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: a live directory, terminated leaf and initialized output.
    syscall(unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })
    .map_err(failure)?;
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
    check_entries(&directory, &[b"generation", b"p.sock", b"v"])?;
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
    let leaf = name(leaf)?;
    // SAFETY: fstatat initializes the output and does not follow a link.
    let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
    let status = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    match syscall(status) {
        Err(error) if absent && error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(error)),
        Ok(_) => {}
    }
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

#[cfg(target_os = "linux")]
pub(crate) fn bind(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    check_length(path)?;
    let directory = checked_directory(
        path.parent()
            .ok_or_else(|| failure("socket has no directory"))?,
    )?;
    bind_at(
        &directory,
        path.file_name()
            .ok_or_else(|| failure("socket has no name"))?,
    )
}

pub(crate) fn bind_at(
    directory: &File,
    leaf: &OsStr,
) -> io::Result<std::os::unix::net::UnixListener> {
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
    let fd =
        syscall(unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) }).map_err(failure)?;
    // SAFETY: socket returned a new descriptor owned here.
    let socket = unsafe { File::from_raw_fd(fd) };
    syscall(unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) }).map_err(failure)?;
    let (read, write) = nix::unistd::pipe().map_err(failure)?;
    for pipe in [&read, &write] {
        syscall(unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })
            .map_err(failure)?;
    }
    let pid = syscall(unsafe { libc::fork() }).map_err(failure)?;
    if pid == 0 {
        // Only async-signal-safe syscalls run between fork and _exit. The
        // child isolates cwd and umask from the multithreaded owner.
        unsafe {
            libc::umask(0o177);
            let result = if libc::fchdir(directory.as_raw_fd()) < 0 {
                -1
            } else {
                libc::bind(fd, (&address as *const libc::sockaddr_un).cast(), length)
            };
            #[cfg(target_os = "macos")]
            let error = if result < 0 { *libc::__error() } else { 0 };
            #[cfg(target_os = "linux")]
            let error = if result < 0 {
                *libc::__errno_location()
            } else {
                0
            };
            libc::write(
                write.as_raw_fd(),
                (&error as *const i32).cast(),
                std::mem::size_of::<i32>(),
            );
            libc::_exit(0);
        }
    }
    drop(write);
    let mut error = [0u8; 4];
    let received = File::from(read).read_exact(&mut error);
    let mut status = 0;
    loop {
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        if waited >= 0 {
            break;
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(failure(io::Error::last_os_error()));
        }
    }
    received.map_err(failure)?;
    let error = i32::from_ne_bytes(error);
    if error != 0 {
        return Err(failure(io::Error::from_raw_os_error(error)));
    }
    check_socket(directory, OsStr::from_bytes(leaf.as_bytes()), false, true)?;
    syscall(unsafe { libc::listen(fd, 128) }).map_err(failure)?;
    use std::os::fd::IntoRawFd;
    Ok(unsafe { std::os::unix::net::UnixListener::from_raw_fd(socket.into_raw_fd()) })
}

/// Remove an existing checked socket, never a link or another file type.
pub(crate) fn remove_socket(path: &Path) -> io::Result<()> {
    let directory = checked_directory(
        path.parent()
            .ok_or_else(|| failure("socket has no directory"))?,
    )?;
    let leaf = path
        .file_name()
        .ok_or_else(|| failure("socket has no name"))?;
    check_socket(&directory, leaf, true, false)?;
    unlink_at(&directory, leaf, 0)
}

fn entries(directory: &File) -> io::Result<Vec<std::ffi::OsString>> {
    let mut names = Vec::new();
    let copy = directory_at(directory, OsStr::new(".")).map_err(failure)?;
    // SAFETY: fdopendir takes ownership only on success.
    let entries = unsafe { libc::fdopendir(copy.as_raw_fd()) };
    if entries.is_null() {
        return Err(failure(io::Error::last_os_error()));
    }
    std::mem::forget(copy);
    struct Entries(*mut libc::DIR);
    impl Drop for Entries {
        fn drop(&mut self) {
            // SAFETY: this is the sole owner of the open directory stream.
            unsafe { libc::closedir(self.0) };
        }
    }
    let entries = Entries(entries);
    loop {
        nix::errno::Errno::clear();
        // SAFETY: the stream remains live; the entry is used before readdir.
        let entry = unsafe { libc::readdir(entries.0) };
        if entry.is_null() {
            let error = nix::errno::Errno::last();
            return if error == nix::errno::Errno::UnknownErrno {
                Ok(names)
            } else {
                Err(failure(error))
            };
        }
        // SAFETY: readdir returned a valid entry with a terminated d_name.
        let leaf = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if leaf != b"." && leaf != b".." {
            names.push(OsStr::from_bytes(leaf).to_owned());
        }
    }
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
