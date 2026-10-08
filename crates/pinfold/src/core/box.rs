//! The box lifecycle: one attached `container run` process owns one box.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};
use std::process::ExitStatus;

use nix::errno::Errno;
use nix::fcntl::{OFlag, openat};
use nix::sys::stat::{Mode, mkdirat};
use nix::unistd::{UnlinkatFlags, unlinkat};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Child;
use tokio::signal::unix::{Signal, SignalKind, signal};

use crate::core::clean;
use crate::core::plan::{Env, Mount, Plan, valid_name};
use crate::core::profile::{Profile, Seed};
use crate::core::runtime::{BoxInfo, apple, runtime};
use crate::core::{artifacts, login, ownership, proxy};
use crate::dirs;

/// A started box, owned by this process.
pub struct Box {
    name: String,
    /// The box's full label set, as the runtime reports it once the box is
    /// ready, the image's labels included.
    pub labels: BTreeMap<String, String>,
    /// The id of the image `image_ref` resolved to.
    pub image_id: String,
    /// The image reference the box was started from: the spec's, or its
    /// profile's.
    pub image_ref: String,
    /// The egress log `up` gave the proxy; `None` without `egress`.
    pub egress_log: Option<PathBuf>,
    ownership: ownership::Claim,
    _project: Option<nix::fcntl::Flock<std::fs::File>>,
    audit: Audit,
    child: Child,
}

/// What stopped the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    StdinEof,
    Signal,
    AuditLog,
    /// The attached `container run` process exited on its own.
    BoxExited(ExitStatus),
}

impl Shutdown {
    /// The reason string of the process interface's `down` line.
    pub fn as_str(&self) -> &'static str {
        match self {
            Shutdown::StdinEof => "stdin-closed",
            Shutdown::Signal => "signal",
            Shutdown::AuditLog => "audit-log",
            Shutdown::BoxExited(_) => "exited",
        }
    }
}

/// Why `up` refused, serialized as the reason string of the process
/// interface's `refused` line. A refused `up` leaves nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefusalReason {
    Spec,
    Profile,
    Runtime,
    ImageMissing,
    NameInUse,
    Login,
}

/// A refused `up`: why, and the detail the runtime or host gave. `box_name`
/// is `None` only when the spec's first JSON value holds no string `name`.
#[derive(Debug)]
pub struct Refusal {
    pub box_name: Option<String>,
    pub reason: RefusalReason,
    pub detail: String,
}

/// Why [`Box::up`] failed: a refusal, SIGTERM or SIGINT before ready, or any
/// other error. Each removes what the start made before it returns.
#[derive(Debug)]
pub enum UpError {
    Refused(Refusal),
    Signal,
    Control,
    Other(io::Error),
}

impl From<io::Error> for UpError {
    fn from(error: io::Error) -> UpError {
        UpError::Other(error)
    }
}

impl From<UpError> for io::Error {
    fn from(error: UpError) -> io::Error {
        match error {
            UpError::Refused(refusal) => {
                let reason = serde_json::to_value(refusal.reason).unwrap_or_default();
                let reason = reason.as_str().unwrap_or_default();
                io::Error::other(format!("{reason}: {}", refusal.detail))
            }
            UpError::Signal | UpError::Control => io::ErrorKind::Interrupted.into(),
            UpError::Other(error) => error,
        }
    }
}

