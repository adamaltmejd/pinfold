//! Disks: named images that outlive boxes. Attachments are recorded under
//! each box's claim; a writable attachment marks the disk unclean until
//! `down` confirms it. A disk pinfold creates is marked unfilled until its
//! box comes ready; an unfilled disk counts as absent.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nix::fcntl::Flock;

use crate::core::r#box::RefusalReason;
use crate::core::clean;
use crate::core::ownership::{self, Attachment, Claim};
use crate::core::plan::{Disk, Plan};
use crate::core::profile::Seed;
use crate::core::runtime::{Runtime, apple};
use crate::dirs;
use crate::init::{STREAM_DIR, STREAM_END, STREAM_EXECUTABLE, STREAM_FILE, STREAM_SYMLINK};

/// One entry of a `from` tree, relative to its root.
#[derive(Clone)]
pub enum Entry {
    Dir(PathBuf),
    File { path: PathBuf, executable: bool },
    Symlink { path: PathBuf, target: PathBuf },
}

/// A new disk whose `from` the box copies in before ready.
#[derive(Clone)]
pub struct Fill {
    pub guest: PathBuf,
    pub from: PathBuf,
    pub entries: Vec<Entry>,
}

/// A profile's `home/` seeds and the writable disk behind `$HOME` to copy
/// them into, in the box before ready.
#[derive(Clone)]
pub struct Seeding {
    /// The disk's guest root.
    pub guest: PathBuf,
    /// The plain path from the disk's guest root to `$HOME`.
    pub relative: PathBuf,
    pub seeds: Vec<Seed>,
}

/// What [`claim`] did: the disks it created, the copies they need, and the
/// writable disks it newly marked unclean.
#[derive(Default)]
pub struct Claimed {
    pub created: Vec<Disk>,
    pub fills: Vec<Fill>,
    pub marked: Vec<String>,
}

impl Claimed {
    /// Undo a claim whose box `name` failed to start, under its disks'
    /// locks.
    pub fn undo(&self, runtime: &dyn Runtime, name: &str) {
        let names: BTreeSet<&str> = self
            .created
            .iter()
            .map(|disk| disk.name.as_str())
            .chain(self.marked.iter().map(String::as_str))
            .collect();
        match locks(&names) {
            Ok(_locks) => self.undo_locked(runtime, name),
            Err(error) => eprintln!("pinfold: startup cleanup: disk locks: {error}"),
        }
    }

    /// Remove the disks the claim created and the marks it added, except
    /// on a disk another box attached since, so a disk no box mounted keeps
    /// its old state.
    fn undo_locked(&self, runtime: &dyn Runtime, name: &str) {
        let attached = match attachments(runtime, Some(name)) {
            Ok(attached) => attached,
            Err(error) => {
                eprintln!("pinfold: startup cleanup: disk attachments: {error}");
                return;
            }
        };
        for disk in self
            .created
            .iter()
            .filter(|disk| !attached.contains_key(&disk.name))
        {
            let result = runtime
                .remove_disk(&disk.name)
                .and_then(|()| unmark(&disk.name, UNCLEAN))
                .and_then(|()| unmark(&disk.name, UNFILLED));
            if let Err(error) = result {
                eprintln!("pinfold: startup cleanup: disk {:?}: {error}", disk.name);
            }
        }
        for disk in self
            .marked
            .iter()
            .filter(|disk| !attached.contains_key(*disk))
        {
            if let Err(error) = unmark(disk, UNCLEAN) {
                eprintln!("pinfold: startup cleanup: disk {disk:?}: {error}");
            }
        }
    }
}

/// Each named disk's lock, taken in sorted order so two `up`s never wait
/// on each other's locks in a cycle.
fn locks(names: &BTreeSet<&str>) -> io::Result<Vec<Flock<File>>> {
    names
        .iter()
        .map(|disk| ownership::disk_lock(disk))
        .collect()
}

