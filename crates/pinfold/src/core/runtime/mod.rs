//! Box runtimes: one per OS, behind a trait.

pub mod apple;
#[cfg(target_os = "linux")]
pub mod firecracker;
#[cfg(target_os = "linux")]
pub mod podman;

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::AtomicBool;

use serde::de::DeserializeOwned;

use crate::core::disk::{Fill, Seeding};
use crate::core::plan::{Env, Plan};
use crate::core::proxy::PROXY_URL;

const BUILD_LOG_BYTES: usize = 64 * 1024;

/// The runtime's native Linux image architecture.
pub const TARGET_ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "amd64"
};

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

/// A VMM's exit code, or `128+n` for signal `n`, as the exit status a
/// runtime client would report: `exited` names 137 for a SIGKILLed VMM.
#[cfg(target_os = "linux")]
fn exit_status(code: i32) -> ExitStatus {
    std::os::unix::process::ExitStatusExt::from_raw((code & 0xff) << 8)
}

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
fn spawn_error(program: &str, error: io::Error) -> io::Error {
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
fn captured(argv: &[&str]) -> io::Result<Result<Vec<u8>, String>> {
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

/// Run `<program> image inspect <reference>` and return its first entry. The
/// inner `Err` is the runtime's stderr when it cannot resolve the reference.
fn inspect<T: DeserializeOwned>(program: &str, reference: &str) -> io::Result<Result<T, String>> {
    let json = match captured(&[program, "image", "inspect", reference])? {
        Ok(json) => json,
        Err(stderr) => return Ok(Err(stderr)),
    };
    let what = format!("{program} image inspect");
    let images: Vec<T> = parse_json(&what, &json)?;
    images
        .into_iter()
        .next()
        .map(Ok)
        .ok_or_else(|| io::Error::other(format!("{what} returned no image")))
}

/// Parse `json`, the output of the command `what` names.
fn parse_json<T: DeserializeOwned>(what: &str, json: &[u8]) -> io::Result<T> {
    serde_json::from_slice(json)
        .map_err(|error| io::Error::other(format!("{what} returned invalid JSON: {error}")))
}

/// One box the runtime knows about, running or not.
#[derive(Debug, Clone)]
pub struct BoxInfo {
    pub id: String,
    pub labels: BTreeMap<String, String>,
    /// The id of the image the box runs, bare hex like [`ImageInfo::id`].
    pub image_id: String,
    /// The image reference the runtime records for the box, in its own
    /// spelling.
    pub image_ref: String,
    /// When the runtime created the box, RFC 3339 in UTC.
    pub created: String,
    /// Whether the runtime reports the box `running`. Any other state,
    /// `unknown` included, is stopped.
    pub running: bool,
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

/// One image the runtime knows about.
#[derive(Debug, Clone)]
pub struct ImageInfo {
    /// The image's content digest; references to one image share it.
    pub id: String,
    /// A reference that names the image, for removal.
    pub reference: String,
    /// The labels recorded on the image.
    pub labels: BTreeMap<String, String>,
    /// The digest the runtime records for the image, `sha256:<hex>`. Only
    /// [`Runtime::resolve_image`] fills it on podman.
    pub digest: Option<String>,
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

/// One image build: a context, a Containerfile, the names to tag the result
/// with, and the labels to record on it.
pub struct BuildRequest<'a> {
    /// The build context.
    pub context: &'a Path,
    /// The Containerfile, inside the context or not.
    pub containerfile: &'a Path,
    /// The image names to tag the built image with.
    pub tags: &'a [String],
    /// Labels to put on the image, for Maintenance to find it by.
    pub labels: &'a BTreeMap<String, String>,
    /// Whether the runtime may reuse its layer cache. Without it every step
    /// reruns.
    pub cache: bool,
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

    /// List every image, with the labels recorded on it.
    fn list_images(&self) -> io::Result<Vec<ImageInfo>>;

    /// Resolve `reference` to a local image with the runtime's own inspect,
    /// so every spelling the runtime resolves is accepted. Never pulls. The
    /// inner `Err` is the runtime's message when it cannot resolve it.
    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>>;

    /// Remove one image by reference, and any layers no image references.
    fn remove_image(&self, reference: &str) -> io::Result<()>;

    /// Remove unused build cache without interrupting builds. Apple: the
    /// builder's unused records. podman: the intermediate images pinfold's cached builds
    /// left that no image builds on.
    fn purge_build_cache(&self) -> io::Result<()>;

    /// What [`Runtime::purge_build_cache`] removes, named for `pinfold
    /// clean` and `doctor`.
    fn build_cache(&self) -> &'static str;

    /// Allocated build-cache storage, including its backing filesystem.
    /// None when the runtime cannot measure it separately.
    fn build_cache_bytes(&self) -> io::Result<Option<u64>> {
        Ok(None)
    }

    /// Every disk the runtime stores.
    fn list_disks(&self) -> io::Result<Vec<DiskInfo>>;

    /// Create an empty disk of `bytes`.
    fn create_disk(&self, name: &str, bytes: u64) -> io::Result<()>;

    /// Remove a disk no box attaches.
    fn remove_disk(&self, name: &str) -> io::Result<()>;

    /// Make the writes a box finished on the disk durable on the host.
    fn sync_disk(&self, name: &str) -> io::Result<()>;

    /// Build an image from [`BuildRequest`]. The inner `Err` is the build's
    /// output, stdout and stderr in order, when the build ran and failed.
    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>>;

    /// The image CLI's program name: `container` on macOS, `podman` on
    /// Linux.
    fn program(&self) -> &'static str;

    /// The runtime's name, for `doctor`.
    fn name(&self) -> &'static str;

    /// The isolation the runtime gives each box, for `doctor`.
    fn isolation(&self) -> &'static str;

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

/// Build `request` with `program`; `cache_flags` is how that runtime spells
/// the request's cache choice. stdout and stderr are captured through one
/// pipe, so the output keeps its order. The inner `Err` is that output when
/// the build fails.
fn build(
    program: &str,
    cache_flags: &[&str],
    request: &BuildRequest,
) -> io::Result<Result<(), String>> {
    let (mut reader, writer) = io::pipe()?;
    let mut command = std::process::Command::new(program);
    if cfg!(target_os = "macos") {
        // Apple includes color settings in the shared builder's identity
        // (GitHub #77). Every pinfold caller must use the same settings.
        command.env_remove("NO_COLOR").env_remove("BUILDKIT_COLORS");
    }
    command
        .arg("build")
        .args(["--build-arg", &format!("TARGETARCH={TARGET_ARCH}")])
        .args(cache_flags)
        .arg("--file")
        .arg(request.containerfile);
    for tag in request.tags {
        command.arg("--tag").arg(tag);
    }
    for (key, value) in request.labels {
        command.arg("--label").arg(format!("{key}={value}"));
    }
    command
        .arg(request.context)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    let child = command.spawn();
    // The command holds the pipe's write ends, and the read below ends only
    // once every write end is closed.
    drop(command);
    let mut child = child.map_err(|error| spawn_error(program, error))?;
    let mut tail = VecDeque::new();
    let mut newlines = 0;
    let mut buffer = [0u8; 8192];
    loop {
        let size = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => size,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        for &byte in &buffer[..size] {
            tail.push_back(byte);
            newlines += usize::from(byte == b'\n');
            while tail.len() > BUILD_LOG_BYTES
                || newlines + usize::from(tail.back() != Some(&b'\n')) > 40
            {
                newlines -= usize::from(tail.pop_front() == Some(b'\n'));
            }
        }
    }
    if child.wait()?.success() {
        Ok(Ok(()))
    } else {
        let bytes: Vec<u8> = tail.into_iter().collect();
        let mut output = String::from_utf8_lossy(&bytes).into_owned();
        // Replacement characters can expand malformed runtime output.
        let start = output.ceil_char_boundary(output.len().saturating_sub(BUILD_LOG_BYTES));
        output.drain(..start);
        Ok(Err(output))
    }
}
