//! Run records: a run's launch record, terminal record and egress log, kept
//! in `runs/<run_id>/` past the box's teardown. The queries read only that
//! directory: no lock, no runtime client.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::core::{now, ownership, rfc3339};
use crate::dirs;

pub const EGRESS_LOG: &str = "egress.jsonl";
const LAUNCH: &str = "launch.json";
const TERMINAL: &str = "terminal.json";
/// The bound on a record, an events line and an events page.
const LIMIT_BYTES: usize = 64 << 10;
pub const MAX_EVENTS: usize = 100;
const INACTIVE_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
const INACTIVE_QUOTA: u64 = 128 << 20;

/// A run id is a generation: 64 lowercase hex characters.
pub fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn dir(run_id: &str) -> io::Result<PathBuf> {
    debug_assert!(valid_id(run_id));
    Ok(dirs::runs_dir()?.join(run_id))
}

/// The run's egress log path.
pub fn egress_log(run_id: &str) -> io::Result<PathBuf> {
    Ok(dir(run_id)?.join(EGRESS_LOG))
}

/// Make the run's directory, 0700, refusing a symlink at `runs/` or the run.
pub fn create(run_id: &str) -> io::Result<()> {
    let runs = dirs::runs_dir()?;
    fs::create_dir_all(runs.parent().expect("runs has a parent"))?;
    private_dir(&runs)?;
    fs::DirBuilder::new().mode(0o700).create(dir(run_id)?)
}

fn private_dir(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if fs::symlink_metadata(path)?.is_dir() {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "{} is not a directory",
                    path.display()
                )))
            }
        }
        Err(error) => Err(error),
    }
}

/// Open the egress log for append, 0600, following no symlink.
pub fn open_log(path: &Path) -> io::Result<File> {
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
}