impl Box {
    /// Start a box, wait for init's `ready` line, and return it. Every
    /// refusal but a taken name is decided before anything is created: the
    /// profile, the host's runtime and the image are checked first. Then
    /// `up` claims the name, and only the claim's owner creates anything.
    ///
    /// The caller registered SIGTERM and SIGINT before reading the spec:
    /// before ready they remove what the start made and `up` returns
    /// [`UpError::Signal`]; after ready [`Box::hold`] watches them.
    pub async fn up(plan: &Plan, init: &Path, signals: &mut Signals) -> Result<Box, UpError> {
        if init.parent().is_none_or(|parent| parent == Path::new("/")) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "init must live in a directory below /",
            )
            .into());
        }

        // The spec rules are the first refusal, before anything is created
        // or resolved.
        plan.validate()
            .map_err(|error| refused(plan, RefusalReason::Spec, error))?;

        // A login route's token is read before the claim, so a missing
        // variable or an unusable Codex login refuses as `login` and leaves
        // nothing. codex's token goes to the proxy.
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let login_plan = plan.clone();
        let mut resolving =
            tokio::task::spawn_blocking(move || login_plan.resolve_login(Some(&worker_cancel)));
        let codex = tokio::select! {
            biased;
            _ = signals.recv() => {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                let _ = resolving.await;
                return Err(UpError::Signal);
            }
            result = &mut resolving => result,
        }
        .map_err(|error| io::Error::other(format!("login worker failed: {error}")))?
        .map_err(|error| refused(plan, RefusalReason::Login, error))?;

        // The profile's image, share and home seeds are the box's to apply;
        // the runtime sees the resolved plan. Resolving writes nothing; the
        // seeds wait until the name is claimed.
        let mut plan = plan.clone();
        let mut profile = resolve_profile(&mut plan)
            .map_err(|error| refused(&plan, RefusalReason::Profile, error.to_string()))?;
        let seeds = profile.as_mut().and_then(|profile| {
            (!profile.home.is_empty()).then(|| std::mem::take(&mut profile.home))
        });

        // Pinfold adds the profile's `share/` above, and the harness mount
        // and a codex login's config dir in `start`. Check the spec's mounts
        // against them before `home_mount` picks the mount behind `$HOME`, so
        // a clash is refused as spec and no tie can hide it.
        let harness = plan.harness.as_deref().map(artifacts::guest);
        let codex_config = codex.as_ref().map(|_| Path::new(login::CODEX_CONFIG_DIR));
        let extra: Vec<&Path> = harness.as_deref().into_iter().chain(codex_config).collect();
        plan.validate_guests(&extra)
            .map_err(|error| refused(&plan, RefusalReason::Spec, error))?;
        let seeding = seeds
            .map(|seeds| home_mount(&plan, seeds))
            .transpose()
            .map_err(|error| refused(&plan, RefusalReason::Profile, error.to_string()))?;

        let runtime = runtime();
        let seccomp = runtime
            .preflight()
            .map_err(|error| refused(&plan, RefusalReason::Runtime, error.to_string()))?;

        // `validate` refused a spec with neither an image nor a profile, and
        // `resolve_profile` fills the image from the profile.
        let image = plan
            .image
            .clone()
            .expect("a validated spec has an image after profile resolution");
        // The runtime resolves the reference, so every spelling it resolves
        // locally is accepted. It is still given `image`, not the id.
        let identity = runtime.resolve_image(&image)?.map_err(|message| {
            refused(
                &plan,
                RefusalReason::ImageMissing,
                format!(
                    "image {image:?} is not present locally; build or pull it first: {message}"
                ),
            )
        })?;
        // The image's identity labels are the box's too: Apple copies no
        // image label onto a box. Every image also carries the other
        // families empty, so pinfold's own label (`dev.pinfold.project` for
        // `pinfold pi`) must win over the image's empty one. A caller spec
        // cannot name this namespace, so no other clash can happen.
        for (key, value) in identity.labels {
            if key.starts_with("dev.pinfold.") {
                plan.labels.entry(key).or_insert(value);
            }
        }
        // The owner label names this process to `list` and to Maintenance,
        // which prunes a box whose owner is gone; for a state dir other than
        // this one, it is also how the owner is judged alive.
        plan.labels.insert(
            clean::OWNER_LABEL.to_string(),
            std::process::id().to_string(),
        );

        let project = plan
            .labels
            .get(clean::PROJECT_LABEL)
            .filter(|id| !id.is_empty())
            .map(|id| ownership::project_lock(id))
            .transpose()?;
        let state_dir = dirs::box_state_dir(&plan.name)?;
        let mut claim = tokio::select! {
            biased;
            _ = signals.recv() => return Err(UpError::Signal),
            claim = ownership::Claim::take(&plan.name) => claim,
        }?
        .ok_or_else(|| {
            refused(
                &plan,
                RefusalReason::NameInUse,
                "name remained busy through the claim deadline".to_string(),
            )
        })?;
        match runtime.list() {
            Ok(boxes) if boxes.iter().all(|box_| box_.id != plan.name) => {}
            Ok(_) => {
                return Err(refused(
                    &plan,
                    RefusalReason::NameInUse,
                    format!("the runtime already has a box named {:?}", plan.name),
                ));
            }
            Err(error) => return Err(error.into()),
        }
        claim.prepare(&state_dir)?;
        plan.labels.insert(
            ownership::GENERATION_LABEL.to_string(),
            claim.generation().to_string(),
        );

        // Dropping a start part-way tears nothing down, so a signal ends it
        // here and `child` says what to remove.
        claim.starting();
        let mut progress = Starting::default();
        let starting = async {
            if let Some(share) = profile.as_ref().and_then(|profile| profile.share.as_ref()) {
                share.materialize()?;
            }
            let (observed, egress_log) = start(
                &mut plan,
                init,
                seccomp.as_deref(),
                seeding.as_ref(),
                &state_dir,
                codex,
                &mut progress,
            )
            .await?;
            if observed.image_id != identity.id {
                return Err(Stop::Failed(io::Error::other(
                    "image-changed: box started a different image",
                )));
            }
            Ok((observed.labels, egress_log))
        };
        let started = tokio::select! {
            biased;
            _ = signals.recv() => Err(Stop::Signal),
            stop = claim.stop_requested() => match stop { Ok(()) => Err(Stop::Control), Err(error) => Err(Stop::Failed(error)) },
            started = starting => started,
        };
        let (labels, egress_log) = match started {
            Ok(started) => started,
            Err(stop) => {
                progress
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if let Some(preparing) = progress.preparing.take() {
                    let _ = preparing.await;
                }
                let (removed, status) = abort(&plan.name, progress.child).await;
                if let Err(error) = claim.finish(removed.is_ok()).await {
                    eprintln!("pinfold: startup cleanup: {error}");
                }
                removed?;
                return Err(match stop {
                    Stop::Signal => UpError::Signal,
                    Stop::Control => UpError::Control,
                    Stop::Failed(error) => UpError::Other(error),
                    Stop::NotReady(failure) => UpError::Other(io::Error::other(match status {
                        Some(Ok(status)) => format!("{failure}: {status}"),
                        Some(Err(error)) => format!("{failure}: {error}"),
                        None => failure,
                    })),
                });
            }
        };

        Ok(Box {
            name: plan.name.clone(),
            labels,
            image_id: identity.id,
            image_ref: image,
            egress_log,
            ownership: claim,
            _project: project,
            audit: progress.audit,
            child: progress.child.expect("a started box has a runtime child"),
        })
    }

    /// Wait until the box exits, stdin closes, or a termination signal
    /// arrives, then stop and remove the box.
    pub async fn hold(&mut self, signals: &mut Signals) -> io::Result<Shutdown> {
        let reason = tokio::select! {
            reason = wait_for_shutdown(signals) => reason,
            stop = self.ownership.stop_requested() => stop.map(|()| Shutdown::Signal),
            _ = audit_failure(&mut self.audit) => Ok(Shutdown::AuditLog),
            status = self.child.wait() => status.map(Shutdown::BoxExited),
        };
        // Tear down whatever ended the wait, so a failed wait leaves no box.
        let down = self.down().await;
        let reason = reason?;
        down?;
        Ok(reason)
    }

    pub async fn stop_requested(&mut self) -> io::Result<()> {
        tokio::select! {
            stop = self.ownership.stop_requested() => stop,
            _ = audit_failure(&mut self.audit) => Err(io::Error::other("audit-log: egress logging failed")),
        }
    }

    /// Stop and remove the box, then delete its state directory.
    pub async fn down(&mut self) -> io::Result<()> {
        let result = runtime().down(&self.name);
        let _ = self.child.wait().await;
        let cleanup = self.ownership.finish(result.is_ok()).await;
        result.and(cleanup)
    }
}

