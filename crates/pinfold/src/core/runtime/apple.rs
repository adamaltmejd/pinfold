//! The Apple `container` runtime adapter.

use crate::core::process::{AsyncCommandGuard, CommandGuard};

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use nix::libc;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};

use crate::core::disk::{self, Directory, Fill, Seeding};
use crate::core::image::{BuildRequest, ImageInfo, ImageService};
use crate::core::plan::{MountKind, Plan, Values};
use crate::core::runtime::{
    BoxInfo, BoxStat, DiskInfo, ExecContext, MemoryStat, PidsStat, Runtime, captured, output,
    parse_json, spawn_error,
};
use crate::core::{clean, ownership, process};
use crate::dirs;
use crate::formatting::terminal_text;

/// Where Apple `container` forwards `SSH_AUTH_SOCK` inside the box.
pub const GUEST_PROXY_SOCKET: &str = "/var/host-services/ssh-auth.sock";

/// Apple `container`.
pub struct Apple;

#[derive(Debug)]
struct PreparationUnremoved(io::Error);

impl std::fmt::Display for PreparationUnremoved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for PreparationUnremoved {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn preparation_unremoved(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|error| error.is::<PreparationUnremoved>())
}

/// The volume behind disk `name`. Apple's volumes are host-wide and a disk
/// belongs to one state root, so the root's key prefixes the name.
fn volume(name: &str) -> io::Result<String> {
    Ok(format!("{}-{name}", dirs::state_root_key()?))
}