/// Lifecycle step 2: claim `disks` for the box `claim` holds, creating any
/// that are new. Every refusal comes before anything is created.
pub fn claim(
    runtime: &dyn Runtime,
    claim: &mut Claim,
    plan: &Plan,
    init: &Path,
) -> Result<Claimed, Result<(RefusalReason, String), io::Error>> {
    let (name, disks) = (plan.name.as_str(), plan.disks.as_slice());
    if disks.is_empty() {
        return Ok(Claimed::default());
    }
    let names: BTreeSet<&str> = disks.iter().map(|disk| disk.name.as_str()).collect();
    let _locks = locks(&names).map_err(Err)?;
    let attached = attachments(runtime, Some(name)).map_err(Err)?;
    let existing: BTreeSet<String> = runtime
        .list_disks()
        .map_err(Err)?
        .into_iter()
        .map(|disk| disk.name)
        .collect();
    let mut claimed = Claimed::default();
    for disk in disks {
        let others = attached.get(&disk.name).map(Vec::as_slice).unwrap_or(&[]);
        if let Some((holder, _)) = others
            .iter()
            .find(|(_, readonly)| !disk.readonly || !readonly)
        {
            return Err(Ok((
                RefusalReason::DiskInUse,
                format!("disk {:?} is attached by box {holder:?}", disk.name),
            )));
        }
        if disk.readonly && unclean(&disk.name).map_err(Err)? {
            return Err(Ok((
                RefusalReason::DiskUnclean,
                format!(
                    "disk {:?} was not detached cleanly; attach it writable once to recover it",
                    disk.name
                ),
            )));
        }
        if existing.contains(&disk.name) && !unfilled(&disk.name).map_err(Err)? {
            continue;
        }
        if let Some(from) = &disk.from {
            let (entries, bytes) = walk(from, &disk.skip).map_err(Err)?;
            if bytes > disk.bytes() {
                return Err(Ok((
                    RefusalReason::Limit,
                    format!(
                        "disk {:?}: from {} holds {bytes} bytes, past its size of {}",
                        disk.name,
                        from.display(),
                        disk.bytes()
                    ),
                )));
            }
            claimed.fills.push(Fill {
                guest: disk.guest.clone(),
                from: from.clone(),
                entries,
            });
        }
        claimed.created.push(disk.clone());
    }
    let mut created = Vec::new();
    let result = (|| {
        for disk in &claimed.created {
            // Marked first, so a crash anywhere in the fill leaves the mark.
            mark(&disk.name, UNFILLED)?;
            if existing.contains(&disk.name) {
                runtime.remove_disk(&disk.name)?;
            }
            runtime.create_disk(&disk.name, disk.bytes())?;
            created.push(disk.clone());
        }
        if cfg!(target_os = "macos") && !created.is_empty() {
            let names: Vec<&str> = created.iter().map(|disk| disk.name.as_str()).collect();
            let image = plan.image.as_deref().expect("up resolved the image");
            let labels = BTreeMap::from([
                (
                    clean::OWNER_LABEL.to_string(),
                    plan.labels[clean::OWNER_LABEL].clone(),
                ),
                (
                    ownership::GENERATION_LABEL.to_string(),
                    claim.generation().to_string(),
                ),
            ]);
            apple::own_disks(&names, image, init, name, &labels)?;
        }
        for disk in disks.iter().filter(|disk| !disk.readonly) {
            if !unclean(&disk.name)? {
                mark(&disk.name, UNCLEAN)?;
                claimed.marked.push(disk.name.clone());
            }
        }
        claim.attach(
            disks
                .iter()
                .map(|disk| Attachment {
                    name: disk.name.clone(),
                    readonly: disk.readonly,
                })
                .collect(),
        )
    })();
    if let Err(error) = result {
        claimed.created = created;
        claimed.undo_locked(runtime, name);
        return Err(Err(error));
    }
    Ok(claimed)
}

/// After a requested stop of a box that ran: the runtime synced and
/// unmounted its disks, so record each writable one clean. A box that ended
/// on its own leaves them unclean.
pub fn detached(disks: &[Disk]) -> io::Result<()> {
    for disk in disks.iter().filter(|disk| !disk.readonly) {
        unmark(&disk.name, UNCLEAN)?;
    }
    Ok(())
}

