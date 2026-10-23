//! The box lifecycle: one `box up` process owns one box.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::ExitStatus;

use tokio::io::AsyncReadExt;
use tokio::signal::unix::{Signal, SignalKind, signal};

use crate::core::clean;
use crate::core::disk::Seeding;
use crate::core::plan::{Disk, Env, Mount, Plan, Route, valid_name};
use crate::core::profile::{Profile, Seed};
use crate::core::runs::{self, Terminal};
use crate::core::runtime::{self, BoxInfo, Child, Prepared, apple, runtime};
use crate::core::{artifacts, disk, login, ownership, proxy};
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
    /// The box's generation, which names its run records.
    pub run_id: String,
    ownership: ownership::Claim,
    /// The spec's disks, recorded clean once `down` removes the box.
    disks: Vec<Disk>,
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
    /// The box ended on its own: its runtime client or its VM exited.
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

    /// The `down` line's detail: the exit status of a box that exited.
    pub fn detail(&self) -> Option<String> {
        match self {
            Shutdown::BoxExited(status) => Some(status.to_string()),
            _ => None,
        }
    }

    pub fn terminal(&self) -> Terminal {
        Terminal {
            reason: self.as_str(),
            detail: self.detail(),
        }
    }
}

/// Why `up` refused, serialized as the reason string of the process
/// interface's `refused` line. Early refusals precede preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefusalReason {
    Spec,
    Profile,
    Runtime,
    ImageMissing,
    NameInUse,
    Login,
    DiskInUse,
    DiskUnclean,
    Limit,
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
/// other error. Failed-start cleanup removes the runtime child and claimed
/// state. It does not roll back fetched artifacts or ordinary home seeds.
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
    /// Start a box, wait for init's `ready` line, and return it. The
    /// profile, the host's runtime and the image are checked first. Then
    /// `up` claims the name, and only the claim's owner creates anything.
    /// File aliases are checked after preparation, before runtime startup.
    /// An alias refusal removes claimed state; prepared artifacts and home
    /// seeds remain.
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
        // against them before `home_disk` picks the disk behind `$HOME`, so
        // a clash is refused as spec and no tie can hide it.
        let harness = plan.harness.as_deref().map(artifacts::guest);
        let codex_config = codex.as_ref().map(|_| Path::new(login::CODEX_CONFIG_DIR));
        let extra: Vec<&Path> = harness.as_deref().into_iter().chain(codex_config).collect();
        plan.validate_guests(&extra)
            .map_err(|error| refused(&plan, RefusalReason::Spec, error))?;
        let runtime = runtime();
        runtime
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
        let seeding = seeds
            .map(|seeds| home_disk(&plan, seeds))
            .transpose()
            .map_err(|error| refused(&plan, RefusalReason::Profile, error.to_string()))?;
        // The image's identity labels are the box's too: Apple copies no
        // image label onto a box. Every image also carries the other
        // families empty, so pinfold's own label (`dev.pinfold.project` for
        // `pinfold pi`) must win over the image's empty one. A caller spec
        // cannot name this namespace, so no other clash can happen.
        for (key, value) in identity.labels.clone() {
            if key.starts_with("dev.pinfold.") {
                plan.labels.entry(key).or_insert(value);
            }
        }
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
        // Linux inventory is pinfold's own claims, which this one now holds.
        if cfg!(target_os = "macos") && runtime.list()?.iter().any(|box_| box_.id == plan.name) {
            return Err(refused(
                &plan,
                RefusalReason::NameInUse,
                format!("the runtime already has a box named {:?}", plan.name),
            ));
        }
        claim.prepare(&state_dir)?;
        // Disk refusals come after the name claim; returning drops the
        // claim, which removes its record and state.
        let claimed = match disk::claim(runtime, &mut claim, &plan, init) {
            Ok(claimed) => claimed,
            Err(Ok((reason, detail))) => return Err(refused(&plan, reason, detail)),
            Err(Err(error)) => return Err(error.into()),
        };
        let run_id = claim.generation().to_string();
        plan.labels
            .insert(ownership::GENERATION_LABEL.to_string(), run_id.clone());
        // Linux's inventory is this record. Its labels are the box's and
        // under them the image's, as a container copies them.
        if cfg!(target_os = "linux") {
            let mut labels = identity.labels.clone();
            labels.extend(plan.labels.clone());
            claim.describe(ownership::Described {
                labels,
                image_id: identity.id.clone(),
                image_ref: image.clone(),
                created: crate::core::rfc3339(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |since| since.as_secs() as i64),
                ),
                memory: plan.memory.clone(),
            })?;
        }

        // Dropping a start part-way tears nothing down, so a signal ends it
        // here and `child` says what to remove.
        claim.starting();
        let mut progress = Starting {
            claimed,
            ..Starting::default()
        };
        let starting = async {
            if let Some(share) = profile.as_ref().and_then(|profile| profile.share.as_ref()) {
                share.materialize()?;
            }
            runs::create(&run_id)?;
            let (observed, egress_log) = start(
                &mut plan,
                init,
                &identity.id,
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
            disk::filled(runtime, &progress.claimed.created)?;
            // Last before ready: every ready run has a launch record, and a
            // run that fails to start leaves none.
            runs::write_launch(&run_id, launch_record(&plan, &observed, &image))?;
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
                let ran = progress.child.is_some();
                let (removed, status) = match progress.child.take() {
                    Some(child) => {
                        let (removed, status) = child.abort(&plan.name).await;
                        (removed, Some(status))
                    }
                    None => (Ok(false), None),
                };
                if let Ok(synced) = removed {
                    // Only a stop pinfold asked of a running box synced its
                    // disks; a box that failed readiness ended, or may have,
                    // on its own. Even then, a disk unclean before this `up`
                    // stays so: an aborted start is no recovery.
                    if ran && (matches!(stop, Stop::NotReady(_)) || !synced) {
                        progress.claimed.marked.clear();
                    }
                    progress.claimed.undo(runtime, &plan.name);
                }
                let removed = removed.map(drop);
                if let Err(error) = runs::remove(&run_id) {
                    eprintln!("pinfold: startup cleanup: {error}");
                }
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
            run_id,
            ownership: claim,
            disks: plan.disks.clone(),
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
            reason = self.ended() => reason,
        };
        // Tear down whatever ended the wait, so a failed wait leaves no box.
        let down = self
            .down(reason.as_ref().ok().map(Shutdown::terminal))
            .await;
        let reason = reason?;
        down?;
        Ok(reason)
    }

    /// Wait for what ends a box without its caller: an accepted `box down`
    /// (`Signal`), an egress log append failure, or the box's init exiting.
    pub async fn ended(&mut self) -> io::Result<Shutdown> {
        tokio::select! {
            stop = self.ownership.stop_requested() => stop.map(|()| Shutdown::Signal),
            _ = audit_failure(&mut self.audit) => Ok(Shutdown::AuditLog),
            status = self.child.wait() => status.map(Shutdown::BoxExited),
        }
    }

    /// Stop and remove the box, write `terminal` as the run's terminal
    /// record, then delete its state directory and acknowledge `box down`.
    /// `None` writes no terminal record.
    pub async fn down(&mut self, terminal: Option<Terminal>) -> io::Result<()> {
        // Before the acknowledgement, so `inspect` after `box down` returns
        // already sees the end. A box that may still run gets none.
        let result = self.child.down(&self.name).await.and_then(|synced| {
            if synced {
                disk::detached(&self.disks)
            } else {
                Ok(())
            }
        });
        if let Some(terminal) = terminal.filter(|_| result.is_ok()) {
            runs::write_terminal(&self.run_id, &terminal);
        }
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
    let remove_orphan = |lock| {
        clean::remove_orphan(
            runtime(),
            name,
            observed.as_ref().map(|record| record.generation.as_str()),
            &lock,
        )
        .map(|_| ())
    };
    if let Some(lock) = ownership::try_name_lock(name)? {
        return remove_orphan(lock);
    }
    let stopped = ownership::request_stop(name, observed.as_ref());
    if matches!(stopped, Ok(true)) {
        return Ok(());
    }
    if let Some(lock) = ownership::try_name_lock(name)? {
        return remove_orphan(lock);
    }
    stopped.map(|_| ())
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

/// Startup resources retained when its future is cancelled.
#[derive(Default)]
struct Starting {
    child: Option<Child>,
    audit: Audit,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    preparing: Option<tokio::task::JoinHandle<Result<(Plan, Prepared), Stop>>>,
    /// The disks claimed for the box, which a failed start releases.
    claimed: disk::Claimed,
}

/// The start after the claim: harness, seeds, proxy, runtime, readiness.
/// The runtime child goes into `child` as soon as it exists, so a failure, or
/// a signal at any await, removes exactly that. Returns the box's labels as
/// the runtime reports them, and the egress log the proxy was given (`None`
/// without `egress`).
async fn start(
    plan: &mut Plan,
    init: &Path,
    image_id: &str,
    seeding: Option<&Seeding>,
    state_dir: &Path,
    codex: Option<login::Token>,
    progress: &mut Starting,
) -> Result<(BoxInfo, Option<PathBuf>), Stop> {
    // Fetch the harness after the claim. Early refusals download nothing;
    // alias admission follows preparation. The spec's own env wins.
    let mut resolved = plan.clone();
    let preparation_dir = state_dir.to_path_buf();
    let cancel = progress.cancel.clone();
    let image_id = image_id.to_string();
    let worker = progress
        .preparing
        .insert(tokio::task::spawn_blocking(move || {
            resolve_harness(&mut resolved, &preparation_dir, &cancel)?;
            let prepared = runtime::prepare(&resolved, &preparation_dir, &image_id, &cancel)?;
            Ok((resolved, prepared))
        }));
    let prepared = worker
        .await
        .map_err(|error| io::Error::other(format!("preparation worker failed: {error}")))?;
    progress.preparing.take();
    let (resolved, prepared) = prepared?;
    *plan = resolved;

    // The proxy comes up before the box, so the socket is listening when
    // the runtime forwards it.
    let mut egress_log = None;
    let runtime = runtime();
    let socket = match &plan.egress {
        Some(egress) => {
            let socket = runtime.proxy_socket(state_dir)?;
            let log = runs::egress_log(&plan.labels[ownership::GENERATION_LABEL])?;
            progress.audit = Audit::Waiting(proxy::start(&socket, egress, codex, log.clone())?);
            egress_log = Some(log);
            Some(socket)
        }
        None => None,
    };

    // A signal that arrived during the steps above wins the select here,
    // before the runtime is asked to create anything.
    tokio::task::yield_now().await;
    let child = progress.child.insert(runtime::start(
        prepared,
        plan,
        init,
        socket.as_deref(),
        &progress.claimed.fills,
        seeding,
    )?);
    tokio::select! {
        biased;
        _ = audit_failure(&mut progress.audit) => return Err(Stop::Failed(io::Error::other("audit-log: egress logging failed"))),
        ready = child.ready() => ready.map_err(Stop::NotReady)?,
    }
    // Apple can forward init's ready before it records the box running.
    // Wait even without egress, before the caller can issue its first exec.
    let actual = if cfg!(target_os = "macos") {
        let actual = apple::wait_until_running(&plan.name)?;
        // Finish the one root exec, the new disks' copies and the seeds
        // before ready reaches the caller.
        if socket.is_some() {
            apple::make_proxy_connectable(&plan.name)?;
        }
        for fill in &progress.claimed.fills {
            apple::copy_in(&plan.name, init, &fill.guest, false, |out| {
                disk::send(fill, out)
            })?;
        }
        if let Some(seeding) = seeding {
            apple::copy_in(&plan.name, init, &seeding.guest, true, |out| {
                disk::send_seeds(&seeding.relative, &seeding.seeds, out)
            })?;
        }
        Some(actual)
    } else {
        None
    };
    // `ready` reports the labels `list` will.
    let actual = match actual {
        Some(actual) => actual,
        None => runtime
            .list()?
            .into_iter()
            .find(|box_| box_.id == plan.name)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "the runtime does not list box {:?} after ready",
                    plan.name
                ))
            })?,
    };
    Ok((actual, egress_log))
}