/// Start the attached `container run` process that owns the box: stdin and
/// stdout piped. When `proxy_socket` is set, carry that host unix socket
/// into the box, where init relays to it.
///
/// The box's environment is not given to the runtime, which records what
/// it is given (#351): [`Client::ready`] sends it to init on stdin.
///
/// `init` is a host path; it and its directory are mounted read-only at the
/// same path, so it is also the path PID 1 runs.
#[cfg_attr(target_os = "linux", expect(dead_code))]
pub fn up(
    plan: &Plan,
    values: &Values,
    init: &Path,
    proxy_socket: Option<&Path>,
) -> io::Result<Client> {
    let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
    let mut command = Command::new("container");
    command
        .args(["run", "-i", "--name", &plan.name])
        .args(["--network", "none", "--cap-drop", "ALL"])
        .args(["--kernel-arg", "sysctl.user.max_user_namespaces=0"])
        .args(["--read-only", "--tmpfs", "/tmp"])
        // Apple creates the image's WORKDIR at creation, which could land
        // on a fresh disk; `/` always exists.
        .args(["--workdir", "/"])
        .arg("--user")
        .arg(format!("{uid}:{gid}"))
        .args(["--progress", "none"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(socket) = proxy_socket {
        // Off-label: the runtime treats the socket as an SSH agent and
        // forwards it to GUEST_PROXY_SOCKET.
        command.arg("--ssh").env("SSH_AUTH_SOCK", socket);
    }
    if let Some(cpus) = plan.cpus {
        command.arg("--cpus").arg(cpus.to_string());
    }
    if let Some(memory) = &plan.memory {
        command.args(["--memory", memory]);
    }
    for (key, value) in &plan.labels {
        command.arg("--label").arg(format!("{key}={value}"));
    }
    let mut vars: BTreeMap<_, _> = super::guest_vars(plan, values).into_iter().collect();
    let shares: Vec<_> = plan
        .mounts
        .iter()
        .filter(|mount| mount.kind == MountKind::Share)
        .collect();
    if !shares.is_empty() {
        let mut count = match vars.get("GIT_CONFIG_COUNT") {
            None => 0usize,
            Some(value) => value
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| {
                    io::Error::other("GIT_CONFIG_COUNT must be a nonnegative integer")
                })?,
        };
        for mount in shares {
            let next = count
                .checked_add(1)
                .ok_or_else(|| io::Error::other("GIT_CONFIG_COUNT is too large"))?;
            vars.insert(format!("GIT_CONFIG_KEY_{count}"), "safe.directory".into());
            vars.insert(
                format!("GIT_CONFIG_VALUE_{count}"),
                crate::core::plan::guest_path(&mount.guest).into_os_string(),
            );
            count = next;
        }
        // Apple mount tops are root-owned in the guest.
        vars.insert("GIT_CONFIG_COUNT".into(), count.to_string().into());
    }
    let vars: Vec<_> = vars
        .into_iter()
        .map(|(name, value)| (name.into(), value))
        .collect();
    let mut env = Vec::new();
    crate::init::exec_wire::write_frame(
        &mut env,
        crate::init::exec_wire::ENV,
        &super::exec_transport::env_payload(&vars),
    )?;
    for mount in &plan.mounts {
        command.arg("--mount").arg(bind(
            &mount.host,
            &mount.guest,
            mount.kind == MountKind::Readonly,
        ));
    }
    for disk in &plan.disks {
        let mut value = OsString::from(format!(
            "type=volume,source={},target=",
            volume(&disk.name)?
        ));
        value.push(&disk.guest);
        if disk.readonly {
            value.push(",readonly");
        }
        command.arg("--mount").arg(value);
    }
    command
        .arg("--mount")
        .arg(init_mount(init))
        .arg("--entrypoint")
        .arg(init)
        .arg(
            plan.image
                .as_deref()
                .expect("box up resolves the profile's image before the runtime runs"),
        )
        .arg("init")
        .args(proxy_socket.map(|_| GUEST_PROXY_SOCKET));
    command
        .spawn_guarded()
        .map(|mut child| {
            let stderr = child.stderr.take();
            Client(child, stderr, None, None, None, env)
        })
        .map_err(|error| spawn_error("container", error))
}

fn bind(host: &Path, guest: &Path, readonly: bool) -> OsString {
    let mut value = OsString::from("type=bind,source=");
    value.push(host);
    value.push(",target=");
    value.push(guest);
    if readonly {
        value.push(",readonly");
    }
    value
}

/// Init's directory, mounted read-only at the same path.
fn init_mount(init: &Path) -> OsString {
    let dir = init
        .parent()
        .expect("init lives in the artifacts directory");
    bind(dir, dir, true)
}

/// The attached `container run` process that owns a box.
#[cfg_attr(target_os = "linux", expect(dead_code))]
pub struct Client(
    Child,
    Option<ChildStderr>,
    Option<StderrDrain>,
    Option<tokio::task::JoinHandle<()>>,
    Option<Removal>,
    /// The ENV frame for init, until `ready` sends it.
    Vec<u8>,
);

const STARTUP_DIAGNOSTICS: usize = 64 * 1024;
const REMOVAL_TIMEOUT: Duration = Duration::from_secs(300);

struct Removal {
    worker: tokio::task::JoinHandle<io::Result<()>>,
    cancel: std::sync::Arc<AtomicBool>,
    deadline: tokio::time::Instant,
}

#[cfg_attr(target_os = "linux", expect(dead_code))]
impl Client {
    /// Send init the box's environment and wait until it reports ready,
    /// which it does only once it holds the environment. The error is why
    /// it did not.
    pub async fn ready(&mut self) -> Result<(), String> {
        let mut input = self
            .0
            .stdin
            .take()
            .ok_or("the runtime did not pipe the box's stdin")?;
        let env = std::mem::take(&mut self.5);
        // Its own task: init reads only once the box has booted, and the
        // frame can exceed the pipe's buffer.
        let writer = tokio::spawn(async move {
            input.write_all(&env).await?;
            input.flush().await?;
            Ok::<_, io::Error>(input)
        });
        if let Err(error) = self.wait_ready().await {
            writer.abort();
            return Err(error);
        }
        let input = writer
            .await
            .map_err(|error| format!("box environment: {error}"))?
            .map_err(|error| format!("box environment: {error}"))?;
        // Held open as before: the box's stdin stays attached.
        self.0.stdin = Some(input);
        Ok(())
    }

    async fn wait_ready(&mut self) -> Result<(), String> {
        let stdout = self
            .0
            .stdout
            .take()
            .ok_or("the runtime did not pipe the box's stdout")?;
        let mut lines = BufReader::new(stdout).lines();
        let mut diagnostics = Vec::with_capacity(STARTUP_DIAGNOSTICS);
        let mut bytes = [0; 8192];
        let mut ended = false;
        let mut child_ended = false;
        let mut deadline = None;
        loop {
            if !child_ended
                && self
                    .0
                    .try_wait()
                    .map_err(|error| format!("box wait failed: {error}"))?
                    .is_some()
            {
                child_ended = true;
                deadline.get_or_insert(tokio::time::Instant::now() + Duration::from_secs(1));
            }
            if (ended && self.1.is_none())
                || deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
            {
                self.1.take();
                return Err(startup_failure(&diagnostics));
            }
            tokio::select! {
                _ = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                    self.1.take();
                    return Err(startup_failure(&diagnostics));
                },
                line = lines.next_line(), if !ended => match line {
                    Ok(Some(line)) if line == "ready" => break,
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        ended = true;
                        deadline.get_or_insert(tokio::time::Instant::now() + Duration::from_secs(1));
                    }
                    Err(error) => return Err(format!("box output failed: {error}")),
                },
                read = async { self.1.as_mut().unwrap().read(&mut bytes).await }, if self.1.is_some() => {
                    match read {
                        Ok(0) | Err(_) => { self.1.take(); }
                        Ok(count) => {
                            let keep = count.min(STARTUP_DIAGNOSTICS - diagnostics.len());
                            diagnostics.extend_from_slice(&bytes[..keep]);
                        }
                    }
                },
                status = self.0.wait(), if !child_ended => {
                    status.map_err(|error| format!("box wait failed: {error}"))?;
                    child_ended = true;
                    deadline.get_or_insert(tokio::time::Instant::now() + Duration::from_secs(1));
                },
            }
        }
        self.2 = self
            .1
            .take()
            .map(|stderr| StderrDrain::start_fd(stderr.into_owned_fd()?))
            .transpose()
            .map_err(|error| format!("box diagnostics failed: {error}"))?;
        // Keep stdout drained; stderr has its own thread during blocking finish_start.
        self.3 = Some(tokio::spawn(async move {
            while let Ok(Some(_)) = lines.next_line().await {}
        }));
        Ok(())
    }

    pub fn exited(&mut self) -> io::Result<bool> {
        self.0.try_wait().map(|status| status.is_some())
    }

    /// Wait until the box ends on its own or is stopped.
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.0.wait().await?;
        self.close_stdout().await;
        let diagnostics = self.close_stderr();
        if !diagnostics.is_empty() {
            let bytes: Vec<_> = diagnostics.into_iter().collect();
            let diagnostics = terminal_text(&String::from_utf8_lossy(&bytes));
            // Exec may have made shared stderr nonblocking. A diagnostic must not panic.
            let _ = writeln!(
                io::stderr().lock(),
                "pinfold: native box ended: {diagnostics}"
            );
        }
        Ok(status)
    }

    fn close_stderr(&mut self) -> VecDeque<u8> {
        self.1.take();
        self.2.take().map_or_else(VecDeque::new, StderrDrain::close)
    }

    async fn close_stdout(&mut self) {
        if let Some(worker) = self.3.as_mut() {
            worker.abort();
            let _ = worker.await;
            self.3.take();
        }
    }

    async fn remove(&mut self, name: &str) -> io::Result<()> {
        if self.4.is_none() {
            let name = name.to_string();
            let cancel = std::sync::Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            self.4 = Some(Removal {
                worker: tokio::task::spawn_blocking(move || {
                    crate::core::process::with_cancellation(worker_cancel, || Apple.down(&name))
                }),
                cancel,
                deadline: tokio::time::Instant::now() + REMOVAL_TIMEOUT,
            });
        }
        let removal = self.4.as_mut().expect("retained removal worker");
        let result = match tokio::time::timeout_at(removal.deadline, &mut removal.worker).await {
            Ok(result) => result.map_err(io::Error::other).and_then(|result| result),
            Err(_) => {
                removal.cancel.store(true, Ordering::Release);
                let _ = (&mut removal.worker).await;
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "cleanup-timeout: native runtime removal",
                ))
            }
        };
        self.4.take();
        result
    }

    /// Stop and remove a ready box. True when the stop was one pinfold
    /// requested of a running box, which syncs its disks.
    pub async fn down(&mut self, name: &str) -> io::Result<bool> {
        self.close_stderr();
        let requested = matches!(self.0.try_wait(), Ok(None));
        // Runtime removal waits for attached output to close. Keep its
        // drain running on the owner while the runtime command blocks.
        let removed = self.remove(name).await;
        let _ = self.0.start_kill();
        let _ = self.0.wait().await;
        self.close_stdout().await;
        removed.map(|()| requested)
    }

    /// Stop a start that ended before ready and remove its box, returning
    /// whether the stop synced its disks and how the box ended.
    pub async fn abort(mut self, name: &str) -> (io::Result<bool>, io::Result<ExitStatus>) {
        self.close_stderr();
        let requested = matches!(self.0.try_wait(), Ok(None));
        // The client goes first, so one still creating the box cannot
        // finish after the removal. The box exists by name even when
        // readiness failed.
        let _ = self.0.start_kill();
        let status = self.0.wait().await;
        self.close_stdout().await;
        (self.remove(name).await.map(|()| requested), status)
    }
}