/// Ask the observed generation to stop. Only an orphan is removed here.
pub fn down(name: &str) -> io::Result<()> {
    if !valid_name(name) {
        return Ok(());
    }
    let observed = ownership::record(name)?;
    if let Some(lock) = ownership::try_name_lock(name)? {
        return clean::remove_orphan(
            runtime(),
            name,
            observed.as_ref().map(|record| record.generation.as_str()),
            &lock,
        )
        .map(|_| ());
    }
    let stopped = ownership::request_stop(name, observed.as_ref());
    if let Some(lock) = ownership::try_name_lock(name)? {
        clean::remove_orphan(
            runtime(),
            name,
            observed.as_ref().map(|record| record.generation.as_str()),
            &lock,
        )?;
        return Ok(());
    }
    stopped
}

/// SIGTERM and SIGINT, handled by `box up` from before its spec is read to
/// its exit, and by `pinfold pi` beside SIGHUP.
pub struct Signals {
    terminate: Signal,
    interrupt: Signal,
    hangup: Option<Signal>,
    last: Option<nix::sys::signal::Signal>,
}

impl Signals {
    pub fn new() -> io::Result<Signals> {
        Ok(Signals {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
            hangup: None,
            last: None,
        })
    }

    pub fn with_hangup() -> io::Result<Signals> {
        let mut signals = Self::new()?;
        signals.hangup = Some(signal(SignalKind::hangup())?);
        Ok(signals)
    }

