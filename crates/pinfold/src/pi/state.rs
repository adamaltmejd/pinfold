//! Per-project state: the state file and the names of the project's disks.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
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

/// `$HOME` in a pi box: the home disk's guest path.
pub const HOME: &str = "/home/pi";
pub const WORK: &str = "/work";

/// A scratch run is independent of both its configuration folder and PID.
pub fn scratch_id() -> io::Result<String> {
    let mut bytes = [0; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(format!("scratch-{}", &sha256_hex(bytes)[..24]))
}

pub fn scratch_disk_names(id: &str) -> [String; 2] {
    [format!("pi-{id}-home"), format!("pi-{id}-work")]
}

/// Scratch origins identify disks without making the folder a project.
pub fn record_scratch(id: &str, root: &Path) -> io::Result<()> {
    let dir = dirs::state_dir()?.join("scratch");
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_vec(&StateFile {
            root: root.to_path_buf(),
            last_run: now(),
        })?,
    )
}

fn scratch_records() -> io::Result<Vec<(String, PathBuf, StateFile)>> {
    let mut records = Vec::new();
    for entry in dirs::entries(&dirs::state_dir()?.join("scratch"))? {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(nonce) = id.strip_prefix("scratch-") else {
            continue;
        };
        if path.extension().is_none_or(|extension| extension != "json")
            || nonce.len() != 24
            || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(state) = serde_json::from_slice::<StateFile>(&bytes) else {
            continue;
        };
        records.push((id.to_string(), path, state));
    }
    Ok(records)
}

pub fn disk_origins() -> io::Result<BTreeMap<String, PathBuf>> {
    let mut origins = BTreeMap::new();
    for project in state_dirs()? {
        for name in disk_names(&project.id) {
            origins.insert(name, project.root.clone());
        }
    }
    for (id, _, state) in scratch_records()? {
        for name in scratch_disk_names(&id) {
            origins.insert(name, state.root.clone());
        }
    }
    Ok(origins)
}

/// Removing one half keeps the other's origin available.
pub fn forget_scratch_origin(removed: &str) -> io::Result<()> {
    let Some(id) = removed.strip_prefix("pi-").and_then(|name| {
        name.strip_suffix("-home")
            .or_else(|| name.strip_suffix("-work"))
    }) else {
        return Ok(());
    };
    let Some(nonce) = id.strip_prefix("scratch-") else {
        return Ok(());
    };
    if nonce.len() != 24 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    let names = scratch_disk_names(id);
    let Some(_locks) = crate::core::disk::RemovalGuard::try_take(&names)? else {
        return Ok(());
    };
    if crate::core::runtime::runtime()
        .list_disks()?
        .iter()
        .all(|disk| !names.contains(&disk.name))
    {
        crate::core::absent_ok(fs::remove_file(
            dirs::state_dir()?
                .join("scratch")
                .join(format!("{id}.json")),
        ))?;
    }
    Ok(())
}

/// Record this run in the project's `state.json`. `root` is canonical.
pub fn record_run(root: &Path) -> io::Result<()> {
    let dir = dirs::state_dir()?.join("projects").join(project_id(root));
    fs::create_dir_all(&dir)?;
    let state = StateFile {
        root: root.to_path_buf(),
        last_run: now(),
    };
    let json = serde_json::to_vec(&state).map_err(io::Error::other)?;
    fs::write(dir.join("state.json"), json)
}

/// The project's home disk, attached at [`HOME`], and its project disk,
/// attached at the project's own path. `id` is a [`project_id`].
pub fn disk_names(id: &str) -> [String; 2] {
    [format!("pi-{id}-home"), format!("pi-{id}-project")]
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
    /// The dir under `projects/`, holding `state.json`.
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