fn startup_failure(diagnostics: &[u8]) -> String {
    let diagnostic = terminal_text(&String::from_utf8_lossy(diagnostics));
    if diagnostic.is_empty() {
        "box exited before ready".to_string()
    } else {
        format!("box exited before ready: {diagnostic}")
    }
}

struct StderrDrain {
    stop: Option<io::PipeWriter>,
    worker: Option<std::thread::JoinHandle<VecDeque<u8>>>,
}

impl StderrDrain {
    fn start_fd(stderr: OwnedFd) -> io::Result<Self> {
        crate::core::pipe::nonblocking(&stderr, "box diagnostics")?;
        let (stop, stopping) = crate::core::process::pipe()?;
        let worker = std::thread::Builder::new()
            .name("box-diagnostics".into())
            .spawn(move || {
                let mut tail = VecDeque::with_capacity(STARTUP_DIAGNOSTICS);
                let mut bytes = [0; 8192];
                let mut closing = false;
                let mut final_bytes = STARTUP_DIAGNOSTICS;
                loop {
                    if !closing {
                        let mut fds = [
                            libc::pollfd {
                                fd: stop.as_raw_fd(),
                                events: libc::POLLIN,
                                revents: 0,
                            },
                            libc::pollfd {
                                fd: stderr.as_raw_fd(),
                                events: libc::POLLIN,
                                revents: 0,
                            },
                        ];
                        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) };
                        if ready < 0 {
                            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                                continue;
                            }
                            break;
                        }
                        closing = fds[0].revents != 0;
                    }
                    let count = unsafe {
                        libc::read(
                            stderr.as_raw_fd(),
                            bytes.as_mut_ptr().cast(),
                            bytes.len().min(final_bytes),
                        )
                    };
                    if count < 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() == io::ErrorKind::Interrupted
                            || (!closing && error.kind() == io::ErrorKind::WouldBlock)
                        {
                            continue;
                        }
                        break;
                    }
                    if count == 0 {
                        break;
                    }
                    let count = count as usize;
                    let remove = (tail.len() + count).saturating_sub(STARTUP_DIAGNOSTICS);
                    tail.drain(..remove);
                    tail.extend(&bytes[..count]);
                    if closing {
                        // Keep already-queued failure text without waiting on a descendant.
                        final_bytes -= count;
                        if final_bytes == 0 {
                            break;
                        }
                    }
                }
                tail
            })?;
        Ok(Self {
            stop: Some(stopping),
            worker: Some(worker),
        })
    }

    fn close(mut self) -> VecDeque<u8> {
        self.stop.take();
        self.worker
            .take()
            .and_then(|worker| worker.join().ok())
            .unwrap_or_default()
    }
}

impl Drop for StderrDrain {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct PreparationInput<'a> {
    input: std::process::ChildStdin,
    cancel: &'a AtomicBool,
}

impl Write for PreparationInput<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            if self.cancel.load(Ordering::Acquire) {
                // Buffered writers retry Interrupted rather than ending the fill.
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "disk preparation cancelled",
                ));
            }
            match self.input.write(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let mut ready = [nix::poll::PollFd::new(
                        self.input.as_fd(),
                        nix::poll::PollFlags::POLLOUT,
                    )];
                    match nix::poll::poll(&mut ready, 20u16) {
                        Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.input.flush()
    }
}