    pub fn last(&self) -> Option<nix::sys::signal::Signal> {
        self.last
    }

    pub async fn recv(&mut self) -> nix::sys::signal::Signal {
        let received = tokio::select! {
            _ = self.terminate.recv() => nix::sys::signal::SIGTERM,
            _ = self.interrupt.recv() => nix::sys::signal::SIGINT,
            _ = async { match self.hangup.as_mut() { Some(hangup) => { hangup.recv().await; }, None => std::future::pending().await } } => nix::sys::signal::SIGHUP,
        };
        self.last = Some(received);
        received
    }
}

/// Why a start ended before ready.
enum Stop {
    Signal,
    Control,
    /// Readiness failed; the runtime child's exit status completes the text.
    NotReady(String),
    Failed(io::Error),
}

impl From<io::Error> for Stop {
    fn from(error: io::Error) -> Stop {
        Stop::Failed(error)
    }
}

/// Stop a failed start's runtime child and remove its box. The caller
/// removes the claimed state only after successful runtime removal.
async fn abort(
    name: &str,
    child: Option<Child>,
) -> (io::Result<()>, Option<io::Result<ExitStatus>>) {
    match child {
        Some(mut child) => {
            // The client goes first, so one still creating the box cannot
            // finish after the removal. The box exists by name even when
            // readiness failed.
            let _ = child.start_kill();
            let status = child.wait().await;
            (runtime().down(name), Some(status))
        }
        None => (Ok(()), None),
    }
}

/// Startup resources retained when its future is cancelled.
#[derive(Default)]
struct Starting {
    child: Option<Child>,
    audit: Audit,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    preparing: Option<tokio::task::JoinHandle<io::Result<Plan>>>,
}

