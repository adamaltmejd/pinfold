//! Container runtimes: one per OS, behind a trait.

pub mod apple;
pub mod podman;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use tokio::process::Child;

use crate::core::plan::Plan;

/// The container runtime for this OS.
pub fn runtime() -> io::Result<&'static dyn Runtime> {
    if cfg!(target_os = "macos") {
        Ok(&apple::Apple)
    } else if cfg!(target_os = "linux") {
        Ok(&podman::Podman)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no container runtime for this OS",
        ))
    }
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

/// A runtime CLI's `--version` output, trimmed. `doctor` uses it.
fn cli_version(program: &str) -> io::Result<String> {
    let output = std::process::Command::new(program)
        .arg("--version")
        .output()
        .map_err(|error| spawn_error(program, error))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{program} --version: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
    /// Whether the box is running.
    pub state: BoxState,
}

/// A box's run state, as `box list` reports it.
#[derive(Debug, Clone, Copy)]
pub enum BoxState {
    Running,
    Stopped,
}

impl BoxState {
    /// The state a runtime's status string names. Only `running` is running;
    /// everything else, `unknown` included, is stopped.
    pub fn from_runtime(state: &str) -> BoxState {
        if state == "running" {
            BoxState::Running
        } else {
            BoxState::Stopped
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BoxState::Running => "running",
            BoxState::Stopped => "stopped",
        }
    }
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
}

/// The local image a reference resolves to.
#[derive(Debug, Clone)]
pub struct ImageIdentity {
    /// The image's content digest, bare hex like [`ImageInfo::id`].
    pub id: String,
    /// The labels recorded on the image.
    pub labels: BTreeMap<String, String>,
}

/// What preflight learned of the host that `up` needs, so one `up` asks the
/// runtime once. A field a runtime does not use is `None`.
#[derive(Debug, Default)]
pub struct Preflight {
    /// The seccomp profile path the runtime reports. podman only.
    pub seccomp_profile: Option<PathBuf>,
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

/// The runtime's build cache, as `pinfold clean` lists it.
pub enum BuildCache {
    /// The bytes [`Runtime::purge_build_cache`] would reclaim.
    Bytes(u64),
    /// A cache the runtime does not measure, named.
    Named(&'static str),
}

/// One OS's container runtime. Command lines are built as data.
pub trait Runtime: Sync {
    /// Start the attached `container run` process that owns the box. When
    /// `proxy_socket` is set, carry that host unix socket into the box.
    /// `preflight` is what [`Runtime::preflight`] returned.
    fn up(
        &self,
        plan: &Plan,
        init: &Path,
        proxy_socket: Option<&Path>,
        preflight: &Preflight,
    ) -> io::Result<Child>;

    /// Refuse a host the runtime cannot serve, before `up` creates any box
    /// state, and return what `up` needs of the host. The default is a
    /// runtime whose binary `up` checks itself.
    fn preflight(&self) -> io::Result<Preflight> {
        Ok(Preflight::default())
    }

    /// Make the carried proxy socket connectable by the box user. Apple only:
    /// the forwarded socket arrives root-owned and mode 000.
    fn make_proxy_connectable(&self, name: &str) -> io::Result<()>;

    /// Stop and remove the box.
    fn down(&self, name: &str) -> io::Result<()>;

    /// Run a command in a running box with inherited stdio. Callers wrap
    /// `argv` with [`exec_through_init`] first, so the box's init raises its
    /// `oom_score_adj` before exec'ing it.
    fn exec(
        &self,
        name: &str,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus>;

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
    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageIdentity, String>>;

    /// Remove one image by reference, and any layers no image references.
    fn remove_image(&self, reference: &str) -> io::Result<()>;

    /// Remove the runtime's build cache. Apple: the builder container and
    /// its layers. podman: the intermediate images pinfold's cached builds
    /// left that no image builds on.
    fn purge_build_cache(&self) -> io::Result<()>;

    /// What [`Runtime::purge_build_cache`] would remove, for `pinfold clean`.
    fn build_cache(&self) -> io::Result<BuildCache>;

    /// Build an image from [`BuildRequest`]. The inner `Err` is the build's
    /// output, stdout and stderr in order, when the build ran and failed.
    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>>;

    /// Pull `reference` and return the digest it resolved to. `None` when
    /// the reference does not resolve, for example `scratch`.
    fn image_digest(&self, reference: &str) -> io::Result<Option<String>>;

    /// The runtime's name, for `doctor`.
    fn name(&self) -> &'static str;

    /// The isolation the runtime gives each box, for `doctor`.
    fn isolation(&self) -> &'static str;

    /// The runtime CLI's version, as the CLI reports it.
    fn version(&self) -> io::Result<String>;
}

/// The path a host path appears at inside the box. The identity on Unix.
pub fn guest_path(host: &Path) -> PathBuf {
    host.to_path_buf()
}

/// The argv that runs `argv` in a box through its init: init raises its own
/// `oom_score_adj` to 1000 and then becomes `argv`, so the kernel's OOM
/// killer takes a box process before init. `init` is the host path
/// `up_argv` mounts at the same guest path and runs as `<init> init`, so the
/// composed command also carries the binary's `init` verb.
pub fn exec_through_init(init: &Path, argv: &[String]) -> Vec<String> {
    let mut wrapped = Vec::with_capacity(argv.len() + 4);
    wrapped.push(guest_path(init).to_string_lossy().into_owned());
    wrapped.push("init".to_string());
    wrapped.push("exec".to_string());
    wrapped.push("--".to_string());
    wrapped.extend_from_slice(argv);
    wrapped
}

/// Run a runtime's build argv with its stdout and stderr captured through
/// one pipe, so the output keeps its order. The inner `Err` is that output
/// when the build fails.
fn captured_build(argv: &[OsString]) -> io::Result<Result<(), String>> {
    let (program, arguments) = argv.split_first().expect("argv is never empty");
    let (mut reader, writer) = io::pipe()?;
    let mut command = std::process::Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    let child = command.spawn();
    // The command holds the pipe's write ends, and the read below ends only
    // once every write end is closed.
    drop(command);
    let mut child = child.map_err(|error| spawn_error(&program.to_string_lossy(), error))?;
    let mut output = Vec::new();
    reader.read_to_end(&mut output)?;
    if child.wait()?.success() {
        Ok(Ok(()))
    } else {
        Ok(Err(String::from_utf8_lossy(&output).into_owned()))
    }
}

/// The content digest of the image `reference` resolves to. `None` when the
/// runtime cannot resolve it. Unlike [`Runtime::image_digest`], this never
/// pulls: the reference is local.
pub fn local_image_id(runtime: &dyn Runtime, reference: &str) -> io::Result<Option<String>> {
    Ok(runtime.resolve_image(reference)?.ok().map(|image| image.id))
}

/// The uid:gid PID 1 and all work run as: the spec's, else the host user's.
pub(crate) fn user(plan: &Plan) -> OsString {
    match plan.user {
        Some(user) => {
            let (uid, gid) = (user.uid, user.gid);
            format!("{uid}:{gid}").into()
        }
        None => {
            let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
            format!("{uid}:{gid}").into()
        }
    }
}

/// A `type=bind` mount value, as both runtimes spell it.
pub(crate) fn bind(host: &Path, guest: &Path, readonly: bool) -> OsString {
    let mut value = OsString::from("type=bind,source=");
    value.push(host);
    value.push(",target=");
    value.push(guest);
    if readonly {
        value.push(",readonly");
    }
    value
}
