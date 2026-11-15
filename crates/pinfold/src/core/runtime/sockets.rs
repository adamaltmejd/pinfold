//! Checked generation socket directories, independent of the state root.

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

fn failure(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("runtime-path: {error}"))
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
    let metadata = file.metadata()?;
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
        marker.write_all(generation.as_bytes())?;
        marker.sync_all()?;
        check(&marker, false)?;
        mkdir_at(&directory, OsStr::new("v")).map_err(failure)?;
        let vsock = directory_at(&directory, OsStr::new("v")).map_err(failure)?;
        check(&vsock, true)?;
        let old = name(OsStr::new(&staging))?;
        let new = name(OsStr::new(&leaf))?;
        // A shortened-generation collision is never reused or replaced.
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
        let mut sockets = Self {
            host: Arc::new(host),
            directory: Arc::new(directory),
            vsock: Arc::new(vsock),
            path: base.join(&staging),
            generation: generation.to_owned(),
        };
        if let Err(error) = syscall(renamed) {
            if sockets.validate().is_ok() {
                let _ = sockets.remove_directory();
            }
            return Err(failure(error));
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
        let current = checked_directory(&self.path)?;
        let expected = self.directory.metadata()?;
        let observed = current.metadata()?;
        if expected.dev() != observed.dev() || expected.ino() != observed.ino() {
            return Err(failure("socket directory changed"));
        }
        let current_v = directory_at(&self.directory, OsStr::new("v")).map_err(failure)?;
        let expected_v = self.vsock.metadata()?;
        let observed_v = current_v.metadata()?;
        if expected_v.dev() != observed_v.dev() || expected_v.ino() != observed_v.ino() {
            return Err(failure("vsock directory changed"));
        }
        check_generation(&self.directory, &self.generation)?;
        check_entries(&self.directory, &[b"generation", b"p.sock", b"v"])?;
        check_entries(&self.vsock, &[b"v.sock", b"v.sock_1000", b"v.sock_3128"])?;
        for (directory, leaves) in [
            (&self.directory, &["p.sock"][..]),
            (&self.vsock, &["v.sock", "v.sock_1000", "v.sock_3128"][..]),
        ] {
            for leaf in leaves {
                check_socket(directory, OsStr::new(leaf), true)?;
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

    #[cfg(target_os = "linux")]
    pub fn vsock_dir(&self) -> io::Result<PathBuf> {
        self.validate()?;
        Ok(self.path.join("v"))
    }

    pub fn remove(generation: &str) -> io::Result<()> {
        let leaf = generation_name(generation)?;
        let (_, host) = host()?;
        let published = remove_directory(&host, OsStr::new(&leaf), generation);
        let staged = remove_directory(&host, OsStr::new(&format!(".r-{generation}")), generation);
        published.and(staged)
    }

    fn remove_directory(&self) -> io::Result<()> {
        remove_directory(&self.host, self.path.file_name().unwrap(), &self.generation)
    }
}

fn check_generation(directory: &File, generation: &str) -> io::Result<()> {
    let mut marker = open_at(
        directory,
        OsStr::new("generation"),
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
    )
    .map_err(failure)?;
    check(&marker, false)?;
    let mut bytes = Vec::new();
    (&mut marker).take(65).read_to_end(&mut bytes)?;
    if bytes != generation.as_bytes() {
        return Err(failure("socket generation differs"));
    }
    Ok(())
}

fn remove_directory(host: &File, leaf: &OsStr, generation: &str) -> io::Result<()> {
    check(host, true)?;
    let directory = match directory_at(host, leaf) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(error)),
        Ok(directory) => directory,
    };
    check(&directory, true)?;
    check_generation(&directory, generation)?;
    check_entries(&directory, &[b"generation", b"p.sock", b"v"])?;
    check_socket(&directory, OsStr::new("p.sock"), true)?;
    let vsock = match directory_at(&directory, OsStr::new("v")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(failure(error)),
        Ok(vsock) => {
            check(&vsock, true)?;
            check_entries(&vsock, &[b"v.sock", b"v.sock_1000", b"v.sock_3128"])?;
            for socket in ["v.sock", "v.sock_1000", "v.sock_3128"] {
                check_socket(&vsock, OsStr::new(socket), true)?;
            }
            Some(vsock)
        }
    };
    if let Some(vsock) = vsock {
        for socket in ["v.sock", "v.sock_1000", "v.sock_3128"] {
            unlink_at(&vsock, OsStr::new(socket), 0)?;
        }
        unlink_at(&directory, OsStr::new("v"), libc::AT_REMOVEDIR)?;
    }
    unlink_at(&directory, OsStr::new("p.sock"), 0)?;
    unlink_at(&directory, OsStr::new("generation"), 0)?;
    unlink_at(host, leaf, libc::AT_REMOVEDIR)
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

fn check_socket(directory: &File, leaf: &OsStr, absent: bool) -> io::Result<()> {
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
        || metadata.st_mode & 0o7777 != 0o600
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

pub(crate) fn bind(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    check_length(path)?;
    let directory = checked_directory(
        path.parent()
            .ok_or_else(|| failure("socket has no directory"))?,
    )?;
    let leaf = path
        .file_name()
        .ok_or_else(|| failure("socket has no name"))?;
    let listener = std::os::unix::net::UnixListener::bind(path).map_err(failure)?;
    let leaf_c = name(leaf)?;
    // Change only the socket just created by bind, never a startup obstacle.
    // SAFETY: a live directory descriptor and a terminated component name.
    syscall(unsafe {
        libc::fchmodat(
            directory.as_raw_fd(),
            leaf_c.as_ptr(),
            0o600,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })
    .map_err(failure)?;
    check_socket(&directory, leaf, false)?;
    Ok(listener)
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
    check_socket(&directory, leaf, true)?;
    unlink_at(&directory, leaf, 0)
}

fn check_entries(directory: &File, allowed: &[&[u8]]) -> io::Result<()> {
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
                Ok(())
            } else {
                Err(failure(error))
            };
        }
        // SAFETY: readdir returned a valid entry with a terminated d_name.
        let leaf = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if leaf != b"." && leaf != b".." && !allowed.contains(&leaf) {
            return Err(failure("unexpected socket-directory entry"));
        }
    }
}