/// Remove a run that never reached ready. A missing run is removed.
pub fn remove(run_id: &str) -> io::Result<()> {
    match fs::remove_dir_all(dir(run_id)?) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Write `record` as the run's `name`, atomically: a fresh 0600 file,
/// renamed into place.
fn write(run_id: &str, name: &str, record: &serde_json::Value) -> io::Result<()> {
    let dir = dir(run_id)?;
    let staging = dir.join(format!(".{name}.tmp"));
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&staging)
        .and_then(|mut file| {
            file.write_all(record.to_string().as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&staging, dir.join(name)));
    if written.is_err() {
        let _ = fs::remove_file(&staging);
    }
    written
}

/// Write the launch record. `record` holds every field but `run_id` and
/// `time`, built from values the box spec carries or `ready` prints.
pub fn write_launch(run_id: &str, mut record: serde_json::Value) -> io::Result<()> {
    record["run_id"] = run_id.into();
    record["time"] = rfc3339(now() as i64).into();
    write(run_id, LAUNCH, &record)
        .map_err(|error| io::Error::new(error.kind(), format!("launch record: {error}")))
}

/// How a run ended: the `down` line's reason and detail, or pi's
/// `harness-exited`.
pub struct Terminal {
    pub reason: &'static str,
    pub detail: Option<String>,
}

/// Write the terminal record. A failure is reported, never returned: it
/// changes neither `down` nor the exit code.
pub fn write_terminal(run_id: &str, terminal: &Terminal) {
    let record = serde_json::json!({
        "run_id": run_id,
        "time": rfc3339(now() as i64),
        "reason": terminal.reason,
        "detail": terminal.detail,
    });
    if let Err(error) = write(run_id, TERMINAL, &record) {
        eprintln!("pinfold: terminal record: {error}");
    }
}

/// A record's state and its value: `available` with the object, or
/// `unavailable` for a file that is unreadable, malformed, or over the bound.
/// `None` when the file is absent.
fn read_record(dir: &Path, name: &str) -> Option<(&'static str, serde_json::Value)> {
    let file = match File::options()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(dir.join(name))
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(_) => return Some(("unavailable", serde_json::Value::Null)),
    };
    let mut bytes = Vec::new();
    let parsed = file
        .take(LIMIT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()
        .filter(|&read| read <= LIMIT_BYTES)
        .and_then(|_| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(serde_json::Value::is_object);
    Some(match parsed {
        Some(record) => ("available", record),
        None => ("unavailable", serde_json::Value::Null),
    })
}

/// A run's directory and its launch record, read once per query.
struct Records {
    dir: PathBuf,
    launch: Option<(&'static str, serde_json::Value)>,
}

impl Records {
    /// `None` for a run with no records.
    fn read(run_id: &str) -> io::Result<Option<Records>> {
        let dir = dir(run_id)?;
        match fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
        let launch = read_record(&dir, LAUNCH);
        Ok(Some(Records { dir, launch }))
    }

    fn no_egress(&self) -> bool {
        matches!(&self.launch, Some(("available", record)) if record["egress"].is_null())
    }

    fn log(&self) -> io::Result<Option<File>> {
        match File::options()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(self.dir.join(EGRESS_LOG))
        {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// `box inspect`: the run's records and its log's state. `None` for a run
/// with no records.
pub fn inspect(run_id: &str) -> io::Result<Option<serde_json::Value>> {
    let Some(records) = Records::read(run_id)? else {
        return Ok(None);
    };
    let terminal = read_record(&records.dir, TERMINAL);
    let log = records.log().ok().flatten();
    if records.launch.is_none() && terminal.is_none() && log.is_none() {
        return Ok(None);
    }
    let state = |record: Option<(&str, serde_json::Value)>, absent: &str| match record {
        Some((state, record)) => serde_json::json!({ "state": state, "record": record }),
        None => serde_json::json!({ "state": absent, "record": null }),
    };
    let egress_log = if records.no_egress() {
        serde_json::json!({ "state": "not-applicable", "path": null, "bytes": null })
    } else {
        match log.as_ref().map(File::metadata) {
            Some(Ok(metadata)) => serde_json::json!({
                "state": "available",
                "path": records.dir.join(EGRESS_LOG).to_str(),
                "bytes": metadata.len(),
            }),
            _ => serde_json::json!({ "state": "unavailable", "path": null, "bytes": null }),
        }
    };
    Ok(Some(serde_json::json!({
        "run_id": run_id,
        "launch": state(records.launch, "unavailable"),
        "terminal": state(terminal, "not-observed"),
        "egress_log": egress_log,
    })))
}

/// An `events` position: a byte offset in one run's log. The run's prefix
/// makes another run's cursor a malformed argument.
pub struct Cursor(pub u64);

impl Cursor {
    pub fn parse(run_id: &str, cursor: &str) -> Option<Cursor> {
        let (offset, run) = cursor.split_once('-')?;
        (run == &run_id[..16] && offset.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| offset.parse().ok().map(Cursor))
            .flatten()
    }

    fn format(&self, run_id: &str) -> String {
        format!("{}-{}", self.0, &run_id[..16])
    }
}

/// `box events`: the log's complete lines after `after`, at most `limit`
/// and [`LIMIT_BYTES`]. `None` for a run with no records.
pub fn events(
    run_id: &str,
    limit: usize,
    after: Option<Cursor>,
) -> io::Result<Option<serde_json::Value>> {
    let Some(records) = Records::read(run_id)? else {
        return Ok(None);
    };
    let page = |state: &str, events: Vec<serde_json::Value>, next: Option<u64>, more: bool| {
        serde_json::json!({
            "run_id": run_id,
            "state": state,
            "events": events,
            "next": next.map(|offset| Cursor(offset).format(run_id)),
            "more": more,
        })
    };
    if records.no_egress() {
        return Ok(Some(page("not-applicable", Vec::new(), None, false)));
    }
    let log = match records.log() {
        Ok(Some(log)) => log,
        Ok(None) if records.launch.is_none() => return Ok(None),
        _ => return Ok(Some(page("unavailable", Vec::new(), None, false))),
    };
    let mut offset = after.map_or(0, |cursor| cursor.0);
    let mut reader = BufReader::new(log);
    if reader.seek(SeekFrom::Start(offset)).is_err() {
        return Ok(Some(page("unavailable", Vec::new(), None, false)));
    }
    let (mut events, mut bytes, mut more) = (Vec::new(), 0, false);
    loop {
        let line = match next_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(_) => return Ok(Some(page("unavailable", Vec::new(), None, false))),
        };
        let event = line
            .text
            .as_deref()
            .and_then(|text| serde_json::from_slice::<serde_json::Value>(text).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({ "unreadable": true }));
        let size = event.to_string().len();
        if events.len() == limit || (!events.is_empty() && bytes + size > LIMIT_BYTES) {
            more = true;
            break;
        }
        events.push(event);
        bytes += size;
        offset += line.consumed;
    }
    let next = (!events.is_empty()).then_some(offset);
    Ok(Some(page("available", events, next, more)))
}

/// One complete line: its text without the newline (`None` when over
/// [`LIMIT_BYTES`]) and the bytes it took.
struct Line {
    text: Option<Vec<u8>>,
    consumed: u64,
}

/// The next complete line, or `None` at the end or before an incomplete
/// final line. An oversized line is read through to its newline.
fn next_line(reader: &mut impl BufRead) -> io::Result<Option<Line>> {
    let mut text = Vec::new();
    let mut consumed = 0u64;
    let mut oversized = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(None);
        }
        let (chunk, done) = match buffer.iter().position(|&byte| byte == b'\n') {
            Some(end) => (&buffer[..end], Some(end + 1)),
            None => (buffer, None),
        };
        if !oversized {
            text.extend_from_slice(chunk);
            if text.len() > LIMIT_BYTES {
                oversized = true;
                text = Vec::new();
            }
        }
        let used = done.unwrap_or(buffer.len());
        consumed += used as u64;
        reader.consume(used);
        if done.is_some() {
            return Ok(Some(Line {
                text: (!oversized).then_some(text),
                consumed,
            }));
        }
    }
}

/// One run directory on disk, for retention.
struct Stored {
    path: PathBuf,
    newest: SystemTime,
    bytes: u64,
}

/// The run directories maintenance removes: inactive runs whose newest file
/// is older than 14 days, then the oldest inactive runs while the inactive
/// rest exceed 128 MiB. The old name-keyed `egress/` directory goes too. A
/// run is live while its generation owns its name's lifetime lock.
pub fn retired() -> io::Result<Vec<PathBuf>> {
    let mut retired = Vec::new();
    let legacy = dirs::state_dir()?.join("egress");
    if fs::symlink_metadata(&legacy).is_ok() {
        retired.push(legacy);
    }
    let records = ownership::records()?;
    let live: BTreeSet<&str> = records
        .iter()
        .filter(|record| ownership::name_alive(&record.name).unwrap_or(true))
        .map(|record| record.generation.as_str())
        .collect();
    let mut inactive = Vec::new();
    for entry in dirs::entries(&dirs::runs_dir()?)? {
        let path = entry.path();
        let Some(run_id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !valid_id(run_id) || !entry.file_type()?.is_dir() || live.contains(run_id) {
            continue;
        }
        if launched_box_alive(&path, run_id) {
            continue;
        }
        let (mut newest, mut bytes) = (None, 0);
        for file in dirs::entries(&path)? {
            let metadata = file.metadata()?;
            newest = newest.max(Some(metadata.modified()?));
            bytes += metadata.len();
        }
        // An empty run, from an owner killed while starting, ages by its
        // directory.
        let newest = match newest {
            Some(newest) => newest,
            None => entry.metadata()?.modified()?,
        };
        inactive.push(Stored {
            path,
            newest,
            bytes,
        });
    }
    inactive.sort_by_key(|run| run.newest);
    let mut kept: u64 = inactive.iter().map(|run| run.bytes).sum();
    for run in inactive {
        let old = run.newest.elapsed().is_ok_and(|age| age > INACTIVE_AGE);
        if old || kept > INACTIVE_QUOTA {
            kept -= run.bytes;
            retired.push(run.path);
        }
    }
    Ok(retired)
}

/// Whether the launch record's box name is held while no other generation
/// is published for it: an owner between removing its ownership record and
/// exiting still holds the lock, and its run is still live.
fn launched_box_alive(dir: &Path, run_id: &str) -> bool {
    let Some(("available", launch)) = read_record(dir, LAUNCH) else {
        return false;
    };
    let Some(name) = launch["box"].as_str() else {
        return false;
    };
    ownership::name_alive(name).unwrap_or(true)
        && ownership::record(name).map_or(true, |record| {
            record.is_none_or(|record| record.generation == run_id)
        })
}
