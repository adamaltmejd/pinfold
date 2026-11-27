//! Automatic maintenance: pinfold only removes what it created.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nix::fcntl::Flock;

use crate::core::image::{ImageInfo, service};
use crate::core::runtime::sockets::Sockets;
use crate::core::runtime::{BoxInfo, Runtime, runtime, unreachable};
use crate::core::{artifacts, ownership, runs};
use crate::dirs;

pub const PROFILE_LABEL: &str = "dev.pinfold.profile";
/// The label naming a project source on an image, and the project on a box.
pub const PROJECT_LABEL: &str = "dev.pinfold.project";
/// The two labels naming a profile's or a project's source on an image. A
/// caller's name lives only in its `localhost/pinfold/image-<NAME>` tags.
pub const FAMILY_LABELS: [&str; 2] = [PROFILE_LABEL, PROJECT_LABEL];
/// The label recording the digest of the image an image was built from.
pub const BASE_LABEL: &str = "dev.pinfold.base";
pub const OWNER_LABEL: &str = "dev.pinfold.owner";
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
    report(
        "artifacts",
        artifacts::unpinned_versions().and_then(remove_paths),
    );
    report("runs", runs::retired().and_then(remove_paths));
    let runtime = runtime();
    match boxes(runtime) {
        // The command's own runtime call, if it makes one, reports it.
        Err(error) if unreachable(&error) => return Ok(()),
        listed => report(
            "boxes",
            listed.map(|boxes| {
                prune_boxes(runtime, &boxes);
            }),
        ),
    }
    report("images", keep_two_images(None));
    report("sockets", prune_leftover_states(runtime));
    Ok(())
}

fn report(what: &str, result: io::Result<()>) {
    if let Err(error) = result {
        eprintln!("pinfold: maintenance: {what}: {error}");
    }
}

/// A pinfold box whose owning `box up` process is gone: the runtime removes
/// the box, and its state dir goes with it.
#[derive(Clone)]
pub struct DeadBox {
    /// The runtime's container id.
    pub id: String,
    /// The state dir `box up` created for it.
    pub state_dir: PathBuf,
    /// The `dev.pinfold.owner` pid, when the label parsed.
    pub owner: Option<i32>,
    pub generation: Option<String>,
    /// The `dev.pinfold.project` label.
    pub project: Option<String>,
}

/// One runtime box list, split into the work for Maintenance: the pinfold
/// boxes whose owner is gone, and the projects whose box is live.
pub struct Boxes {
    pub dead: Vec<DeadBox>,
    /// The `dev.pinfold.project` ids of pinfold boxes whose owner is alive.
    pub live_projects: BTreeSet<String>,
    /// Pinfold boxes the runtime reports running, with their memory limit,
    /// for doctor.
    pub running: Vec<(String, Option<u64>)>,
}

/// Whether `box_` is pinfold's: it carries the owner label. Another
/// container may carry other `dev.pinfold.` labels, since a runtime copies
/// an image's labels onto a container a user starts from it.
pub fn pinfold_box(box_: &BoxInfo) -> bool {
    box_.labels.contains_key(OWNER_LABEL)
}

/// Read the runtime's box list once.
pub fn boxes(runtime: &dyn Runtime) -> io::Result<Boxes> {
    let mut dead = Vec::new();
    let mut live_projects = BTreeSet::new();
    let mut running = Vec::new();
    for box_ in runtime.list()? {
        if !pinfold_box(&box_) {
            continue;
        }
        if box_.running {
            running.push((box_.id.clone(), box_.memory_limit));
        }
        let observed = (|| {
            let (owner, alive) = owner(&box_)?;
            let state_dir = ownership::record(&box_.id)?
                .map(|record| record.state_dir)
                .unwrap_or(dirs::box_state_dir(&box_.id)?);
            Ok::<_, io::Error>((owner, alive, state_dir))
        })();
        let (owner, alive, state_dir) = match observed {
            Ok(observed) => observed,
            Err(error) => {
                eprintln!("pinfold: box inventory {:?}: {error}", box_.id);
                // Unknown ownership must still protect this project's state.
                if let Some(project) = box_.labels.get(PROJECT_LABEL) {
                    live_projects.insert(project.clone());
                }
                continue;
            }
        };
        let project = box_.labels.get(PROJECT_LABEL).cloned();
        if alive {
            live_projects.extend(project);
            continue;
        }
        dead.push(DeadBox {
            state_dir,
            id: box_.id,
            owner,
            generation: box_.labels.get(ownership::GENERATION_LABEL).cloned(),
            project,
        });
    }
    Ok(Boxes {
        dead,
        live_projects,
        running,
    })
}

