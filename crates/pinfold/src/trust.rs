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
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::core::hex;
use crate::dirs;
use crate::pi::state;

/// The project config file trust covers.
const TOML_FILE: &str = ".pinfold.toml";

/// The recorded hashes of a project's config inputs. `None` is a recorded
/// absence.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec(&trust).map_err(io::Error::other)?;
    fs::write(path, json)
}

/// Refuse when the project's config inputs no longer match the record. No
/// record is a record of two absences: a project with no `.pinfold.toml`
/// and no project Containerfile has nothing to trust.
pub fn check(root: &Path, config: &Config) -> io::Result<()> {
    let current = current(config);
    let stored = read(root)?.unwrap_or_default();
    if stored.toml != current.toml {
        return Err(refused(format!(
            "{TOML_FILE} is new or changed; run `pinfold allow` to trust it"
        )));
    }
    if stored.containerfile != current.containerfile {
        return Err(refused(
            "the project Containerfile is new or changed; run `pinfold allow` to trust it",
        ));
    }
    Ok(())
}

/// The hashes of the project's config inputs as they are now.
fn current(config: &Config) -> Trust {
    Trust {
        // Hash the bytes `Config` read, not a second read a live box could
        // rewrite in between.
        toml: config.project_toml.as_deref().map(hash),
        containerfile: config.containerfile.as_deref().map(hash),
    }
}

/// The recorded hashes for `root`, or `None` when nothing was recorded.
fn read(root: &Path) -> io::Result<Option<Trust>> {
    let path = record_path(root)?;
    match fs::read(&path) {
        Ok(bytes) => {
            let trust = serde_json::from_slice(&bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("read {}: {error}", path.display()),
                )
            })?;
            Ok(Some(trust))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("read {}: {error}", path.display()),
        )),
    }
}

/// The trust record for `root`: `<project-id>.json` under the state dir.
fn record_path(root: &Path) -> io::Result<PathBuf> {
    Ok(dirs::state_dir()?
        .join("trust")
        .join(format!("{}.json", state::project_id(root)?)))
}

/// The sha256 of `bytes`, hex-encoded.
fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn refused(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.into())
}