fn wait_preparation(
    child: &mut std::process::Child,
    cancel: &AtomicBool,
) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            child.wait()?;
            return Err(io::ErrorKind::Interrupted.into());
        }
        match nix::poll::poll(&mut [], 20u16) {
            Ok(_) | Err(nix::errno::Errno::EINTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

impl Runtime for Apple {
    fn preflight(&self) -> io::Result<()> {
        let reported = self.version()?;
        let version = reported
            .strip_prefix("container CLI version ")
            .and_then(|value| value.split_whitespace().next())
            .unwrap_or(&reported);
        let release: Option<[u64; 3]> = version
            .split('+')
            .next()
            .unwrap_or_default()
            .split('.')
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .and_then(|parts| parts.try_into().ok());
        if !release.is_some_and(|release| release >= [1, 6, 0]) {
            return Err(io::Error::other(format!(
                "container reports version {version:?}; pinfold requires stable container 1.6.0 or newer"
            )));
        }
        Ok(())
    }

    fn finish_start(
        &self,
        name: &str,
        init: &Path,
        proxy_socket: Option<&Path>,
        seeding: Option<&Seeding>,
        cancel: &AtomicBool,
    ) -> io::Result<BoxInfo> {
        // Init can report ready before Apple records the box running.
        // Wait even without egress, before the caller's first exec.
        let actual = wait_until_running(name, cancel)?;
        if proxy_socket.is_some() {
            make_proxy_connectable(name, init)?;
        }
        if let Some(seeding) = seeding {
            copy_in(name, init, seeding, cancel)?;
        }
        Ok(actual)
    }

    fn down(&self, name: &str) -> io::Result<()> {
        // Apple's forced removal races two process-output waiters (#83).
        // Kill through the existing monitor, then remove without forcing.
        // A stopped or absent box can refuse kill; removal still decides
        // success.
        let _ = output(&["container", "kill", "--signal", "KILL", name]);
        // Apple can report a killed box `running` for a moment, and `rm`
        // refuses it then; `stop` returns once it has stopped. Only a box
        // that refused kill while running gets stop's SIGTERM, 5 s grace and
        // kill, which is #83's second waiter.
        let _ = output(&["container", "stop", name]);
        // stderr is captured: an absent box fails `rm` with the runtime's
        // not-found text, which is noise once the box is gone.
        let Err(error) = output(&["container", "rm", name]) else {
            return Ok(());
        };
        // A targeted native absence is removal evidence; display omission is not.
        if container(name)?.is_none() {
            return Ok(());
        }
        Err(error)
    }

    /// The limits Apple reports, and null for what the VM cannot answer
    /// (kills, memory use, pids use).
    fn stat(&self, name: &str) -> io::Result<BoxStat> {
        let resources = container(name)?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no box named {name:?}"))
            })?
            .configuration
            .resources;
        Ok(BoxStat {
            name: name.to_string(),
            oom_kills: None,
            memory: MemoryStat {
                limit: resources.memory,
                ..MemoryStat::default()
            },
            pids: PidsStat::default(),
        })
    }

    fn list(&self) -> io::Result<Vec<BoxInfo>> {
        Ok(containers()?.into_iter().map(box_info).collect())
    }

    fn inspect(&self, name: &str) -> io::Result<Option<BoxInfo>> {
        Ok(container(name)?.map(box_info))
    }

    fn list_disks(&self) -> io::Result<Vec<DiskInfo>> {
        let json = output(&["container", "volume", "list", "--format", "json"])?;
        let volumes: Vec<ListedVolume> = parse_json("container volume list", &json)?;
        // Another root's volumes, and any pinfold did not make, are no disks.
        let prefix = volume("")?;
        let mut disks = Vec::new();
        for listed in volumes {
            let configuration = listed.configuration;
            let Some(name) = configuration.name.strip_prefix(&prefix) else {
                continue;
            };
            disks.push(DiskInfo {
                allocated: allocated_bytes(Path::new(&configuration.source))?,
                name: name.to_string(),
                size: configuration.size,
            });
        }
        Ok(disks)
    }

    fn create_disk(&self, name: &str, bytes: u64) -> io::Result<()> {
        let size = format!("{}G", bytes >> 30);
        output(&["container", "volume", "create", "-s", &size, &volume(name)?]).map(drop)
    }

    fn remove_disk(&self, name: &str) -> io::Result<()> {
        output(&["container", "volume", "rm", &volume(name)?]).map(drop)
    }

    /// The guest synced; what the volume keeps through a host crash is
    /// the runtime's.
    fn sync_disk(&self, _name: &str) -> io::Result<()> {
        Ok(())
    }

    fn exec(
        &self,
        name: &str,
        init: &Path,
        context: &ExecContext<'_>,
        argv: &[String],
    ) -> io::Result<ExitStatus> {
        native_exec(name, init, context, argv)
    }

    fn exec_command(
        &self,
        name: &str,
        init: &Path,
        context: &ExecContext<'_>,
        argv: &[String],
    ) -> io::Result<std::process::Command> {
        let size = if context.tty {
            super::terminal::window()?
        } else {
            None
        };
        exec_command(name, init, context, argv, size)
    }

    /// A networkless helper owns fresh roots, then drops privileges to
    /// prepare generated-mount parents on fresh and reused disks.
    fn own_disks(
        &self,
        names: &[&str],
        plan: &Plan,
        init: &Path,
        generation: &str,
        directories: &[Directory],
        fills: &[Fill],
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        if cancel.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if names.is_empty() && directories.is_empty() {
            return Ok(());
        }
        let name = &plan.name;
        let image = plan.image.as_deref().expect("up resolved the image");
        let labels = BTreeMap::from([
            (
                clean::OWNER_LABEL.to_string(),
                plan.labels[clean::OWNER_LABEL].clone(),
            ),
            (
                ownership::GENERATION_LABEL.to_string(),
                generation.to_string(),
            ),
        ]);
        let owner = format!("{}:{}", nix::unistd::getuid(), nix::unistd::getgid());
        let mut command = std::process::Command::new("container");
        command
            .args(["run", "-i", "--progress", "none", "--network", "none"])
            .args(["--read-only", "--cap-drop", "ALL", "--cap-add", "CAP_CHOWN"])
            .args(["--workdir", "/"])
            .args(["--name", name])
            .args(["--user", "0:0", "--mount"])
            .arg(init_mount(init));
        if !directories.is_empty() || !fills.is_empty() {
            command.args(["--cap-add", "CAP_SETUID", "--cap-add", "CAP_SETGID"]);
        }
        for (key, value) in &labels {
            command.arg("--label").arg(format!("{key}={value}"));
        }
        let mut setup: Vec<OsString> = vec!["init".into(), "setup".into()];
        let disks: Vec<_> = plan
            .disks
            .iter()
            .filter(|disk| {
                names.contains(&disk.name.as_str())
                    || directories.iter().any(|directory| {
                        directory.disk == crate::core::plan::guest_path(&disk.guest)
                    })
            })
            .collect();
        for (index, disk) in disks.iter().enumerate() {
            let guest = format!("/pinfold-disk/{index}");
            command.arg("--mount").arg(format!(
                "type=volume,source={},target={guest}",
                volume(&disk.name)?
            ));
            if names.contains(&disk.name.as_str()) {
                setup.extend(["--own".into(), owner.clone().into(), guest.into()]);
            }
        }
        if !directories.is_empty() || !fills.is_empty() {
            setup.extend(["--user".into(), owner.into()]);
        }
        for fill in fills {
            let index = disks
                .iter()
                .position(|disk| {
                    crate::core::plan::guest_path(&disk.guest)
                        == crate::core::plan::guest_path(&fill.guest)
                })
                .ok_or_else(|| io::Error::other("initial population has no disk"))?;
            setup.extend(["--fill".into(), format!("/pinfold-disk/{index}").into()]);
        }
        for directory in directories {
            let index = disks
                .iter()
                .position(|disk| directory.disk == crate::core::plan::guest_path(&disk.guest))
                .ok_or_else(|| io::Error::other("directory preparation has no disk"))?;
            setup.extend([
                "--mkdir".into(),
                format!("/pinfold-disk/{index}").into(),
                directory.relative.clone().into_os_string(),
            ]);
        }
        let mut spawned = false;
        let result = (|| -> io::Result<std::process::Output> {
            let mut child = command
                .arg("--entrypoint")
                .arg(init)
                .arg(image)
                .args(setup)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn_guarded()
                .map_err(|error| spawn_error("container", error))?;
            spawned = true;
            let result = (|| {
                let drain = StderrDrain::start_fd(
                    child.stderr.take().expect("setup stderr is piped").into(),
                )?;
                let sent = (|| {
                    let input = child.stdin.take().expect("setup stdin is piped");
                    crate::core::pipe::nonblocking(&input, "disk preparation")?;
                    let mut input = PreparationInput { input, cancel };
                    for fill in fills {
                        disk::send(fill, &mut input)?;
                    }
                    Ok::<_, io::Error>(())
                })();
                let status = if sent.is_err() {
                    let _ = child.kill();
                    child.wait()
                } else {
                    wait_preparation(&mut child, cancel)
                };
                let stderr = drain.close().into_iter().collect();
                let status = status?;
                if status.success() {
                    sent?;
                }
                Ok(std::process::Output {
                    status,
                    stdout: Vec::new(),
                    stderr,
                })
            })();
            if result.is_err() {
                let _ = child.kill();
                let _ = child.wait();
            }
            result
        })();
        if !spawned {
            return result.map(drop);
        }
        // The box reuses the helper's name, so the helper must be gone first.
        // Apple reports a container `running` for a moment after `run` returns,
        // and `rm` refuses it then; `run --rm` removes it just as late. `stop`
        // returns once it has stopped, after which `rm` succeeds.
        let (stopped, removed, unremoved) =
            crate::core::process::with_deadline(REMOVAL_TIMEOUT, |_| {
                let stopped = output(&["container", "stop", name]);
                let removed = output(&["container", "rm", name]);
                let unremoved = removed.is_err() && !matches!(container(name), Ok(None));
                (stopped, removed, unremoved)
            });
        let leftover = match (&stopped, &removed) {
            (Ok(_), Ok(_)) => String::new(),
            _ => format!("; removing helper {name}: {stopped:?}, {removed:?}"),
        };
        let result = match result {
            Err(error) if leftover.is_empty() => Err(error),
            Err(error) => Err(io::Error::new(error.kind(), format!("{error}{leftover}"))),
            Ok(result) if !result.status.success() => Err(io::Error::other(format!(
                "prepare writable disks: {}{leftover}",
                terminal_text(String::from_utf8_lossy(&result.stderr).trim())
            ))),
            Ok(_) => stopped.and(removed).map(drop),
        };
        result.map_err(|error| {
            if unremoved {
                io::Error::other(PreparationUnremoved(error))
            } else {
                error
            }
        })
    }

    fn describe(&self) -> &'static str {
        "Apple container (one VM per box)"
    }

    /// The CLI's version, as the CLI reports it.
    fn version(&self) -> io::Result<String> {
        let stdout = output(&["container", "--version"])?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_string())
    }
}

