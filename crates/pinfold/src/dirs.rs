//! Host directories.
//!
//! The XDG variables win when they name an absolute path; otherwise the
//! literal XDG-style paths under the user's home are used on both macOS and
//! Linux. macOS applications usually use `~/Library`, but pinfold's paths are
//! the same on every host it runs on.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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

/// `$XDG_STATE_HOME/pinfold/boxes/NAME`: the state dir `box up` claims for
/// box NAME. An empty name is the directory that holds them all.
pub fn box_state_dir(name: &str) -> io::Result<PathBuf> {
    Ok(state_dir()?.join("boxes").join(name))
}

/// Create `dir` in one step: `fill` writes into a staging sibling, which is
/// then renamed into place, so a failed or killed fill never leaves a
/// half-written directory where the next caller looks. A concurrent caller
/// may win the rename; its directory is accepted.
pub fn install_dir(dir: &Path, fill: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let parent = dir
        .parent()
        .ok_or_else(|| io::Error::other(format!("{} has no parent", dir.display())))?;
    let staging = parent.join(format!(".tmp-{}", std::process::id()));
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
    match std::env::var_os(variable) {
        Some(value) if Path::new(&value).is_absolute() => Ok(PathBuf::from(value).join("pinfold")),
        _ => Ok(home_dir()?.join(fallback).join("pinfold")),
    }
}

/// `HOME`, else the passwd entry, which std reads for a stripped environment.
fn home_dir() -> io::Result<PathBuf> {
    std::env::home_dir()
        .ok_or_else(|| io::Error::other("HOME is not set and the uid has no passwd entry"))
}
