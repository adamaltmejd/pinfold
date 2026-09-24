//! Container runtimes: one per OS, behind a trait.

pub mod apple;
pub mod podman;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use tokio::process::{Child, Command};

use crate::core::plan::{Env, Plan};
use crate::core::proxy::PROXY_URL;

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

/// Run `argv` with stdin closed and return its stdout. A failed run's error
/// names the command and carries its stderr; a failed spawn keeps its kind.
fn output(argv: &[&str]) -> io::Result<Vec<u8>> {
    let (program, arguments) = argv.split_first().expect("argv is never empty");
    let output = std::process::Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| spawn_error(program, error))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{}: {}",
            argv.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

/// Run `argv` for its status, with stdin and stdout closed and stderr passed
/// through. A failed run's error names the command and its status.
fn run(argv: &[&str]) -> io::Result<()> {
    let (program, arguments) = argv.split_first().expect("argv is never empty");
    let status = std::process::Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|error| spawn_error(program, error))?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{}: {status}", argv.join(" "))))
    }
}

/// A runtime CLI's `--version` output, trimmed. `doctor` uses it.
fn cli_version(program: &str) -> io::Result<String> {
    let stdout = output(&[program, "--version"])?;
    Ok(String::from_utf8_lossy(&stdout).trim().to_string())
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
    /// state, and return what `up` needs of the host.
    fn preflight(&self) -> io::Result<Preflight>;

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

/// The argv that runs `argv` in a box through its init: init raises its own
/// `oom_score_adj` to 1000 and then becomes `argv`, so the kernel's OOM
/// killer takes a box process before init. `init` is the host path
/// `up_argv` mounts at the same guest path and runs as `<init> init`, so the
/// composed command also carries the binary's `init` verb.
pub fn exec_through_init(init: &Path, argv: &[String]) -> Vec<String> {
    let mut wrapped = Vec::with_capacity(argv.len() + 4);
    wrapped.push(init.to_string_lossy().into_owned());
    wrapped.push("init".to_string());
    wrapped.push("exec".to_string());
    wrapped.push("--".to_string());
    wrapped.extend_from_slice(argv);
    wrapped
}

/// The `up` command for a runtime's run `argv`: stdin and stdout piped, and
/// `env` exported. Values go through the child's environment, so argv holds
/// only names and `ps` cannot read a secret. A `from` entry names a host
/// variable the box sees under the entry's key; the two need not match.
fn up_command(
    argv: &[OsString],
    env: &mut dyn Iterator<Item = (&String, &Env)>,
    egress: bool,
) -> Command {
    let (program, arguments) = argv.split_first().expect("argv is never empty");
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    for (name, value) in env {
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
    command.env("NODE_USE_ENV_PROXY", "1");
    if egress {
        command.env("HTTPS_PROXY", PROXY_URL);
        command.env("http_proxy", PROXY_URL);
    }
    command
}

/// Run a command in a running box through `program`'s exec, with inherited
/// stdio.
fn exec(
    program: &str,
    name: &str,
    tty: bool,
    workdir: Option<&Path>,
    argv: &[String],
) -> io::Result<ExitStatus> {
    let argv = exec_argv(program, name, tty, workdir, argv);
    let mut command = std::process::Command::new(program);
    command.args(&argv[1..]);
    if !tty {
        // Without a TTY the runtime's exec gets its own process group, so a
        // terminal signal reaches pinfold and not the exec.
        command.process_group(0);
    }
    command
        .status()
        .map_err(|error| spawn_error(program, error))
}

/// The `<program> exec` argv, as data.
///
/// The process inherits the run's user, so box-created files stay the host
/// user's. With a TTY, keep the host's terminal identity: the runtime
/// otherwise reports its own `TERM`.
fn exec_argv(
    program: &str,
    name: &str,
    tty: bool,
    workdir: Option<&Path>,
    argv: &[String],
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![program.into(), "exec".into(), "-i".into()];
    if tty {
        args.push("-t".into());
        for variable in ["TERM", "COLORTERM"] {
            if let Ok(value) = std::env::var(variable) {
                args.push("--env".into());
                args.push(format!("{variable}={value}").into());
            }
        }
    }
    if let Some(dir) = workdir {
        args.push("--workdir".into());
        args.push(dir.as_os_str().to_os_string());
    }
    args.push(name.into());
    for arg in argv {
        args.push(OsString::from(arg));
    }
    args
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
    let argv = build_argv(program, cache_flags, request);
    let (mut reader, writer) = io::pipe()?;
    let mut command = std::process::Command::new(program);
    command
        .args(&argv[1..])
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

/// The `<program> build` argv for one build, as data.
fn build_argv(program: &str, cache_flags: &[&str], request: &BuildRequest) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![program.into(), "build".into()];
    argv.extend(cache_flags.iter().map(OsString::from));
    argv.push("--file".into());
    argv.push(request.containerfile.into());
    for tag in request.tags {
        argv.push("--tag".into());
        argv.push(tag.into());
    }
    for (key, value) in request.labels {
        argv.push("--label".into());
        argv.push(format!("{key}={value}").into());
    }
    argv.push(request.context.into());
    argv
}

/// Pull `reference` with `program`, then return the digest `parse` reads
/// from its inspect. `None` when the reference does not resolve.
fn image_digest(
    program: &str,
    reference: &str,
    parse: fn(&[u8]) -> io::Result<Option<String>>,
) -> io::Result<Option<String>> {
    // A floating tag must be pulled for its digest to be current and present
    // to inspect. `scratch` and other non-registry references cannot be
    // pulled; they simply have no digest.
    let _ = run(&[program, "image", "pull", reference]);
    let output = std::process::Command::new(program)
        .args(["image", "inspect", reference])
        .output()
        .map_err(|error| spawn_error(program, error))?;
    if !output.status.success() {
        return Ok(None);
    }
    parse(&output.stdout)
}

/// The content digest of the image `reference` resolves to. `None` when the
/// runtime cannot resolve it. Unlike [`Runtime::image_digest`], this never
/// pulls: the reference is local.
pub fn local_image_id(runtime: &dyn Runtime, reference: &str) -> io::Result<Option<String>> {
    Ok(runtime.resolve_image(reference)?.ok().map(|image| image.id))
}

/// Whether a local image is built, and whether it was built on what `base`
/// resolves to now, as `doctor`, `config` and `pinfold pi` report it.
pub enum ImageStatus {
    Missing,
    /// Built, and on the current base when a base was asked about.
    Current,
    /// Built on base digest `recorded`; `base` now resolves to `current`.
    Stale {
        recorded: Option<String>,
        current: Option<String>,
    },
}

/// The status of image `reference`. With `base`, the digest the image
/// records in `dev.pinfold.base` is compared with the one `base` resolves
/// to now. Never pulls.
pub fn image_status(
    runtime: &dyn Runtime,
    reference: &str,
    base: Option<&str>,
) -> io::Result<ImageStatus> {
    let Ok(image) = runtime.resolve_image(reference)? else {
        return Ok(ImageStatus::Missing);
    };
    let Some(base) = base else {
        return Ok(ImageStatus::Current);
    };
    let recorded = image
        .labels
        .get(crate::core::clean::BASE_LABEL)
        .filter(|digest| !digest.is_empty())
        .cloned();
    let current = local_image_id(runtime, base)?;
    Ok(if recorded == current {
        ImageStatus::Current
    } else {
        ImageStatus::Stale { recorded, current }
    })
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
