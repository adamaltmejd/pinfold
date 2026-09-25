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
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg, OFlag, openat};
use nix::sys::stat::{Mode, mkdirat};
use nix::unistd::{Pid, UnlinkatFlags, unlinkat};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Child;
use tokio::signal::unix::{Signal, SignalKind, signal};

use crate::core::clean;
use crate::core::plan::{Env, HARNESS_PI, Mount, Plan};
use crate::core::profile::{Profile, Seed};
use crate::core::runtime::{Preflight, runtime};
use crate::core::{artifacts, proxy};
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
    state_dir: PathBuf,
    /// The lock on the state dir's `pid` file. Holding it is what makes the
    /// owner alive to every checker.
    _lock: Flock<File>,
    child: Child,
}

/// What stopped the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    /// The owner's stdin reached EOF.
    StdinEof,
    /// SIGTERM or SIGINT arrived.
    Signal,
    /// The attached `container run` process exited on its own.
    BoxExited(ExitStatus),
}

impl Shutdown {
    /// The reason string of the process interface's `down` line.
    pub fn as_str(&self) -> &'static str {
        match self {
            Shutdown::StdinEof => "stdin-closed",
            Shutdown::Signal => "signal",
            Shutdown::BoxExited(_) => "exited",
        }
    }
}

/// Why `up` refused. A refused `up` leaves nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// The spec did not parse or validate.
    Spec,
    /// The profile is missing or cannot be applied.
    Profile,
    /// The runtime refused the host, or its binary is missing.
    Runtime,
    /// The image is not present locally.
    ImageMissing,
    /// The box name is already in use.
    NameInUse,
}

impl RefusalReason {
    /// The reason string of the process interface's `refused` line.
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalReason::Spec => "spec",
            RefusalReason::Profile => "profile",
            RefusalReason::Runtime => "runtime",
            RefusalReason::ImageMissing => "image-missing",
            RefusalReason::NameInUse => "name-in-use",
        }
    }
}

