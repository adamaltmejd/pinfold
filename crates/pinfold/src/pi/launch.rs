//! Assemble one project's box and run a command in it.

use std::collections::BTreeMap;
use std::env;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::runtime::Builder;

use crate::config::{Config, ProjectMount};
use crate::core::r#box::{Box, Shutdown, Signals, UpError};
use crate::core::clean;
use crate::core::crossing::CrossingError;
use crate::core::image::{ImageInfo, service};
use crate::core::plan::{Disk, Egress, Env, Mount, MountKind, Plan};
use crate::core::runtime::{ExecContext, Runtime, runtime};
use crate::core::{artifacts, crossing, disk, image, sha256_hex};
use crate::formatting::terminal_text;
use crate::pi::context::Context;
use crate::pi::state::{self, project_id};

/// `$1` is the project's guest path, `$2` the host's object format, `$3`
/// the host's branch or empty. Creates the box's repository there.
const INIT_SCRIPT: &str = r#"
exec git init -q --object-format="$2" ${3:+"--initial-branch=$3"} "$1"
"#;

/// `$1` is the project's new guest repository. Checks out the host's
/// counterpart of its unborn branch, when the push brought one.
const CHECKOUT_SCRIPT: &str = r#"
cd "$1" || exit
ref=$(git symbolic-ref -q HEAD) || exit 0
host=$(git rev-parse -q --verify "refs/remotes/host/${ref#refs/heads/}") || exit 0
exec git merge -q --ff-only "$host"
"#;

/// The first start of a project disk pinfold created (Git), whose host
/// record it consumes before anything crosses: create the box's repository, add the host's `box`
/// remote unless one exists, push the checkout's branches in and check out
/// the host's branch. A failure is reported, never retried, and never stops
/// the command.
pub(crate) fn first_push(
    name: &str,
    init: &Path,
    root: &Path,
    cancel: &std::sync::atomic::AtomicBool,
) {
    let Some(guest) = root.to_str() else { return };
    let failed = |detail: &str| {
        eprintln!(
            "pinfold: sync-failed: {}; check that `git remote get-url box` prints pinfold::{} (or `git remote add box pinfold::{}`), then `git push box BRANCH`",
            terminal_text(detail.trim()),
            terminal_text(guest),
            terminal_text(guest)
        )
    };
    let [_, disk] = state::disk_names(&project_id(root));
    match disk::first_push(&disk) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => return failed(&error.to_string()),
    }
    let Some(format) = git(root, &["rev-parse", "--show-object-format"], cancel) else {
        return failed("the host's object format could not be read");
    };
    // `--short` would print `heads/<b>` when a tag shares the branch's name.
    let branch = git(root, &["symbolic-ref", "-q", "HEAD"], cancel)
        .and_then(|head| head.strip_prefix("refs/heads/").map(str::to_string))
        .unwrap_or_default();
    match in_box(name, init, INIT_SCRIPT, &[guest, &format, &branch], cancel) {
        Ok(()) => {}
        Err((_, detail)) => return failed(&detail),
    }
    if git(root, &["config", "--get", "remote.box.url"], cancel).is_none()
        && git(
            root,
            &["remote", "add", "box", &format!("pinfold::{guest}")],
            cancel,
        )
        .is_none()
    {
        return failed("the host's `box` remote could not be added");
    }
    let Some(branches) = git(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/heads/"],
        cancel,
    ) else {
        return failed("the host's branches could not be listed");
    };
    let sources: Vec<String> = branches.lines().map(str::to_string).collect();
    // No branch: nothing to push.
    if sources.is_empty() {
        return;
    }
    let refspecs = ["+refs/heads/*:refs/remotes/host/*".to_string()];
    if let Err(CrossingError::Refused(_, detail) | CrossingError::Failed(detail)) =
        crossing::push_cancellable(
            runtime(),
            init,
            name,
            root,
            guest,
            &sources,
            &refspecs,
            cancel,
        )
    {
        return failed(&detail);
    }
    if let Err((_, detail)) = in_box(name, init, CHECKOUT_SCRIPT, &[guest], cancel) {
        failed(&detail);
    }
}