/// The start after the claim: harness, seeds, proxy, runtime, readiness.
/// The runtime child goes into `child` as soon as it exists, so a failure, or
/// a signal at any await, removes exactly that. Returns the box's labels as
/// the runtime reports them, and the egress log the proxy was given (`None`
/// without `egress`).
async fn start(
    plan: &mut Plan,
    init: &Path,
    seccomp: Option<&Path>,
    seeding: Option<&Seeding>,
    state_dir: &Path,
    codex: Option<login::Token>,
    progress: &mut Starting,
) -> Result<(BoxInfo, Option<PathBuf>), Stop> {
    // The harness artifact is fetched and folded in after the claim, so a
    // refused box downloads nothing. The spec's own env wins.
    let mut prepared = plan.clone();
    let preparation_dir = state_dir.to_path_buf();
    let cancel = progress.cancel.clone();
    let worker = progress
        .preparing
        .insert(tokio::task::spawn_blocking(move || {
            resolve_harness(&mut prepared, &preparation_dir, &cancel)?;
            Ok(prepared)
        }));
    let prepared = worker
        .await
        .map_err(|error| io::Error::other(format!("harness worker failed: {error}")))?;
    progress.preparing.take();
    *plan = prepared?;

    if let Some(seeding) = seeding {
        seed_home(seeding)?;
    }

    // The proxy comes up before the box, so the socket is listening when
    // the runtime forwards it.
    let mut egress_log = None;
    let socket = match &plan.egress {
        Some(egress) => {
            let socket = state_dir.join("proxy.sock");
            let log = dirs::egress_dir()?.join(format!("{}.jsonl", plan.name));
            progress.audit = Audit::Waiting(proxy::start(&socket, egress, codex, log.clone())?);
            egress_log = Some(log);
            Some(socket)
        }
        None => None,
    };

    // A signal that arrived during the steps above wins the select here,
    // before the runtime is asked to create anything.
    tokio::task::yield_now().await;
    let runtime = runtime();
    let child = progress
        .child
        .insert(runtime.up(plan, init, socket.as_deref(), seccomp)?);
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("runtime up did not pipe the box's stdout"))?;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let next = tokio::select! {
            biased;
            _ = audit_failure(&mut progress.audit) => return Err(Stop::Failed(io::Error::other("audit-log: egress logging failed"))),
            next = lines.next_line() => next,
        };
        match next {
            Ok(Some(line)) if line == "ready" => break,
            Ok(Some(_)) => {}
            Ok(None) => return Err(Stop::NotReady("box exited before ready".to_string())),
            Err(error) => return Err(Stop::NotReady(format!("box output failed: {error}"))),
        }
    }
    // Apple can forward init's ready before it records the box running.
    // Wait even without egress, before the caller can issue its first exec.
    if cfg!(target_os = "macos") {
        apple::wait_until_running(&plan.name)?;
        // Finish the one root exec before ready reaches the caller.
        if socket.is_some() {
            apple::make_proxy_connectable(&plan.name)?;
        }
    }
    // Keep the pipe drained so a talkative box cannot block on it.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    // `ready` reports the labels `list` will: podman adds every image label
    // to the box, so the plan's set is not the box's.
    let actual = runtime
        .list()?
        .into_iter()
        .find(|box_| box_.id == plan.name)
        .ok_or_else(|| {
            io::Error::other(format!(
                "the runtime does not list box {:?} after ready",
                plan.name
            ))
        })?;
    Ok((actual, egress_log))
}

async fn wait_for_shutdown(signals: &mut Signals) -> io::Result<Shutdown> {
    let mut stdin = tokio::io::stdin();
    let mut buffer = [0u8; 4096];
    loop {
        tokio::select! {
            _ = signals.recv() => return Ok(Shutdown::Signal),
            read = stdin.read(&mut buffer) => {
                if read? == 0 {
                    return Ok(Shutdown::StdinEof);
                }
            },
        }
    }
}

