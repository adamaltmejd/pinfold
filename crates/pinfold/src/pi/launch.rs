//! Assemble one project's box and run a command in it.

use std::collections::BTreeMap;
use std::env;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::runtime::Builder;

use crate::config::{Config, ProjectMount};
use crate::core::r#box::{Box, Shutdown, Signals, UpError};
use crate::core::clean;
use crate::core::crossing::CrossingError;
use crate::core::image::{ImageInfo, service};
use crate::core::plan::{Disk, Egress, Env, Mount, MountKind, Plan};
use crate::core::runtime::Runtime;
use crate::core::{artifacts, disk, image, sha256_hex};
use crate::formatting::terminal_text;
use crate::pi::context::Context;
use crate::pi::state::{self, project_id};

/// Refuse a root without `.git`, which has no branches to cross, and one
/// whose `.git` is not a directory (a worktree's or a submodule's).
pub(crate) fn require_repo(root: &Path) -> io::Result<()> {
    let dot_git = root.join(".git");
    let refused = |detail: String| Err(io::Error::new(io::ErrorKind::InvalidInput, detail));
    match std::fs::symlink_metadata(&dot_git) {
        Ok(metadata) if !metadata.is_dir() => refused(format!(
            "worktree: {} is not a directory; run pinfold from the main checkout",
            terminal_text(&dot_git.to_string_lossy())
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => refused(format!(
            "not-git: {} is not a git repository; run `git init` and commit",
            terminal_text(&root.to_string_lossy())
        )),
        Err(error) => Err(error),
        Ok(_) => Ok(()),
    }
}

/// One crossing for the Git remote helper: find the disk project at the
/// current directory, let CHECK refuse its root, then run WORK against the
/// box that attaches its project disk, or a transient box on that disk,
/// without egress, when none does. WORK gets the runtime, init, the box's
/// name, the root, its guest path and a flag that a signal, the box's end
/// or Git closing GIT, the helper's input, sets.
pub(crate) fn cross<T, W>(
    check: impl FnOnce(&Path) -> io::Result<()>,
    git: BorrowedFd<'_>,
    work: W,
) -> io::Result<Result<T, CrossingError>>
where
    T: Send + 'static,
    W: FnOnce(
            &'static dyn Runtime,
            &Path,
            &str,
            &Path,
            &str,
            &AtomicBool,
        ) -> Result<T, CrossingError>
        + Send
        + 'static,
{
    let runtime = Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let mut signals = Signals::new()?;
        let interrupted = |signal: nix::sys::signal::Signal| {
            io::Error::new(io::ErrorKind::Interrupted, format!("interrupted by {signal}"))
        };
        let cwd = state::canonical(&env::current_dir()?)?;
        crate::pi::session::refuse_covering_share(&cwd, &mut signals).await?;
        let context = Context::candidate(&cwd)?;
        let root = context.root.clone();
        check(&root)?;
        let mut lease = match crate::pi::session::borrow(&root, &mut signals).await {
            Ok(lease) => lease,
            Err(error) => return Err(signals.last().map_or(error, interrupted)),
        };
        let result = async {
            // The owner's root probe precedes reading a different configuration root.
            let mut config = Config::load(&root)?;
            // Shares never grant crossing leases; a live lease is a disk owner.
            if lease.name().is_some() {
                config.mount = ProjectMount::Disk;
            }
            context.check_worktree(config.mount)?;
            config.check_host_paths(&root)?;
            // Admission precedes automatic maintenance; the lease still
            // holds shared project ownership.
            clean::maintain();
            let id = project_id(&root);
            let [_, disk] = state::disk_names(&id);
            utf8(&root, "project root")?;
            let guest = state::PROJECT.to_string();
            let box_runtime = crate::core::runtime::runtime();
            let init = artifacts::init_path()?;
            let (name, mut box_) = if let Some(name) = lease.name().map(str::to_string) {
                lease.release_launch();
                (name, None)
            } else {
                if !box_runtime
                    .list_disks()?
                    .iter()
                    .any(|listed| listed.name == disk)
                    || disk::unfilled(&disk)?
                {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "this project has no project disk yet; start it with `pinfold -- CMD`",
                    ));
                }
                let image = resolve_image(&config, &id);
                for warning in image_warnings(&config, &image)? {
                    eprintln!("{warning}");
                }
                let plan = Plan {
                    disks: vec![empty_writable_disk(disk, Path::new(state::PROJECT))],
                    ..box_plan(&config, &id, &image)?
                };
                let box_ = match Box::up(&plan, &mut signals).await {
                    Ok(box_) => box_,
                    Err(UpError::Control) => {
                        return Ok(Err(CrossingError::Failed(
                            "the transient box was stopped before ready".to_string(),
                        )));
                    }
                    Err(UpError::Signal) => {
                        return Err(interrupted(
                            signals
                                .last()
                                .unwrap_or(nix::sys::signal::Signal::SIGTERM),
                        ));
                    }
                    Err(UpError::Refused(refusal)) => {
                        return Ok(Err(CrossingError::Refused(refusal.reason, refusal.detail)));
                    }
                    Err(error) => return Err(error.into()),
                };
                (plan.name, Some(box_))
            };
            if box_.is_none() {
                artifacts::init()?;
            }
            let cancel = std::sync::Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let target = name.clone();
            let mut worker = tokio::task::spawn_blocking(move || {
                work(box_runtime, &init, &target, &root, &guest, &worker_cancel)
            });
            let (crossed, terminal) = tokio::select! {
                biased;
                crossed = &mut worker => (
                    crossed.map_err(|error| io::Error::other(format!("crossing failed: {error}"))),
                    Some(Shutdown::Signal.terminal()),
                ),
                ended = async {
                    match box_.as_mut() {
                        Some(box_) => box_.ended().await,
                        None => {
                            let _ = lease.ended().await;
                            Ok(Shutdown::Signal)
                        },
                    }
                } => {
                    cancel.store(true, Ordering::Release);
                    let _ = worker.await;
                    let generation = box_.as_ref().map_or(name.as_str(), |box_| box_.run_id.as_str());
                    match ended {
                        Ok(ended) => (
                            Err(io::Error::other(format!("the box ended during the transfer ({generation})"))),
                            Some(ended.terminal()),
                        ),
                        Err(error) => (
                            Err(io::Error::other(format!(
                                "{} ({generation}); pinfold box inspect {generation}",
                                terminal_text(&error.to_string())
                            ))),
                            None,
                        ),
                    }
                },
                signal = signals.recv() => {
                    cancel.store(true, Ordering::Release);
                    let _ = worker.await;
                    (Err(interrupted(signal)), Some(Shutdown::Signal.terminal()))
                },
                // Cancelling closes the box command's connection, so the
                // box stops it as a departed client's.
                Ok(()) = git_gone(git) => {
                    cancel.store(true, Ordering::Release);
                    let _ = worker.await;
                    (
                        Err(io::Error::new(io::ErrorKind::Interrupted, "Git ended")),
                        Some(Shutdown::Signal.terminal()),
                    )
                },
            };
            let down = match box_.as_mut() {
                Some(box_) => box_.down(terminal).await,
                None => Ok(()),
            };
            let crossed = crossed?;
            down?;
            Ok(crossed)
        }
        .await;
        let left = lease.leave(&mut signals).await;
        match left {
            Ok(()) => result,
            Err(error) if matches!(result, Ok(Ok(_))) => Err(error),
            Err(error) => {
                eprintln!("pinfold: {}", terminal_text(&error.to_string()));
                result
            }
        }
    })
}

