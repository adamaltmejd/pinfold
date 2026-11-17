//! Assemble one project's box and run a command in it.

use std::collections::BTreeMap;
use std::env;
use std::io;
use std::path::{Path, PathBuf};

use tokio::runtime::Builder;

use crate::cli;
use crate::config::{Config, ProjectMount};
use crate::core::r#box::{Box, Shutdown, Signals, UpError};
use crate::core::clean;
use crate::core::crossing::CrossingError;
use crate::core::image::{ImageInfo, service};
use crate::core::plan::{Disk, Egress, Env, Mount, MountKind, Plan};
use crate::core::runtime::{ExecContext, runtime};
use crate::core::{artifacts, crossing, disk, image, sha256_hex};
use crate::formatting::terminal_text;
use crate::pi::context::Context;
use crate::pi::state::{self, project_id};

/// `$1` is the project's guest path, `$2` the host's branch or empty.
/// Creates the box's repository there unless `.git` exists.
const INIT_SCRIPT: &str = r#"
[ -e "$1/.git" ] && exit
exec git init -q ${2:+"--initial-branch=$2"} "$1"
"#;

/// `$1` is the project's guest repository, `$2` the push's remote. Exits 0
/// without a merge unless the checked-out branch is unborn, or clean and a
/// proper ancestor of its `$2/` counterpart.
const FAST_FORWARD_SCRIPT: &str = r#"
cd "$1" || exit
ref=$(git symbolic-ref -q HEAD) || exit 0
host=$(git rev-parse -q --verify "refs/remotes/$2/${ref#refs/heads/}") || exit 0
if head=$(git rev-parse -q --verify HEAD); then
  [ "$head" != "$host" ] && git merge-base --is-ancestor "$head" "$host" || exit 0
  # The refresh keeps stat-only differences from counting as changes.
  git update-index -q --refresh >/dev/null
  git diff-index --quiet HEAD -- || exit 0
fi
exec git merge -q --ff-only "$host"
"#;

/// Create the box's repository if the project disk has none, push the
/// checkout's branches into it, then fast-forward its checkout (Git). A
/// failure is reported and never stops the command.
pub(crate) fn sync(name: &str, init: &Path, root: &Path, cancel: &std::sync::atomic::AtomicBool) {
    let Some(guest) = root.to_str() else { return };
    let failed = |detail: &str| eprintln!("pinfold: sync-failed: {}", terminal_text(detail.trim()));
    // `--short` would print `heads/<b>` when a tag shares the branch's name.
    let branch = git(root, &["symbolic-ref", "-q", "HEAD"], cancel)
        .and_then(|head| head.strip_prefix("refs/heads/").map(str::to_string))
        .unwrap_or_default();
    if let Err(detail) = in_box(name, init, INIT_SCRIPT, &[guest, &branch], cancel) {
        return failed(&detail);
    }
    // No branch: nothing to push.
    if !git(root, &["for-each-ref", "--count=1", "refs/heads/"], cancel)
        .is_some_and(|out| !out.is_empty())
    {
        return;
    }
    if let Err(CrossingError::Refused(_, detail) | CrossingError::Failed(detail)) =
        crossing::push_cancellable(runtime(), init, name, root, guest, "host", cancel)
    {
        return failed(&detail);
    }
    if let Err(detail) = in_box(name, init, FAST_FORWARD_SCRIPT, &[guest, "host"], cancel) {
        failed(&detail);
    }
}

