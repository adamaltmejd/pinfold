//! Container runtimes: one per OS, behind a trait.

pub mod apple;
pub mod podman;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use serde::de::DeserializeOwned;
use tokio::process::{Child, Command};

use crate::core::plan::{Env, Plan};
use crate::core::proxy::PROXY_URL;

/// The container runtime for this OS: Apple `container` on macOS, rootless
/// podman on Linux, the only targets pinfold builds for.
pub fn runtime() -> &'static dyn Runtime {
    if cfg!(target_os = "macos") {
        &apple::Apple
    } else {
        &podman::Podman
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

/// One OS's container runtime. Command lines are built as data.
pub trait Runtime: Sync {
    /// Start the attached `container run` process that owns the box. When
    /// `proxy_socket` is set, carry that host unix socket into the box.
    /// `seccomp` is the profile [`Runtime::preflight`] returned.
    fn up(
        &self,
        plan: &Plan,
        init: &Path,
        proxy_socket: Option<&Path>,
        seccomp: Option<&Path>,
    ) -> io::Result<Child>;

    /// Refuse a host the runtime cannot serve, before `up` creates any box
    /// state, and return the seccomp profile the runtime reports, which
    /// `up` needs. podman only; Apple returns `None`.
    fn preflight(&self) -> io::Result<Option<PathBuf>>;

    /// Stop and remove the box.
    fn down(&self, name: &str) -> io::Result<()>;

    /// Run `argv` in a running box with inherited stdio, through the box's
    /// init: init raises its own `oom_score_adj` to 1000 and then becomes
    /// `argv`, so the kernel's OOM killer takes a box process before init.
    /// `init` is the host path `up` mounts at the same guest path. The
    /// process inherits the run's user, so box-created files stay the host
    /// user's.
    fn exec(
        &self,
        name: &str,
        init: &Path,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus> {
        let program = self.program();
        let mut command = std::process::Command::new(program);
        command.args(["exec", "-i"]);
        if tty {
            // Keep the host's terminal identity: the runtime otherwise
            // reports its own `TERM`.
            command.arg("-t");
            for variable in ["TERM", "COLORTERM"] {
                if let Ok(value) = std::env::var(variable) {
                    command.arg("--env").arg(format!("{variable}={value}"));
                }
            }
        } else {
            // Without a TTY the runtime's exec gets its own process group, so
            // a terminal signal reaches pinfold and not the exec.
            command.process_group(0);
        }
        if let Some(dir) = workdir {
            command.arg("--workdir").arg(dir);
        }
        command
            .arg(name)
            .arg(init)
            .args(["init", "exec", "--"])
            .args(argv)
            .status()
            .map_err(|error| spawn_error(program, error))
    }

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

    /// Remove the runtime's build cache. Apple: the builder container and
    /// its layers. podman: the intermediate images pinfold's cached builds
    /// left that no image builds on.
    fn purge_build_cache(&self) -> io::Result<()>;

    /// What [`Runtime::purge_build_cache`] removes, named for `pinfold
    /// clean` and `doctor`. Its size is not measured.
    fn build_cache(&self) -> &'static str;

    /// Build an image from [`BuildRequest`]. The inner `Err` is the build's
    /// output, stdout and stderr in order, when the build ran and failed.
    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>>;

    /// The runtime CLI's program name.
    fn program(&self) -> &'static str;

    /// The runtime's name, for `doctor`.
    fn name(&self) -> &'static str;

    /// The isolation the runtime gives each box, for `doctor`.
    fn isolation(&self) -> &'static str;

    /// The runtime CLI's version, as the CLI reports it.
    fn version(&self) -> io::Result<String> {
        let stdout = output(&[self.program(), "--version"])?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_string())
    }
}

/// The attached `<program> run` command that owns a box: stdin and stdout
/// piped, and `env` exported.
///
/// `extra` is what the runtime adds to the controls every box gets. `init`
/// is a host path; it and its directory are mounted read-only at the same
/// path, so it is also the path PID 1 runs. With `guest_socket`, init relays
/// to that guest path, which `extra` carries the proxy socket to.
///
/// Env values go through the child's environment, so argv holds only names
/// and `ps` cannot read a secret. A `from` entry names a host variable the
/// box sees under the entry's key; the two need not match.
fn up<'a>(
    program: &str,
    plan: &Plan,
    init: &Path,
    guest_socket: Option<&str>,
    extra: Vec<OsString>,
    env: impl Iterator<Item = (&'a String, &'a Env)>,
) -> Command {
    // PID 1 and all work run as the host user's uid:gid.
    let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
    let mut command = Command::new(program);
    command
        .args(["run", "-i", "--name", &plan.name])
        .args(["--network", "none", "--cap-drop", "ALL"])
        .args(["--read-only", "--tmpfs", "/tmp"])
        .arg("--user")
        .arg(format!("{uid}:{gid}"))
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(cpus) = plan.cpus {
        command.arg("--cpus").arg(cpus.to_string());
    }
    if let Some(memory) = &plan.memory {
        command.args(["--memory", memory]);
    }
    for (key, value) in &plan.labels {
        command.arg("--label").arg(format!("{key}={value}"));
    }
    for (name, value) in env {
        // Names only: the runtime reads the value from our environment.
        command.args(["--env", name]);
        match value {
            Env::Exact(value) => {
                command.env(name, value);
            }
            Env::From { from } => {
                if let Some(value) = std::env::var_os(from) {
                    command.env(name, value);
                }
            }
        }
    }
    // Always applied: Node's fetch reads the proxy variables only with this
    // set.
    command.args(["--env", "NODE_USE_ENV_PROXY"]);
    command.env("NODE_USE_ENV_PROXY", "1");
    if plan.egress.is_some() {
        command.args(["--env", "HTTPS_PROXY", "--env", "http_proxy"]);
        command.env("HTTPS_PROXY", PROXY_URL);
        command.env("http_proxy", PROXY_URL);
    }
    for mount in &plan.mounts {
        command
            .arg("--mount")
            .arg(bind(&mount.host, &mount.guest, mount.readonly));
    }
    // The init binary comes from the host and runs as PID 1.
    let init_dir = init
        .parent()
        .expect("box::up refuses an init without a directory");
    command
        .arg("--mount")
        .arg(bind(init_dir, init_dir, true))
        .arg("--entrypoint")
        .arg(init)
        .arg(
            plan.image
                .as_deref()
                .expect("box up resolves the profile's image before the runtime runs"),
        )
        .arg("init")
        .args(guest_socket);
    command
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
    command
        .arg("build")
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
    let mut output = Vec::new();
    reader.read_to_end(&mut output)?;
    if child.wait()?.success() {
        Ok(Ok(()))
    } else {
        Ok(Err(String::from_utf8_lossy(&output).into_owned()))
    }
}

/// The content digest of the image `reference` resolves to. `None` when the
/// runtime cannot resolve it. Never pulls: the reference is local.
pub fn local_image_id(runtime: &dyn Runtime, reference: &str) -> io::Result<Option<String>> {
    Ok(runtime.resolve_image(reference)?.ok().map(|image| image.id))
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
