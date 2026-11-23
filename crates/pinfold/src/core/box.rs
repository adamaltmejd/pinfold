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
use crate::core::plan::{Disk, Env, Login, Mount, MountKind, Plan, Route, Values, valid_name};
use crate::core::profile::{Profile, Seed, Share};
use crate::core::runs::{self, Terminal};
use crate::core::runtime::{self, BoxInfo, Child, Prepared, runtime};
use crate::core::{disk, login, ownership, proxy};
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
    #[cfg(target_os = "linux")]
    _image_lease: nix::fcntl::Flock<std::fs::File>,
    /// The spec's disks, recorded clean once `down` removes the box.
    disks: Vec<Disk>,
    _project: Option<nix::fcntl::Flock<std::fs::File>>,
    child: Child,
    proxy: Option<proxy::Server>,
    audit_done: Option<bool>,
    terminal_attempted: bool,
    down: bool,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefusalReason {
    Spec,
    Profile,
    Runtime,
    RuntimePath,
    ImageMissing,
    NameInUse,
    Login,
    DiskInUse,
    DiskMissing,
    DiskUnclean,
    Limit,
}

impl RefusalReason {
    pub fn token(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    /// Whether pinfold writes every detail of this reason itself, naming
    /// keys and paths only, never a tool's output.
    pub fn authored(self) -> bool {
        matches!(self, RefusalReason::Spec | RefusalReason::Login)
    }

    /// The reason a token names, if it names one.
    pub fn from_token(token: &str) -> Option<RefusalReason> {
        serde_json::from_value(serde_json::Value::String(token.to_owned())).ok()
    }
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
                io::Error::other(format!("{}: {}", refusal.reason.token(), refusal.detail))
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
    /// Share layout and aliases are checked before maintenance or preparation.
    ///
    /// The caller registered SIGTERM and SIGINT before reading the spec:
    /// before ready they remove what the start made and `up` returns
    /// [`UpError::Signal`]; after ready [`Box::hold`] watches them.
    pub async fn up(plan: &Plan, signals: &mut Signals) -> Result<Box, UpError> {
        Self::up_disks(plan, &Values::read(plan), signals, false, None, None, None).await
    }

    /// Export must never create a missing disk or inspect one another box holds.
    pub(crate) async fn up_existing(plan: &Plan, signals: &mut Signals) -> Result<Box, UpError> {
        Self::up_disks(plan, &Values::read(plan), signals, true, None, None, None).await
    }

    /// `values` are the ones the project client read and sent.
    pub(crate) async fn up_project(
        plan: &Plan,
        values: &Values,
        profile: Profile,
        image_id: &str,
        signals: &mut Signals,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Box, UpError> {
        Self::up_disks(
            plan,
            values,
            signals,
            false,
            Some(profile),
            Some(image_id),
            Some(cancel),
        )
        .await
    }

    async fn up_disks(
        plan: &Plan,
        values: &Values,
        signals: &mut Signals,
        existing_only: bool,
        profile: Option<Profile>,
        expected_image: Option<&str>,
        mut cancel_start: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> Result<Box, UpError> {
        // The spec rules are the first refusal, before anything is created
        // or resolved. Only `box up` reads `op` itself: the project client
        // sends its values. `op` may wait on the user, so it runs once every
        // check before the claim that needs no value has passed; until
        // then validation checks `from` sources alone.
        plan.validate(Some(values))
            .map_err(|error| refused(plan, RefusalReason::Spec, error))?;
        let reads_op = expected_image.is_none() && plan.reads_op();
        let (requested, mut values) = (plan, values.clone());
        let init_path = crate::core::artifacts::init_path()?;
        let init = init_path.as_path();
        let destinations =
            login_destinations(plan).map_err(|error| refused(plan, RefusalReason::Spec, error))?;
        let directories = generated_directories(plan, &destinations);

        // The profile's image, share and home seeds are the box's to apply;
        // the runtime sees the resolved plan. Resolving writes nothing; the
        // seeds wait until the name is claimed.
        let mut plan = plan.clone();
        let mut profile = resolve_profile(&mut plan, profile)
            .map_err(|error| refused(&plan, RefusalReason::Profile, error.to_string()))?;
        let seeds = profile.as_mut().and_then(|profile| {
            (!profile.home.is_empty()).then(|| std::mem::take(&mut profile.home))
        });
        let layout_init = init.to_path_buf();
        let layout_destinations = destinations.clone();
        let maintenance = expected_image.is_none() && !existing_only;
        let (mut plan, profile) = startup_work(signals, &mut cancel_start, None, move || {
            crate::core::mounts::prepare(&mut plan, profile.as_ref(), &layout_init)
                .map_err(|error| refused(&plan, RefusalReason::Spec, error.to_string()))?;
            let extra: Vec<&Path> = layout_destinations.iter().map(PathBuf::as_path).collect();
            validate_generated(&plan, &extra)
                .map_err(|error| refused(&plan, RefusalReason::Spec, error))?;
            if maintenance {
                clean::maintain();
            }
            crate::core::artifacts::init()?;
            Ok((plan, profile))
        })
        .await?;

        // The project's login tokens are read before its early claim.
        let early_codex = match expected_image {
            Some(_) => Some(resolve_logins(&plan, &values, signals, &mut cancel_start).await?),
            None => None,
        };

        let runtime = runtime();
        let state_dir = dirs::box_state_dir(&plan.name)?;
        // Project inputs were checked before bootstrap. Publish control before
        // helpers can wait, but never replace an existing name's evidence.
        let mut early_claim = if expected_image.is_some() {
            let mut claim = take_claim(&plan, signals, &mut cancel_start).await?;
            if ownership::record(&plan.name)?.is_some() {
                return Err(refused(
                    &plan,
                    RefusalReason::NameInUse,
                    "the project box name already has an ownership record".to_string(),
                ));
            }
            claim
                .prepare_fresh(&state_dir)
                .map_err(|error| ownership_error(&plan, error))?;
            Some(claim)
        } else {
            None
        };
        let preflight_plan = plan.clone();
        startup_work(
            signals,
            &mut cancel_start,
            early_claim.as_mut(),
            move || {
                runtime.preflight().map_err(|error| {
                    refused(&preflight_plan, RefusalReason::Runtime, error.to_string())
                })
            },
        )
        .await?;

        // `validate` refused a spec with neither an image nor a profile, and
        // `resolve_profile` fills the image from the profile.
        let image = plan
            .image
            .clone()
            .expect("a validated spec has an image after profile resolution");
        // The runtime resolves the reference, so every spelling it resolves
        // locally is accepted. It is still given `image`, not the id.
        let image_plan = plan.clone();
        let selected = image.clone();
        #[cfg(not(target_os = "linux"))]
        let identity = startup_work(signals, &mut cancel_start, early_claim.as_mut(), move || {
            crate::core::image::service()
                .resolve_image(&selected)?
                .map_err(|message| {
                    refused(
                        &image_plan,
                        RefusalReason::ImageMissing,
                        format!(
                            "image {selected:?} is not present locally; build or pull it first: {message}"
                        ),
                    )
                })
        })
        .await?;
        #[cfg(target_os = "linux")]
        let (identity, image_lease) = startup_work(
            signals,
            &mut cancel_start,
            early_claim.as_mut(),
            move || {
                crate::core::image::linux::catalog::resolve_leased(&selected)?
                    .map_err(|detail| refused(&image_plan, RefusalReason::ImageMissing, detail))
            },
        )
        .await?;
        if expected_image.is_some_and(|expected| expected != identity.id) {
            return Err(
                io::Error::other("image-changed: selected image changed before startup").into(),
            );
        }
        let seeding = seeds
            .map(|seeds| home_disk(&plan, seeds))
            .transpose()
            .map_err(|error| refused(&plan, RefusalReason::Profile, error.to_string()))?;
        let codex = match early_codex {
            Some(codex) => codex,
            None => {
                if reads_op {
                    let reading = plan.clone();
                    values = startup_work(signals, &mut cancel_start, None, move || {
                        values.read_op(&reading);
                        Ok(values)
                    })
                    .await?;
                    requested
                        .validate(Some(&values))
                        .map_err(|error| refused(&plan, RefusalReason::Spec, error))?;
                }
                resolve_logins(&plan, &values, signals, &mut cancel_start).await?
            }
        };
        // The image's identity labels are the box's too: Apple copies no
        // image label onto a box. Every image also carries the other
        // families empty, so pinfold's own label (`dev.pinfold.project` for
        // `pinfold -- CMD`) must win over the image's empty one. A caller spec
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
        let prepared_early = early_claim.is_some();
        let mut claim = match early_claim {
            Some(claim) => claim,
            None => take_claim(&plan, signals, &mut cancel_start).await?,
        };
        // Linux inventory is pinfold's own claims, which this one now holds.
        #[cfg(target_os = "macos")]
        let occupied = {
            let inventory_name = plan.name.clone();
            startup_work(
                signals,
                &mut cancel_start,
                prepared_early.then_some(&mut claim),
                move || Ok(runtime.list()?.iter().any(|box_| box_.id == inventory_name)),
            )
            .await?
        };
        #[cfg(not(target_os = "macos"))]
        let occupied = false;
        if occupied {
            return Err(refused(
                &plan,
                RefusalReason::NameInUse,
                format!("the runtime already has a box named {:?}", plan.name),
            ));
        }
        if !prepared_early {
            claim
                .prepare(&state_dir)
                .map_err(|error| ownership_error(&plan, error))?;
        }
        // Disk refusals come after the name claim; returning drops the
        // claim, which removes its record and state.
        let mut control = claim.take_startup_control();
        let disk_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = disk_cancel.clone();
        let (disk_plan, disk_init, disk_directories) =
            (plan.clone(), init.to_path_buf(), directories.clone());
        let mut preparing_disks = tokio::task::spawn_blocking(move || {
            let result = crate::core::process::with_cancellation(worker_cancel.clone(), || {
                disk::claim(
                    runtime,
                    &mut claim,
                    &disk_plan,
                    &disk_init,
                    existing_only,
                    &disk_directories,
                    &worker_cancel,
                )
            });
            (claim, result)
        });
        let (prepared_disks, stopped) = tokio::select! {
            biased;
            _ = signals.recv() => {
                disk_cancel.store(true, std::sync::atomic::Ordering::Release);
                (preparing_disks.await, Some(UpError::Signal))
            },
            _ = cancelled(&mut cancel_start) => {
                disk_cancel.store(true, std::sync::atomic::Ordering::Release);
                (preparing_disks.await, Some(UpError::Control))
            },
            stop = control.stop_requested() => {
                disk_cancel.store(true, std::sync::atomic::Ordering::Release);
                let error = match stop { Ok(()) => UpError::Control, Err(error) => UpError::Other(error) };
                (preparing_disks.await, Some(error))
            },
            result = &mut preparing_disks => (result, None),
        };
        let (mut claim, claimed) =
            prepared_disks.map_err(|_| io::Error::other("disk preparation worker failed"))?;
        claim.restore_startup_control(control);
        if let Some(stopped) = stopped {
            #[cfg(not(target_os = "linux"))]
            if let Err(Err(error)) = &claimed
                && runtime::apple::preparation_unremoved(error)
            {
                let _ = claim.finish(false).await;
                return Err(match claimed {
                    Err(Err(error)) => error.into(),
                    _ => unreachable!(),
                });
            }
            if let Ok(claimed) = &claimed {
                claim.retire()?;
                claimed.undo(runtime, &plan.name);
            }
            claim.finish(true).await?;
            return Err(stopped);
        }
        let claimed = match claimed {
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
                created: crate::core::rfc3339(crate::core::now() as i64),
                memory: plan.memory.clone(),
            })?;
        }

        // Dropping a start part-way tears nothing down, so a signal ends it
        // here and `child` says what to remove.
        claim.starting();
        let mut progress = Starting {
            claimed,
            directories,
            destinations,
            sockets: claim.sockets().clone(),
            child: None,
            proxy: None,
            audit_done: None,
            cancel: std::sync::Arc::default(),
            preparing: None,
            finishing: None,
        };
        let starting = async {
            if let Some(share) = profile.as_ref().and_then(|profile| profile.share.as_ref()) {
                share.materialize(&state_dir)?;
            }
            runs::create(&run_id)?;
            let (observed, egress_log) = start(
                &mut plan,
                &values,
                init,
                &identity.id,
                seeding.as_ref(),
                &state_dir,
                codex,
                &mut progress,
            )
            .await?;
            // Apple runs the reference; Linux boots `identity.id` itself.
            #[cfg(not(target_os = "linux"))]
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
            _ = cancelled(&mut cancel_start) => Err(Stop::Control),
            stop = claim.stop_requested() => match stop { Ok(()) => Err(Stop::Control), Err(error) => Err(Stop::Failed(error)) },
            started = starting => started,
        };
        let started = started.and_then(|result| {
            claim.ready().map_err(Stop::Failed)?;
            Ok(result)
        });
        let (labels, egress_log) = match started {
            Ok(started) => started,
            Err(stop) => {
                progress
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if let Some(preparing) = progress.preparing.take() {
                    let _ = preparing.await;
                }
                if let Some(finishing) = progress.finishing.take() {
                    let _ = finishing.await;
                }
                let ran = progress.child.is_some();
                let (removed, status) = match progress.child.take() {
                    Some(child) => {
                        let (removed, status) = child.abort(&plan.name).await;
                        (removed, Some(status))
                    }
                    None => {
                        #[cfg(target_os = "linux")]
                        let removed = runtime.down(&plan.name).map(|()| false);
                        #[cfg(not(target_os = "linux"))]
                        let removed = Ok(false);
                        (removed, None)
                    }
                };
                let workers = stop_proxy(&mut progress.proxy);
                let removed = removed
                    .and_then(|synced| workers.map(|()| synced))
                    .and_then(|synced| claim.retire().map(|()| synced));
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
                    crate::core::process::diagnostic(format_args!("startup cleanup: {error}"));
                }
                if let Err(error) = claim.finish(removed.is_ok()).await {
                    crate::core::process::diagnostic(format_args!("startup cleanup: {error}"));
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
            #[cfg(target_os = "linux")]
            _image_lease: image_lease,
            disks: plan.disks.clone(),
            _project: project,
            proxy: progress.proxy,
            audit_done: progress.audit_done,
            terminal_attempted: false,
            down: false,
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
        let reason = if self.audit_failed() {
            Ok(Shutdown::AuditLog)
        } else {
            reason
        }?;
        down?;
        Ok(reason)
    }

    /// Wait for what ends a box without its caller: an accepted `box down`
    /// (`Signal`), an egress log append failure, or the box's init exiting.
    pub async fn ended(&mut self) -> io::Result<Shutdown> {
        tokio::select! {
            stop = self.ownership.stop_requested() => stop.map(|()| Shutdown::Signal),
            _ = audit_failure(&mut self.proxy, &mut self.audit_done) => Ok(Shutdown::AuditLog),
            status = self.child.wait() => status.map(Shutdown::BoxExited),
        }
    }

    /// Check shutdown once before admission, including a runtime exit whose
    /// status is not yet published by its waiter.
    pub async fn ended_if_ready(&mut self) -> io::Result<Option<Shutdown>> {
        let observed = tokio::select! {
            biased;
            reason = self.ended() => Some(reason),
            _ = std::future::ready(()) => None,
        };
        if let Some(reason) = observed {
            return reason.map(Some);
        }
        if self.child.exited()? {
            return self.ended().await.map(Some);
        }
        Ok(None)
    }

    /// The notification remains available after the proxy workers join.
    pub(crate) fn audit_failed(&mut self) -> bool {
        if self.audit_done.is_none()
            && let Some(server) = self.proxy.as_mut()
        {
            self.audit_done = match server.failure.try_recv() {
                Ok(()) => Some(true),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Some(false),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
            };
        }
        self.audit_done == Some(true)
    }

    /// Stop and remove the box, write `terminal` as the run's terminal
    /// record, then delete its state directory and acknowledge `box down`.
    /// `None` writes no terminal record unless the egress audit failed.
    pub async fn down(&mut self, terminal: Option<Terminal>) -> io::Result<()> {
        if self.down {
            return Ok(());
        }
        // Before the acknowledgement, so `inspect` after `box down` returns
        // already sees the end. A box that may still run gets none.
        let removed = self.child.down(&self.name).await;
        let workers = stop_proxy(&mut self.proxy);
        let terminal = if self.audit_failed() {
            Some(Shutdown::AuditLog.terminal())
        } else {
            terminal
        };
        let result = removed
            .and_then(|synced| workers.map(|()| synced))
            .and_then(|synced| {
                if synced {
                    disk::detached(&self.disks)
                } else {
                    Ok(())
                }
            });
        if !self.terminal_attempted
            && let Some(terminal) = terminal.filter(|_| result.is_ok())
        {
            self.terminal_attempted = true;
            runs::write_terminal(&self.run_id, &terminal);
        }
        let cleanup = self.ownership.finish(result.is_ok()).await;
        let result = result.and(cleanup);
        self.down = result.is_ok();
        if result.is_err() {
            runs::write_teardown_failure(&self.run_id);
        }
        result
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
/// its exit, and by project commands beside SIGHUP.
pub struct Signals {
    terminate: Signal,
    interrupt: Signal,
    hangup: Option<Signal>,
    /// An input whose writer closing is received as SIGHUP, by every
    /// `recv` from then on.
    closed: Option<tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>>,
    last: Option<nix::sys::signal::Signal>,
}

impl Signals {
    pub fn new() -> io::Result<Signals> {
        Ok(Signals {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
            hangup: None,
            closed: None,
            last: None,
        })
    }

    pub fn with_hangup() -> io::Result<Signals> {
        let mut signals = Self::new()?;
        signals.hangup = Some(signal(SignalKind::hangup())?);
        Ok(signals)
    }

    /// Also receive SIGHUP once INPUT's writers have all closed it:
    /// readable with nothing to read. It stays received, so no retry
    /// outlives it. Input that does arrive ends the
    /// watch, since only a closed writer is a hangup.
    pub fn with_closing(mut self, input: std::os::fd::BorrowedFd<'_>) -> io::Result<Signals> {
        // SAFETY: the owned duplicate keeps the registered descriptor stable.
        self.closed = Some(unsafe {
            tokio::io::unix::AsyncFd::register_with_interest(
                input.try_clone_to_owned()?,
                tokio::io::Interest::READABLE,
            )
        }?);
        Ok(self)
    }

    pub fn last(&self) -> Option<nix::sys::signal::Signal> {
        self.last
    }

    pub async fn recv(&mut self) -> nix::sys::signal::Signal {
        let received = tokio::select! {
            _ = self.terminate.recv() => nix::sys::signal::SIGTERM,
            _ = self.interrupt.recv() => nix::sys::signal::SIGINT,
            _ = async { match self.hangup.as_mut() { Some(hangup) => { hangup.recv().await; }, None => std::future::pending().await } } => nix::sys::signal::SIGHUP,
            // The readiness is never cleared, so this resolves again.
            _ = closed(self.closed.as_ref()) => nix::sys::signal::SIGHUP,
        };
        self.last = Some(received);
        received
    }
}

/// Resolve once INPUT is readable with nothing to read; never otherwise.
async fn closed(input: Option<&tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>>) {
    use std::os::fd::AsRawFd;
    if let Some(input) = input
        && input.readable().await.is_ok()
    {
        let mut pending: nix::libc::c_int = 0;
        // SAFETY: FIONREAD writes one int.
        if unsafe { nix::libc::ioctl(input.as_raw_fd(), nix::libc::FIONREAD, &mut pending) } == 0
            && pending == 0
        {
            return;
        }
    }
    std::future::pending().await
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
struct Starting {
    sockets: runtime::sockets::Sockets,
    child: Option<Child>,
    proxy: Option<proxy::Server>,
    audit_done: Option<bool>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    preparing: Option<tokio::task::JoinHandle<Result<(Plan, Prepared), Stop>>>,
    finishing: Option<tokio::task::JoinHandle<io::Result<BoxInfo>>>,
    /// The disks claimed for the box, which a failed start releases.
    claimed: disk::Claimed,
    directories: Vec<disk::Directory>,
    /// The login configuration directories, from the spec's checked logins.
    destinations: Vec<PathBuf>,
}

/// The start after the claim: route files, seeds, proxy, runtime, readiness.
/// The runtime child goes into `child` as soon as it exists, so a failure, or
/// a signal at any await, removes exactly that. Returns the box's labels as
/// the runtime reports them, and the egress log the proxy was given (`None`
/// without `egress`).
#[allow(clippy::too_many_arguments)]
async fn start(
    plan: &mut Plan,
    values: &Values,
    init: &Path,
    image_id: &str,
    seeding: Option<&Seeding>,
    state_dir: &Path,
    codex: Option<login::Token>,
    progress: &mut Starting,
) -> Result<(BoxInfo, Option<PathBuf>), Stop> {
    // Render nonsecret route files only after the claim.
    let mut resolved = plan.clone();
    let preparation_dir = state_dir.to_path_buf();
    let preparation_sockets = progress.sockets.clone();
    let cancel = progress.cancel.clone();
    let image_id = image_id.to_string();
    let generated = progress.destinations.clone();
    let worker = progress
        .preparing
        .insert(tokio::task::spawn_blocking(move || {
            prepare_logins(&mut resolved, &preparation_dir, &generated)?;
            let prepared = runtime::prepare(
                &resolved,
                &preparation_dir,
                &preparation_sockets,
                &image_id,
                &cancel,
                &generated,
            )?;
            Ok((resolved, prepared))
        }));
    let prepared = worker.await;
    progress.preparing.take();
    let (resolved, prepared) = prepared
        .map_err(|error| io::Error::other(format!("preparation worker failed: {error}")))??;
    *plan = resolved;

    // The proxy comes up before the box, so the socket is listening when
    // the runtime forwards it.
    let mut egress_log = None;
    let runtime = runtime();
    let socket = match &plan.egress {
        Some(egress) => {
            let socket = progress.sockets.proxy()?;
            let log = runs::egress_log(&plan.labels[ownership::GENERATION_LABEL])?;
            progress.proxy = Some(proxy::start(
                progress.sockets.bind_proxy()?,
                egress,
                values,
                codex,
                log.clone(),
            )?);
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
        values,
        init,
        socket.as_deref(),
        &progress.claimed.fills,
        seeding,
        &progress.directories,
    )?);
    tokio::select! {
        biased;
        _ = audit_failure(&mut progress.proxy, &mut progress.audit_done) => return Err(Stop::Failed(io::Error::other("audit-log: egress logging failed"))),
        ready = child.ready() => ready.map_err(Stop::NotReady)?,
    }
    let (name, init, seeding) = (plan.name.clone(), init.to_path_buf(), seeding.cloned());
    let cancel = progress.cancel.clone();
    let finishing = progress
        .finishing
        .insert(tokio::task::spawn_blocking(move || {
            crate::core::process::with_cancellation(cancel.clone(), || {
                runtime.finish_start(&name, &init, socket.as_deref(), seeding.as_ref(), &cancel)
            })
        }));
    let actual = finishing.await;
    progress.finishing.take();
    let actual =
        actual.map_err(|error| io::Error::other(format!("startup worker failed: {error}")))??;
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
    let mounts: Vec<serde_json::Value> = plan
        .mounts
        .iter()
        .map(|mount| {
            serde_json::json!({
                "kind": mount.kind,
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

/// Check project structure without resolving credentials or materializing
/// shares, and each `from` variable's presence in `values` when given.
pub(crate) fn validate_project(
    plan: &Plan,
    values: Option<&Values>,
    profile: &Profile,
) -> Result<(), String> {
    plan.validate(None)?;
    if let Some(values) = values {
        plan.check_variables(values)?;
    }
    let destinations = login_destinations(plan)?;
    let extra: Vec<&Path> = destinations.iter().map(PathBuf::as_path).collect();
    validate_generated(plan, &extra)?;
    let mut resolved = plan.clone();
    if let Some(share) = &profile.share {
        resolved
            .mounts
            .push(profile_share(share, &plan.name).map_err(|error| error.to_string())?);
    }
    validate_generated(&resolved, &extra)?;
    if !profile.home.is_empty() {
        home_disk(&resolved, Vec::new()).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Destinations and HOME are checked before any preparation or credential lookup.
fn login_destinations(plan: &Plan) -> Result<Vec<PathBuf>, String> {
    let mut destinations = Vec::new();
    if plan.logins().any(|(_, login)| login.login == "claude") {
        destinations.push(login::CLAUDE_CONFIG_DIR.into());
    }
    if plan.logins().any(|(_, login)| login.is_codex()) {
        destinations.push(login::CODEX_CONFIG_DIR.into());
    }
    if plan.logins().any(|(_, login)| login.is_pi()) {
        let home = home_disk(plan, Vec::new()).map_err(|error| error.to_string())?;
        destinations.push(home.guest.join(home.relative).join(login::PI_LOGIN_DIR));
    }
    Ok(destinations)
}

fn validate_generated(plan: &Plan, extra: &[&Path]) -> Result<(), String> {
    plan.validate_guests(extra)?;
    for destination in extra {
        let destination = crate::core::plan::guest_path(destination);
        for mount in &plan.mounts {
            let guest = crate::core::plan::guest_path(&mount.guest);
            if guest.starts_with(&destination) || destination.starts_with(&guest) {
                return Err(format!(
                    "mount {} conflicts with generated directory {}",
                    guest.display(),
                    destination.display()
                ));
            }
        }
        for disk in &plan.disks {
            let guest = crate::core::plan::guest_path(&disk.guest);
            if guest.starts_with(&destination) {
                return Err(format!(
                    "disk {} conflicts with generated directory {}",
                    guest.display(),
                    destination.display()
                ));
            }
        }
        if disk_under(plan, &destination).is_some_and(|(disk, _)| disk.readonly) {
            return Err(format!(
                "generated directory {} is on a read-only disk",
                destination.display()
            ));
        }
        if extra.iter().any(|other| {
            let other = crate::core::plan::guest_path(other);
            other != destination
                && (other.starts_with(&destination) || destination.starts_with(other))
        }) {
            return Err("generated configuration directories overlap".into());
        }
    }
    Ok(())
}

fn generated_directories(plan: &Plan, destinations: &[PathBuf]) -> Vec<disk::Directory> {
    destinations
        .iter()
        .filter_map(|destination| {
            disk_under(plan, destination).map(|(disk, relative)| disk::Directory {
                disk: crate::core::plan::guest_path(&disk.guest),
                relative,
            })
        })
        .collect()
}

/// The deepest disk whose guest path holds `path`, and `path` below it.
fn disk_under<'a>(plan: &'a Plan, path: &Path) -> Option<(&'a Disk, PathBuf)> {
    plan.disks
        .iter()
        .filter_map(|disk| {
            let guest = crate::core::plan::guest_path(&disk.guest);
            Some((disk, path.strip_prefix(&guest).ok()?.to_path_buf()))
        })
        .max_by_key(|(disk, _)| {
            crate::core::plan::guest_path(&disk.guest)
                .components()
                .count()
        })
}

/// Render route names and provider metadata, never resolved credentials.
fn prepare_logins(plan: &mut Plan, state_dir: &Path, destinations: &[PathBuf]) -> io::Result<()> {
    let logins: Vec<(String, Login)> = plan
        .logins()
        .map(|(route, login)| (route.to_owned(), login.clone()))
        .collect();
    let allow = plan
        .egress
        .as_ref()
        .map(|egress| egress.allow.join(","))
        .unwrap_or_default();
    plan.env
        .entry("PINFOLD_ALLOW".to_owned())
        .or_insert(Env::Exact(allow));
    for destination in destinations {
        let (name, file, contents) = if destination == Path::new(login::CLAUDE_CONFIG_DIR) {
            let (route, _) = logins
                .iter()
                .find(|(_, login)| login.login == "claude")
                .expect("Claude route");
            (
                "claude",
                "managed-settings.json",
                login::claude_config(route),
            )
        } else if destination == Path::new(login::CODEX_CONFIG_DIR) {
            let (route, _) = logins
                .iter()
                .find(|(_, login)| login.is_codex())
                .expect("Codex route");
            ("codex", "config.toml", login::codex_config(route))
        } else {
            let routes = logins
                .iter()
                .filter(|(_, login)| login.is_pi())
                .map(|(route, login)| {
                    Ok((
                        route.as_str(),
                        login.pi_provider().map_err(io::Error::other)?,
                    ))
                })
                .collect::<io::Result<Vec<_>>>()?;
            ("login", "index.ts", login::pi_extension(&routes))
        };
        let dir = state_dir.join(name);
        fs::create_dir(&dir)?;
        fs::write(dir.join(file), contents)?;
        plan.mounts.push(Mount {
            host: dir,
            guest: destination.clone(),
            kind: MountKind::Readonly,
        });
    }
    Ok(())
}

/// Load the spec's profile and fold its image and `share/` mount into the
/// plan without extracting files. Shared files and `$HOME` seeds wait until
/// the name is claimed.
fn resolve_profile(plan: &mut Plan, supplied: Option<Profile>) -> io::Result<Option<Profile>> {
    let Some(name) = plan.profile.clone() else {
        return Ok(None);
    };
    let profile = match supplied {
        Some(profile) => profile,
        None => Profile::load(&name)?,
    };
    if plan.image.is_none() {
        let Some(recipe) = profile.recipe() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "image-source-missing: profile has no Containerfile; select an image",
            ));
        };
        plan.image = Some(recipe.image_ref());
    }
    if let Some(share) = &profile.share {
        plan.mounts.push(profile_share(share, &plan.name)?);
    }
    Ok(Some(profile))
}

/// The read-only mount of a profile's share.
fn profile_share(share: &Share, name: &str) -> io::Result<Mount> {
    Ok(Mount {
        host: share.path(name)?,
        guest: PathBuf::from("/opt/pinfold/profile"),
        kind: MountKind::Readonly,
    })
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
            format!("HOME for profile {name:?} or Pi autoload: {why}"),
        )
    };
    let Some(Env::Exact(home)) = plan.env.get("HOME") else {
        return Err(refuse("the box spec has no exact HOME env entry".into()));
    };
    let home = Path::new(home);
    if !home.is_absolute()
        || home
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(refuse("HOME must be an absolute plain path".into()));
    }
    let Some((disk, relative)) = disk_under(plan, home) else {
        return Err(refuse("HOME is not on a disk".into()));
    };
    if disk.readonly {
        return Err(refuse(format!(
            "HOME is on the read-only disk {}",
            disk.guest.display()
        )));
    }
    // `..` would step out of the disk and `.` is noise.
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(refuse(format!(
            "HOME is not a plain path under {}",
            disk.guest.display()
        )));
    }
    Ok(Seeding {
        guest: crate::core::plan::guest_path(&disk.guest),
        relative: relative.to_path_buf(),
        seeds,
    })
}

async fn take_claim(
    plan: &Plan,
    signals: &mut Signals,
    cancel_start: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<ownership::Claim, UpError> {
    tokio::select! {
        biased;
        _ = signals.recv() => return Err(UpError::Signal),
        _ = cancelled(cancel_start) => return Err(UpError::Control),
        claim = ownership::Claim::take(&plan.name) => claim,
    }
    .map_err(|error| ownership_error(plan, error))?
    .ok_or_else(|| {
        refused(
            plan,
            RefusalReason::NameInUse,
            "name remained busy through the claim deadline".to_string(),
        )
    })
}

async fn startup_work<T: Send + 'static>(
    signals: &mut Signals,
    cancel_start: &mut Option<tokio::sync::watch::Receiver<bool>>,
    mut claim: Option<&mut ownership::Claim>,
    work: impl FnOnce() -> Result<T, UpError> + Send + 'static,
) -> Result<T, UpError> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let mut control = claim
        .as_deref_mut()
        .map(ownership::Claim::take_startup_control);
    let mut worker = tokio::task::spawn_blocking(move || {
        crate::core::process::with_cancellation(worker_cancel, work)
    });
    let (result, stopped) = tokio::select! {
        biased;
        _ = signals.recv() => (Err(UpError::Signal), true),
        _ = cancelled(cancel_start) => (Err(UpError::Control), true),
        stopped = async {
            match control.as_mut() {
                Some(control) => control.stop_requested().await,
                None => std::future::pending().await,
            }
        } => (Err(stopped.map_or_else(UpError::Other, |()| UpError::Control)), true),
        result = &mut worker => (
            result.map_err(|_| io::Error::other("startup worker failed")).map_err(UpError::Other).and_then(|result| result),
            false,
        ),
    };
    if stopped {
        cancel.store(true, std::sync::atomic::Ordering::Release);
        let _ = worker.await;
    }
    if let Some(control) = control {
        let claim = claim.expect("startup control belongs to the prepared claim");
        claim.restore_startup_control(control);
        if result.is_err() {
            claim.finish(true).await?;
        }
    }
    result
}

/// Read each login route's token before the claim, so a missing value or
/// an unusable Codex login refuses as `login` and leaves nothing. codex's
/// token goes to the proxy.
async fn resolve_logins(
    plan: &Plan,
    values: &Values,
    signals: &mut Signals,
    cancel_start: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<Option<login::Token>, UpError> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    // The worker gets only the login routes' values.
    let login_values = values.only(plan.logins().filter_map(|(_, login)| login.source()?.ok()));
    let login_plan = plan.clone();
    let mut resolving = tokio::task::spawn_blocking(move || {
        login_plan.resolve_login(&login_values, Some(&worker_cancel))
    });
    tokio::select! {
        biased;
        _ = cancelled(cancel_start) => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = resolving.await;
            return Err(UpError::Control);
        }
        _ = signals.recv() => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = resolving.await;
            return Err(UpError::Signal);
        }
        result = &mut resolving => result,
    }
    .map_err(|error| io::Error::other(format!("login worker failed: {error}")))?
    .map_err(|error| refused(plan, RefusalReason::Login, error))
}

async fn cancelled(cancel: &mut Option<tokio::sync::watch::Receiver<bool>>) {
    if let Some(cancel) = cancel {
        let _ = cancel.wait_for(|stop| *stop).await;
        return;
    }
    std::future::pending::<()>().await;
}

fn stop_proxy(proxy: &mut Option<proxy::Server>) -> io::Result<()> {
    proxy.as_mut().map_or(Ok(()), proxy::Server::stop)
}

async fn audit_failure(proxy: &mut Option<proxy::Server>, observed: &mut Option<bool>) {
    if observed.is_none()
        && let Some(server) = proxy
    {
        *observed = Some((&mut server.failure).await.is_ok());
    }
    if *observed == Some(true) {
        return;
    }
    std::future::pending::<()>().await;
}

fn ownership_error(plan: &Plan, error: io::Error) -> UpError {
    if runtime::sockets::is_path_error(&error) {
        refused(plan, RefusalReason::RuntimePath, error.to_string())
    } else {
        UpError::Other(error)
    }
}