/// The native client owns only private pipes, never caller terminal fds.
fn native_exec(
    name: &str,
    init: &Path,
    context: &ExecContext<'_>,
    argv: &[String],
) -> io::Result<ExitStatus> {
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::ExitStatusExt;
    let (connection, bridge) = UnixStream::pair()?;
    let mut command = std::process::Command::new("container");
    command
        .args(["exec", "-i", name])
        .arg(init)
        .args(["init", "session-exec"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut client = crate::core::process::GroupChild::spawn(&mut command)?;
    drop(command);
    let diagnostics = StderrDrain::start_fd(client.take_stderr().expect("piped stderr").into())?;
    let mut input = client.take_stdin();
    let mut output = client.take_stdout();
    crate::core::pipe::nonblocking(input.as_ref().expect("piped stdin"), "native exec")?;
    crate::core::pipe::nonblocking(output.as_ref().expect("piped stdout"), "native exec")?;
    bridge.set_nonblocking(true)?;
    let stopped = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let worker_context = ExecContext {
            cancel: Some(&stopped),
            ..*context
        };
        let worker =
            scope.spawn(move || super::exec_transport::exec(connection, &worker_context, argv));
        let mut pending_input = VecDeque::new();
        let mut pending_output = VecDeque::new();
        let mut exited = None;
        let relayed = (|| -> io::Result<()> {
            loop {
                if worker.is_finished()
                    || context
                        .cancel
                        .is_some_and(|cancel| cancel.load(Ordering::Acquire))
                {
                    return Ok(());
                }
                if exited.is_none() && client.exited()? {
                    exited = Some(Instant::now() + Duration::from_secs(1));
                    input.take();
                    pending_input.clear();
                }
                if exited.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Ok(());
                }
                if output.is_none() && pending_output.is_empty() {
                    let _ = bridge.shutdown(std::net::Shutdown::Write);
                }
                let mut fds = [
                    libc::pollfd {
                        fd: if input.is_some() && pending_input.is_empty() {
                            bridge.as_raw_fd()
                        } else {
                            -1
                        },
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: if pending_input.is_empty() {
                            -1
                        } else {
                            input.as_ref().map_or(-1, AsRawFd::as_raw_fd)
                        },
                        events: libc::POLLOUT,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: if pending_output.is_empty() {
                            output.as_ref().map_or(-1, AsRawFd::as_raw_fd)
                        } else {
                            -1
                        },
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: if pending_output.is_empty() {
                            -1
                        } else {
                            bridge.as_raw_fd()
                        },
                        events: libc::POLLOUT,
                        revents: 0,
                    },
                ];
                // The finite poll also observes native exit without trusting its pipes' EOF.
                if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 20) } < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if fds[0].revents != 0 && !bridge_read(&mut &bridge, &mut pending_input)? {
                    input.take();
                }
                if fds[1].revents != 0
                    && let Some(writer) = input.as_mut()
                    && !bridge_write(writer, &mut pending_input)?
                {
                    input.take();
                    pending_input.clear();
                }
                if fds[2].revents != 0
                    && let Some(reader) = output.as_mut()
                    && !bridge_read(reader, &mut pending_output)?
                {
                    output.take();
                }
                if fds[3].revents != 0 && !bridge_write(&mut &bridge, &mut pending_output)? {
                    return Ok(());
                }
            }
        })();
        stopped.store(true, Ordering::Release);
        let _ = bridge.shutdown(std::net::Shutdown::Both);
        drop((input, output));
        let _ = client.finish();
        let result = worker
            .join()
            .map_err(|_| io::Error::other("native exec worker failed"))?;
        match result {
            Ok(code) => Ok(code),
            Err(error) => relayed.and(Err(error)),
        }
    });
    let diagnostics: Vec<_> = diagnostics.close().into_iter().collect();
    if result.is_err() && !diagnostics.is_empty() {
        crate::core::process::diagnostic(String::from_utf8_lossy(&diagnostics));
    }
    result.map(|code| ExitStatus::from_raw(code << 8))
}

