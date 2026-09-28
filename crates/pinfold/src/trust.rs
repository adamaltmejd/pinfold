//! Project trust: the project's `.pinfold.toml` and the project Containerfile
//! it names are used only when `pinfold allow` recorded their hashes.
//!
//! The record is one JSON file per project at
//! `~/.local/state/pinfold/trust/<project-id>.json`, keyed by the same id
//! the project state uses. An absent `.pinfold.toml` is recorded as an
//! absence, so creating one later is a change. A project with no
//! `.pinfold.toml` and no project Containerfile has nothing to trust and
//! runs; the profile's Containerfile is the user's file and is not trusted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::core::sha256_hex;
use crate::dirs;
use crate::pi::state;

/// The recorded hashes of a project's config inputs. `None` is a recorded
/// absence.
#[derive(Default, Serialize, Deserialize)]
struct Trust {
    /// sha256 of the project's `.pinfold.toml`.
    toml: Option<String>,
    /// sha256 of the project Containerfile, `None` when the project names
    /// none.
    containerfile: Option<String>,
}

/// Record the current hashes for the project rooted at `root`.
pub fn allow(root: &Path) -> io::Result<()> {
    let config = Config::load(root)?;
    let trust = current(&config);
    let path = record_path(root)?;
    fs::create_dir_all(path.parent().expect("under the state dir"))?;
    let json = serde_json::to_vec(&trust).map_err(io::Error::other)?;
    fs::write(path, json)
}

/// Refuse when the project's config inputs no longer match the record.
pub fn check(root: &Path, config: &Config) -> io::Result<()> {
    let current = current(config);
    let path = record_path(root)?;
    let stored: Trust = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Trust::default()),
        Err(error) => Err(error),
    }
    .map_err(|error| io::Error::new(error.kind(), format!("read {}: {error}", path.display())))?;
    let changed = if stored.toml != current.toml {
        ".pinfold.toml"
    } else if stored.containerfile != current.containerfile {
        "the project Containerfile"
    } else {
        return Ok(());
    };
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{changed} is new or changed; run `pinfold allow` to trust it"),
    ))
}

/// The hashes of the project's config inputs as they are now.
fn current(config: &Config) -> Trust {
    Trust {
        toml: config.project_toml.as_deref().map(sha256_hex),
        containerfile: config
            .containerfile
            .as_ref()
            .map(|(_, bytes)| sha256_hex(bytes)),
    }
}

/// The trust record for `root`: `<project-id>.json` under the state dir.
fn record_path(root: &Path) -> io::Result<PathBuf> {
    Ok(dirs::state_dir()?
        .join("trust")
        .join(format!("{}.json", state::project_id(root))))
}
