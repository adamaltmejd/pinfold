//! Automatic maintenance: pinfold only removes what it created.
//!
//! After a build, keep the newest two images per source. At most once a day,
//! at the start of any command, prune leftovers: boxes whose owner is gone,
//! sockets in state dirs no live owner holds, artifact versions no pin names,
//! and egress logs older than 14 days. The pass reports a problem on stderr
//! and never fails the command it runs before.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use nix::fcntl::{Flock, FlockArg};
use nix::sys::signal::kill;
use nix::unistd::Pid;

use crate::core::artifacts;
use crate::core::runtime::{Runtime, runtime};
use crate::dirs;

/// The label naming a profile source on an image.
pub const PROFILE_LABEL: &str = "dev.pinfold.profile";
/// The label naming a project source on an image, and the project on a box.
pub const PROJECT_LABEL: &str = "dev.pinfold.project";
/// The label recording the digest of the image an image was built from.
pub const BASE_LABEL: &str = "dev.pinfold.base";
/// The label naming the build that produced an image. Its value starts with
/// the build's nanoseconds since the epoch in hex, so it orders builds.
pub const BUILD_LABEL: &str = "dev.pinfold.build";
/// The label naming the `box up` process that owns a box.
pub const OWNER_LABEL: &str = "dev.pinfold.owner";

/// Egress logs older than this are removed.
const EGRESS_LOG_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
/// The daily pass runs at most once in this interval.
const PASS_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Run the daily pass if a day has passed. A problem goes to stderr; the
/// caller's command continues.
pub fn maintain() {
    if let Err(error) = maintain_due() {
        eprintln!("pinfold: maintenance: {error}");
    }
}

fn maintain_due() -> io::Result<()> {
    let state = dirs::state_dir()?;
    let stamp = state.join("maintenance");
    let now = unix_seconds();
    if let Some(last) = fs::read_to_string(&stamp)
        .ok()
        .and_then(|last| last.trim().parse::<u64>().ok())
        && now.saturating_sub(last) < PASS_INTERVAL.as_secs()
    {
        return Ok(());
    }
    // Claim the day before the work, so a failed pass waits for the next day
    // instead of running before every command.
    fs::create_dir_all(&state)?;
    fs::write(&stamp, now.to_string())?;
    daily();
    Ok(())
}

/// The pass itself. Every step is attempted; a failure is reported and the
/// rest continue.
fn daily() {
    match runtime() {
        Ok(runtime) => report("boxes", prune_boxes(runtime).map(drop)),
        Err(error) => eprintln!("pinfold: maintenance: boxes: {error}"),
    }
    report("sockets", prune_sockets());
    report("artifacts", artifacts::prune_unpinned());
    report("egress logs", prune_egress_logs());
}

fn report(what: &str, result: io::Result<()>) {
    if let Err(error) = result {
        eprintln!("pinfold: maintenance: {what}: {error}");
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Whether the `box up` that claimed `state_dir` is alive: it holds the lock
/// on the dir's `pid` file. A missing or empty `pid` is a start between its
/// claim and its lock, so it counts as alive. Pid reuse and EPERM cannot
/// make a dead owner look alive.
pub fn owner_alive(state_dir: &Path) -> bool {
    let Ok(file) = File::open(state_dir.join("pid")) else {
        return true;
    };
    let Ok(mut file) = Flock::lock(file, FlockArg::LockExclusiveNonblock) else {
        return true;
    };
    let mut pid = String::new();
    if file.read_to_string(&mut pid).is_err() {
        return true;
    }
    pid.trim().is_empty()
}

/// Whether a listed box's owner is alive. A box whose state dir exists under
/// this state root is judged by its lock. One from another state root keeps
/// the label pid check: judging it by a lock this root cannot see would call
/// every other root's live box dead.
pub fn box_owner_alive(state_dir: &Path, owner: Option<i32>) -> bool {
    if state_dir.is_dir() {
        owner_alive(state_dir)
    } else {
        owner.is_some_and(pid_alive)
    }
}

/// Whether a process with this pid exists. Only for a box from another state
/// root; everything else goes by [`owner_alive`].
fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    match kill(Pid::from_raw(pid), None) {
        Ok(()) | Err(nix::errno::Errno::EPERM) => true,
        Err(_) => false,
    }
}

/// A pinfold box whose owning `box up` process is gone: the runtime removes
/// the box, and its state dir goes with it.
pub struct DeadBox {
    /// The runtime's container id.
    pub id: String,
    /// The state dir `box up` created for it.
    pub state_dir: PathBuf,
    /// The `dev.pinfold.owner` pid, when the label parsed.
    pub owner: Option<i32>,
}

impl DeadBox {
    /// Take the box down and remove its state dir.
    pub fn remove(&self, runtime: &dyn Runtime) -> io::Result<()> {
        runtime.down(&self.id)?;
        let _ = fs::remove_dir_all(&self.state_dir);
        Ok(())
    }
}

/// One runtime box list, split into the work for Maintenance: the pinfold
/// boxes whose owner is gone, and the projects whose box is live.
pub struct Boxes {
    /// The pinfold boxes whose owning `box up` process is gone.
    pub dead: Vec<DeadBox>,
    /// The `dev.pinfold.project` ids of pinfold boxes whose owner is alive.
    pub live_projects: BTreeSet<String>,
}

/// Read the runtime's box list once. A pinfold box with neither a state dir
/// here nor an owner label counts as gone: nothing holds it.
pub fn boxes(runtime: &dyn Runtime) -> io::Result<Boxes> {
    let state = dirs::state_dir()?.join("boxes");
    let mut dead = Vec::new();
    let mut live_projects = BTreeSet::new();
    for box_ in runtime.list()? {
        if !box_
            .labels
            .keys()
            .any(|key| key.starts_with("dev.pinfold."))
        {
            continue;
        }
        let owner = box_
            .labels
            .get(OWNER_LABEL)
            .and_then(|pid| pid.parse::<i32>().ok());
        let state_dir = state.join(&box_.id);
        if box_owner_alive(&state_dir, owner) {
            if let Some(project) = box_.labels.get(PROJECT_LABEL) {
                live_projects.insert(project.clone());
            }
            continue;
        }
        dead.push(DeadBox {
            state_dir,
            id: box_.id,
            owner,
        });
    }
    Ok(Boxes {
        dead,
        live_projects,
    })
}

/// Remove boxes pinfold labeled whose owning `box up` process is gone, and
/// the state dirs that name them. Return the removed boxes, so the caller
/// can report each removal.
pub fn prune_boxes(runtime: &dyn Runtime) -> io::Result<Vec<DeadBox>> {
    let dead = boxes(runtime)?.dead;
    for box_ in &dead {
        box_.remove(runtime)?;
    }
    Ok(dead)
}

/// State dirs whose owner is gone and that hold a leftover proxy socket. A
/// live `box up` locks its `pid` file before it binds the socket, so a
/// socket whose `pid` no lock holds is leftover.
pub fn leftover_socket_dirs() -> io::Result<Vec<PathBuf>> {
    let boxes = dirs::state_dir()?.join("boxes");
    let entries = match fs::read_dir(&boxes) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut leftover = Vec::new();
    for entry in entries {
        let entry = entry?;
        let dir = entry.path();
        if owner_alive(&dir) {
            continue;
        }
        // A dir without a socket is a start that failed before it could
        // listen; it holds nothing worth reclaiming, and removing it could
        // race a start.
        if fs::symlink_metadata(dir.join("proxy.sock")).is_ok() {
            leftover.push(dir);
        }
    }
    Ok(leftover)
}

/// Remove state dirs whose owner is gone.
pub fn prune_sockets() -> io::Result<()> {
    for dir in leftover_socket_dirs()? {
        let _ = fs::remove_dir_all(&dir);
    }
    Ok(())
}

/// Egress logs older than [`EGRESS_LOG_AGE`].
pub fn old_egress_logs() -> io::Result<Vec<PathBuf>> {
    let dir = dirs::egress_dir()?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let Some(cutoff) = SystemTime::now().checked_sub(EGRESS_LOG_AGE) else {
        return Ok(Vec::new());
    };
    let mut old = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() && metadata.modified().is_ok_and(|modified| modified < cutoff) {
            old.push(entry.path());
        }
    }
    Ok(old)
}

