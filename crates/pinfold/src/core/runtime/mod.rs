//! Container runtimes: one per OS, behind a trait.

pub mod apple;
pub mod podman;

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, poll};
use serde::de::DeserializeOwned;
use tokio::process::{Child, Command};

use crate::core::pipe::{nonblocking, remaining};
use crate::core::plan::{Env, Plan};
use crate::core::proxy::PROXY_URL;
use crate::init::BOX_ENV_PREFIX;

const BUILD_LOG_BYTES: usize = 64 * 1024;

/// The runtime's native Linux image architecture.
pub const TARGET_ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "amd64"
};

fn guest_env(command: &mut std::process::Command, name: &str, value: &OsStr) {
    let transport = format!("{BOX_ENV_PREFIX}{name}");
    let encoded: String = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    command.args(["--env", &transport]).env(transport, encoded);
}

/// The container runtime for this OS: Apple `container` on macOS, rootless
/// podman on Linux, the only targets pinfold builds for.
pub fn runtime() -> &'static dyn Runtime {
    if cfg!(target_os = "macos") {
        &apple::Apple
    } else {
        &podman::Podman
    }
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

/// Listing must fail without claiming an empty inventory when the runtime
/// waits on storage (GitHub #89). Never use this deadline for mutations.
pub(crate) fn list_output(argv: &[&str], cancel: Option<&AtomicBool>) -> io::Result<Vec<u8>> {
    const LIMIT: Duration = Duration::from_secs(20);
    let deadline = Instant::now() + LIMIT;
    let (program, arguments) = argv.split_first().expect("argv is never empty");
    let mut child = std::process::Command::new(program)
        .args(arguments)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| spawn_error(program, error))?;
    let mut reaped = false;
    let mut exit_observer = None;
    let result = (|| {
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        nonblocking(&stdout, "runtime listing")?;
        nonblocking(&stderr, "runtime listing")?;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (mut out_done, mut err_done) = (false, false);
        loop {
            remaining(deadline, cancel, "runtime listing")?;
            if !out_done {
                out_done = listing_bytes(&mut stdout, &mut out, deadline, cancel)?;
            }
            if !err_done {
                err_done = listing_bytes(&mut stderr, &mut err, deadline, cancel)?;
            }
            if out_done && err_done {
                // Keep the group leader unreaped until descendants close
                // both pipes, so failure cleanup cannot target a reused PID.
                let status = match child.try_wait()? {
                    Some(status) => status,
                    None => {
                        let pid = child.id();
                        let (send, receive) = mpsc::channel();
                        exit_observer = Some(std::thread::Builder::new().spawn(move || {
                            loop {
                                let mut info = std::mem::MaybeUninit::uninit();
                                // Observe without reaping, so timeout cleanup still owns the PID.
                                let waited = unsafe {
                                    nix::libc::waitid(
                                        nix::libc::P_PID,
                                        pid,
                                        info.as_mut_ptr(),
                                        nix::libc::WEXITED | nix::libc::WNOWAIT,
                                    )
                                };
                                match nix::errno::Errno::result(waited) {
                                    Err(nix::errno::Errno::EINTR) => continue,
                                    result => {
                                        let _ = send.send(result.map(|_| ()));
                                        break;
                                    }
                                }
                            }
                        })?);
                        loop {
                            let timeout = remaining(deadline, cancel, "runtime listing")?
                                .min(Duration::from_millis(100));
                            match receive.recv_timeout(timeout) {
                                Ok(result) => {
                                    result?;
                                    break;
                                }
                                Err(RecvTimeoutError::Timeout) => continue,
                                Err(error) => return Err(io::Error::other(error)),
                            }
                        }
                        child.wait()?
                    }
                };
                reaped = true;
                return if status.success() {
                    Ok(out)
                } else {
                    Err(io::Error::other(format!(
                        "{}: {}",
                        argv.join(" "),
                        String::from_utf8_lossy(&err).trim()
                    )))
                };
            }
            let mut fds = Vec::new();
            if !out_done {
                fds.push(PollFd::new(stdout.as_fd(), PollFlags::POLLIN));
            }
            if !err_done {
                fds.push(PollFd::new(stderr.as_fd(), PollFlags::POLLIN));
            }
            let timeout =
                remaining(deadline, cancel, "runtime listing")?.min(Duration::from_millis(100));
            let millis = timeout.as_millis().clamp(1, u16::MAX as u128) as u16;
            match poll(&mut fds, millis) {
                Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
    })();
    if !reaped {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(child.id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = child.kill();
    }
    let observer_result = exit_observer.map_or(Ok(()), |observer| {
        observer
            .join()
            .map_err(|_| io::Error::other("runtime listing exit observer panicked"))
    });
    if !reaped {
        child.wait()?;
    }
    observer_result?;
    result.map_err(|error| {
        if error.kind() == io::ErrorKind::TimedOut {
            io::Error::new(
                error.kind(),
                format!(
                    "runtime-timeout: {} exceeded {} seconds",
                    argv.join(" "),
                    LIMIT.as_secs()
                ),
            )
        } else {
            error
        }
    })
}

fn listing_bytes(
    reader: &mut impl Read,
    bytes: &mut Vec<u8>,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<bool> {
    let mut buffer = [0u8; 8192];
    loop {
        remaining(deadline, cancel, "runtime listing")?;
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(size) => bytes.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
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
                if let Some(value) = std::env::var_os(variable) {
                    guest_env(&mut command, variable, &value);
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
    fn list(&self, cancel: Option<&AtomicBool>) -> io::Result<Vec<BoxInfo>>;

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

/// The runtime CLI's absolute path: the first executable named `program`
/// on pinfold's own `PATH`. The spec's `env` reaches the child only after
/// this, so a spec `PATH` cannot choose the runtime (GitHub #70). A missing
/// runtime keeps the not-found error that names it.
fn runtime_path(program: &str) -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(program);
            if candidate.is_file()
                && nix::unistd::access(&candidate, nix::unistd::AccessFlags::X_OK).is_ok()
            {
                return Ok(candidate);
            }
        }
    }
    Err(spawn_error(program, io::ErrorKind::NotFound.into()))
}

/// The attached `<program> run` command that owns a box: stdin and stdout
/// piped. `program` is resolved on pinfold's own `PATH`. Guest values are
/// encoded under prefixed names, so they cannot configure the host client.
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
) -> io::Result<Command> {
    let program = runtime_path(program)?;
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
        let value = match value {
            Env::Exact(value) => Some(OsString::from(value)),
            Env::From { from } => std::env::var_os(from),
        };
        if let Some(value) = value {
            guest_env(command.as_std_mut(), name, &value);
        }
    }
    // Always applied: Node's fetch reads the proxy variables only with this
    // set. These override any spec entries with the same names.
    guest_env(command.as_std_mut(), "NODE_USE_ENV_PROXY", OsStr::new("1"));
    if plan.egress.is_some() {
        guest_env(command.as_std_mut(), "HTTPS_PROXY", OsStr::new(PROXY_URL));
        guest_env(command.as_std_mut(), "http_proxy", OsStr::new(PROXY_URL));
    }
    for mount in &plan.mounts {
        command
            .arg("--mount")
            .arg(bind(&mount.host, &mount.guest, mount.readonly));
    }
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
    Ok(command)
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
