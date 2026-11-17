//! Share admission and the read-only Git attachments it adds.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::plan::{Mount, MountKind, Plan, guest_path};
use super::profile::Profile;
use crate::dirs;

/// Reused by project admission; `extra` names its configuration files too.
pub(crate) fn check_paths_outside(
    root: &Path,
    profile: Option<&Profile>,
    extra: impl IntoIterator<Item = PathBuf>,
) -> io::Result<()> {
    let root = fs::canonicalize(root)?;
    let config = dirs::config_dir()?;
    let mut paths = vec![
        config.clone(),
        dirs::state_dir()?,
        dirs::cache_dir()?,
        dirs::artifacts_dir()?,
        super::ownership::host_dir()?,
    ];
    paths.extend(extra);
    if let Some(profile) = profile {
        let dir = config.join("profiles").join(&profile.name);
        for file in ["pinfold.toml", "Containerfile", "home", "share"] {
            paths.push(dir.join(file));
        }
        paths.push(dir);
        if let Some(share) = &profile.share {
            paths.push(share.path()?);
        }
    }
    for path in paths {
        if source_beneath(&dirs::resolve_host_path(&path)?, &root)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "host-path-in-project: {} resolves inside {}",
                    crate::formatting::terminal_text(&path.to_string_lossy()),
                    crate::formatting::terminal_text(&root.to_string_lossy())
                ),
            ));
        }
    }
    Ok(())
}

