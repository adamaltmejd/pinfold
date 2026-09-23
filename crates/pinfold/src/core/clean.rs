//! Automatic maintenance: pinfold only removes what it created.
//!
//! After a build, keep the newest two images per source. At most once a day,
//! at the start of any command, prune leftovers: boxes whose owner is gone,
//! sockets in state dirs no live owner holds, artifact versions no pin names,
//! and egress logs older than 14 days. The pass reports a problem on stderr
//! and never fails the command it runs before.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::time::{Duration, SystemTime};

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
        Ok(runtime) => report("boxes", prune_boxes(runtime)),
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

/// Whether a process with this pid exists.
pub fn alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    match kill(Pid::from_raw(pid), None) {
        Ok(()) | Err(nix::errno::Errno::EPERM) => true,
        Err(_) => false,
    }
}

/// Remove boxes pinfold labeled whose owning `box up` process is gone, and
/// the state dirs that name them.
pub fn prune_boxes(runtime: &dyn Runtime) -> io::Result<()> {
    let boxes = dirs::state_dir()?.join("boxes");
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
        if owner.is_some_and(alive) {
            continue;
        }
        runtime.down(&box_.id)?;
        let _ = fs::remove_dir_all(boxes.join(&box_.id));
    }
    Ok(())
}

/// Remove state dirs whose owner is gone. A live `box up` writes its pid
/// before it binds the proxy socket, so a socket without a live pid is
/// leftover.
fn prune_sockets() -> io::Result<()> {
    let boxes = dirs::state_dir()?.join("boxes");
    let entries = match fs::read_dir(&boxes) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let dir = entry.path();
        let pid = fs::read_to_string(dir.join("pid"))
            .ok()
            .and_then(|pid| pid.trim().parse::<i32>().ok());
        if pid.is_some_and(alive) {
            continue;
        }
        let has_socket = fs::symlink_metadata(dir.join("proxy.sock")).is_ok();
        // A live `box up` writes its pid before it binds the socket, so a
        // socket with no live pid is leftover. A dir without a socket is a
        // start that failed before it could listen; it holds nothing worth
        // reclaiming, and removing it could race a start.
        if has_socket {
            let _ = fs::remove_dir_all(&dir);
        }
    }
    Ok(())
}

/// Remove egress logs older than [`EGRESS_LOG_AGE`].
fn prune_egress_logs() -> io::Result<()> {
    let dir = dirs::egress_dir()?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let Some(cutoff) = SystemTime::now().checked_sub(EGRESS_LOG_AGE) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry?;
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() && metadata.modified().is_ok_and(|modified| modified < cutoff) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Keep the newest two images carrying `label = source`, removing older ones
/// and the layers no image references. Called after a successful build.
pub fn keep_two_images(runtime: &dyn Runtime, label: &str, source: &str) -> io::Result<()> {
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
    for (_, _, references) in images.into_iter().skip(2) {
        for reference in references {
            runtime.remove_image(&reference)?;
        }
    }
    Ok(())
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
