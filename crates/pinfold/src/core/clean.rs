//! Automatic maintenance: pinfold only removes what it created.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nix::fcntl::{Flock, FlockArg};
use nix::sys::signal::kill;
use nix::unistd::Pid;

use crate::core::artifacts;
use crate::core::runtime::{BoxInfo, ImageInfo, Runtime, runtime};
use crate::dirs;

/// The label naming a profile source on an image.
pub const PROFILE_LABEL: &str = "dev.pinfold.profile";
/// The label naming a project source on an image, and the project on a box.
pub const PROJECT_LABEL: &str = "dev.pinfold.project";
/// The label naming a caller image's source: the name it built.
pub const IMAGE_LABEL: &str = "dev.pinfold.image";
/// The three labels naming an image's source, one per build family.
pub const FAMILY_LABELS: [&str; 3] = [PROFILE_LABEL, PROJECT_LABEL, IMAGE_LABEL];
/// The label podman puts on the intermediate images of a cached build, so
/// `clean` prunes only pinfold's build cache.
pub const LAYER_LABEL: &str = "dev.pinfold.layer";
/// The label recording the digest of the image an image was built from.
pub const BASE_LABEL: &str = "dev.pinfold.base";
/// The label naming the `box up` process that owns a box.
pub const OWNER_LABEL: &str = "dev.pinfold.owner";

/// Egress logs older than this are removed.
const EGRESS_LOG_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
/// The daily pass runs at most once in this interval.
const PASS_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// A caller build tag made within this window is never removed: the ref its
/// `built` line named must still come up when the caller uses it later.
const CALLER_IMAGE_GRACE: Duration = Duration::from_secs(60 * 60);

/// Run the daily pass if a day has passed. A problem goes to stderr; the
/// caller's command continues.
pub fn maintain() {
    if let Err(error) = maintain_due() {
        eprintln!("pinfold: maintenance: {error}");
    }
}

/// The stamp's mtime is the last pass.
fn maintain_due() -> io::Result<()> {
    let state = dirs::state_dir()?;
    let stamp = state.join("maintenance");
    if fs::metadata(&stamp)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|last| last.elapsed().ok())
        .is_some_and(|age| age < PASS_INTERVAL)
    {
        return Ok(());
    }
    // Claim the day before the work, so a failed pass waits for the next day
    // instead of running before every command.
    fs::create_dir_all(&state)?;
    fs::write(&stamp, [])?;
    // Every step is attempted; a failure is reported and the rest continue.
    report("boxes", prune_boxes(runtime()).map(drop));
    report("images", keep_two_images_per_source(runtime()));
    report("sockets", leftover_socket_dirs().and_then(remove_paths));
    report(
        "artifacts",
        artifacts::unpinned_versions().and_then(remove_paths),
    );
    report("egress logs", old_egress_logs().and_then(remove_paths));
    Ok(())
}

fn report(what: &str, result: io::Result<()>) {
    if let Err(error) = result {
        eprintln!("pinfold: maintenance: {what}: {error}");
    }
}

/// Whether the `box up` that claimed `state_dir` is alive: it holds the lock
/// on the dir's `pid` file. A missing or empty `pid` is a start between its
/// claim and its lock, so it counts as alive. Pid reuse and EPERM cannot
/// make a dead owner look alive.
pub fn owner_alive(state_dir: &Path) -> bool {
    let Ok(file) = File::open(state_dir.join("pid")) else {
        return true;
    };
    // Read under the lock, so an owner cannot lock and write in between.
    let Ok(_lock) = Flock::lock(file, FlockArg::LockExclusiveNonblock) else {
        return true;
    };
    owner_pid(state_dir).is_none()
}