/// Resolve once Git has closed GIT: readable with nothing to read. Git
/// sends nothing while it waits on a crossing, so input that does arrive
/// ends the watch unresolved.
async fn git_gone(git: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: the owned duplicate keeps the registered descriptor stable.
    let git = unsafe {
        tokio::io::unix::AsyncFd::register_with_interest(
            git.try_clone_to_owned()?,
            tokio::io::Interest::READABLE,
        )
    }?;
    let _ready = git.readable().await?;
    let mut pending: nix::libc::c_int = 0;
    // SAFETY: FIONREAD writes one int.
    if unsafe { nix::libc::ioctl(git.as_raw_fd(), nix::libc::FIONREAD, &mut pending) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if pending == 0 {
        return Ok(());
    }
    std::future::pending().await
}

/// The admitted image source, also reported without resolving the runtime.
pub(crate) fn resolve_image(config: &Config, project: &str) -> String {
    if let Some(image) = &config.image {
        image.clone()
    } else if config.containerfile.is_some() {
        format!("localhost/pinfold/project-{project}:latest")
    } else {
        config
            .profile
            .recipe()
            .expect("config admitted a recipe")
            .image_ref()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MissingManagedImage {
    pub(crate) reference: String,
    pub(crate) mount: ProjectMount,
    profile: Option<String>,
}

impl std::fmt::Display for MissingManagedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "image {} is missing; run `pinfold build", self.reference)?;
        if let Some(profile) = &self.profile {
            write!(f, " --profile {profile}")?;
        }
        write!(f, "`")
    }
}

impl std::error::Error for MissingManagedImage {}

pub(crate) fn image_warnings(config: &Config, image: &str) -> io::Result<Vec<String>> {
    image_warnings_for(config, image, &service().resolve_image(image)?)
}