fn bridge_read(input: &mut impl io::Read, pending: &mut VecDeque<u8>) -> io::Result<bool> {
    let mut bytes = [0; 32 * 1024];
    match input.read(&mut bytes) {
        Ok(0) => Ok(false),
        Ok(read) => {
            pending.extend(&bytes[..read]);
            Ok(true)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

fn bridge_write(output: &mut impl Write, pending: &mut VecDeque<u8>) -> io::Result<bool> {
    match output.write(pending.as_slices().0) {
        Ok(0) => Ok(false),
        Ok(written) => {
            pending.drain(..written);
            Ok(true)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error),
    }
}

/// `container exec` of `argv` through init, preserving terminal identity on a TTY.
fn exec_command(
    name: &str,
    init: &Path,
    context: &ExecContext<'_>,
    argv: &[String],
    size: Option<(u16, u16)>,
) -> io::Result<std::process::Command> {
    let mut command = std::process::Command::new("container");
    command.args(["exec", "-i"]);
    let mut metadata = crate::init::ExecMetadata {
        agent: context.agent.map(str::to_string),
        ..Default::default()
    };
    if context.tty {
        command.arg("-t");
        // Init applies a known initial size before a fast command starts.
        super::exec_transport::tty_metadata(&mut metadata, size);
    } else {
        // Without a TTY the runtime's exec gets its own process group, so
        // a terminal signal reaches pinfold and not the exec.
        command.process_group(0);
    }
    command.args(["--env", crate::init::EXEC_METADATA]);
    command.env(crate::init::EXEC_METADATA, metadata.encode());
    command
        .arg(name)
        .arg(init)
        .args(["init", "exec", "--box-env", "--"])
        .args(argv);
    Ok(command)
}

/// Wait, bounded, until inspection reports box `name` running. Apple records
/// `.running` only after the box's first process starts, so `ready` can
/// arrive first; an exec is refused in that gap.
fn wait_until_running(name: &str, cancel: &AtomicBool) -> io::Result<BoxInfo> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let state = Apple.inspect(name)?;
        match state {
            Some(box_) if box_.running => return Ok(box_),
            _ if Instant::now() >= deadline => {
                return Err(io::Error::other(match state {
                    Some(_) => format!("box {name:?} is not running after ready"),
                    _ => format!("the runtime does not report box {name:?} after ready"),
                }));
            }
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Make the forwarded proxy socket in box `name` connectable by the box
/// user: it arrives root-owned. The one transient root exec.
fn make_proxy_connectable(name: &str, init: &Path) -> io::Result<()> {
    let result = std::process::Command::new("container")
        .args(["exec", "--user", "0:0", "--workdir", "/", name])
        .arg(init)
        .args(["init", "proxy-socket"])
        .stdin(Stdio::null())
        .output_guarded()
        .map_err(|error| spawn_error("container", error))?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "proxy socket permissions: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    Ok(())
}

/// Copy missing profile seeds without buffering another copy of their bytes.
fn copy_in(name: &str, init: &Path, seeding: &Seeding, cancel: &AtomicBool) -> io::Result<()> {
    let mut child = crate::core::process::GroupChild::spawn(
        std::process::Command::new("container")
            .args(["exec", "-i", name])
            .arg(init)
            .args(["init", "receive", "--missing"])
            .arg(&seeding.guest)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )
    .map_err(|error| spawn_error("container", error))?;
    let mut input = child.take_stdin().expect("piped seed input");
    let mut stderr = child.take_stderr();
    crate::core::pipe::nonblocking(&input, "seed copy")?;
    if let Some(stderr) = stderr.as_ref() {
        crate::core::pipe::nonblocking(stderr, "seed diagnostics")?;
    }
    let mut diagnostics = Vec::new();
    let sent = disk::send_seeds(
        &seeding.relative,
        &seeding.seeds,
        &mut SeedInput {
            input: &mut input,
            stderr: &mut stderr,
            diagnostics: &mut diagnostics,
            cancel,
        },
    );
    drop(input);
    if let Err(error) = sent {
        return Err(io::Error::new(error.kind(), format!("seed copy: {error}")));
    }
    let mut exited = None;
    let mut wait = crate::core::process::ExitWait::new();
    let status = loop {
        if cancel.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if exited.is_none() && child.exited()? {
            exited = Some(Instant::now());
        }
        crate::core::process::drain(&mut stderr, &mut diagnostics, Some(16 * 1024))?;
        if let Some(began) = exited
            && (stderr.is_none() || began.elapsed() >= Duration::from_secs(1))
        {
            break child.finish()?;
        }
        let mut fds: Vec<_> = stderr
            .iter()
            .map(|pipe| nix::poll::PollFd::new(pipe.as_fd(), nix::poll::PollFlags::POLLIN))
            .collect();
        wait.poll(&mut fds)?;
    };
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "copy into {}: {}",
            seeding.guest.display(),
            String::from_utf8_lossy(&diagnostics).trim()
        )))
    }
}

struct SeedInput<'a> {
    input: &'a mut std::process::ChildStdin,
    stderr: &'a mut Option<std::process::ChildStderr>,
    diagnostics: &'a mut Vec<u8>,
    cancel: &'a AtomicBool,
}

impl Write for SeedInput<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            if self.cancel.load(Ordering::Acquire) {
                // Buffered writers retry Interrupted rather than ending the copy.
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "startup cancelled",
                ));
            }
            crate::core::process::drain(self.stderr, self.diagnostics, Some(16 * 1024))?;
            match self.input.write(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    seed_poll(self.input, self.stderr.as_ref())?
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn seed_poll(
    input: &std::process::ChildStdin,
    stderr: Option<&std::process::ChildStderr>,
) -> io::Result<()> {
    let mut fds = [
        libc::pollfd {
            fd: input.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        },
        libc::pollfd {
            fd: stderr.map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 100) } < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

/// One `container list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedContainer {
    id: String,
    #[serde(default)]
    configuration: ListedConfiguration,
    #[serde(default)]
    status: ListedStatus,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ListedConfiguration {
    labels: BTreeMap<String, String>,
    image: ListedBoxImage,
    /// ISO 8601, which is RFC 3339.
    #[serde(rename = "creationDate")]
    created: String,
    resources: ListedResources,
    mounts: Vec<ListedMount>,
}

#[derive(Deserialize)]
struct ListedMount {
    source: String,
    destination: String,
}

fn allocated_bytes(path: &Path) -> io::Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    let mut bytes = metadata.blocks() * 512;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            bytes += allocated_bytes(&entry?.path())?;
        }
    }
    Ok(bytes)
}

/// The image a box runs, as the runtime records it.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ListedBoxImage {
    reference: String,
    descriptor: ListedBoxImageDescriptor,
}

#[derive(Default, Deserialize)]
struct ListedBoxImageDescriptor {
    /// `sha256:<hex>`; the hex is the image list's `id`.
    #[serde(default)]
    digest: String,
}

/// The limits the runtime reports for a box.
#[derive(Default, Deserialize)]
struct ListedResources {
    #[serde(rename = "memoryInBytes")]
    memory: Option<u64>,
}

#[derive(Default, Deserialize)]
struct ListedStatus {
    #[serde(default)]
    state: String,
    /// Set once the box has run. A box still booting lists `stopped`
    /// without it.
    #[serde(default, rename = "startedDate")]
    started_date: Option<String>,
}

/// Every box `container list` reports, running or not.
fn containers() -> io::Result<Vec<ListedContainer>> {
    let json = output(&["container", "list", "--all", "--format", "json"])?;
    parse_json("container list", &json)
}

fn box_info(container: ListedContainer) -> BoxInfo {
    let configuration = container.configuration;
    let digest = configuration.image.descriptor.digest;
    BoxInfo {
        id: container.id,
        labels: configuration.labels,
        image_id: digest
            .strip_prefix("sha256:")
            .unwrap_or(&digest)
            .to_string(),
        image_ref: configuration.image.reference,
        created: configuration.created,
        running: container.status.state == "running",
        stopped: container.status.state == "stopped" && container.status.started_date.is_some(),
    }
}

