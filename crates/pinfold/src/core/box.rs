//! The box lifecycle: one attached `container run` process owns one box.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
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
use tokio::signal::unix::{SignalKind, signal};

use crate::core::clean;
use crate::core::plan::{Env, Mount, Plan};
use crate::core::profile::{Profile, Seed};
use crate::core::proxy::Proxy;
use crate::core::runtime::{Runtime, runtime};
use crate::dirs;

/// A started box, owned by this process.
pub struct Box {
    name: String,
    /// The box's full label set: the spec's, the owner's, and the image's
    /// identity labels.
    pub labels: BTreeMap<String, String>,
    state_dir: PathBuf,
    child: Child,
    runtime: &'static dyn Runtime,
    proxy: Option<Proxy>,
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

/// Why `up` refused before it created anything.
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

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.reason.as_str(), self.detail)
    }
}

impl std::error::Error for Refusal {}

/// Why [`Box::up`] failed: a refusal decided before anything was created, or
/// an error after creation began.
#[derive(Debug)]
pub enum UpError {
    Refused(Refusal),
    Other(io::Error),
}

impl fmt::Display for UpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpError::Refused(refusal) => refusal.fmt(formatter),
            UpError::Other(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for UpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UpError::Refused(_) => None,
            UpError::Other(error) => Some(error),
        }
    }
}

impl From<io::Error> for UpError {
    fn from(error: io::Error) -> UpError {
        UpError::Other(error)
    }
}

impl From<UpError> for io::Error {
    fn from(error: UpError) -> io::Error {
        match error {
            UpError::Refused(refusal) => io::Error::other(refusal),
            UpError::Other(error) => error,
        }
    }
}