/// The launch record's fields from the prepared plan and the started box.
/// Built field by field: the plan holds env values and header sources, which
/// the record never does.
fn launch_record(plan: &Plan, observed: &BoxInfo, image_ref: &str) -> serde_json::Value {
    let runtime = runtime();
    let egress = plan.egress.as_ref().map(|egress| {
        let routes: serde_json::Map<String, serde_json::Value> = egress
            .routes
            .iter()
            .map(|(name, route)| {
                let (kind, to) = match route {
                    Route::Address(address) => ("host-service", address.as_str()),
                    Route::Inject(inject) => ("injecting", inject.to.as_str()),
                    Route::Login(login) => ("login", login.origin().unwrap_or_default()),
                };
                (name.clone(), serde_json::json!({ "kind": kind, "to": to }))
            })
            .collect();
        serde_json::json!({ "allow": &egress.allow, "routes": routes })
    });
    let harness =
        plan.harness.as_deref().and_then(artifacts::harness).map(
            |harness| serde_json::json!({ "name": &harness.name, "version": &harness.version }),
        );
    let mounts: Vec<serde_json::Value> = plan
        .mounts
        .iter()
        .map(|mount| {
            serde_json::json!({
                "host": mount.host.to_string_lossy(),
                "guest": mount.guest.to_string_lossy(),
            })
        })
        .collect();
    let disks: Vec<serde_json::Value> = plan
        .disks
        .iter()
        .map(|disk| {
            serde_json::json!({
                "name": &disk.name,
                "guest": disk.guest.to_string_lossy(),
                "readonly": disk.readonly,
            })
        })
        .collect();
    serde_json::json!({
        "box": &plan.name,
        "pinfold": env!("CARGO_PKG_VERSION"),
        "runtime": {
            "name": if cfg!(target_os = "macos") { "apple" } else { "firecracker" },
            "version": runtime.version().ok(),
        },
        "image": { "id": &observed.image_id, "ref": image_ref },
        "profile": &plan.profile,
        "harness": harness,
        "labels": &observed.labels,
        "mounts": mounts,
        "disks": disks,
        "env": plan.env.keys().collect::<Vec<_>>(),
        "egress": egress,
        "cpus": plan.cpus,
        "memory": &plan.memory,
    })
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
        });
    }
    Ok(())
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

/// The disk behind the spec's exact `$HOME` and the plain path from its
/// guest root to `$HOME`, for the profile's `seeds`. It must be writable
/// and the path must not step out of it. Called after the duplicate-guest
/// check, so no two disks can hold `$HOME`.
fn home_disk(plan: &Plan, seeds: Vec<Seed>) -> io::Result<Seeding> {
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
    let Some((disk, relative)) = plan
        .disks
        .iter()
        .filter_map(|disk| Some((disk, home.strip_prefix(&disk.guest).ok()?)))
        .max_by_key(|(disk, _)| disk.guest.components().count())
    else {
        return Err(refuse(format!("$HOME={} is not on a disk", home.display())));
    };
    if disk.readonly {
        return Err(refuse(format!(
            "$HOME={} is on the read-only disk {}",
            home.display(),
            disk.guest.display()
        )));
    }
    // `..` would step out of the disk and `.` is noise.
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(refuse(format!(
            "$HOME={} is not a plain path under {}",
            home.display(),
            disk.guest.display()
        )));
    }
    Ok(Seeding {
        guest: disk.guest.clone(),
        relative: relative.to_path_buf(),
        seeds,
    })
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