/// Fold the spec's harness into the plan: install the pinned harness if it
/// is not cached, mount it read-only, and set its environment. The spec's
/// own env wins over the harness defaults. A codex login's config is written
/// into the box's state dir and mounted read-only at `/etc/codex`.
fn resolve_harness(
    plan: &mut Plan,
    state_dir: &Path,
    cancel: &std::sync::atomic::AtomicBool,
) -> io::Result<()> {
    let Some(harness) = plan.harness.as_deref().and_then(artifacts::harness) else {
        return Ok(());
    };
    plan.mounts.push(Mount {
        host: harness.install(Some(cancel))?,
        guest: artifacts::guest(&harness.name),
        readonly: true,
    });
    let login = plan
        .login()
        .map(|(route, login)| (route.to_owned(), login.clone()));
    let allow = plan
        .egress
        .as_ref()
        .map(|egress| egress.allow.join(","))
        .unwrap_or_default();
    // The first default for a name wins: the harness's, then a login route's
    // placeholders, then the allowlist.
    let defaults = harness
        .env
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .chain(
            login
                .iter()
                .flat_map(|(route, login)| login.env(route))
                .map(|(name, value)| (name.to_owned(), value)),
        )
        .chain([("PINFOLD_ALLOW".to_owned(), allow)]);
    for (name, value) in defaults {
        plan.env.entry(name).or_insert(Env::Exact(value));
    }
    if let Some((route, _)) = login.filter(|(_, login)| login.is_codex()) {
        let dir = state_dir.join("codex");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("config.toml"), login::codex_config(&route))?;
        plan.mounts.push(Mount {
            host: dir,
            guest: PathBuf::from(login::CODEX_CONFIG_DIR),
            readonly: true,
        });
    }
    Ok(())
}

/// A profile's `home/` seeds and the writable mount behind `$HOME` to copy
/// them into.
struct Seeding {
    mount: Mount,
    /// The plain path from the mount's guest root to `$HOME`.
    relative: PathBuf,
    seeds: Vec<Seed>,
}

/// Load the spec's profile and fold its image and `share/` mount into the
/// plan without extracting files. Shared files and `$HOME` seeds wait until
/// the name is claimed.
fn resolve_profile(plan: &mut Plan) -> io::Result<Option<Profile>> {
    let Some(name) = plan.profile.clone() else {
        return Ok(None);
    };
    let profile = Profile::load(&name)?;
    if plan.image.is_none() {
        plan.image = Some(profile.image_ref());
    }
    if let Some(share) = &profile.share {
        plan.mounts.push(Mount {
            host: share.path()?,
            guest: PathBuf::from("/opt/pinfold/profile"),
            readonly: true,
        });
    }
    Ok(Some(profile))
}

/// A refusal carrying `plan`'s box name.
fn refused(plan: &Plan, reason: RefusalReason, detail: impl Into<String>) -> UpError {
    UpError::Refused(Refusal {
        box_name: Some(plan.name.clone()),
        reason,
        detail: detail.into(),
    })
}

/// The mount behind the spec's exact `$HOME` and the plain path from its
/// guest root to `$HOME`, for the profile's `seeds`. The mount must be
/// writable and the path must not step out of it. Called after the
/// duplicate-guest check, so no two mounts can hold `$HOME`.
fn home_mount(plan: &Plan, seeds: Vec<Seed>) -> io::Result<Seeding> {
    let name = plan.profile.as_deref().unwrap_or_default();
    let refuse = |why: String| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("profile {name:?} seeds $HOME, but {why}"),
        )
    };
    let Some(Env::Exact(home)) = plan.env.get("HOME") else {
        return Err(refuse("the box spec has no exact HOME env entry".into()));
    };
    let home = Path::new(home);
    // The deepest mount holding `$HOME`. The duplicate check ran first, so
    // no two mounts can name one guest path and the pick is unambiguous.
    let Some((mount, relative)) = plan
        .mounts
        .iter()
        .filter_map(|mount| Some((mount, home.strip_prefix(&mount.guest).ok()?)))
        .max_by_key(|(mount, _)| mount.guest.components().count())
    else {
        return Err(refuse(format!(
            "$HOME={} is not on a mount",
            home.display()
        )));
    };
    if mount.readonly {
        return Err(refuse(format!(
            "$HOME={} is on the read-only mount {}",
            home.display(),
            mount.guest.display()
        )));
    }
    // The walk below handles normal components only; `..` would step out of
    // the mount and `.` is noise.
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(refuse(format!(
            "$HOME={} is not a plain path under the mount {}",
            home.display(),
            mount.guest.display()
        )));
    }
    Ok(Seeding {
        mount: mount.clone(),
        relative: relative.to_path_buf(),
        seeds,
    })
}