/// Labels identify the generation; the stable lock decides its liveness.
pub fn owner(box_: &BoxInfo) -> io::Result<(Option<i32>, bool)> {
    let owner = box_
        .labels
        .get(OWNER_LABEL)
        .and_then(|pid| pid.parse::<i32>().ok());
    let matching = ownership::record(&box_.id)?.is_some_and(|record| {
        box_.labels.get(ownership::GENERATION_LABEL) == Some(&record.generation)
    });
    let alive = matching && ownership::name_alive(&box_.id)?;
    Ok((owner, alive))
}

/// Remove the observed generation only while its name is unowned.
/// Return whether a matching runtime box was removed.
pub fn remove_orphan(
    runtime: &dyn Runtime,
    name: &str,
    generation: Option<&str>,
    _lock: &Flock<File>,
) -> io::Result<bool> {
    let mut removed = false;
    if let Some(current) = runtime.inspect(name)? {
        if !pinfold_box(&current)
            || current
                .labels
                .get(ownership::GENERATION_LABEL)
                .map(String::as_str)
                != generation
        {
            return Ok(false);
        }
        runtime.down(name)?;
        removed = true;
    }
    if let Some(record) = ownership::record(name)? {
        if Some(record.generation.as_str()) == generation {
            // Apple's `/tmp` volume outlives a removal that failed after
            // the box went, which kept the record; finish it first.
            #[cfg(not(target_os = "linux"))]
            if !removed {
                runtime.down(name)?;
            }
            ownership::remove_record(name, &record.generation)?;
        }
    } else {
        let _ = fs::remove_dir_all(dirs::box_state_dir(name)?);
    }
    Ok(removed)
}

/// Inventory is advisory. A restarted generation is never removed by it.
pub fn prune_boxes(runtime: &dyn Runtime, boxes: &Boxes) -> Vec<DeadBox> {
    let mut removed = Vec::new();
    for box_ in &boxes.dead {
        let result = (|| {
            let Some(lock) = ownership::try_name_lock(&box_.id)? else {
                return Ok(false);
            };
            remove_orphan(runtime, &box_.id, box_.generation.as_deref(), &lock)
        })();
        match result {
            Ok(true) => removed.push(box_.clone()),
            Ok(false) => {}
            Err(error) => eprintln!("pinfold: prune {:?}: {error}", box_.id),
        }
    }
    removed
}

