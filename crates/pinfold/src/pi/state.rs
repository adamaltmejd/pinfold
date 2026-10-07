//! Per-project state: the project home and its state file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::{now, sha256_hex};
use crate::dirs;

/// What `state.json` records for Maintenance: the checkout to check and the
/// last run to age.
#[derive(Serialize, Deserialize)]
struct StateFile {
    /// The canonical project root.
    root: PathBuf,
    /// Seconds since the epoch.
    last_run: u64,
}

/// Create the project home and record this run in `state.json`; return the
/// home. `root` is canonical.
pub fn record_run(root: &Path) -> io::Result<PathBuf> {
    let home = project_home(root)?;
    fs::create_dir_all(&home)?;
    let state = StateFile {
        root: root.to_path_buf(),
        last_run: now(),
    };
    let json = serde_json::to_vec(&state).map_err(io::Error::other)?;
    fs::write(home.with_file_name("state.json"), json)?;
    Ok(home)
}

/// The project home `pinfold pi` mounts as the box's `$HOME`, computed
/// without creating it. `root` is canonical.
pub fn project_home(root: &Path) -> io::Result<PathBuf> {
    Ok(dirs::state_dir()?
        .join("projects")
        .join(project_id(root))
        .join("home"))
}

/// Canonicalize `path`, naming it in the error.
pub(crate) fn canonical(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("canonicalize {}: {error}", path.display()),
        )
    })
}

/// The id for an already canonical `root`. Trust records use the same id.
pub fn project_id(root: &Path) -> String {
    let mut name = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.replace(
                |ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')),
                "-",
            )
        })
        .unwrap_or_else(|| "root".to_string());
    // Leave room for pi-, the path hash and the pid in Apple's 63-byte ID.
    name.truncate(32);
    let hash = sha256_hex(root.as_os_str().as_encoded_bytes());
    format!("{name}-{}", &hash[..12])
}

/// One project state dir, as Maintenance sees it.
pub struct StateDir {
    /// The id under `projects/`, which box labels name.
    pub id: String,
    /// The dir under `projects/`, holding `state.json` and `home`.
    pub dir: PathBuf,
    /// The checkout the dir records.
    pub root: PathBuf,
    /// Seconds since the epoch of the last run.
    pub last_run: u64,
}

/// Every project state dir with a readable `state.json`. A dir without one
/// is a first start that failed before it recorded anything; it names no
/// checkout, so Maintenance leaves it alone.
pub fn state_dirs() -> io::Result<Vec<StateDir>> {
    Ok(dirs::entries(&dirs::state_dir()?.join("projects"))?
        .into_iter()
        .filter_map(|entry| {
            let dir = entry.path();
            let id = entry.file_name().to_str()?.to_string();
            let json = fs::read(dir.join("state.json")).ok()?;
            let state = serde_json::from_slice::<StateFile>(&json).ok()?;
            Some(StateDir {
                id,
                dir,
                root: state.root,
                last_run: state.last_run,
            })
        })
        .collect())
}