/// A refused `up`: why, and the detail the runtime or host gave. `box_name`
/// is `None` only when the spec did not parse.
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
                io::Error::other(format!("{}: {}", refusal.reason.as_str(), refusal.detail))
            }
            UpError::Signal => io::ErrorKind::Interrupted.into(),
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
    /// With `signals`, the caller registered SIGTERM and SIGINT before
    /// reading the spec: before ready they remove what the start made and
    /// `up` returns [`UpError::Signal`]; after ready [`Box::hold`] watches
    /// them. Without, the caller handles its own.
    pub async fn up(
        plan: &Plan,
        init: &Path,
        signals: Option<&mut Signals>,
    ) -> Result<Box, UpError> {
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

        // The profile's image, share and home seeds are the box's to apply;
        // the runtime sees the resolved plan. Resolving writes nothing; the
        // seeds wait until the name is claimed.
        let mut plan = plan.clone();
        let profile = resolve_profile(&mut plan)
            .map_err(|error| refused(&plan, RefusalReason::Profile, error.to_string()))?;

        // Pinfold adds the profile's `share/` above and the harness mount in
        // `start`. Check the spec's mounts against both before `home_mount`
        // picks the mount behind `$HOME`, so a clash is refused as spec and
        // no tie can hide it.
        let harness =
            (plan.harness.as_deref() == Some(HARNESS_PI)).then(|| Path::new(artifacts::GUEST_PI));
        plan.validate_guests(harness)
            .map_err(|error| refused(&plan, RefusalReason::Spec, error))?;
        let seeding = profile
            .map(|profile| resolve_seeding(&plan, profile))
            .transpose()
            .map_err(|error| refused(&plan, RefusalReason::Profile, error.to_string()))?;

        let runtime = runtime();
        let preflight = runtime
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

        let (state_dir, lock) = claim(&plan)?;
        match runtime.list() {
            Ok(boxes) if boxes.iter().all(|box_| box_.id != plan.name) => {}
            listed => {
                abort(&plan.name, &state_dir, Parts::default()).await;
                return Err(match listed {
                    Ok(_) => refused(
                        &plan,
                        RefusalReason::NameInUse,
                        format!("the runtime already has a box named {:?}", plan.name),
                    ),
                    Err(error) => error.into(),
                });
            }
        }

        // Dropping a start part-way tears nothing down, so a signal ends it
        // here and `parts` says what to remove.
        let mut parts = Parts::default();
        let starting = start(
            &mut plan,
            init,
            &preflight,
            seeding.as_ref(),
            &state_dir,
            &mut parts,
        );
        let started = match signals {
            Some(signals) => tokio::select! {
                biased;
                _ = signals.recv() => Err(Stop::Signal),
                started = starting => started,
            },
            None => starting.await,
        };
        let labels = match started {
            Ok(labels) => labels,
            Err(stop) => {
                let status = abort(&plan.name, &state_dir, parts).await;
                return Err(match stop {
                    Stop::Signal => UpError::Signal,
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
            state_dir,
            _lock: lock,
            child: parts
                .child
                .take()
                .expect("a started box has a runtime child"),
        })
    }

    /// Wait until the box exits, stdin closes, or a termination signal
    /// arrives, then stop and remove the box.
    pub async fn hold(&mut self, signals: &mut Signals) -> io::Result<Shutdown> {
        let reason = tokio::select! {
            reason = wait_for_shutdown(signals) => reason,
            status = self.child.wait() => status.map(Shutdown::BoxExited),
        };
        // Tear down whatever ended the wait, so a failed wait leaves no box.
        let down = self.down().await;
        let reason = reason?;
        down?;
        Ok(reason)
    }

    /// Stop and remove the box, then delete its state directory.
    pub async fn down(&mut self) -> io::Result<()> {
        let result = runtime().down(&self.name);
        let _ = self.child.wait().await;
        let _ = tokio::fs::remove_dir_all(&self.state_dir).await;
        result
    }
}

/// Take box `name` down from outside its owner: signal the owning `box up`
/// through the state dir and wait for it to remove the box. The pid is
/// signalled only while its lock is held, so a reused pid is never hit. A
/// dead owner means remove the leftover directly.
pub fn down(name: &str) -> io::Result<()> {
    let state = dirs::box_state_dir(name)?;
    if let Some(pid) = clean::owner_pid(&state)
        && clean::owner_alive(&state)
    {
        nix::sys::signal::kill(Pid::from_raw(pid), nix::sys::signal::SIGTERM)
            .map_err(io::Error::other)?;
        for _ in 0..1000 {
            if !state.exists() {
                return Ok(());
            }
            if !clean::owner_alive(&state) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    // No box can hold the empty name, and podman refuses it ("name or ID
    // cannot be empty"), so an empty name is an absent box and the runtime
    // is never asked.
    if !name.is_empty() {
        runtime().down(name)?;
    }
    let _ = fs::remove_dir_all(&state);
    Ok(())
}

/// SIGTERM and SIGINT, handled by `box up` from before its spec is read to
/// its exit, and by `pinfold pi` beside SIGHUP.
pub struct Signals {
    terminate: Signal,
    interrupt: Signal,
}

impl Signals {
    pub fn new() -> io::Result<Signals> {
        Ok(Signals {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    pub async fn recv(&mut self) -> nix::sys::signal::Signal {
        tokio::select! {
            _ = self.terminate.recv() => nix::sys::signal::SIGTERM,
            _ = self.interrupt.recv() => nix::sys::signal::SIGINT,
        }
    }
}

/// Why a start ended before ready.
enum Stop {
    Signal,
    /// Readiness failed; the runtime child's exit status completes the text.
    NotReady(String),
    Failed(io::Error),
}

impl From<io::Error> for Stop {
    fn from(error: io::Error) -> Stop {
        Stop::Failed(error)
    }
}

/// What a start has made so far besides the claimed state dir.
#[derive(Default)]
struct Parts {
    child: Option<Child>,
}

/// Remove what a failed start left: stop the runtime child, take the box
/// down, then remove the claimed state dir. The box goes down first, so no
/// checker sees a box whose state dir is gone.
async fn abort(name: &str, state_dir: &Path, parts: Parts) -> Option<io::Result<ExitStatus>> {
    let status = match parts.child {
        Some(mut child) => {
            // The client goes first, so one still creating the box cannot
            // finish after the removal. The box exists by name even when
            // readiness failed.
            let _ = child.start_kill();
            let status = child.wait().await;
            let _ = runtime().down(name);
            Some(status)
        }
        None => None,
    };
    let _ = fs::remove_dir_all(state_dir);
    status
}

/// Claim `plan`'s name: create its state dir exclusively, then lock and
/// write the `pid` file. A dir whose owner is alive is `name-in-use`; a dead
/// owner's dir is removed and the claim tried once more.
fn claim(plan: &Plan) -> Result<(PathBuf, Flock<File>), UpError> {
    fs::create_dir_all(dirs::boxes_dir()?)?;
    let state_dir = dirs::box_state_dir(&plan.name)?;
    for retry in [true, false] {
        match fs::create_dir(&state_dir) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        if !retry || clean::owner_alive(&state_dir) {
            return Err(refused(
                plan,
                RefusalReason::NameInUse,
                format!("state dir {} is held by a live owner", state_dir.display()),
            ));
        }
        if let Err(error) = fs::remove_dir_all(&state_dir)
            && error.kind() != io::ErrorKind::NotFound
        {
            return Err(error.into());
        }
    }
    match lock_pid(&state_dir) {
        Ok(lock) => Ok((state_dir, lock)),
        Err(error) => {
            let _ = fs::remove_dir_all(&state_dir);
            Err(error.into())
        }
    }
}

/// Create the claimed dir's `pid` file, lock it, and write this process's
/// pid. The lock is held until the process exits or the box is torn down.
fn lock_pid(state_dir: &Path) -> io::Result<Flock<File>> {
    // std opens with O_CLOEXEC, so no runtime child inherits the lock and
    // holds it past this process.
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(state_dir.join("pid"))?;
    // Blocking: a checker holds the lock only for a moment.
    let mut lock =
        Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, errno)| io::Error::from(errno))?;
    lock.write_all(std::process::id().to_string().as_bytes())?;
    Ok(lock)
}

/// The start after the claim: harness, seeds, proxy, runtime, readiness.
/// What it creates goes into `parts` as soon as it exists, so a failure, or
/// a signal at any await, removes exactly that. Returns the box's labels as
/// the runtime reports them.
async fn start(
    plan: &mut Plan,
    init: &Path,
    preflight: &Preflight,
    seeding: Option<&Seeding>,
    state_dir: &Path,
    parts: &mut Parts,
) -> Result<BTreeMap<String, String>, Stop> {
    // The harness artifact is fetched and folded in after the claim, so a
    // refused box downloads nothing. The spec's own env wins.
    resolve_harness(plan)?;

    if let Some(seeding) = seeding {
        seed_home(seeding)?;
    }

    // The proxy comes up before the box, so the socket is listening when
    // the runtime forwards it.
    let socket = match &plan.egress {
        Some(egress) => {
            let socket = state_dir.join("proxy.sock");
            let log = dirs::egress_dir()?.join(format!("{}.jsonl", plan.name));
            proxy::start(&socket, egress, log)?;
            Some(socket)
        }
        None => None,
    };

    // A signal that arrived during the steps above wins the select here,
    // before the runtime is asked to create anything.
    tokio::task::yield_now().await;
    let runtime = runtime();
    let child = parts
        .child
        .insert(runtime.up(plan, init, socket.as_deref(), preflight)?);
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("runtime up did not pipe the box's stdout"))?;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) if line == "ready" => break,
            Ok(Some(_)) => {}
            Ok(None) => return Err(Stop::NotReady("box exited before ready".to_string())),
            Err(error) => return Err(Stop::NotReady(format!("box output failed: {error}"))),
        }
    }
    // Apple only: the forwarded socket arrives root-owned and mode 000.
    // The one root exec happens before ready reaches the caller, so no
    // work can race it.
    if socket.is_some() {
        runtime.make_proxy_connectable(&plan.name)?;
    }
    // Keep the pipe drained so a talkative box cannot block on it.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    // `ready` reports the labels `list` will: podman adds every image label
    // to the box, so the plan's set is not the box's.
    let labels = runtime
        .list()?
        .into_iter()
        .find(|box_| box_.id == plan.name)
        .map(|box_| box_.labels)
        .ok_or_else(|| {
            io::Error::other(format!(
                "the runtime does not list box {:?} after ready",
                plan.name
            ))
        })?;
    Ok(labels)
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

/// Fold the spec's harness into the plan: fetch the pinned artifact if it is
/// not cached, mount it read-only, and set its environment. The spec's own
/// env wins over the harness defaults.
fn resolve_harness(plan: &mut Plan) -> io::Result<()> {
    if plan.harness.as_deref() != Some(HARNESS_PI) {
        return Ok(());
    }
    plan.mounts.push(Mount {
        host: artifacts::pi()?,
        guest: PathBuf::from(artifacts::GUEST_PI),
        readonly: true,
    });
    plan.env
        .entry("PI_TELEMETRY".to_string())
        .or_insert(Env::Exact("0".to_string()));
    plan.env
        .entry("PI_SKIP_VERSION_CHECK".to_string())
        .or_insert(Env::Exact("1".to_string()));
    let allow = plan
        .egress
        .as_ref()
        .map(|egress| egress.allow.join(","))
        .unwrap_or_default();
    plan.env
        .entry("PINFOLD_ALLOW".to_string())
        .or_insert(Env::Exact(allow));
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

/// A loaded profile's name and `home/` seeds, before the mount behind
/// `$HOME` is chosen from the final mount list.
struct ResolvedProfile {
    name: String,
    seeds: Vec<Seed>,
}

/// Load the spec's profile and fold its image and `share/` mount into the
/// plan. The `$HOME` seeds wait until the name is claimed. Returns the
/// seeds when the profile has `home/`.
fn resolve_profile(plan: &mut Plan) -> io::Result<Option<ResolvedProfile>> {
    let Some(name) = plan.profile.clone() else {
        return Ok(None);
    };
    let profile = Profile::load(&name)?;
    if plan.image.is_none() {
        plan.image = Some(profile.image_ref());
    }
    if let Some(share) = &profile.share {
        plan.mounts.push(Mount {
            host: share.clone(),
            guest: PathBuf::from("/opt/pinfold/profile"),
            readonly: true,
        });
    }
    if profile.home.is_empty() {
        return Ok(None);
    }
    Ok(Some(ResolvedProfile {
        name,
        seeds: profile.home,
    }))
}

/// The writable mount behind `$HOME` and the seeds to copy into it. Called
/// after the duplicate-guest check, so no two mounts can hold `$HOME`.
fn resolve_seeding(plan: &Plan, profile: ResolvedProfile) -> io::Result<Seeding> {
    let (mount, relative) = home_mount(plan, &profile.name)?;
    Ok(Seeding {
        mount,
        relative,
        seeds: profile.seeds,
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

/// The mount behind the spec's exact `$HOME` and the plain path from its
/// guest root to `$HOME`, for profile `name`'s seeds. The mount must be
/// writable and the path must not step out of it.
fn home_mount(plan: &Plan, name: &str) -> io::Result<(Mount, PathBuf)> {
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
    Ok((mount.clone(), relative.to_path_buf()))
}

/// Copy seeds into the host `$HOME`, skipping anything already there. The
/// walk starts at the mount's host directory and follows no symlink: the box
/// can write the mount, so a planted symlink must not redirect a seed
/// outside it.
fn seed_home(seeding: &Seeding) -> io::Result<()> {
    let host = &seeding.mount.host;
    let root = open_mount_dir(host)?;
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

/// Open the mount's host directory. The spec names it and the runtime mounts
/// it, so it is the seed walk's trust root.
fn open_mount_dir(host: &Path) -> io::Result<OwnedFd> {
    nix::fcntl::open(host, OFlag::O_DIRECTORY | OFlag::O_CLOEXEC, Mode::empty())
        .map_err(|error| seed_error(error, &format!("open mount {}", host.display())))
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
