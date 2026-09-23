//! Per-project state: the project home and its state file.
//!
//! A project is its canonical root path. The id is
//! `<sanitized-name>-<short-hash-of-root>`, so two checkouts with the same
//! directory name stay apart, and a moved checkout is a new project. The
//! home lives outside the checkout, under the state dir, and belongs to the
//! project from its first start.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::dirs;

/// Hex characters of the root hash in a project id.
const HASH_LENGTH: usize = 12;

/// One project's state: its id, root and `$HOME`.
pub struct ProjectState {
    /// `<sanitized-name>-<short-hash>`.
    pub id: String,
    /// The canonical project root the id was computed from.
    pub root: PathBuf,
    /// The project home, mounted as the box's `$HOME`.
    pub home: PathBuf,
}

/// What `state.json` records for Maintenance: the checkout to check and the
/// last run to age.
#[derive(Serialize)]
struct StateFile {
    /// The canonical project root.
    root: PathBuf,
    /// Seconds since the epoch.
    last_run: u64,
}

impl ProjectState {
    /// Load this project's state, creating the home and refreshing
    /// `state.json`.
    pub fn load_or_create(root: &Path) -> io::Result<ProjectState> {
        let root = canonical(root)?;
        let id = id_for(&root);
        let dir = dirs::state_dir()?.join("projects").join(&id);
        let home = dir.join("home");
        fs::create_dir_all(&home)?;
        let state = StateFile {
            root: root.clone(),
            last_run: now(),
        };
        let json = serde_json::to_vec(&state).map_err(io::Error::other)?;
        fs::write(dir.join("state.json"), json)?;
        Ok(ProjectState { id, root, home })
    }
}

/// The id for `root`, canonicalizing it first. Trust records use the same
/// id.
pub fn project_id(root: &Path) -> io::Result<String> {
    Ok(id_for(&canonical(root)?))
}

fn canonical(root: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(root).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("canonicalize {}: {error}", root.display()),
        )
    })
}

/// The id for an already canonical `root`.
fn id_for(root: &Path) -> String {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "root".to_string());
    let mut hasher = Sha256::new();
    hasher.update(root.as_os_str().as_encoded_bytes());
    let mut hash = String::with_capacity(HASH_LENGTH);
    for byte in hasher.finalize().iter().take(HASH_LENGTH / 2) {
        write!(hash, "{byte:02x}").expect("writing to a string cannot fail");
    }
    format!("{name}-{hash}")
}

/// A path component: ASCII alphanumerics, `-`, `_` and `.` stay, everything
/// else becomes `-`.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
