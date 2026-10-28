//! Box runtimes: one per OS, behind a trait.

pub mod apple;
#[cfg(target_os = "linux")]
pub mod firecracker;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::AtomicBool;

use serde::de::DeserializeOwned;

use crate::core::disk::{Fill, Seeding};
use crate::core::plan::{Env, Plan};
use crate::core::proxy::PROXY_URL;

/// The box runtime for this OS: Apple `container` on macOS, Firecracker on
/// Linux, the only targets pinfold builds for.
pub fn runtime() -> &'static dyn Runtime {
    #[cfg(target_os = "linux")]
    return &firecracker::Firecracker;
    #[cfg(not(target_os = "linux"))]
    return &apple::Apple;
}

#[cfg(target_os = "linux")]
pub use firecracker::Prepared;

/// What [`prepare`] made for one start. Apple needs nothing.
#[cfg(not(target_os = "linux"))]
pub struct Prepared;

/// The slow, cancellable part of a start, run on a blocking thread before
/// [`start`]: on Linux the VMM's install, the root and init disks and the
/// mounts' images. `image_id` is the image `up` resolved; a tag that moved
/// since fails as `image-changed`.
pub fn prepare(
    plan: &Plan,
    state_dir: &Path,
    image_id: &str,
    cancel: &AtomicBool,
) -> io::Result<Prepared> {
    #[cfg(target_os = "linux")]
    return firecracker::prepare(plan, state_dir, image_id, cancel);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (plan, state_dir, image_id, cancel);
        Ok(Prepared)
    }
}

/// Start the box. On Linux the boot carries the new disks' `fills` and the
/// profile's `seeding`; Apple's caller copies them in after ready.
pub fn start(
    prepared: Prepared,
    plan: &Plan,
    init: &Path,
    proxy_socket: Option<&Path>,
    fills: &[Fill],
    seeding: Option<&Seeding>,
) -> io::Result<Child> {
    #[cfg(target_os = "linux")]
    {
        let _ = init;
        firecracker::Vm::up(prepared, plan, proxy_socket, fills, seeding)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (Prepared, _, _) = (prepared, fills, seeding);
        apple::up(plan, init, proxy_socket)
    }
}

#[cfg(not(target_os = "linux"))]
pub use apple::Client as Child;
/// What `box up` holds for its box: on Linux the VM it runs itself, on
/// macOS the attached runtime client. Each has `ready`, `wait`, `down` and
/// `abort`.
#[cfg(target_os = "linux")]
pub use firecracker::Vm as Child;

pub(crate) fn require_version(program: &str, version: &str, minimum: [u64; 3]) -> io::Result<()> {
    let release: Option<[u64; 3]> = version
        .split('+')
        .next()
        .unwrap_or_default()
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .and_then(|parts| parts.try_into().ok());
    if !release.is_some_and(|release| release >= minimum) {
        let [major, minor, patch] = minimum;
        return Err(io::Error::other(format!(
            "{program} reports version {version:?}; pinfold requires stable {program} {major}.{minor}.{patch} or newer"
        )));
    }
    Ok(())
}

/// Turn a failed spawn into pinfold's message. A missing binary is named as
/// missing; any other failure names the program and keeps the OS text.
pub(crate) fn spawn_error(program: &str, error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::NotFound {
        io::Error::new(
            error.kind(),
            format!("{program} is not installed or not on PATH"),
        )
    } else {
        io::Error::new(error.kind(), format!("{program}: {error}"))
    }
}

/// Run `argv` with stdin closed and return its stdout. The inner `Err` is
/// its stderr when the run fails; a failed spawn keeps its kind.
pub(crate) fn captured(argv: &[&str]) -> io::Result<Result<Vec<u8>, String>> {
    let (program, arguments) = argv.split_first().expect("argv is never empty");
    let output = std::process::Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| spawn_error(program, error))?;
    if !output.status.success() {
        return Ok(Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()));
    }
    Ok(Ok(output.stdout))
}

/// Run `argv` with stdin closed and return its stdout. A failed run's error
/// names the command and carries its stderr; a failed spawn keeps its kind.
pub(crate) fn output(argv: &[&str]) -> io::Result<Vec<u8>> {
    captured(argv)?.map_err(|stderr| io::Error::other(format!("{}: {stderr}", argv.join(" "))))
}

/// Parse `json`, the output of the command `what` names.
pub(crate) fn parse_json<T: DeserializeOwned>(what: &str, json: &[u8]) -> io::Result<T> {
    serde_json::from_slice(json)
        .map_err(|error| io::Error::other(format!("{what} returned invalid JSON: {error}")))
}