/// Copy seeds into the host `$HOME`, skipping anything already there. The
/// walk starts at the mount's host directory and follows no symlink: the box
/// can write the mount, so a planted symlink must not redirect a seed
/// outside it.
fn seed_home(seeding: &Seeding) -> io::Result<()> {
    let host = &seeding.mount.host;
    // The spec names the mount's host directory and the runtime mounts it,
    // so it is the seed walk's trust root.
    let root = nix::fcntl::open(host, OFlag::O_DIRECTORY | OFlag::O_CLOEXEC, Mode::empty())
        .map_err(|error| seed_error(error, &format!("open mount {}", host.display())))?;
    for seed in &seeding.seeds {
        seed_file(
            &root,
            host,
            &seeding.relative.join(&seed.path),
            &seed.contents,
        )?;
    }
    Ok(())
}

/// Write one seed at `relative` below `root`, walking it with `openat` and
/// `mkdirat` and following no symlink. Anything already at the destination,
/// a dangling symlink included, is left alone. `display` names `root`.
fn seed_file(root: &OwnedFd, display: &Path, relative: &Path, contents: &[u8]) -> io::Result<()> {
    let mut dir = root.try_clone()?;
    let mut current = display.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("seed path {} is not a plain file path", relative.display()),
            ));
        };
        current.push(name);
        if components.peek().is_some() {
            dir = open_seed_dir(&dir, name, &current)?;
            continue;
        }
        let fd = match openat(
            &dir,
            name,
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::from_bits_truncate(0o666),
        ) {
            Ok(fd) => fd,
            // Present, whatever it is; a seed is copied only when missing.
            Err(Errno::EEXIST | Errno::ELOOP) => return Ok(()),
            Err(error) => return Err(seed_error(error, &format!("create {}", current.display()))),
        };
        let mut file = File::from(fd);
        if let Err(error) = file.write_all(contents) {
            // A partial seed must not look present to the next start.
            drop(file);
            let _ = unlinkat(&dir, name, UnlinkatFlags::NoRemoveDir);
            return Err(io::Error::new(
                error.kind(),
                format!("write {}: {error}", current.display()),
            ));
        }
    }
    Ok(())
}

/// Open a seed's parent directory, creating it when missing. `O_NOFOLLOW`
/// refuses a symlink planted at any component below `$HOME`.
fn open_seed_dir(dir: &OwnedFd, name: &OsStr, path: &Path) -> io::Result<OwnedFd> {
    let flags = OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    match openat(dir, name, flags, Mode::empty()) {
        Ok(fd) => Ok(fd),
        Err(Errno::ENOENT) => {
            match mkdirat(dir, name, Mode::from_bits_truncate(0o777)) {
                Ok(()) | Err(Errno::EEXIST) => {}
                Err(error) => {
                    return Err(seed_error(
                        error,
                        &format!("create directory {}", path.display()),
                    ));
                }
            }
            // Open the directory just made; a symlink swapped in meanwhile
            // is refused here.
            openat(dir, name, flags, Mode::empty())
                .map_err(|error| seed_error(error, &format!("open directory {}", path.display())))
        }
        Err(error) => Err(seed_error(
            error,
            &format!("open directory {}", path.display()),
        )),
    }
}

fn seed_error(error: Errno, context: &str) -> io::Error {
    let kind = io::Error::from(error).kind();
    io::Error::new(kind, format!("{context}: {error}"))
}

#[derive(Default)]
enum Audit {
    #[default]
    Off,
    Waiting(tokio::sync::oneshot::Receiver<()>),
    Failed,
}

async fn audit_failure(audit: &mut Audit) {
    if let Audit::Waiting(receiver) = audit {
        // A consumed notification stays failed for every subsequent wait.
        *audit = if receiver.await.is_ok() {
            Audit::Failed
        } else {
            Audit::Off
        };
    }
    if matches!(audit, Audit::Failed) {
        return;
    }
    std::future::pending::<()>().await;
}