/// What `box prune --unknown NAME` did.
pub enum Unknown {
    Absent,
    /// The owner pid the claim's label names, when it parsed.
    Pruned(Option<i32>),
    /// A reason token and its detail; the claim stands.
    Refused(&'static str, String),
}

/// Retire box `name` alone, proving release first when its liveness is
/// unknown (Process interface).
pub fn prune_unknown(runtime: &dyn Runtime, name: &str) -> io::Result<Unknown> {
    let Some(lock) = ownership::try_name_lock(name)? else {
        return Ok(Unknown::Refused(
            "name-in-use",
            format!("box {name:?} has a live owner"),
        ));
    };
    let Some(record) = ownership::record(name)? else {
        return Ok(Unknown::Absent);
    };
    #[cfg(target_os = "linux")]
    if let Err((reason, detail)) = crate::core::runtime::firecracker::release_unknown(&record)? {
        return Ok(Unknown::Refused(reason, detail));
    }
    let owner = record
        .described
        .as_ref()
        .and_then(|described| described.labels.get(OWNER_LABEL))
        .and_then(|pid| pid.parse().ok());
    remove_orphan(runtime, name, Some(&record.generation), &lock)?;
    // Apple's inspection can name another generation, which removes nothing.
    if ownership::record(name)?.is_some_and(|current| current.generation == record.generation) {
        return Err(io::Error::other(format!(
            "box {name:?}: the runtime lists another generation"
        )));
    }
    Ok(Unknown::Pruned(owner))
}

/// Measured leftovers; removal must reacquire ownership and recheck.
pub fn leftover_socket_dirs() -> io::Result<Vec<PathBuf>> {
    let mut protected = BTreeSet::new();
    for result in ownership::record_results()? {
        let record = match result {
            Ok(record) => record,
            Err(error) => {
                eprintln!("pinfold: socket measurement: {error}");
                continue;
            }
        };
        let alive = match ownership::name_alive(&record.name) {
            Ok(alive) => alive,
            Err(error) => {
                eprintln!("pinfold: socket measurement {:?}: {error}", record.name);
                true
            }
        };
        if alive {
            protected.insert(record.generation);
        }
    }
    Ok(Sockets::leftovers(&protected)?
        .into_iter()
        .map(|(_, path)| path)
        .collect())
}

pub fn prune_leftover_states(runtime: &dyn Runtime) -> io::Result<()> {
    let mut corrupt = false;
    for result in ownership::record_results()? {
        let record = match result {
            Ok(record) => record,
            Err(error) => {
                corrupt = true;
                eprintln!("pinfold: prune: {error}");
                continue;
            }
        };
        let result = (|| {
            let Some(_lock) = ownership::try_name_lock(&record.name)? else {
                return Ok(());
            };
            #[cfg(target_os = "linux")]
            {
                // Linux inventory depends on every record. Check this claim
                // directly so an unrelated corrupt record cannot block it.
                if ownership::record(&record.name)?
                    .is_none_or(|current| current.generation != record.generation)
                {
                    return Ok(());
                }
                runtime.down(&record.name)?;
                ownership::remove_record(&record.name, &record.generation)
            }
            #[cfg(not(target_os = "linux"))]
            remove_orphan(runtime, &record.name, Some(&record.generation), &_lock).map(|_| ())
        })();
        if let Err(error) = result {
            eprintln!("pinfold: prune {:?}: {error}", record.name);
        }
    }
    if corrupt {
        eprintln!(
            "pinfold: socket cleanup: recordless sweep skipped because an ownership record is unreadable"
        );
        return Ok(());
    }
    let protected = match ownership::records() {
        Ok(records) => records
            .into_iter()
            .map(|record| record.generation)
            .collect(),
        Err(error) => {
            eprintln!("pinfold: socket cleanup: recordless sweep skipped: {error}");
            return Ok(());
        }
    };
    for (generation, _) in Sockets::leftovers(&protected)? {
        let result = (|| {
            // Publication precedes generation-directory creation. A directory
            // with no matching claim cannot belong to an unpublished start.
            if ownership::records()?
                .iter()
                .any(|record| record.generation == generation)
                || runtime
                    .list()?
                    .iter()
                    .any(|box_| box_.labels.get(ownership::GENERATION_LABEL) == Some(&generation))
            {
                return Ok(());
            }
            Sockets::remove(&generation)
        })();
        if let Err(error) = result {
            eprintln!("pinfold: socket cleanup: {error}");
        }
    }
    Ok(())
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

/// Keep the newest two builds of every source any image names: each
/// non-empty family label, and every caller name a `localhost/pinfold/image-<NAME>`
/// tag names. `only` limits the pass to one `(label, source)`, the one a
/// build just made. One image listing and one box listing serve every
/// source. Call it after dead boxes are pruned: its box listing is
/// retention's one in-use rule. A failing source is reported and the rest
/// continue.
pub fn keep_two_images(only: Option<(Option<&str>, &str)>) -> io::Result<()> {
    let images = service::list_images()?;
    let in_use = in_use_images(runtime())?;
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
        if only.is_some_and(|only| only != (label, source.as_str())) {
            continue;
        }
        if let Err(error) = keep_two_images_from(&images, &in_use, &mut references, label, &source)
        {
            eprintln!("pinfold: maintenance: images: {source}: {error}");
        }
    }
    Ok(())
}

/// The ids of the images every listed box runs: the one in-use rule
/// [`keep_two_images`] and [`remove_images`] share. Dead or alive, every
/// listed box counts.
fn in_use_images(runtime: &dyn Runtime) -> io::Result<BTreeSet<String>> {
    let records = ownership::records()?;
    let mut ids: BTreeSet<String> = runtime
        .list()?
        .into_iter()
        .map(|box_| box_.image_id)
        .collect();
    for record in records.into_iter().filter(|record| !record.retiring) {
        if let Some(described) = record.described {
            ids.insert(described.image_id);
        }
    }
    #[cfg(target_os = "linux")]
    for image in crate::core::image::linux::catalog::list()? {
        if crate::core::image::linux::catalog::leased(&image.id)? {
            ids.insert(image.id);
        }
    }
    Ok(ids)
}

/// The caller image name a reference tags, `localhost/pinfold/image-<NAME>:<tag>`.
/// `None` for any other reference.
fn caller_source(reference: &str) -> Option<&str> {
    let (repository, _) = reference.rsplit_once(':')?;
    repository.strip_prefix("localhost/pinfold/image-")
}

/// The entries in `images` for a source, one per reference. `label` is
/// `Some` for a profile or a project, which carry `label = source`; it is
/// `None` for a caller name, which lives only in its
/// `localhost/pinfold/image-<NAME>` tags. The filter [`keep_two_images`] and
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
/// tags, from a pass's listings: `images` and `in_use` are read once for
/// every source, and `references` counts each image's tags as removals
/// happen, so a later source sees what the earlier ones left. A tag of an
/// image a listed box uses is removed while another reference still names
/// the image; the image's last tag stays, and is reported like a failed
/// removal.
fn keep_two_images_from(
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
        if let Err(error) = service::remove_image(&reference) {
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
pub struct Removed {
    /// The ids it untagged, one each.
    pub ids: Vec<String>,
    /// The ids whose last tag stayed because a listed box uses the image.
    pub in_use: Vec<String>,
}

/// Retire a caller image name: remove its tags, except an image's last tag
/// while a listed box uses it. Return the ids untagged and whose last tag
/// stayed. Another name's tags on a shared image stay.
pub fn remove_images(source: &str) -> io::Result<Removed> {
    let listing = service::list_images()?;
    let in_use = in_use_images(runtime())?;
    // Every reference the store lists, so an image's last tag counts
    // another name's and another source's, not only this source's. Two
    // builds of unchanged inputs share one image, and two names one by tag.
    let mut references = reference_counts(&listing);
    let (mut untagged, mut kept) = (BTreeSet::new(), BTreeSet::new());
    for image in source_images(&listing, None, source) {
        let count = references.entry(image.id.clone()).or_default();
        // With a box using the image, its last tag stays.
        if in_use.contains(&image.id) && *count <= 1 {
            kept.insert(image.id.clone());
            continue;
        }
        service::remove_image(&image.reference)?;
        *count -= 1;
        untagged.insert(image.id.clone());
    }
    Ok(Removed {
        ids: untagged.difference(&kept).cloned().collect(),
        in_use: kept.into_iter().collect(),
    })
}

/// A build tag's time: its `<build>` starts with the build's nanoseconds
/// since the epoch in hex. `None` for `latest`, a dangling image's id and
/// any other reference that is not a build tag.
fn build_time(reference: &str) -> Option<SystemTime> {
    let (_, tag) = reference.rsplit_once(':')?;
    let nanos = u64::from_str_radix(tag.split('-').next()?, 16).ok()?;
    Some(UNIX_EPOCH + Duration::from_nanos(nanos))
}