/// Run `script` with `args` in the box as the box user; a failure's detail
/// is its stderr.
fn in_box(
    name: &str,
    init: &Path,
    script: &str,
    args: &[&str],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let argv: Vec<String> = ["sh", "-c", script, "sh"]
        .iter()
        .chain(args)
        .map(|arg| arg.to_string())
        .collect();
    let mut command = runtime()
        .exec_command(name, init, &ExecContext::default(), &argv)
        .map_err(|error| error.to_string())?;
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let output = crossing::output_discard_cancellable(&mut command, None, cancel)
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
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

/// `pinfold push` and `pinfold fetch`: cross this checkout's branches with
/// the box that attaches its project disk, or with a transient box on that
/// disk, without egress, when none does.
pub fn crossing(verb: &str, args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(cli::usage(verb, &format!("{verb} takes no arguments")));
    }
    let runtime = Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let mut signals = Signals::new()?;
        let cwd = state::canonical(&env::current_dir()?)?;
        let root = crate::pi::session::recorded_root(&cwd)?
            .map(Ok).unwrap_or_else(|| project_root(&cwd))?;
        let mut lease = match crate::pi::session::borrow(&root, &mut signals).await {
            Ok(lease) => lease,
            Err(error) => {
                return match signals.last() {
                    Some(signal) => Ok(128 + signal as i32),
                    None => Err(error),
                };
            }
        };
        let result = async {
            // The owner's root probe precedes reading a different configuration root.
            let config = Config::load(&root)?;
            config.check_host_paths(&root)?;
            let id = project_id(&root);
            let [_, disk] = state::disk_names(&id);
            let guest = utf8(&root, "project root")?.to_string();
            let (verb, remote) = if verb == "push" {
                ("push", "host")
            } else {
                ("fetch", "pinfold")
            };
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
                let name = plan.name.clone();
                let box_ = match Box::up(&plan, &mut signals).await {
                    Ok(box_) => box_,
                    Err(UpError::Control) => {
                        return report_crossing(verb, &name, remote, Err(CrossingError::Failed(
                            "the transient box was stopped before ready".to_string(),
                        )), &mut signals).await;
                    }
                    Err(UpError::Signal) => {
                        return Ok(128
                            + signals.last().unwrap_or(nix::sys::signal::Signal::SIGTERM) as i32);
                    }
                    Err(UpError::Refused(refusal)) => {
                        return report_crossing(verb, &name, remote, Err(CrossingError::Refused(refusal.reason, refusal.detail)), &mut signals).await;
                    }
                    Err(error) => {
                        return report_crossing(verb, &name, remote, Err(CrossingError::Failed(
                            io::Error::from(error).to_string(),
                        )), &mut signals).await;
                    }
                };
                (name, Some(box_))
            };
            if box_.is_none() {
                artifacts::init()?;
            }
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let target = name.clone();
            let mut worker = tokio::task::spawn_blocking(move || {
                if verb == "push" {
                    crossing::push_cancellable(
                        box_runtime,
                        &init,
                        &target,
                        &root,
                        &guest,
                        remote,
                        &worker_cancel,
                    )
                } else {
                    crossing::fetch_cancellable(
                        box_runtime,
                        &init,
                        &target,
                        &guest,
                        &root,
                        remote,
                        &worker_cancel,
                    )
                }
            });
            let (code, terminal) = tokio::select! {
                biased;
                crossed = &mut worker => {
                    let code = match crossed {
                        Ok(crossed) => report_crossing(verb, &name, remote, crossed, &mut signals).await,
                        Err(error) => Err(io::Error::other(format!("{verb} failed: {error}"))),
                    };
                    (code, Some(Shutdown::Signal.terminal()))
                },
                ended = async {
                    match box_.as_mut() {
                        Some(box_) => box_.ended().await,
                        None => {
                            let _ = lease.ended().await;
                            Ok(Shutdown::Signal)
                        },
                    }
                } => {
                    cancel.store(true, std::sync::atomic::Ordering::Release);
                    let _ = worker.await;
                    match ended {
                        Ok(ended) => (Ok(125), Some(ended.terminal())),
                        Err(error) => {
                            let generation = box_.as_ref().map_or(name.as_str(), |box_| box_.run_id.as_str());
                            let message = format!("pinfold {verb}: {} ({generation}); pinfold box inspect {generation}", terminal_text(&error.to_string()));
                            let _ = crate::pi::session::forward_diagnostics(message.into_bytes(), &mut signals).await;
                            (Ok(125), None)
                        },
                    }
                },
                signal = signals.recv() => {
                    cancel.store(true, std::sync::atomic::Ordering::Release);
                    let _ = worker.await;
                    (Ok(128 + signal as i32), Some(Shutdown::Signal.terminal()))
                },
            };
            let down = match box_.as_mut() {
                Some(box_) => box_.down(terminal).await,
                None => Ok(()),
            };
            let code = code?;
            down?;
            Ok(code)
        }
        .await;
        let left = lease.leave(&mut signals).await;
        match left {
            Ok(()) => result,
            Err(error) if matches!(result, Ok(0)) => Err(error),
            Err(error) => {
                let message = format!("pinfold {verb}: {}", terminal_text(&error.to_string()));
                let _ = crate::pi::session::forward_diagnostics(message.into_bytes(), &mut signals).await;
                result
            }
        }
    })
}

async fn report_crossing(
    verb: &str,
    name: &str,
    remote: &str,
    result: Result<crossing::Refs, CrossingError>,
    signals: &mut Signals,
) -> io::Result<i32> {
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (report, code) = cli::crossing_report(verb, name, remote, result);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        crate::core::process::with_cancellation(worker_cancel.clone(), || {
            let mut output = crate::core::process::StdioRelay::new(1, false)?;
            let line = format!("{report}\n");
            let mut bytes = line.as_bytes();
            while !bytes.is_empty() {
                if worker_cancel.load(Ordering::Acquire) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                match output.write(bytes) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(written) => bytes = &bytes[written..],
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                        ) =>
                    {
                        nix::poll::poll(&mut [], 20u16)?;
                    }
                    Err(error) => return Err(error),
                }
            }
            output.close();
            while !output.finished()? {
                if worker_cancel.load(Ordering::Acquire) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                nix::poll::poll(&mut [], 20u16)?;
            }
            Ok(code)
        })
    });
    tokio::select! {
        biased;
        result = &mut worker => result.map_err(io::Error::other)?,
        signal = signals.recv() => {
            cancel.store(true, Ordering::Release);
            let _ = worker.await;
            Ok(128 + signal as i32)
        },
    }
}

/// The canonical project root for `cwd`: the git top level, else `cwd`.
/// Refuses a `cwd` inside a git directory, and one outside the root.
pub(crate) fn project_root(cwd: &Path) -> io::Result<PathBuf> {
    Ok(Context::discover(cwd)?.root)
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

pub(crate) fn image_warnings(config: &Config, image: &str) -> io::Result<Vec<String>> {
    if config.image.is_some() {
        return match service().resolve_image(image)? {
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
    let Ok(built) = service().resolve_image(image)? else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("image {image} is missing; run `{build}`"),
        ));
    };
    let mut warnings = Vec::new();
    if let Some(warning) = containerfile_warning(&built, containerfile, build) {
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
            git: false,
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