/// Each disk's attachments: box name and read-only flag, from every claim
/// published in this state root whose owner holds its name or whose box
/// the runtime has not stopped. A dead owner's box attaches nothing once
/// the runtime lists it stopped or not at all, even before `prune` removes
/// it. `except` is the caller's own box.
pub fn attachments(
    runtime: &dyn Runtime,
    except: Option<&str>,
) -> io::Result<BTreeMap<String, Vec<(String, bool)>>> {
    let mut attached: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    let boxes = match fs::canonicalize(dirs::boxes_dir()?) {
        Ok(boxes) => boxes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(attached),
        Err(error) => return Err(error),
    };
    let mut records = Vec::new();
    for record in ownership::records()? {
        if record.disks.is_empty() || Some(record.name.as_str()) == except {
            continue;
        }
        // A claim's state dir goes only as its box is torn down.
        match fs::canonicalize(&record.state_dir) {
            Ok(dir) if dir.parent() == Some(boxes.as_path()) => records.push(record),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if records.is_empty() {
        return Ok(attached);
    }
    let unstopped: BTreeSet<String> = runtime
        .list()?
        .into_iter()
        .filter(|b| !b.stopped)
        .map(|b| b.id)
        .collect();
    for record in records {
        if !unstopped.contains(&record.name) && !ownership::name_alive(&record.name)? {
            continue;
        }
        for disk in record.disks {
            attached
                .entry(disk.name)
                .or_default()
                .push((record.name.clone(), disk.readonly));
        }
    }
    Ok(attached)
}

/// Written before a writable attachment; only a confirmed `down` removes it.
const UNCLEAN: &str = "unclean";
/// Written before pinfold creates a disk; removed once its box is ready.
const UNFILLED: &str = "unfilled";

fn marker(name: &str, kind: &str) -> io::Result<PathBuf> {
    Ok(dirs::state_dir()?
        .join("disks")
        .join(format!("{name}.{kind}")))
}

fn marked(name: &str, kind: &str) -> io::Result<bool> {
    fs::exists(marker(name, kind)?)
}

/// Whether a writable attachment of the disk ended without `down`.
pub fn unclean(name: &str) -> io::Result<bool> {
    marked(name, UNCLEAN)
}

/// Whether pinfold created the disk and its box never came ready: such a
/// disk counts as absent.
pub fn unfilled(name: &str) -> io::Result<bool> {
    marked(name, UNFILLED)
}

/// Durably: a lost mark would let a dead box's disk pass as clean or filled.
fn mark(name: &str, kind: &str) -> io::Result<()> {
    let path = marker(name, kind)?;
    let dir = path.parent().expect("a marker has a directory");
    fs::create_dir_all(dir)?;
    File::create(&path)?.sync_all()?;
    File::open(dir)?.sync_all()
}

fn unmark(name: &str, kind: &str) -> io::Result<()> {
    crate::core::absent_ok(fs::remove_file(marker(name, kind)?))
}

/// Once the box is ready, its new disks hold their `from`, synced in the
/// guest. Each is made durable before its mark goes, and the removal is
/// durable too: a mark that came back after a crash would have the next
/// `up` replace a disk that holds work.
pub fn filled(runtime: &dyn Runtime, created: &[Disk]) -> io::Result<()> {
    for disk in created {
        runtime.sync_disk(&disk.name)?;
        unmark(&disk.name, UNFILLED)?;
    }
    if !created.is_empty() {
        File::open(dirs::state_dir()?.join("disks"))?.sync_all()?;
    }
    Ok(())
}

/// `from`'s tree without following symlinks, parents first, and the sum of
/// its regular files' sizes. Other file types and the `skip` paths, with
/// everything below them, are left out.
fn walk(from: &Path, skip: &BTreeSet<PathBuf>) -> io::Result<(Vec<Entry>, u64)> {
    let mut entries = Vec::new();
    let mut bytes = 0u64;
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let mut children: Vec<_> = fs::read_dir(from.join(&dir))?.collect::<Result<_, _>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = dir.join(child.file_name());
            if skip.contains(&path) {
                continue;
            }
            let metadata = fs::symlink_metadata(child.path())?;
            let kind = metadata.file_type();
            if kind.is_dir() {
                entries.push(Entry::Dir(path.clone()));
                pending.push(path);
            } else if kind.is_file() {
                bytes = bytes.saturating_add(metadata.len());
                entries.push(Entry::File {
                    path,
                    executable: metadata.permissions().mode() & 0o111 != 0,
                });
            } else if kind.is_symlink() {
                entries.push(Entry::Symlink {
                    target: fs::read_link(child.path())?,
                    path,
                });
            }
        }
    }
    Ok((entries, bytes))
}