impl Box {
    /// Start a box, wait for init's `ready` line, and return it. Every
    /// refusal is decided before anything is created: the profile, the
    /// host's runtime, the image and the name are checked first, and home
    /// seeds are written only after all of them pass.
    pub async fn up(plan: &Plan, init: &Path) -> Result<Box, UpError> {
        if !init.is_absolute() {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "init path must be absolute").into(),
            );
        }
        if init.parent().is_none_or(|parent| parent == Path::new("/")) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "init must live in a directory below /",
            )
            .into());
        }

        // The profile's image, share and home seeds are the box's to apply;
        // the runtime sees the resolved plan. Resolving writes nothing; the
        // seeds wait until every refusal has been decided.
        let mut plan = plan.clone();
        let profile = resolve_profile(&mut plan)
            .map_err(|error| refused(&plan, RefusalReason::Profile, error.to_string()))?;

        let runtime =
            runtime().map_err(|error| refused(&plan, RefusalReason::Runtime, error.to_string()))?;
        runtime
            .preflight()
            .map_err(|error| refused(&plan, RefusalReason::Runtime, error.to_string()))?;

        let image = plan.image.clone().ok_or_else(|| {
            refused(
                &plan,
                RefusalReason::Spec,
                "a box spec needs an image or a profile",
            )
        })?;
        let Some(image_info) = runtime
            .list_images()?
            .into_iter()
            .find(|info| info.reference == image)
        else {
            return Err(refused(
                &plan,
                RefusalReason::ImageMissing,
                format!("image {image:?} is not present locally; build or pull it first"),
            ));
        };
        // The image's identity labels are the box's too, so a caller reading
        // `ready` gets them without a second `list`. The spec's labels win.
        for (key, value) in image_info.labels {
            if key.starts_with("dev.pinfold.") {
                plan.labels.entry(key).or_insert(value);
            }
        }
        if let Some(detail) = name_in_use(runtime, &plan.name)? {
            return Err(refused(&plan, RefusalReason::NameInUse, detail));
        }

        // Home seeds are written only after the checks pass, just before the
        // runtime starts the box.
        if let Some(profile) = &profile
            && let Some((mount, relative)) = &profile.seed
        {
            seed_home(mount, relative, &profile.home)?;
        }

        let state_dir = dirs::state_dir()?.join("boxes").join(&plan.name);
        let log = match &plan.egress {
            Some(_) => Some(dirs::egress_dir()?.join(format!("{}.jsonl", plan.name))),
            None => None,
        };
        tokio::fs::create_dir_all(&state_dir).await?;
        tokio::fs::write(state_dir.join("pid"), std::process::id().to_string()).await?;

        // The proxy comes up before the box, so the socket is listening when
        // the runtime forwards it.
        let proxy = match (&plan.egress, log) {
            (Some(egress), Some(log)) => {
                match Proxy::start(state_dir.join("proxy.sock"), egress, log) {
                    Ok(proxy) => Some(proxy),
                    Err(error) => {
                        let _ = tokio::fs::remove_dir_all(&state_dir).await;
                        return Err(error.into());
                    }
                }
            }
            _ => None,
        };

        let mut child = match runtime.up(&plan, init, proxy.as_ref().map(Proxy::socket)) {
            Ok(child) => child,
            Err(error) => {
                if let Some(proxy) = proxy {
                    proxy.close();
                }
                let _ = tokio::fs::remove_dir_all(&state_dir).await;
                return Err(error.into());
            }
        };
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("runtime up did not pipe the box's stdout"))?;
        let mut lines = BufReader::new(stdout).lines();
        let failure = loop {
            match lines.next_line().await {
                Ok(Some(line)) if line == "ready" => break None,
                Ok(Some(_)) => {}
                Ok(None) => break Some("box exited before ready".to_string()),
                Err(error) => break Some(format!("box output failed: {error}")),
            }
        };
        if let Some(failure) = failure {
            // The container exists by name even when readiness failed; remove
            // it before the state dir that names it.
            let _ = runtime.down(&plan.name);
            let _ = child.start_kill();
            let status = child.wait().await;
            if let Some(proxy) = proxy {
                proxy.close();
            }
            let _ = tokio::fs::remove_dir_all(&state_dir).await;
            return Err(io::Error::other(match status {
                Ok(status) => format!("{failure}: {status}"),
                Err(error) => format!("{failure}: {error}"),
            })
            .into());
        }
        // Apple only: the forwarded socket arrives root-owned and mode 000.
        // The one root exec happens before ready reaches the caller, so no
        // work can race it.
        if proxy.is_some()
            && let Err(error) = runtime.make_proxy_connectable(&plan.name)
        {
            let _ = runtime.down(&plan.name);
            let _ = child.start_kill();
            let _ = child.wait().await;
            if let Some(proxy) = proxy {
                proxy.close();
            }
            let _ = tokio::fs::remove_dir_all(&state_dir).await;
            return Err(error.into());
        }
        // Keep the pipe drained so a talkative box cannot block on it.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

        Ok(Box {
            name: plan.name.clone(),
            labels: plan.labels,
            state_dir,
            child,
            runtime,
            proxy,
        })
    }

    /// Wait until the box exits, stdin closes, or a termination signal
    /// arrives, then stop and remove the box.
    pub async fn hold(&mut self) -> io::Result<Shutdown> {
        let reason = tokio::select! {
            reason = wait_for_shutdown() => reason?,
            status = self.child.wait() => Shutdown::BoxExited(status?),
        };
        self.down().await?;
        Ok(reason)
    }

    /// Stop and remove the box, then delete its state directory.
    pub async fn down(&mut self) -> io::Result<()> {
        let result = self.runtime.down(&self.name);
        let _ = self.child.wait().await;
        if let Some(proxy) = self.proxy.take() {
            proxy.close();
        }
        let _ = tokio::fs::remove_dir_all(&self.state_dir).await;
        result
    }
}

async fn wait_for_shutdown() -> io::Result<Shutdown> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut stdin = tokio::io::stdin();
    let mut buffer = [0u8; 4096];
    loop {
        tokio::select! {
            _ = terminate.recv() => return Ok(Shutdown::Signal),
            _ = interrupt.recv() => return Ok(Shutdown::Signal),
            read = stdin.read(&mut buffer) => {
                if read? == 0 {
                    return Ok(Shutdown::StdinEof);
                }
            },
        }
    }
}

/// A profile resolved before the refusal checks: its `home/` seeds and,
/// when it has any, the writable mount behind `$HOME` to copy them into.
struct ResolvedProfile {
    home: Vec<Seed>,
    seed: Option<(Mount, PathBuf)>,
}

/// Load the spec's profile and fold its image and `share/` mount into the
/// plan. The seed target is computed here but nothing is written, so every
/// refusal that depends on the profile is decided before creation.
fn resolve_profile(plan: &mut Plan) -> io::Result<Option<ResolvedProfile>> {
    let Some(name) = plan.profile.clone() else {
        return Ok(None);
    };
    let profile = Profile::load(&name)?;
    if plan.image.is_none() {
        plan.image = Some(format!("pinfold/profile-{name}:latest"));
    }
    if let Some(share) = &profile.share {
        plan.mounts.push(Mount {
            host: share.clone(),
            guest: PathBuf::from("/opt/pinfold/profile"),
            readonly: true,
        });
    }
    let seed = if profile.home.is_empty() {
        None
    } else {
        let home = exact_home(plan, &name)?;
        let (mount, relative) = home_mount(plan, &name, &home)?;
        Some((mount.clone(), relative))
    };
    Ok(Some(ResolvedProfile {
        home: profile.home,
        seed,
    }))
}