/// Inspect one native target; only its explicit not-found response proves absence.
fn container(name: &str) -> io::Result<Option<ListedContainer>> {
    let output = std::process::Command::new("container")
        .args(["inspect", "--", name])
        .stdin(Stdio::null())
        .output_guarded()
        .map_err(|error| spawn_error("container", error))?;
    if !output.status.success() {
        let absent = format!("Error: container not found: {name}");
        if output.status.code() == Some(1)
            && output.stdout.is_empty()
            && output.stderr.trim_ascii() == absent.as_bytes()
        {
            return Ok(None);
        }
        return Err(io::Error::other(format!(
            "container inspect: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut containers: Vec<ListedContainer> = parse_json("container inspect", &output.stdout)?;
    if containers.len() != 1 || containers[0].id != name {
        return Err(io::Error::other(
            "container inspect returned an unmatched target",
        ));
    }
    Ok(containers.pop())
}

/// One `container volume list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedVolume {
    configuration: ListedVolumeConfiguration,
}

#[derive(Deserialize)]
struct ListedVolumeConfiguration {
    name: String,
    #[serde(rename = "sizeInBytes")]
    size: u64,
    /// The volume's image on the host.
    source: String,
}

/// One `container image list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedImage {
    id: String,
    configuration: ListedImageConfiguration,
    /// Each variant's OCI image config sits at `/config/config`.
    #[serde(default)]
    variants: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct ListedImageConfiguration {
    name: String,
    descriptor: Option<ListedImageDescriptor>,
}

#[derive(Deserialize)]
struct ListedImageDescriptor {
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

/// One image entry and its recorded labels.
fn image_info(image: ListedImage) -> io::Result<ImageInfo> {
    // Build labels are OCI image config labels; a locally built image also
    // carries name annotations on its index descriptor.
    let mut labels = image
        .configuration
        .descriptor
        .map(|descriptor| descriptor.annotations)
        .unwrap_or_default();
    for variant in &image.variants {
        if let Some(found) = variant.pointer("/config/config/Labels") {
            labels.extend(
                BTreeMap::<String, String>::deserialize(found).map_err(|error| {
                    io::Error::other(format!("image {} has invalid labels: {error}", image.id))
                })?,
            );
        }
    }
    Ok(ImageInfo {
        id: image.id,
        reference: image.configuration.name,
        labels,
    })
}

/// Prune target for the builder's unused cache after each build, 16 GB
/// in buildctl's decimal MB. One four-image chain measured 2.5 GB (#392)
/// and its backing storage 3.3 GiB; the target leaves room for several
/// chains. Apple's own GC keeps 32 GiB.
const BUILDER_KEEP_MB: &str = "16000";
const BUILD_LOG_BYTES: usize = 64 * 1024;

/// Prune unused BuildKit records in the shared builder. BuildKit keeps
/// records in active use; deleting the shared builder instead interrupts
/// builds in other processes (#75).
fn prune_builder(flags: &[&str]) -> io::Result<()> {
    let mut command = vec![
        "container",
        "exec",
        "buildkit",
        "buildctl",
        "prune",
        "--all",
    ];
    command.extend_from_slice(flags);
    output(&command).map(drop)
}

/// Return freed guest blocks to the host by online trim, without replacing
/// the builder or stopping active builds (#86).
fn trim_builder() -> io::Result<()> {
    // The guest's ext4 trims freed blocks only once the journal has committed
    // their release; without the sync a trim just after a prune returns
    // little or nothing (#417). Sync only the root filesystem: a plain sync
    // also waits on the builder's VirtioFS mounts, without bound. The sync is
    // best-effort: BusyBox does not report syncfs errors, and the trim's
    // result is what counts.
    let _ = output(&["container", "exec", "buildkit", "sync", "-f", "/"]);
    output(&["container", "clean", "buildkit"]).map(drop)
}

/// Every pinfold build holds this host lock shared; `clean` holds it
/// exclusive while it stops the builder.
const BUILDER_LOCK: &str = "builder:apple";
/// The build-records query took 0.3 s and the stop 0.4 s when measured.
const BUILDER_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const BUILDER_STOP_TIMEOUT: Duration = Duration::from_secs(60);

/// Stop the builder when nothing builds on it. Its host process keeps each
/// build's unlinked export open until it stops (#440); the next build
/// restarts it with its cache.
fn stop_idle_builder() -> io::Result<()> {
    // Held through the stop so no pinfold build starts in between.
    let idle = ownership::try_flock(
        ownership::open_lock(BUILDER_LOCK)?,
        FlockArg::LockExclusiveNonblock,
    )?;
    // Every Apple build waits on the lock, so nothing under it may hang,
    // a write to a stderr nobody drains included: decide, release, print.
    let busy = match idle {
        None => Ok(Some("a build is running")),
        Some(_) => match process::with_deadline(BUILDER_QUERY_TIMEOUT, |_| builder_busy()) {
            Ok(true) => Ok(Some("a build is running")),
            Ok(false) => process::with_deadline(BUILDER_STOP_TIMEOUT, |_| {
                output(&["container", "builder", "stop"]).map(drop)
            })
            .map(|()| None)
            .map_err(|error| match error.kind() {
                io::ErrorKind::Interrupted => io::Error::other(format!(
                    "container builder stop did not finish within {}s",
                    BUILDER_STOP_TIMEOUT.as_secs()
                )),
                _ => error,
            }),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                Ok(Some("the builder did not answer"))
            }
            Err(error) => Err(error),
        },
    };
    drop(idle);
    if let Some(reason) = busy? {
        let _ = writeln!(
            io::stderr(),
            "pinfold clean: build cache: {reason}, so cleanup could not complete; run `pinfold clean` again after it"
        );
    }
    Ok(())
}

/// Whether BuildKit has a build in progress, pinfold's or another caller's:
/// a build record without a completion time. A killed client's build
/// completes its record at once.
fn builder_busy() -> io::Result<bool> {
    #[derive(Deserialize)]
    struct Event {
        record: Record,
    }
    #[derive(Deserialize)]
    struct Record {
        #[serde(rename = "CompletedAt")]
        completed_at: Option<serde::de::IgnoredAny>,
    }
    let json = output(&[
        "container",
        "exec",
        "buildkit",
        "buildctl",
        "debug",
        "histories",
        "--format",
        "{{json .}}",
    ])?;
    for event in serde_json::Deserializer::from_slice(&json).into_iter::<Event>() {
        let event = event.map_err(|error| {
            io::Error::other(format!(
                "buildctl debug histories returned invalid JSON: {error}"
            ))
        })?;
        if event.record.completed_at.is_none() {
            return Ok(true);
        }
    }
    Ok(false)
}

impl ImageService for Apple {
    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        let json = output(&["container", "image", "list", "--format", "json"])?;
        let images: Vec<ListedImage> = parse_json("container image list", &json)?;
        images.into_iter().map(image_info).collect()
    }

    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>> {
        // `image inspect` prints the same entries as `image list`.
        match inspect_image(reference)? {
            Ok(image) => image_info(image).map(Ok),
            Err(stderr) => Ok(Err(stderr)),
        }
    }

    fn remove_image(&self, reference: &str) -> io::Result<()> {
        // `image delete` also collects the layers no image references.
        output(&["container", "image", "delete", reference]).map(|_| ())
    }

    fn purge_build_cache(&self) -> io::Result<()> {
        let Some(builder) = container("buildkit")? else {
            return Ok(());
        };
        if builder.status.state != "running" {
            // `builder start` may replace an existing instance when its
            // defaults changed; start this instance without replacing it.
            output(&["container", "start", "buildkit"])?;
        }
        prune_builder(&[])?;
        trim_builder()?;
        stop_idle_builder()
    }

    fn build_cache_bytes(&self) -> io::Result<u64> {
        let Some(builder) = container("buildkit")? else {
            return Ok(0);
        };
        // The exports mount records the runtime's actual app root, including
        // a custom root. Count allocated blocks: rootfs.ext4 is sparse and
        // BuildKit's logical cache size misses retained filesystem space (#74).
        let exports = builder
            .configuration
            .mounts
            .iter()
            .find(|mount| mount.destination == "/var/lib/container-builder-shim/exports")
            .map(|mount| Path::new(&mount.source))
            .ok_or_else(|| io::Error::other("builder exports mount is missing"))?;
        let root = exports
            .parent()
            .ok_or_else(|| io::Error::other("builder exports mount has no parent"))?;
        allocated_bytes(&root.join("containers/buildkit"))
    }

    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>> {
        // Through the post-build prune and trim, which also need the builder.
        let _building = Flock::lock(ownership::open_lock(BUILDER_LOCK)?, FlockArg::LockShared)
            .map_err(|(_, error)| io::Error::from(error))?;
        refresh_base(&std::fs::read(request.containerfile)?)?;
        let result = run_build(request)?;
        // Neither step fails the build.
        if prune_builder(&["--keep-storage", BUILDER_KEEP_MB]).is_err() {
            eprintln!("pinfold: could not prune the builder cache");
        }
        if trim_builder().is_err() {
            eprintln!("pinfold: could not reclaim unused builder blocks");
        }
        Ok(result)
    }

    fn program(&self) -> &'static str {
        "container"
    }
}

/// Refresh Apple's external base on the requested platform before building.
/// Local pinfold bases and unresolved arguments never trigger a pull.
fn refresh_base(containerfile: &[u8]) -> io::Result<()> {
    let Ok(text) = std::str::from_utf8(containerfile) else {
        return Ok(());
    };
    let mut platform = None;
    let base = text
        .lines()
        .map(str::split_whitespace)
        .find_map(|mut words| {
            if !words.next()?.eq_ignore_ascii_case("FROM") {
                return None;
            }
            for word in words {
                if let Some(value) = word.strip_prefix("--platform=") {
                    platform = Some(value);
                } else if !word.starts_with("--") {
                    return Some(word);
                }
            }
            None
        });
    // Build arguments are resolved by the builder. Guessing an unresolved
    // platform here can refresh every architecture (GitHub #78).
    if platform.is_some_and(|platform| platform.contains('$')) {
        return Ok(());
    }
    match base {
        Some("scratch") => Ok(()),
        Some(base) if !base.contains('$') => {
            // A pinfold-built base can never be pulled, so read its digest
            // locally before spending the retries on a doomed pull. Every
            // pinfold build records [`clean::BASE_LABEL`], empty when its
            // own base did not resolve, so its presence marks the image; a
            // caller image's family labels are all empty.
            if let Ok(image) = Apple.resolve_image(base)?
                && image.labels.contains_key(clean::BASE_LABEL)
            {
                return Ok(());
            }
            // A floating tag must be pulled for its digest to be current and
            // present to inspect. An unresolvable base has no digest.
            let platform = platform.unwrap_or("linux/arm64");
            let _ = output(&["container", "image", "pull", "--platform", platform, base]);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Run `container image inspect <reference>` and return its first entry. The
/// inner `Err` is the runtime's stderr when it cannot resolve the reference.
fn inspect_image(reference: &str) -> io::Result<Result<ListedImage, String>> {
    let json = match captured(&["container", "image", "inspect", reference])? {
        Ok(json) => json,
        Err(stderr) => return Ok(Err(stderr)),
    };
    let images: Vec<ListedImage> = parse_json("container image inspect", &json)?;
    images
        .into_iter()
        .next()
        .map(Ok)
        .ok_or_else(|| io::Error::other("container image inspect returned no image"))
}

/// Run `container build` for `request`. stdout and stderr are captured
/// through one pipe, so the output keeps its order. The inner `Err` is that
/// output when the build fails.
fn run_build(request: &BuildRequest) -> io::Result<Result<(), String>> {
    let (reader, writer) = crate::core::process::pipe()?;
    let mut command = std::process::Command::new("container");
    // Apple includes color settings in the shared builder's identity
    // (GitHub #77). Every pinfold caller must use the same settings.
    command.env_remove("NO_COLOR").env_remove("BUILDKIT_COLORS");
    command
        .arg("build")
        // The macOS CLI is arm64 only.
        .args(["--build-arg", "TARGETARCH=arm64"])
        .args(if request.cache {
            None
        } else {
            Some("--no-cache")
        })
        .arg("--file")
        .arg(request.containerfile);
    if let Some((name, value)) = request.build_arg {
        command.arg("--build-arg").arg(format!("{name}={value}"));
    }
    for tag in request.tags {
        command.arg("--tag").arg(tag);
    }
    for (key, value) in request.labels {
        command.arg("--label").arg(format!("{key}={value}"));
    }
    command
        // A bare relative context such as `--json` would parse as an option.
        .arg(std::path::absolute(request.context)?)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    let child = command.spawn_guarded();
    // The command holds the pipe's write ends, and the read below ends only
    // once every write end is closed.
    drop(command);
    let mut child = child.map_err(|error| spawn_error("container", error))?;
    let mut tail = VecDeque::new();
    let mut newlines = 0;
    for byte in io::Read::bytes(io::BufReader::new(reader)) {
        let byte = match byte {
            Ok(byte) => byte,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        tail.push_back(byte);
        newlines += usize::from(byte == b'\n');
        while tail.len() > BUILD_LOG_BYTES
            || newlines + usize::from(tail.back() != Some(&b'\n')) > 40
        {
            newlines -= usize::from(tail.pop_front() == Some(b'\n'));
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