/// One box the runtime knows about, running or not.
#[derive(Debug, Clone)]
pub struct BoxInfo {
    pub id: String,
    pub labels: BTreeMap<String, String>,
    /// The id of the image the box runs, bare hex like [`crate::core::image::ImageInfo::id`].
    pub image_id: String,
    /// The image reference the runtime records for the box, in its own
    /// spelling.
    pub image_ref: String,
    /// When the runtime created the box, RFC 3339 in UTC.
    pub created: String,
    /// Whether the runtime reports the box `running`.
    pub running: bool,
    /// Whether the runtime reports the box in its terminal stopped state. A
    /// box starting, stopping or `unknown` may still write its disks.
    pub stopped: bool,
}

/// One box's run facts, as `pinfold box stat` reports them. A field is
/// `None` when the runtime cannot answer it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BoxStat {
    /// The box `list` names.
    #[serde(rename = "box")]
    pub name: String,
    /// The number of times the box's cgroup killed a process for memory.
    /// Monotonic for the box's life; the caller keeps its own baseline.
    pub oom_kills: Option<u64>,
    pub memory: MemoryStat,
    pub pids: PidsStat,
}

/// One box's memory use and limit, in bytes.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MemoryStat {
    pub current: Option<u64>,
    pub peak: Option<u64>,
    pub limit: Option<u64>,
}

/// One box's pids use and limit.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PidsStat {
    pub current: Option<u64>,
    pub limit: Option<u64>,
}

/// One disk the runtime stores.
#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    /// The disk's size in bytes.
    pub size: u64,
    /// The host blocks its image holds, in bytes.
    pub allocated: u64,
}

/// One OS's box runtime. Command lines and VM configurations are built as
/// data.
pub trait Runtime: Sync {
    /// Refuse a host the runtime cannot serve, before `up` creates any box
    /// state.
    fn preflight(&self) -> io::Result<()>;

    /// Where the box's proxy listens, in its claimed `state_dir`.
    fn proxy_socket(&self, state_dir: &Path) -> io::Result<PathBuf> {
        Ok(state_dir.join("proxy.sock"))
    }

    /// Stop and remove the box of an owner that is gone.
    fn down(&self, name: &str) -> io::Result<()>;

    /// Run `argv` in a running box with inherited stdio, through the box's
    /// init: init raises its own `oom_score_adj` to 1000 and then becomes
    /// `argv`, so the kernel's OOM killer takes a box process before init.
    /// `init` is the host path Apple's `up` mounts at the same guest path;
    /// Firecracker's guest has its own. The process inherits the run's
    /// user, so box-created files stay the host user's.
    fn exec(
        &self,
        name: &str,
        init: &Path,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus>;

    /// The command [`Runtime::exec`] runs, for a caller that pipes its stdio.
    fn exec_command(
        &self,
        name: &str,
        init: &Path,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> std::process::Command;

    /// One box's OOM kills, memory and pids facts. `name` is the box
    /// `list` names. Fields the runtime cannot answer are `None`.
    fn stat(&self, name: &str) -> io::Result<BoxStat>;

    /// List every box, running or not.
    fn list(&self) -> io::Result<Vec<BoxInfo>>;

    /// Every disk the runtime stores.
    fn list_disks(&self) -> io::Result<Vec<DiskInfo>>;

    /// Create an empty disk of `bytes`.
    fn create_disk(&self, name: &str, bytes: u64) -> io::Result<()>;

    /// Remove a disk no box attaches.
    fn remove_disk(&self, name: &str) -> io::Result<()>;

    /// Make the writes a box finished on the disk durable on the host.
    fn sync_disk(&self, name: &str) -> io::Result<()>;

    /// The runtime's name and the isolation it gives each box, for `doctor`.
    fn describe(&self) -> &'static str;

    /// The box runtime's version.
    fn version(&self) -> io::Result<String>;
}

/// The box's environment in order, a later entry winning: the spec's, a
/// `from` entry read from pinfold's own environment under the entry's key,
/// then the always-applied variables.
fn guest_vars(plan: &Plan) -> Vec<(String, OsString)> {
    let mut vars: Vec<(String, OsString)> = plan
        .env
        .iter()
        .filter_map(|(name, value)| {
            let value = match value {
                Env::Exact(value) => Some(OsString::from(value)),
                Env::From { from } => std::env::var_os(from),
            };
            Some((name.clone(), value?))
        })
        .collect();
    // Always applied: Node's fetch reads the proxy variables only with this
    // set. These override any spec entries with the same names.
    vars.push(("NODE_USE_ENV_PROXY".into(), "1".into()));
    if plan.egress.is_some() {
        vars.push(("HTTPS_PROXY".into(), PROXY_URL.into()));
        vars.push(("http_proxy".into(), PROXY_URL.into()));
    }
    vars
}
