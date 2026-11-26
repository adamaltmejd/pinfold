//! Box runtimes: one per OS, behind a trait.

pub mod apple;
pub(crate) mod exec_transport;
#[cfg(target_os = "linux")]
pub mod firecracker;
pub(crate) mod sockets;
pub(crate) mod terminal;

use crate::core::process::CommandGuard;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::AtomicBool;

use serde::de::DeserializeOwned;

use crate::core::disk::{Directory, Fill, Seeding};
use crate::core::plan::{Env, Plan, Values};
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

/// What [`prepare`] made for one start: on Apple, the CA bundle file of a
/// box with a routed host.
#[cfg(not(target_os = "linux"))]
pub struct Prepared(Option<PathBuf>);

/// The slow, cancellable part of a start, run on a blocking thread before
/// [`start`]: on Linux the VMM's install, the root and init disks and the
/// mounts' images; on Apple the CA bundle. `image_id` is the image `up`
/// resolved and leased.
pub fn prepare(
    plan: &Plan,
    state_dir: &Path,
    sockets: &sockets::Sockets,
    image_id: &str,
    cancel: &AtomicBool,
    generated: &[PathBuf],
    init: &Path,
) -> io::Result<Prepared> {
    #[cfg(target_os = "linux")]
    {
        let _ = init;
        firecracker::prepare(plan, state_dir, sockets, image_id, cancel, generated)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (sockets, generated);
        let bundle = plan
            .trust
            .as_deref()
            .map(|ca| apple::trust_bundle(plan, init, state_dir, image_id, ca, cancel))
            .transpose()?;
        Ok(Prepared(bundle))
    }
}

/// Start the box. Linux boot applies initial fills before generated mounts
/// and seeds before ready. Apple populated disks under the claim and applies
/// seeds in [`Runtime::finish_start`].
#[allow(clippy::too_many_arguments)]
pub fn start(
    prepared: Prepared,
    plan: &Plan,
    values: &Values,
    init: &Path,
    proxy_socket: Option<&Path>,
    fills: &[Fill],
    seeding: Option<&Seeding>,
    directories: &[Directory],
) -> io::Result<Child> {
    #[cfg(target_os = "linux")]
    {
        let _ = init;
        firecracker::Vm::up(
            prepared,
            plan,
            values,
            proxy_socket,
            fills,
            seeding,
            directories,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (Prepared(bundle), _, _, _) = (prepared, fills, seeding, directories);
        apple::up(plan, values, init, proxy_socket, bundle.as_deref())
    }
}

#[cfg(not(target_os = "linux"))]
pub use apple::Client as Child;
/// What `box up` holds for its box: on Linux the VM it runs itself, on
/// macOS the attached runtime client. Each has `ready`, `wait`, `down` and
/// `abort`.
#[cfg(target_os = "linux")]
pub use firecracker::Vm as Child;

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
        .output_guarded()
        .map_err(|error| spawn_error(program, error))?;
    if !output.status.success() {
        return Ok(Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()));
    }
    Ok(Ok(output.stdout))
}

/// The Apple container service cannot be reached. Only the Apple adapter
/// makes it, so [`unreachable`] matches no other failure.
#[derive(Debug)]
struct Unreachable;

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cannot reach the container service; run `container system start`")
    }
}

impl std::error::Error for Unreachable {}

/// Whether `error` says the Apple container service cannot be reached.
pub fn unreachable(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<Unreachable>())
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
    /// The box's memory limit in bytes, as the runtime records it.
    pub memory_limit: Option<u64>,
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

/// Metadata for one command, independent of the box's environment.
#[derive(Default)]
pub struct ExecContext<'a> {
    /// A project command's guest project root and its working directory
    /// relative to it.
    pub project_directory: Option<(&'a Path, &'a Path)>,
    pub tty: bool,
    pub agent: Option<&'a str>,
    pub cancel: Option<&'a AtomicBool>,
    /// A project command's stdin, stdout and stderr in place of pinfold's.
    pub stdio: Option<[BorrowedFd<'a>; 3]>,
}

/// One OS's box runtime. Command lines and VM configurations are built as
/// data.
pub trait Runtime: Sync {
    /// Refuse a host the runtime cannot serve, before `up` creates any box
    /// state.
    fn preflight(&self) -> io::Result<()>;

    /// Complete backend initialization after init's ready protocol, then
    /// return the box as inventory observes it. The caller publishes ready
    /// and records disks filled only after this succeeds.
    fn finish_start(
        &self,
        name: &str,
        init: &Path,
        proxy_socket: Option<&Path>,
        seeding: Option<&Seeding>,
        cancel: &AtomicBool,
    ) -> io::Result<BoxInfo>;

    /// Stop and remove the box of an owner that is gone.
    fn down(&self, name: &str) -> io::Result<()>;

    /// Run `argv` in a running box with inherited or given stdio, through the box's
    /// init: init raises its own `oom_score_adj` to 1000 and then becomes
    /// `argv`, so the kernel's OOM killer takes a box process before init.
    /// `init` is the host path Apple's `up` mounts at the same guest path;
    /// Firecracker's guest has its own. The process inherits the run's
    /// user, so box-created files stay the host user's. TTY exec preserves
    /// the host's terminal identity.
    fn exec(
        &self,
        name: &str,
        init: &Path,
        context: &ExecContext<'_>,
        argv: &[String],
    ) -> io::Result<ExitStatus>;

    /// The command [`Runtime::exec`] runs, for a caller that pipes its stdio.
    fn exec_command(&self, name: &str, init: &Path, argv: &[String]) -> std::process::Command;

    /// One box's OOM kills, memory and pids facts. `name` is the box
    /// `list` names. Fields the runtime cannot answer are `None`.
    fn stat(&self, name: &str) -> io::Result<BoxStat>;

    /// Advisory inventory. Mutation must recheck the relevant claims.
    fn list(&self) -> io::Result<Vec<BoxInfo>>;

    fn inspect(&self, name: &str) -> io::Result<Option<BoxInfo>>;

    /// Every disk the runtime stores.
    fn list_disks(&self) -> io::Result<Vec<DiskInfo>>;

    /// Create an empty disk of `bytes`.
    fn create_disk(&self, name: &str, bytes: u64) -> io::Result<()>;

    /// Give fresh roots to the box user and prepare directories on fresh
    /// or reused disks. The caller holds the disk locks and owns rollback.
    #[allow(clippy::too_many_arguments)]
    fn own_disks(
        &self,
        names: &[&str],
        plan: &Plan,
        init: &Path,
        generation: &str,
        directories: &[Directory],
        fills: &[Fill],
        cancel: &AtomicBool,
    ) -> io::Result<()>;

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
/// `from` entry's value from `values` under the entry's key, then the
/// always-applied variables.
fn guest_vars(plan: &Plan, values: &Values) -> Vec<(String, OsString)> {
    let mut vars: Vec<(String, OsString)> = plan
        .env
        .iter()
        .filter_map(|(name, value)| {
            let value = match value {
                Env::Exact(value) => Some(OsString::from(value)),
                Env::Secret { .. } => values.get(&value.source()?.ok()?).map(OsString::from),
            };
            Some((name.clone(), value?))
        })
        .collect();
    // Node's fetch reads the proxy
    // variables only with this set. These override any spec entries with the
    // same names.
    vars.push(("NODE_USE_ENV_PROXY".into(), "1".into()));
    if plan.egress.is_some() {
        vars.push(("HTTPS_PROXY".into(), PROXY_URL.into()));
        vars.push(("http_proxy".into(), PROXY_URL.into()));
    }
    vars
}
