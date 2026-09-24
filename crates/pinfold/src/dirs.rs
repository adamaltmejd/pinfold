//! Host directories.
//!
//! The XDG variables win when they name an absolute path; otherwise the
//! literal XDG-style paths under the user's home are used on both macOS and
//! Linux. macOS applications usually use `~/Library`, but pinfold's paths are
//! the same on every host it runs on.

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

fn xdg(variable: &str, fallback: &str) -> io::Result<PathBuf> {
    match std::env::var_os(variable) {
        Some(value) if Path::new(&value).is_absolute() => Ok(PathBuf::from(value).join("pinfold")),
        _ => Ok(home_dir()?.join(fallback).join("pinfold")),
    }
}

fn home_dir() -> io::Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    // HOME can be unset in a stripped environment; the passwd entry still
    // names the user's home.
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .map_err(io::Error::other)?
        .map(|user| user.dir)
        .ok_or_else(|| io::Error::other("HOME is not set and the uid has no passwd entry"))
}
