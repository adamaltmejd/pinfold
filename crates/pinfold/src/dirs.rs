//! Host directories.
//!
//! The XDG variables win when they name an absolute path; otherwise the
//! literal XDG-style paths under `$HOME` are used on both macOS and Linux.
//! macOS applications usually use `~/Library`, but pinfold's paths are the
//! same on every host it runs on.

use std::path::{Path, PathBuf};

/// `$XDG_CONFIG_HOME/pinfold`, else `~/.config/pinfold`.
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_STATE_HOME/pinfold`, else `~/.local/state/pinfold`.
pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_CACHE_HOME/pinfold`, else `~/.cache/pinfold`.
pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache")
}

fn xdg(variable: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(variable) {
        Some(value) if Path::new(&value).is_absolute() => PathBuf::from(value).join("pinfold"),
        _ => home_dir().join(fallback).join("pinfold"),
    }
}

fn home_dir() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}