/// The pid the owning `box up` wrote to `state_dir`'s `pid` file. Only
/// [`owner_alive`] says whether that process still holds the dir.
pub fn owner_pid(state_dir: &Path) -> Option<i32> {
    fs::read_to_string(state_dir.join("pid"))
        .ok()?
        .trim()
        .parse()
        .ok()
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
        let (owner, alive) = owner(&box_)?;
        let state_dir = dirs::box_state_dir(&box_.id)?;
        if alive {
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

/// A box's owner pid from its label, and whether that owner is alive. A box
/// whose state dir exists under this state root is judged by its lock. One
/// from another state root is judged by whether its label's pid exists:
/// judging it by a lock this root cannot see would call every other root's
/// live box dead.
pub fn owner(box_: &BoxInfo) -> io::Result<(Option<i32>, bool)> {
    let owner = box_
        .labels
        .get(OWNER_LABEL)
        .and_then(|pid| pid.parse::<i32>().ok());
    let state_dir = dirs::box_state_dir(&box_.id)?;
    let alive = if state_dir.is_dir() {
        owner_alive(&state_dir)
    } else {
        // Given a pid of 0 or below, kill probes a group of processes.
        owner.is_some_and(|pid| {
            pid > 0
                && matches!(
                    kill(Pid::from_raw(pid), None),
                    Ok(()) | Err(nix::errno::Errno::EPERM)
                )
        })
    };
    Ok((owner, alive))
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
    let mut leftover = Vec::new();
    for entry in dirs::entries(&dirs::boxes_dir()?)? {
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

/// Egress logs older than [`EGRESS_LOG_AGE`].
pub fn old_egress_logs() -> io::Result<Vec<PathBuf>> {
    let mut old = Vec::new();
    for entry in dirs::entries(&dirs::egress_dir()?)? {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok());
        if metadata.is_file() && age.is_some_and(|age| age > EGRESS_LOG_AGE) {
            old.push(entry.path());
        }
    }
    Ok(old)
}

/// Remove each path, a directory with everything under it. A path already
/// gone counts as removed.
pub fn remove_paths(paths: Vec<PathBuf>) -> io::Result<()> {
    for path in paths {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(&path)?,
            Ok(_) => fs::remove_file(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The bytes a file or directory holds, without following symlinks. An
/// unreadable entry counts as nothing.
pub fn path_bytes(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(metadata) if metadata.is_dir() => fs::read_dir(path)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| path_bytes(&entry.path()))
                    .sum()
            })
            .unwrap_or(0),
        _ => 0,
    }
}

/// Apply [`keep_two_images`] to every source any image names: each
/// non-empty family label, and every caller name a `pinfold/image-<NAME>`
/// tag names. A failing source is reported and the rest continue.
pub fn keep_two_images_per_source(runtime: &dyn Runtime) -> io::Result<()> {
    let mut sources: BTreeSet<(&'static str, String)> = BTreeSet::new();
    for image in runtime.list_images()? {
        for label in FAMILY_LABELS {
            if let Some(source) = image.labels.get(label).filter(|value| !value.is_empty()) {
                sources.insert((label, source.clone()));
            }
        }
        if let Some(source) = caller_source(&image.reference) {
            sources.insert((IMAGE_LABEL, source.to_string()));
        }
    }
    for (label, source) in sources {
        if let Err(error) = keep_two_images(runtime, label, &source) {
            eprintln!("pinfold: maintenance: images: {source}: {error}");
        }
    }
    Ok(())
}

/// The ids of the images listed boxes run, from one box list, with the boxes
/// that pin each. The in-use check [`keep_two_images`] and [`remove_images`]
/// share, so both see one listing and one rule.
fn in_use_images(runtime: &dyn Runtime) -> io::Result<BTreeMap<String, Vec<String>>> {
    let mut in_use: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for box_ in runtime.list()? {
        in_use.entry(box_.image_id).or_default().push(box_.id);
    }
    Ok(in_use)
}

/// The caller image name a reference tags, `[localhost/]pinfold/image-<NAME>:<tag>`,
/// in either spelling the runtime lists. `None` for any other reference.
fn caller_source(reference: &str) -> Option<&str> {
    let reference = reference.strip_prefix("localhost/").unwrap_or(reference);
    let (repository, _) = reference.rsplit_once(':')?;
    repository.strip_prefix("pinfold/image-")
}

/// The runtime's image entries for a source, one per reference. A profile
/// and a project carry `label = source`; a caller name lives only in its
/// `pinfold/image-<NAME>` tags. The listing [`keep_two_images`] and
/// [`remove_images`] share.
fn source_images(runtime: &dyn Runtime, label: &str, source: &str) -> io::Result<Vec<ImageInfo>> {
    Ok(runtime
        .list_images()?
        .into_iter()
        .filter(|image| {
            if label == IMAGE_LABEL {
                caller_source(&image.reference) == Some(source)
            } else {
                image.labels.get(label).map(String::as_str) == Some(source)
            }
        })
        .collect())
}

/// Keep the newest two builds in [`source_images`], removing older build
/// tags. An image a listed box reports is never offered to the runtime:
/// Apple's delete would remove it under the box, so pinfold skips it
/// itself and reports it like a failed removal.
pub fn keep_two_images(runtime: &dyn Runtime, label: &str, source: &str) -> io::Result<()> {
    // One list before anything goes: the ids of the images boxes pin.
    let in_use = in_use_images(runtime)?;
    // One entry per build tag, oldest first, naming its image.
    let mut builds: BTreeMap<(SystemTime, String), String> = BTreeMap::new();
    for image in source_images(runtime, label, source)? {
        if let Some(built) = build_time(&image.reference) {
            builds.insert((built, image.reference), image.id);
        }
    }
    let mut failures = Vec::new();
    for ((built, reference), id) in builds.into_iter().rev().skip(2) {
        if label == IMAGE_LABEL && !built.elapsed().is_ok_and(|age| age > CALLER_IMAGE_GRACE) {
            continue;
        }
        if let Some(boxes) = in_use.get(&id) {
            failures.push(format!("{reference}: in use by box {}", boxes.join(", ")));
            continue;
        }
        if let Err(error) = runtime.remove_image(&reference) {
            // Every removal carries its reference in the runtime's error;
            // join them into the one line the caller prints.
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    Err(io::Error::other(failures.join("; ")))
}

/// What [`remove_images`] removed, and what a listed box still uses.
/// `pinfold image rm` prints both lists.
pub struct Removed {
    /// The ids it untagged, one each.
    pub ids: Vec<String>,
    /// The source's images a listed box uses, so their tags stayed.
    pub in_use: Vec<String>,
}

/// Retire one source: remove every [`source_images`] reference that no
/// listed box uses, whatever its age. For a caller name that is its tags
/// alone, so another name's tags on a shared image stay; a profile's or a
/// project's whole image goes. Return the ids it untagged and the ids a box
/// still uses. The listing and in-use check are [`keep_two_images`]'s.
pub fn remove_images(runtime: &dyn Runtime, label: &str, source: &str) -> io::Result<Removed> {
    let in_use = in_use_images(runtime)?;
    // Every reference the source lists, grouped by image id: two builds of
    // unchanged inputs share one image, and two names share one by tag.
    let mut images: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for image in source_images(runtime, label, source)? {
        images.entry(image.id).or_default().push(image.reference);
    }
    let mut removed = Removed {
        ids: Vec::new(),
        in_use: Vec::new(),
    };
    for (id, references) in images {
        if in_use.contains_key(&id) {
            removed.in_use.push(id);
            continue;
        }
        for reference in references {
            runtime.remove_image(&reference)?;
        }
        removed.ids.push(id);
    }
    Ok(removed)
}

/// A build tag's time: its `<build>` starts with the build's nanoseconds
/// since the epoch in hex. `None` for `latest`, a dangling image's id and
/// any other reference that is not a build tag.
fn build_time(reference: &str) -> Option<SystemTime> {
    let (_, tag) = reference.rsplit_once(':')?;
    let nanos = u64::from_str_radix(tag.split('-').next()?, 16).ok()?;
    Some(UNIX_EPOCH + Duration::from_nanos(nanos))
}