/// Run `script` with `args` in the box as the box user; a failure carries
/// its exit code and stderr.
fn in_box(
    name: &str,
    init: &Path,
    script: &str,
    args: &[&str],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), (Option<i32>, String)> {
    let argv: Vec<String> = ["sh", "-c", script, "sh"]
        .iter()
        .chain(args)
        .map(|arg| arg.to_string())
        .collect();
    let mut command = runtime()
        .exec_command(name, init, &ExecContext::default(), &argv)
        .map_err(|error| (None, error.to_string()))?;
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let output = crossing::output_discard_cancellable(&mut command, None, cancel)
        .map_err(|error| (None, error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err((
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

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
/// name, the root, its guest path and a flag that a signal or the box's end
/// sets.
pub(crate) fn cross<T, W>(
    check: impl FnOnce(&Path) -> io::Result<()>,
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
            let id = project_id(&root);
            let [_, disk] = state::disk_names(&id);
            let guest = utf8(&root, "project root")?.to_string();
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
                ensure_image(&config, &image)?;
                let plan = Plan {
                    disks: vec![empty_writable_disk(disk, &root)],
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

/// The admitted image source, also reported without resolving the runtime.
pub(crate) fn resolve_image(config: &Config, project: &str) -> String {
    if let Some(image) = &config.image {
        image.clone()
    } else if config.containerfile.is_some() {
        format!("localhost/pinfold/project-{project}:latest")
    } else {
        config.profile.image_ref()
    }
}

/// Refuse when the image has not been built, naming its build command, and
/// Warn when its Containerfile or its profile base changed. `doctor`
/// reports the same warnings.
pub(crate) fn ensure_image(config: &Config, image: &str) -> io::Result<()> {
    for warning in image_warnings(config, image)? {
        eprintln!("{warning}");
    }
    Ok(())
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
    let profile_build = format!("pinfold build --profile {}", config.profile.name);
    let (base, build, containerfile) = match &config.containerfile {
        Some((_, bytes)) => (
            config
                .profile
                .containerfile
                .as_ref()
                .map(|_| config.profile.image_ref()),
            "pinfold build",
            bytes.as_slice(),
        ),
        None => (
            None,
            profile_build.as_str(),
            config
                .profile
                .containerfile
                .as_deref()
                .expect("config admitted a recipe"),
        ),
    };
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
    let mut warnings = Vec::new();
    if let Some(warning) = containerfile_warning(built, containerfile, build) {
        warnings.push(warning);
    }
    if let Some(base) = base {
        let recorded = built
            .labels
            .get(clean::BASE_LABEL)
            .filter(|digest| !digest.is_empty());
        let current = service().resolve_image(&base)?.ok();
        if let Some(profile) = &current
            && let Some(bytes) = &config.profile.containerfile
            && let Some(warning) = containerfile_warning(profile, bytes, &profile_build)
        {
            warnings.push(warning);
        }
        let current = current.as_ref().map(|image| image.id.as_str());
        if recorded.map(String::as_str) != current {
            warnings.push(format!(
                "pinfold: image-outdated: project image {image} was built from profile image {}; the current profile image is {}; run `pinfold build`",
                recorded.map_or("(none)", String::as_str),
                current.unwrap_or("(none)")
            ));
        }
    }
    Ok(warnings)
}

fn containerfile_warning(image: &ImageInfo, bytes: &[u8], build: &str) -> Option<String> {
    let expected = sha256_hex(bytes);
    (image.labels.get(image::CONTAINERFILE_LABEL) != Some(&expected)).then(|| {
        format!(
            "pinfold: image-outdated: image {} has changed or unrecorded Containerfile inputs; run `{build}`",
            image.reference
        )
    })
}

pub(crate) fn build_plan(config: &Config, id: &str, root: &Path, image: &str) -> io::Result<Plan> {
    let mut env = BTreeMap::from([("HOME".to_string(), Env::Exact(state::HOME.to_string()))]);
    env.extend(config.env.iter().map(|name| {
        let from = format!("PINFOLD_ENV_{name}");
        (name.clone(), Env::From { from })
    }));
    let [home, project] = state::disk_names(id);
    let mut disks = vec![empty_writable_disk(home, Path::new(state::HOME))];
    let mounts = match config.mount {
        ProjectMount::Disk => {
            disks.push(empty_writable_disk(project, root));
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
fn empty_writable_disk(name: String, root: &Path) -> Disk {
    Disk {
        name,
        guest: root.to_path_buf(),
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

/// Run git in `dir`: its UTF-8 stdout less one trailing newline, or `None`
/// when it fails.
fn git(dir: &Path, args: &[&str], cancel: &std::sync::atomic::AtomicBool) -> Option<String> {
    let mut command = crossing::git_command(dir);
    command.args(args).stdin(std::process::Stdio::null());
    let output = crossing::output_cancellable(&mut command, None, cancel).ok()?;
    let stdout = String::from_utf8(output.stdout).ok()?;
    let stdout = stdout.strip_suffix('\n').unwrap_or(&stdout);
    output.status.success().then(|| stdout.to_string())
}
