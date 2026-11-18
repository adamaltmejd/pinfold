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
    bidirectional: bool,
) -> io::Result<()> {
    check_extra_paths_outside(root, protected_paths(profile, extra)?, bidirectional)
}

/// The pi layer adds its resolved project configuration paths to share admission.
pub(crate) fn check_extra_paths_outside(
    root: &Path,
    paths: impl IntoIterator<Item = PathBuf>,
    bidirectional: bool,
) -> io::Result<()> {
    let root = fs::canonicalize(root)?;
    for path in paths {
        if source_beneath(&path, &root)? || (bidirectional && source_beneath(&root, &path)?) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "host-path-in-project: {} {} {}",
                    crate::formatting::terminal_text(&path.to_string_lossy()),
                    if bidirectional {
                        "overlaps"
                    } else {
                        "resolves inside"
                    },
                    crate::formatting::terminal_text(&root.to_string_lossy())
                ),
            ));
        }
    }
    Ok(())
}

fn protected_paths(
    profile: Option<&Profile>,
    extra: impl IntoIterator<Item = PathBuf>,
) -> io::Result<BTreeSet<PathBuf>> {
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
    paths
        .into_iter()
        .map(|path| dirs::resolve_host_path(&path))
        .collect()
}

struct ProtectedGit {
    owner: usize,
    host: PathBuf,
    guest: PathBuf,
}

/// Runs as the host uid, before credentials, claims or preparation writes.
pub(crate) fn prepare(plan: &mut Plan, profile: Option<&Profile>, init: &Path) -> io::Result<()> {
    let shares = plan
        .mounts
        .iter()
        .any(|mount| mount.kind == MountKind::Share);
    if !shares {
        return Ok(());
    }
    let init = guest_path(
        init.parent()
            .ok_or_else(|| invalid("init has no directory"))?,
    );
    let paths = protected_paths(profile, [dirs::resolve_host_path(&init)?])?;
    let mut protection = Vec::new();
    for (owner, mount) in plan.mounts.iter_mut().enumerate() {
        mount.guest = guest_path(&mount.guest);
        if mount.kind == MountKind::Share {
            if mount.guest.file_name() == Some(std::ffi::OsStr::new("*")) {
                return Err(invalid(
                    "share destination would be a Git safe.directory pattern",
                ));
            }
            if !fs::symlink_metadata(&mount.host)?.is_dir() {
                return Err(invalid("share source must be a plain directory"));
            }
            mount.host = fs::canonicalize(&mount.host)?;
            check_extra_paths_outside(&mount.host, paths.iter().cloned(), true)?;
            if overlaps(&mount.guest, &init)
                || (plan.egress.is_some()
                    && overlaps(&mount.guest, Path::new("/var/host-services")))
            {
                return Err(invalid("share overlaps supplied init or forwarded socket"));
            }
            let git = mount.host.join(".git");
            match fs::symlink_metadata(&git) {
                Ok(metadata) if metadata.is_dir() => protection.push(ProtectedGit {
                    owner,
                    host: git,
                    guest: mount.guest.join(".git"),
                }),
                Ok(_) => return Err(invalid("share .git must be a plain directory")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        } else {
            mount.host = dirs::resolve_host_path(&mount.host)?;
        }
    }
    let guests: Vec<_> = plan
        .mounts
        .iter()
        .map(|m| (m.guest.clone(), m.kind == MountKind::Share))
        .chain(plan.disks.iter().map(|d| (guest_path(&d.guest), false)))
        .collect();
    for (index, mount) in plan.mounts.iter().enumerate() {
        for git in &protection {
            if index != git.owner && overlaps(&mount.guest, &git.guest) {
                return Err(invalid("attachment overlaps protected Git destination"));
            }
            if source_beneath(&mount.host, &git.host)? {
                return Err(invalid("attachment source exposes protected Git metadata"));
            }
            if mount.kind == MountKind::Share && source_beneath(&git.host, &mount.host)? {
                let own = protection.iter().find(|other| other.owner == index);
                let protected_here = match own {
                    Some(own) => same_inode(&own.host, &git.host)?,
                    None => false,
                };
                if !protected_here {
                    return Err(invalid(
                        "overlapping share exposes protected Git metadata writable",
                    ));
                }
            }
        }
        let git = protection
            .iter()
            .find(|git| git.owner == index)
            .map(|git| nix::sys::stat::lstat(&git.host).map(|m| (m.st_dev, m.st_ino)))
            .transpose()?;
        for (guest, child_share) in &guests {
            if (mount.kind == MountKind::Share || *child_share)
                && let Ok(relative) = guest.strip_prefix(&mount.guest)
            {
                check_destination(&mount.host, relative, git)?;
            }
        }
    }
    for disk in &plan.disks {
        for git in &protection {
            if overlaps(&guest_path(&disk.guest), &git.guest) {
                return Err(invalid("disk overlaps protected Git destination"));
            }
            let share = &plan.mounts[git.owner];
            if let Ok(relative) = guest_path(&disk.guest).strip_prefix(&share.guest) {
                let metadata = nix::sys::stat::lstat(&git.host)?;
                check_destination(
                    &share.host,
                    relative,
                    Some((metadata.st_dev, metadata.st_ino)),
                )?;
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
    plan.mounts.extend(protection.into_iter().map(|git| Mount {
        host: git.host,
        guest: git.guest,
        kind: MountKind::Readonly,
        git: true,
    }));
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

fn same_inode(a: &Path, b: &Path) -> io::Result<bool> {
    let a = fs::symlink_metadata(a)?;
    let b = fs::symlink_metadata(b)?;
    Ok((a.dev(), a.ino()) == (b.dev(), b.ino()))
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

fn check_destination(
    root: &Path,
    relative: &Path,
    git: Option<(nix::libc::dev_t, nix::libc::ino_t)>,
) -> io::Result<()> {
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
        let metadata = nix::sys::stat::fstat(&directory)?;
        if git == Some((metadata.st_dev, metadata.st_ino)) {
            return Err(invalid(
                "attachment destination touches protected Git directory",
            ));
        }
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
    super::directory_entries(directory, |fd, name| {
        if super::process::is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let child = path.join(name);
        let metadata = fstatat(fd, name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        if excluded == Some((metadata.st_dev, metadata.st_ino)) {
            return Ok(());
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
        Ok(())
    })
}

fn invalid(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail.into())
}
