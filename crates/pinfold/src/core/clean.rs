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
/// The two labels naming a profile's or a project's source on an image. A
/// caller's name lives only in its `pinfold/image-<NAME>` tags.
pub const FAMILY_LABELS: [&str; 2] = [PROFILE_LABEL, PROJECT_LABEL];
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
    // One box list serves the boxes step; retention lists the boxes again
    // after pruning, so a box started meanwhile or a dead box a failed
    // prune left behind still pins its image.
    let runtime = runtime();
    report(
        "boxes",
        boxes(runtime).and_then(|boxes| prune_boxes(runtime, &boxes)),
    );
    report("images", keep_two_images_per_source(runtime));
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

/// One runtime box list, split into the work for Maintenance: the pinfold
/// boxes whose owner is gone, and the projects whose box is live.
pub struct Boxes {
    /// The pinfold boxes whose owning `box up` process is gone.
    pub dead: Vec<DeadBox>,
    /// The `dev.pinfold.project` ids of pinfold boxes whose owner is alive.
    pub live_projects: BTreeSet<String>,
}

/// Whether `box_` is pinfold's: it carries the owner label. Another
/// container may carry other `dev.pinfold.` labels, since podman copies an
/// image's labels onto a container a user starts from it.
pub fn pinfold_box(box_: &BoxInfo) -> bool {
    box_.labels.contains_key(OWNER_LABEL)
}

