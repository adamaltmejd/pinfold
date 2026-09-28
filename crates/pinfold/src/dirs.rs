//! Host directories.
//!
//! The XDG variables win when they name an absolute path; otherwise the
//! literal XDG-style paths under the user's home are used on both macOS and
//! Linux.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::core::sha256_hex;

pub fn config_dir() -> io::Result<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn state_dir() -> io::Result<PathBuf> {
    xdg("XDG_STATE_HOME", ".local/state")
}

pub fn cache_dir() -> io::Result<PathBuf> {
    xdg("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_CACHE_HOME/pinfold/artifacts`: pinned release binaries and the
/// embedded init.
pub fn artifacts_dir() -> io::Result<PathBuf> {
    Ok(cache_dir()?.join("artifacts"))
}

/// `$XDG_STATE_HOME/pinfold/egress`: one JSON log per box, kept past the
/// box's teardown for Maintenance to age out.
pub fn egress_dir() -> io::Result<PathBuf> {
    Ok(state_dir()?.join("egress"))
}

/// `$XDG_STATE_HOME/pinfold/boxes`: the directory holding one state dir per
/// box.
pub fn boxes_dir() -> io::Result<PathBuf> {
    Ok(state_dir()?.join("boxes"))
}

/// `$XDG_STATE_HOME/pinfold/boxes/KEY`: the state dir `box up` claims for a
/// box. KEY is the first 16 hex digits of the sha256 of the name, so the
/// socket path's length does not depend on the name.
pub fn box_state_dir(name: &str) -> io::Result<PathBuf> {
    Ok(boxes_dir()?.join(&sha256_hex(name)[..16]))
}

/// The file an install writes into `dir`: one installed path per line,
/// relative to `dir`. It is written at the install root, so a payload that
/// is a subdirectory of `dir` (a harness) never mounts it.
const MANIFEST: &str = ".manifest";

/// Create `dir` in one step: `fill` writes into a staging sibling, which is
/// then renamed into place, so a failed or killed fill never leaves a
/// half-written directory where the next caller looks. A concurrent caller
/// may win the rename; its directory is accepted.
///
/// An existing `dir` counts only when [`installed`] accepts it. Otherwise it
/// is moved aside and filled again, so the next use heals a cache the OS or
/// a user damaged.
pub fn install_dir(dir: &Path, fill: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    if installed(dir)? {
        return Ok(());
    }
    if dir.is_dir() {
        let broken = dir.with_file_name(format!(".broken-{}", std::process::id()));
        let _ = fs::remove_dir_all(&broken);
        match fs::rename(dir, &broken) {
            Ok(()) => {
                let _ = fs::remove_dir_all(&broken);
            }
            // A concurrent caller moved it first.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let staging = dir.with_file_name(format!(".tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let filled = fill(&staging).and_then(|()| {
        fs::write(staging.join(MANIFEST), manifest(&staging)?)?;
        fs::rename(&staging, dir)
    });
    if let Err(error) = filled {
        let _ = fs::remove_dir_all(&staging);
        if !dir.is_dir() {
            return Err(error);
        }
    }
    Ok(())
}

/// Whether `dir` is a complete install: it holds a manifest, and every path
/// the manifest names, relative to `dir`, exists. A missing `dir` or
/// manifest is not an install.
pub fn installed(dir: &Path) -> io::Result<bool> {
    let manifest = match fs::read(dir.join(MANIFEST)) {
        Ok(manifest) => manifest,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(manifest
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .all(|line| dir.join(OsStr::from_bytes(line)).exists()))
}

/// The manifest bytes for the install staging at `dir`: every file below it,
/// relative to it, sorted, one per line.
fn manifest(dir: &Path) -> io::Result<Vec<u8>> {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(root, &path, files)?;
            } else {
                files.push(
                    path.strip_prefix(root)
                        .expect("a read_dir path stays under root")
                        .to_path_buf(),
                );
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    files.sort();
    let mut manifest = Vec::new();
    for file in files {
        manifest.extend_from_slice(file.as_os_str().as_bytes());
        manifest.push(b'\n');
    }
    Ok(manifest)
}

/// The entries of `dir`; a missing `dir` has none.
pub fn entries(dir: &Path) -> io::Result<Vec<fs::DirEntry>> {
    match fs::read_dir(dir) {
        Ok(entries) => entries.collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn xdg(variable: &str, fallback: &str) -> io::Result<PathBuf> {
    let base = match std::env::var_os(variable) {
        Some(value) if Path::new(&value).is_absolute() => PathBuf::from(value),
        _ => std::env::home_dir()
            .ok_or_else(|| io::Error::other("HOME is not set and the uid has no passwd entry"))?
            .join(fallback),
    };
    Ok(base.join("pinfold"))
}
