//! Share admission.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::plan::{MountKind, Plan, guest_path};
use super::profile::{Profile, Share};
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
    // `artifacts/init` may be a symlink resolving elsewhere. Its top always
    // resolves, even as a stray file; `prepare` names the init's own
    // directory. On Linux the init is pinfold's own executable: protecting
    // its directory would refuse a checkout holding a pinfold build.
    if cfg!(target_os = "macos") {
        paths.push(dirs::artifacts_dir()?.join("init"));
    }
    paths.extend(extra);
    if let Some(profile) = profile {
        // Every profile in the chain configures the box.
        for link in profile.links() {
            let dir = config.join("profiles").join(&link.name);
            for file in ["pinfold.toml", "Containerfile", "home", "share"] {
                paths.push(dir.join(file));
            }
            paths.push(dir);
        }
        // A copied share lives in the box's state dir, under the state root.
        if let Some(Share::Directory(path)) = &profile.share {
            paths.push(path.clone());
        }
    }
    paths
        .into_iter()
        .map(|path| dirs::resolve_host_path(&path))
        .collect()
}

/// Runs as the host uid, before credentials, claims or preparation writes.
pub(crate) fn prepare(plan: &mut Plan, profile: Option<&Profile>, init: &Path) -> io::Result<()> {
    let init = guest_path(
        init.parent()
            .ok_or_else(|| invalid("init has no directory"))?,
    );
    // Apple attaches init's directory at its host path, and later execs
    // resolve init through it; box work must not be able to rename an
    // ancestor. Firecracker boots init from its own detached disk.
    let supplied = |guest: &Path| {
        overlaps(guest, &init)
            || (plan.egress.is_some() && overlaps(guest, Path::new("/var/host-services")))
    };
    if cfg!(target_os = "macos")
        && plan
            .disks
            .iter()
            .any(|disk| !disk.readonly && supplied(&guest_path(&disk.guest)))
    {
        return Err(invalid(
            "writable disk overlaps supplied init or forwarded socket",
        ));
    }
    let shares = plan
        .mounts
        .iter()
        .any(|mount| mount.kind == MountKind::Share);
    if !shares {
        return Ok(());
    }
    let paths = protected_paths(profile, [dirs::resolve_host_path(&init)?])?;
    for mount in plan.mounts.iter_mut() {
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
            if supplied(&mount.guest) {
                return Err(invalid("share overlaps supplied init or forwarded socket"));
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
    for mount in &plan.mounts {
        for (guest, child_share) in &guests {
            if (mount.kind == MountKind::Share || *child_share)
                && let Ok(relative) = guest.strip_prefix(&mount.guest)
            {
                check_destination(&mount.host, relative)?;
            }
        }
    }
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

fn invalid(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail.into())
}