/// Warnings for `image` from an already resolved lookup.
pub(crate) fn image_warnings_for(
    config: &Config,
    image: &str,
    resolved: &Result<ImageInfo, String>,
) -> io::Result<Vec<String>> {
    if config.image.is_some() {
        return match resolved {
            Ok(_) => Ok(Vec::new()),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "image-missing: image {image} is not present locally; build or pull it explicitly"
                ),
            )),
        };
    }
    let built = match resolved {
        Ok(built) => built,
        #[cfg(target_os = "macos")]
        Err(message) if *message != format!("Error: image not found: {image}") => {
            return Err(io::Error::other(format!(
                "container image inspect: {message}"
            )));
        }
        Err(_) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                MissingManagedImage {
                    reference: image.to_owned(),
                    mount: config.mount,
                    profile: config
                        .containerfile
                        .is_none()
                        .then(|| config.profile.name.clone()),
                },
            ));
        }
    };
    // The selected image and the images it builds on, top first, each with
    // its recipe: the project's, then each profile in the chain that has one.
    let build = match &config.containerfile {
        Some(_) => "pinfold build".to_string(),
        None => format!("pinfold build --profile {}", config.profile.name),
    };
    let mut chain = Vec::new();
    if let Some((_, bytes)) = &config.containerfile {
        chain.push((Some(built.clone()), bytes.as_slice()));
    }
    for link in config.profile.links().into_iter().rev() {
        let Some(bytes) = &link.containerfile else {
            continue;
        };
        let info = if chain.is_empty() {
            Some(built.clone())
        } else {
            service().resolve_image(&link.image_ref())?.ok()
        };
        chain.push((info, bytes.as_slice()));
    }
    // From the root up: an image is outdated when its recipe changed, its
    // recorded base is not the current base image, or that base is outdated.
    let mut warnings = Vec::new();
    let mut outdated = false;
    for index in (0..chain.len()).rev() {
        let Some(info) = &chain[index].0 else {
            outdated = true;
            continue;
        };
        let recorded = info
            .labels
            .get(clean::BASE_LABEL)
            .filter(|digest| !digest.is_empty())
            .map(String::as_str);
        let reason = if recipe_changed(info, chain[index].1) {
            Some("has changed or unrecorded Containerfile inputs".to_string())
        } else if let Some((base, _)) = chain.get(index + 1) {
            let current = base.as_ref().map(|base| base.id.as_str());
            if recorded != current {
                Some(format!(
                    "was built from image {}; its base is now {}",
                    recorded.unwrap_or("(none)"),
                    current.unwrap_or("(none)")
                ))
            } else {
                outdated.then(|| "builds on an outdated image".to_string())
            }
        } else {
            recorded.map(|recorded| format!("was built from image {recorded}; it now has no base"))
        };
        if let Some(reason) = reason {
            warnings.push(format!(
                "pinfold: image-outdated: image {} {reason}; run `{build}`",
                info.reference
            ));
            outdated = true;
        }
    }
    Ok(warnings)
}

/// Whether `image` lacks the recorded SHA-256 of `bytes`.
fn recipe_changed(image: &ImageInfo, bytes: &[u8]) -> bool {
    image.labels.get(image::CONTAINERFILE_LABEL) != Some(&sha256_hex(bytes))
}

pub(crate) fn build_plan(config: &Config, id: &str, root: &Path, image: &str) -> io::Result<Plan> {
    // HOME is the project box's own; like the always-applied names it wins
    // over configured and forwarded entries.
    let mut env = config.env.clone();
    env.insert("HOME".to_string(), Env::Exact(state::HOME.to_string()));
    let guest = state::guest_root(config.mount, root);
    env.entry("PINFOLD_PROJECT".to_string())
        .or_insert(Env::Exact(utf8(guest, "project root")?.to_string()));
    let [home, project] = state::disk_names(id);
    let mut disks = vec![empty_writable_disk(home, Path::new(state::HOME))];
    let mounts = match config.mount {
        ProjectMount::Disk => {
            disks.push(empty_writable_disk(project, guest));
            Vec::new()
        }
        ProjectMount::Share => vec![Mount {
            host: root.to_path_buf(),
            guest: root.to_path_buf(),
            kind: MountKind::Share,
        }],
    };
    Ok(Plan {
        profile: Some(config.profile.name.clone()),
        disks,
        mounts,
        env,
        egress: Some(Egress {
            allow: config.allow.clone(),
            routes: config.routes.clone(),
        }),
        ..box_plan(config, id, image)?
    })
}

/// The plan both a project box and a transient crossing box start from: the
/// image, the project label and resources, and nothing else.
fn box_plan(config: &Config, id: &str, image: &str) -> io::Result<Plan> {
    let mut random = [0u8; 6];
    use ring::rand::SecureRandom;
    ring::rand::SystemRandom::new()
        .fill(&mut random)
        .map_err(|_| io::Error::other("owner name generation failed"))?;
    Ok(Plan {
        name: format!("pi-{id}-{}", crate::core::hex(&random)),
        image: Some(image.to_string()),
        labels: BTreeMap::from([(clean::PROJECT_LABEL.to_string(), id.to_string())]),
        cpus: Some(config.cpus),
        memory: Some(config.memory.clone()),
        ..Plan::default()
    })
}

/// An empty writable disk; no host tree is copied into it.
fn empty_writable_disk(name: String, guest: &Path) -> Disk {
    Disk {
        name,
        guest: guest.to_path_buf(),
        size: None,
        from: None,
        readonly: false,
    }
}

/// `path` as a string, for a spec or report value; `what` names it in the
/// error. Both are JSON, so a non-UTF-8 path is refused, not made lossy.
pub(crate) fn utf8<'a>(path: &'a Path, what: &str) -> io::Result<&'a str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "path-not-utf8: {what} {} is not valid UTF-8",
                terminal_text(&path.to_string_lossy())
            ),
        )
    })
}