/// Read the runtime's box list once. A pinfold box whose owner label no
/// live process matches counts as gone.
pub fn boxes(runtime: &dyn Runtime) -> io::Result<Boxes> {
    let mut dead = Vec::new();
    let mut live_projects = BTreeSet::new();
    for box_ in runtime.list()? {
        if !pinfold_box(&box_) {
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

/// Remove the dead boxes [`boxes`] found, and the state dirs that name
/// them.
pub fn prune_boxes(runtime: &dyn Runtime, boxes: &Boxes) -> io::Result<()> {
    for box_ in &boxes.dead {
        runtime.down(&box_.id)?;
        let _ = fs::remove_dir_all(&box_.state_dir);
    }
    Ok(())
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
/// tag names. One image listing and one box listing serve every source, so
/// a pass costs two runtime listings however many sources it has. Call it
/// after dead boxes are pruned: its box listing is retention's one in-use
/// rule. A failing source is reported and the rest continue.
pub fn keep_two_images_per_source(runtime: &dyn Runtime) -> io::Result<()> {
    let images = runtime.list_images()?;
    let in_use = in_use_images(runtime)?;
    let mut references = reference_counts(&images);
    let mut sources: BTreeSet<(Option<&'static str>, String)> = BTreeSet::new();
    for image in &images {
        for label in FAMILY_LABELS {
            if let Some(source) = image.labels.get(label).filter(|value| !value.is_empty()) {
                sources.insert((Some(label), source.clone()));
            }
        }
        if let Some(source) = caller_source(&image.reference) {
            sources.insert((None, source.to_string()));
        }
    }
    for (label, source) in sources {
        if let Err(error) =
            keep_two_images_from(runtime, &images, &in_use, &mut references, label, &source)
        {
            eprintln!("pinfold: maintenance: images: {source}: {error}");
        }
    }
    Ok(())
}

/// The ids of the images every listed box runs: the one in-use rule [`keep_two_images_per_source`],
/// [`keep_two_images`] and [`remove_images`] share. Dead or alive, every
/// listed box counts.
fn in_use_images(runtime: &dyn Runtime) -> io::Result<BTreeSet<String>> {
    Ok(runtime
        .list()?
        .into_iter()
        .map(|box_| box_.image_id)
        .collect())
}

/// The caller image name a reference tags, `[localhost/]pinfold/image-<NAME>:<tag>`,
/// in either spelling the runtime lists. `None` for any other reference.
fn caller_source(reference: &str) -> Option<&str> {
    let reference = reference.strip_prefix("localhost/").unwrap_or(reference);
    let (repository, _) = reference.rsplit_once(':')?;
    repository.strip_prefix("pinfold/image-")
}

/// The entries in `images` for a source, one per reference. `label` is
/// `Some` for a profile or a project, which carry `label = source`; it is
/// `None` for a caller name, which lives only in its
/// `pinfold/image-<NAME>` tags. The filter [`keep_two_images`] and
/// [`remove_images`] share.
fn source_images<'a>(
    images: &'a [ImageInfo],
    label: Option<&str>,
    source: &str,
) -> Vec<&'a ImageInfo> {
    images
        .iter()
        .filter(|image| match label {
            Some(label) => image.labels.get(label).map(String::as_str) == Some(source),
            None => caller_source(&image.reference) == Some(source),
        })
        .collect()
}

/// The number of references the store lists for each image id, so the
/// in-use rule can tell an image's last tag. "Last tag" counts every
/// reference in the store, of any name or source.
fn reference_counts(images: &[ImageInfo]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for image in images {
        *counts.entry(image.id.clone()).or_default() += 1;
    }
    counts
}

/// Keep the newest two builds in [`source_images`], removing older build
/// tags. A tag of an image a listed box uses is removed while another
/// reference still names the image; the image's last tag stays, and is
/// reported like a failed removal.
pub fn keep_two_images(runtime: &dyn Runtime, label: Option<&str>, source: &str) -> io::Result<()> {
    // One listing before anything goes: the ids of the images boxes pin,
    // and every reference in the store, so an in-use image's last tag is
    // known.
    let images = runtime.list_images()?;
    let in_use = in_use_images(runtime)?;
    let mut references = reference_counts(&images);
    keep_two_images_from(runtime, &images, &in_use, &mut references, label, source)
}

/// Apply [`keep_two_images`]'s rule to one source from a pass's listings:
/// `images` and `in_use` are read once for every source, and `references`
/// counts each image's tags as removals happen, so a later source sees what
/// the earlier ones left.
fn keep_two_images_from(
    runtime: &dyn Runtime,
    images: &[ImageInfo],
    in_use: &BTreeSet<String>,
    references: &mut BTreeMap<String, usize>,
    label: Option<&str>,
    source: &str,
) -> io::Result<()> {
    // One entry per build tag, oldest first, naming its image.
    let mut builds: BTreeMap<(SystemTime, String), String> = BTreeMap::new();
    for image in source_images(images, label, source) {
        if let Some(built) = build_time(&image.reference) {
            builds.insert((built, image.reference.clone()), image.id.clone());
        }
    }
    let mut failures = Vec::new();
    for ((built, reference), id) in builds.into_iter().rev().skip(2) {
        if label.is_none() && !built.elapsed().is_ok_and(|age| age > CALLER_IMAGE_GRACE) {
            continue;
        }
        // A box keeps an image's last tag alive; any other tag of that
        // image only untags it, so it goes.
        if in_use.contains(&id) && references.get(&id).copied().unwrap_or(0) <= 1 {
            failures.push(format!("{reference}: in use"));
            continue;
        }
        if let Err(error) = runtime.remove_image(&reference) {
            // Every removal carries its reference in the runtime's error;
            // join them into the one line the caller prints.
            failures.push(error.to_string());
        } else {
            *references.entry(id).or_default() -= 1;
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    Err(io::Error::other(failures.join("; ")))
}

/// What [`remove_images`] removed, and what a listed box still uses.
/// `pinfold image rm` prints both lists.
#[derive(Default)]
pub struct Removed {
    /// The ids it untagged, one each.
    pub ids: Vec<String>,
    /// The ids whose last tag stayed because a listed box uses the image.
    pub in_use: Vec<String>,
}

/// Retire a caller image name: remove its tags, except an image's last tag
/// while a listed box uses it. Return the ids untagged and whose last tag
/// stayed. Another name's tags on a shared image stay.
pub fn remove_images(runtime: &dyn Runtime, source: &str) -> io::Result<Removed> {
    let listing = runtime.list_images()?;
    let in_use = in_use_images(runtime)?;
    // Every reference the store lists, so an image's last tag counts
    // another name's and another source's, not only this source's.
    let references = reference_counts(&listing);
    // This source's references, grouped by image id: two builds of
    // unchanged inputs share one image, and two names share one by tag.
    let mut images: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for image in source_images(&listing, None, source) {
        images
            .entry(image.id.clone())
            .or_default()
            .push(image.reference.clone());
    }
    let mut removed = Removed::default();
    for (id, tags) in images {
        // The image's last tag is this source's when it tags every
        // reference of the id; with a box using the image, that one stays.
        let last = references.get(&id).copied().unwrap_or(0) == tags.len();
        if last && in_use.contains(&id) {
            for tag in &tags[..tags.len() - 1] {
                runtime.remove_image(tag)?;
            }
            removed.in_use.push(id);
            continue;
        }
        for reference in &tags {
            runtime.remove_image(reference)?;
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