/// Runs as the host uid, before credentials, claims or preparation writes.
pub(crate) fn prepare(plan: &mut Plan, profile: Option<&Profile>, init: &Path) -> io::Result<()> {
    let mut protection = Vec::new();
    for mount in &mut plan.mounts {
        mount.guest = guest_path(&mount.guest);
        if mount.kind == MountKind::Share {
            if !fs::symlink_metadata(&mount.host)?.is_dir() {
                return Err(invalid("share source must be a plain directory"));
            }
            mount.host = fs::canonicalize(&mount.host)?;
            check_paths_outside(&mount.host, profile, [])?;
            // A share inside a protected root exposes that root's contents too.
            for path in [
                dirs::config_dir()?,
                dirs::state_dir()?,
                dirs::cache_dir()?,
                super::ownership::host_dir()?,
            ] {
                if source_beneath(&mount.host, &dirs::resolve_host_path(&path)?)? {
                    return Err(invalid("share source is inside a protected host path"));
                }
            }
            let git = mount.host.join(".git");
            match fs::symlink_metadata(&git) {
                Ok(metadata) if metadata.is_dir() => protection.push(Mount {
                    host: git,
                    guest: mount.guest.join(".git"),
                    kind: MountKind::Readonly,
                    git: true,
                }),
                Ok(_) => return Err(invalid("share .git must be a plain directory")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        } else {
            mount.host = dirs::resolve_host_path(&mount.host)?;
        }
    }
    let init = guest_path(
        init.parent()
            .ok_or_else(|| invalid("init has no directory"))?,
    );
    if cfg!(target_os = "macos") {
        plan.validate_guests(&[&init]).map_err(invalid)?;
    }
    let guests: Vec<_> = plan
        .mounts
        .iter()
        .map(|m| m.guest.clone())
        .chain(plan.disks.iter().map(|d| guest_path(&d.guest)))
        .collect();
    for guest in &guests {
        if cfg!(target_os = "macos")
            && (overlaps(guest, &init)
                || (plan.egress.is_some() && overlaps(guest, Path::new("/var/host-services"))))
        {
            return Err(invalid(
                "attachment overlaps supplied init or forwarded socket",
            ));
        }
    }
    for (index, mount) in plan.mounts.iter().enumerate() {
        for git in &protection {
            let owner = mount.kind == MountKind::Share && mount.host.join(".git") == git.host;
            if !owner && overlaps(&mount.guest, &git.guest) {
                return Err(invalid("attachment overlaps protected Git destination"));
            }
            if source_beneath(&mount.host, &git.host)? {
                return Err(invalid("attachment source exposes protected Git metadata"));
            }
        }
        if mount.kind == MountKind::Share {
            for other in &plan.mounts[..index] {
                if other.kind == MountKind::Share
                    && (source_beneath(&mount.host, &other.host)?
                        || source_beneath(&other.host, &mount.host)?)
                {
                    return Err(invalid("writable share sources overlap"));
                }
            }
        }
        // A nested host attachment cannot be redirected by its parent share.
        for guest in &guests {
            if let Ok(relative) = guest.strip_prefix(&mount.guest) {
                check_destination(&mount.host, relative)?;
            }
        }
    }
    for disk in &plan.disks {
        for git in &protection {
            if overlaps(&guest_path(&disk.guest), &git.guest) {
                return Err(invalid("disk overlaps protected Git destination"));
            }
            if let Some(from) = &disk.from {
                let from = dirs::resolve_host_path(from)?;
                if source_beneath(&from, &git.host)? {
                    return Err(invalid("disk source exposes protected Git metadata"));
                }
            }
        }
    }
    let mut protected = BTreeSet::new();
    for git in &protection {
        walk(&git.host, None, &mut |path, metadata| {
            if metadata.st_mode & nix::libc::S_IFMT == nix::libc::S_IFLNK {
                return Err(invalid(format!(
                    "symlink in protected Git: {}",
                    path.display()
                )));
            }
            if metadata.st_mode & nix::libc::S_IFMT == nix::libc::S_IFREG && metadata.st_nlink > 1 {
                protected.insert((metadata.st_dev, metadata.st_ino));
            }
            Ok(())
        })?;
    }
    for share in plan.mounts.iter().filter(|m| m.kind == MountKind::Share) {
        let git = protection
            .iter()
            .find(|git| git.host == share.host.join(".git"))
            .map(|git| nix::sys::stat::lstat(&git.host).map(|m| (m.st_dev, m.st_ino)))
            .transpose()?;
        walk(&share.host, git, &mut |path, metadata| {
            if metadata.st_mode & nix::libc::S_IFMT == nix::libc::S_IFREG
                && protected.contains(&(metadata.st_dev, metadata.st_ino))
            {
                return Err(invalid(format!(
                    "writable alias of protected Git: {}",
                    path.display()
                )));
            }
            Ok(())
        })?;
    }
    plan.mounts.extend(protection);
    Ok(())
}

// realpath retains alternate-case spellings on some host filesystems.
fn source_beneath(path: &Path, root: &Path) -> io::Result<bool> {
    if path.starts_with(root) {
        return Ok(true);
    }
    let root = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if (metadata.dev(), metadata.ino()) == (root.dev(), root.ino()) => {
                return Ok(true);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

fn check_destination(root: &Path, relative: &Path) -> io::Result<()> {
    use nix::fcntl::{OFlag, open, openat};
    use nix::sys::stat::Mode;
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW;
    let mut directory = match open(root, flags, Mode::empty()) {
        Ok(directory) => directory,
        Err(nix::errno::Errno::ENOENT) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for component in relative.components() {
        if super::process::is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        directory = match openat(&directory, component.as_os_str(), flags, Mode::empty()) {
            Ok(directory) => directory,
            Err(nix::errno::Errno::ENOENT) => break,
            Err(error) => {
                return Err(invalid(format!(
                    "attachment destination is not a plain directory: {}: {error}",
                    root.join(relative).display()
                )));
            }
        };
    }
    Ok(())
}

fn walk(
    root: &Path,
    excluded: Option<(nix::libc::dev_t, nix::libc::ino_t)>,
    inspect: &mut impl FnMut(&Path, &nix::libc::stat) -> io::Result<()>,
) -> io::Result<()> {
    use nix::fcntl::{OFlag, open};
    use nix::sys::stat::Mode;
    let directory = open(
        root,
        OFlag::O_RDONLY | OFlag::O_CLOEXEC | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )?;
    walk_directory(directory, root, excluded, inspect)
}

fn walk_directory(
    directory: std::os::fd::OwnedFd,
    path: &Path,
    excluded: Option<(nix::libc::dev_t, nix::libc::ino_t)>,
    inspect: &mut impl FnMut(&Path, &nix::libc::stat) -> io::Result<()>,
) -> io::Result<()> {
    use nix::fcntl::{AtFlags, OFlag};
    use nix::sys::stat::{Mode, fstatat};
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::os::unix::ffi::OsStrExt;
    struct Directory(*mut nix::libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe {
                nix::libc::closedir(self.0);
            }
        }
    }
    let raw = directory.as_raw_fd();
    let pointer = unsafe { nix::libc::fdopendir(raw) };
    if pointer.is_null() {
        return Err(io::Error::last_os_error());
    }
    let _ = directory.into_raw_fd();
    let directory = Directory(pointer);
    let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(raw) };
    loop {
        if super::process::is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        nix::errno::Errno::clear();
        let entry = unsafe { nix::libc::readdir(directory.0) };
        if entry.is_null() {
            let error = nix::errno::Errno::last_raw();
            return if error == 0 {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(error))
            };
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        let name = std::ffi::OsStr::from_bytes(name);
        let child = path.join(name);
        let metadata = fstatat(fd, name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        if excluded == Some((metadata.st_dev, metadata.st_ino)) {
            continue;
        }
        inspect(&child, &metadata)?;
        if metadata.st_mode & nix::libc::S_IFMT == nix::libc::S_IFDIR {
            let child_fd = nix::fcntl::openat(
                fd,
                name,
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )?;
            walk_directory(child_fd, &child, excluded, inspect)?;
        }
    }
}

fn invalid(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail.into())
}
