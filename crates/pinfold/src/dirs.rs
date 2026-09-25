//! Host directories.
//!
//! The XDG variables win when they name an absolute path; otherwise the
//! literal XDG-style paths under the user's home are used on both macOS and
//! Linux.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::sha256_hex;

/// `$XDG_CONFIG_HOME/pinfold`, else `~/.config/pinfold`.
pub fn config_dir() -> io::Result<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_STATE_HOME/pinfold`, else `~/.local/state/pinfold`.
pub fn state_dir() -> io::Result<PathBuf> {
    xdg("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_CACHE_HOME/pinfold`, else `~/.cache/pinfold`.
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

/// Create `dir` in one step: `fill` writes into a staging sibling, which is
/// then renamed into place, so a failed or killed fill never leaves a
/// half-written directory where the next caller looks. A concurrent caller
/// may win the rename; its directory is accepted.
pub fn install_dir(dir: &Path, fill: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let staging = dir.with_file_name(format!(".tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    if let Err(error) = fill(&staging).and_then(|()| fs::rename(&staging, dir)) {
        let _ = fs::remove_dir_all(&staging);
        if !dir.is_dir() {
            return Err(error);
        }
    }
    Ok(())
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