/// Remove egress logs older than [`EGRESS_LOG_AGE`].
pub fn prune_egress_logs() -> io::Result<()> {
    for log in old_egress_logs()? {
        fs::remove_file(log)?;
    }
    Ok(())
}

/// The bytes a file or directory holds, without following symlinks. An
/// unreadable entry counts as nothing.
pub fn path_bytes(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            total += path_bytes(&entry.path());
        }
    }
    total
}

/// The total bytes of `paths`.
pub fn total_bytes<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> u64 {
    paths.into_iter().map(|path| path_bytes(path)).sum()
}

/// Keep the newest two images carrying `label = source`, removing older ones
/// and the layers no image references. Called after a successful build. Every
/// older image is tried; a failure keeps that image and the rest still run,
/// so one in-use image never stops the others from going. An image a listed
/// box reports is never offered to the runtime: Apple's delete would remove
/// it under the box, so pinfold skips it itself and reports it like a failed
/// removal.
pub fn keep_two_images(runtime: &dyn Runtime, label: &str, source: &str) -> io::Result<()> {
    // One list before anything goes: the ids of the images boxes pin.
    let mut in_use: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for box_ in runtime.list()? {
        in_use.entry(box_.image_id).or_default().insert(box_.id);
    }
    let mut groups: BTreeMap<String, (u128, Vec<String>)> = BTreeMap::new();
    for image in runtime.list_images()? {
        if image.labels.get(label).map(String::as_str) != Some(source) {
            continue;
        }
        let rank = build_rank(&image.labels);
        let group = groups.entry(image.id).or_insert((0, Vec::new()));
        group.0 = group.0.max(rank);
        group.1.push(image.reference);
    }
    let mut images: Vec<(u128, String, Vec<String>)> = groups
        .into_iter()
        .map(|(id, (rank, references))| (rank, id, references))
        .collect();
    // Newest first; the digest orders equal ranks deterministically.
    images.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let mut failures = Vec::new();
    for (_, id, references) in images.into_iter().skip(2) {
        if let Some(boxes) = in_use.get(&id) {
            // Count a skip like a failed removal: the image stays, the next
            // build tries again, and the one line names it and its boxes.
            for reference in references {
                failures.push(format!(
                    "{reference}: in use by box {}",
                    boxes.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            continue;
        }
        for reference in references {
            if let Err(error) = runtime.remove_image(&reference) {
                // Every removal carries its reference in the runtime's
                // error; join them into the one line the caller prints.
                failures.push(error.to_string());
            }
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    Err(io::Error::other(failures.join("; ")))
}

/// The build time of an image from its build label, which starts with the
/// build's nanoseconds since the epoch in hex.
fn build_rank(labels: &BTreeMap<String, String>) -> u128 {
    labels
        .get(BUILD_LABEL)
        .and_then(|value| value.split('-').next())
        .and_then(|nanos| u128::from_str_radix(nanos, 16).ok())
        .unwrap_or(0)
}
