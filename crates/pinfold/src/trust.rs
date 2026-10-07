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
use std::path::{Component, Path, PathBuf};

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
    validate_host_paths(root, &config)?;
    let trust = current(&config);
    let path = record_path(root)?;
    fs::create_dir_all(path.parent().expect("under the state dir"))?;
    let json = serde_json::to_vec(&trust).map_err(io::Error::other)?;
    fs::write(path, json)
}

/// Refuse when the project's config inputs no longer match the record.
pub fn check(root: &Path, config: &Config) -> io::Result<()> {
    validate_host_paths(root, config)?;
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

/// Host authority and executable inputs must stay outside the writable project.
pub fn validate_host_paths(root: &Path, config: &Config) -> io::Result<()> {
    let root = fs::canonicalize(root)?;
    let config_dir = dirs::config_dir()?;
    let state_dir = dirs::state_dir()?;
    let cache_dir = dirs::cache_dir()?;
    let mut paths = vec![
        config_dir.join("profiles").join(&config.profile.name),
        config_dir,
        state_dir.join("trust"),
        state::project_home(&root)?,
        state_dir,
        dirs::artifacts_dir()?,
        cache_dir,
        crate::core::ownership::host_dir()?,
    ];
    if let Some(share) = &config.profile.share {
        paths.push(share.path()?);
    }
    for path in paths {
        if resolve_host_path(&path)?.starts_with(&root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "host-path-in-project: {} resolves inside {}",
                    path.display(),
                    root.display()
                ),
            ));
        }
    }
    Ok(())
}

/// Resolve existing ancestors too, before a new XDG directory is created.
fn resolve_host_path(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink()) {
                let target = fs::read_link(path)?;
                return resolve_host_path(
                    &path.parent().expect("a symlink has a parent").join(target),
                );
            }
            let parent = path.parent().ok_or(error)?;
            let mut resolved = resolve_host_path(parent)?;
            match path.components().next_back() {
                Some(Component::Normal(name)) => resolved.push(name),
                Some(Component::ParentDir) => {
                    resolved.pop();
                }
                Some(Component::CurDir) => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "host path has no parent",
                    ));
                }
            }
            Ok(resolved)
        }
        Err(error) => Err(error),
    }
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

fn record_path(root: &Path) -> io::Result<PathBuf> {
    Ok(dirs::state_dir()?
        .join("trust")
        .join(format!("{}.json", state::project_id(root))))
}