/// The spec's exact `$HOME` entry, when a profile seeds it.
fn exact_home(plan: &Plan, name: &str) -> io::Result<PathBuf> {
    plan.env
        .get("HOME")
        .and_then(|value| match value {
            Env::Exact(value) => Some(PathBuf::from(value)),
            Env::From { .. } => None,
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "profile {name:?} seeds $HOME, but the box spec has no exact HOME env entry"
                ),
            )
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

/// Whether a box already holds `name`: the runtime lists it, or its state
/// dir holds a live owner pid. The detail names which.
fn name_in_use(runtime: &dyn Runtime, name: &str) -> io::Result<Option<String>> {
    if runtime.list()?.iter().any(|box_| box_.id == name) {
        return Ok(Some(format!(
            "the runtime already has a box named {name:?}"
        )));
    }
    let state = dirs::state_dir()?.join("boxes").join(name);
    let pid = fs::read_to_string(state.join("pid"))
        .ok()
        .and_then(|pid| pid.trim().parse::<i32>().ok());
    if let Some(pid) = pid
        && clean::alive(pid)
    {
        return Ok(Some(format!(
            "state dir {} is held by live pid {pid}",
            state.display()
        )));
    }
    Ok(None)
}

/// The mount behind a guest `$HOME` and the plain path from its guest root
/// to `$HOME`. The mount must be writable and the path must not step out of
/// it.
fn home_mount<'a>(plan: &'a Plan, name: &str, home: &Path) -> io::Result<(&'a Mount, PathBuf)> {
    let mut best: Option<(&'a Mount, &Path)> = None;
    for mount in &plan.mounts {
        let Ok(relative) = home.strip_prefix(&mount.guest) else {
            continue;
        };
        let deeper = best.as_ref().is_none_or(|(best, _)| {
            mount.guest.components().count() > best.guest.components().count()
        });
        if deeper {
            best = Some((mount, relative));
        }
    }
    let Some((mount, relative)) = best else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "profile {name:?} seeds $HOME, but $HOME={} is not on a mount",
                home.display()
            ),
        ));
    };
    if mount.readonly {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "profile {name:?} seeds $HOME, but $HOME={} is on the read-only mount {}",
                home.display(),
                mount.guest.display()
            ),
        ));
    }
    // The walk below handles normal components only; `..` would step out of
    // the mount and `.` is noise.
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "profile {name:?} seeds $HOME, but $HOME={} is not a plain path under the mount {}",
                home.display(),
                mount.guest.display()
            ),
        ));
    }
    Ok((mount, relative.to_path_buf()))
}

/// Copy seeds into the host `$HOME`, skipping anything already there. The
/// walk starts at the mount's host directory and follows no symlink: the box
/// can write the mount, so a planted symlink must not redirect a seed
/// outside it.
fn seed_home(mount: &Mount, relative: &Path, seeds: &[Seed]) -> io::Result<()> {
    let root = open_mount_dir(&mount.host)?;
    let (home, home_path) = open_seed_path(&root, relative, &mount.host)?;
    for seed in seeds {
        seed_file(&home, &home_path, &seed.path, &seed.contents)?;
    }
    Ok(())
}

/// Open the mount's host directory. The spec names it and the runtime mounts
/// it, so it is the seed walk's trust root.
fn open_mount_dir(host: &Path) -> io::Result<OwnedFd> {
    nix::fcntl::open(host, OFlag::O_DIRECTORY | OFlag::O_CLOEXEC, Mode::empty())
        .map_err(|error| seed_error(error, &format!("open mount {}", host.display())))
}

/// Walk `relative` from `dir`, creating missing directories with `mkdirat`
/// and refusing a symlink at any component. `display` names `dir` in errors.
fn open_seed_path(
    dir: &OwnedFd,
    relative: &Path,
    display: &Path,
) -> io::Result<(OwnedFd, PathBuf)> {
    let mut dir = dir.try_clone()?;
    let mut current = display.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "seed path {} is not a plain directory path",
                    relative.display()
                ),
            ));
        };
        current.push(name);
        dir = open_seed_dir(&dir, name, &current)?;
    }
    Ok((dir, current))
}

/// Write one seed with `openat` and `mkdirat`, following no symlink. Anything
/// already at the destination, a dangling symlink included, is left alone.
fn seed_file(root: &OwnedFd, home: &Path, relative: &Path, contents: &[u8]) -> io::Result<()> {
    let mut dir = root.try_clone()?;
    let mut current = home.to_path_buf();
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
