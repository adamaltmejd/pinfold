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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
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
#[derive(Serialize, Deserialize)]
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

/// One project state dir, as Maintenance sees it.
pub struct StateDir {
    /// The id under `projects/`, which box labels name.
    pub id: String,
    /// The dir under `projects/`, holding `state.json` and `home`.
    pub dir: PathBuf,
    /// The `$HOME` mounted into the project's boxes.
    pub home: PathBuf,
    /// The checkout the dir records.
    pub root: PathBuf,
    /// Seconds since the epoch of the last run.
    pub last_run: u64,
}

impl StateDir {
    /// Whether `clean` should remove this state: the checkout is gone, or
    /// with `unused`, the project has not run for that long.
    pub fn stale(&self, unused: Option<Duration>) -> bool {
        if !self.root.exists() {
            return true;
        }
        let Some(unused) = unused else {
            return false;
        };
        now().saturating_sub(self.last_run) > unused.as_secs()
    }
}

/// Every project state dir with a readable `state.json`. A dir without one
/// is a first start that failed before it recorded anything; it names no
/// checkout, so Maintenance leaves it alone.
pub fn state_dirs() -> io::Result<Vec<StateDir>> {
    let projects = dirs::state_dir()?.join("projects");
    let entries = match fs::read_dir(&projects) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry?;
        let dir = entry.path();
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(json) = fs::read(dir.join("state.json")) else {
            continue;
        };
        let Ok(state) = serde_json::from_slice::<StateFile>(&json) else {
            continue;
        };
        dirs.push(StateDir {
            id,
            home: dir.join("home"),
            dir,
            root: state.root,
            last_run: state.last_run,
        });
    }
    Ok(dirs)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