/// One stream entry's tag and path.
fn header(out: &mut impl Write, tag: u8, path: &Path) -> io::Result<()> {
    let bytes = path.as_os_str().as_bytes();
    out.write_all(&[tag])?;
    out.write_all(&(bytes.len() as u32).to_be_bytes())?;
    out.write_all(bytes)
}

/// Write `fill`'s tree as the stream `init receive` reads.
pub fn send(fill: &Fill, out: &mut dyn Write) -> io::Result<()> {
    let mut out = io::BufWriter::new(out);
    for entry in &fill.entries {
        match entry {
            Entry::Dir(path) => header(&mut out, STREAM_DIR, path)?,
            Entry::File { path, executable } => {
                // Opened without following a symlink swapped in since the walk.
                let mut file = fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(nix::libc::O_NOFOLLOW)
                    .open(fill.from.join(path))?;
                let size = file.metadata()?.len();
                let tag = if *executable {
                    STREAM_EXECUTABLE
                } else {
                    STREAM_FILE
                };
                header(&mut out, tag, path)?;
                out.write_all(&size.to_be_bytes())?;
                let copied = io::copy(&mut io::Read::take(&mut file, size), &mut out)?;
                if copied != size {
                    return Err(io::Error::other(format!(
                        "{} shrank during the copy",
                        fill.from.join(path).display()
                    )));
                }
            }
            Entry::Symlink { path, target } => {
                header(&mut out, STREAM_SYMLINK, path)?;
                let target = target.as_os_str().as_bytes();
                out.write_all(&(target.len() as u32).to_be_bytes())?;
                out.write_all(target)?;
            }
        }
    }
    out.write_all(&[STREAM_END])?;
    out.flush()
}

/// Write `seeds` below `home`, a path relative to the disk's root, as the
/// stream `init receive --missing` reads: each seed's directories, then
/// the seed.
pub fn send_seeds(home: &Path, seeds: &[Seed], out: &mut dyn Write) -> io::Result<()> {
    let mut out = io::BufWriter::new(out);
    let mut dirs = BTreeSet::new();
    for seed in seeds {
        let path = home.join(&seed.path);
        let mut parents: Vec<&Path> = path
            .ancestors()
            .skip(1)
            .filter(|dir| !dir.as_os_str().is_empty())
            .collect();
        parents.reverse();
        for dir in parents {
            if dirs.insert(dir.to_path_buf()) {
                header(&mut out, STREAM_DIR, dir)?;
            }
        }
        header(&mut out, STREAM_FILE, &path)?;
        out.write_all(&(seed.contents.len() as u64).to_be_bytes())?;
        out.write_all(&seed.contents)?;
    }
    out.write_all(&[STREAM_END])?;
    out.flush()
}

/// What `disk rm` did.
pub enum Removal {
    Removed { existed: bool },
    InUse(String),
}

/// Remove a disk no box attaches.
pub fn remove(runtime: &dyn Runtime, name: &str) -> io::Result<Removal> {
    let _lock = ownership::disk_lock(name)?;
    if let Some((holder, _)) = attachments(runtime, None)?
        .remove(name)
        .and_then(|boxes| boxes.into_iter().next())
    {
        return Ok(Removal::InUse(holder));
    }
    let existed = runtime.list_disks()?.iter().any(|disk| disk.name == name);
    if existed {
        runtime.remove_disk(name)?;
    }
    unmark(name, UNCLEAN)?;
    unmark(name, UNFILLED)?;
    Ok(Removal::Removed { existed })
}
