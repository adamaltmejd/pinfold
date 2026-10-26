//! `pinfold pi`: assemble one project's box and run pi in it.

use std::collections::BTreeMap;
use std::env;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::Command;

use tokio::runtime::Builder;

use crate::cli;
use crate::config::Config;
use crate::core::r#box::{Box, RefusalReason, Shutdown, Signals, UpError};
use crate::core::clean;
use crate::core::crossing::CrossingError;
use crate::core::plan::{Disk, Egress, Env, Plan};
use crate::core::runs::{self, Terminal};
use crate::core::runtime::{ImageInfo, runtime};
use crate::core::{artifacts, crossing, disk, image, ownership, sha256_hex};
use crate::pi::state::{self, canonical, project_id};
use crate::trust;

const HARNESS: &str = "pi";

/// Run a `pi`/`pinfold pi` invocation and return pi's exit code.
pub fn run(args: &[String]) -> io::Result<i32> {
    let cwd = canonical(&env::current_dir()?)?;
    let root = project_root(&cwd)?;
    refuse_worktree(&root)?;
    let config = Config::load(&root)?;
    trust::check(&root, &config)?;
    let id = project_id(&root);
    let _project = ownership::project_lock(&id)?;
    state::record_run(&root)?;
    let image = resolve_image(&config, &id);
    ensure_image(&config, &image)?;
    let plan = build_plan(&config, &id, &root, &image)?;
    run_box(&plan, &root, &cwd, args)
}

/// `$1` is the project's guest repository, `$2` the push's remote. Exits 0
/// without a merge unless the checked-out branch is clean and a proper
/// ancestor of its `$2/` counterpart.
const FAST_FORWARD_SCRIPT: &str = r#"
cd "$1" || exit
ref=$(git symbolic-ref -q HEAD) || exit 0
head=$(git rev-parse -q --verify HEAD) || exit 0
host=$(git rev-parse -q --verify "refs/remotes/$2/${ref#refs/heads/}") || exit 0
[ "$head" != "$host" ] && git merge-base --is-ancestor "$head" "$host" || exit 0
# The refresh keeps stat-only differences from counting as changes.
git update-index -q --refresh >/dev/null
git diff-index --quiet HEAD -- || exit 0
exec git merge -q --ff-only "$host"
"#;

/// Push the checkout's branches into the ready box, then fast-forward its
/// checkout (Git). A failure is reported and never stops pi.
fn sync(name: &str, init: &Path, root: &Path) {
    // No repository or no branch: nothing to push.
    if !root.join(".git").is_dir()
        || !git(root, &["for-each-ref", "--count=1", "refs/heads/"])
            .is_ok_and(|out| !out.is_empty())
    {
        return;
    }
    let Some(guest) = root.to_str() else { return };
    let failed = |detail: &str| eprintln!("pinfold pi: sync-failed: {}", detail.trim());
    if let Err(CrossingError::Refused(_, detail) | CrossingError::Failed(detail)) =
        crossing::push(runtime(), init, name, root, guest, "host")
    {
        return failed(&detail);
    }
    let argv = ["sh", "-c", FAST_FORWARD_SCRIPT, "sh", guest, "host"].map(str::to_owned);
    let mut command = runtime().exec_command(name, init, false, None, &argv);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return failed(&error.to_string()),
    };
    let stderr = crossing::kept_stderr(&mut child);
    match child.wait() {
        Ok(status) if status.success() => {}
        Ok(_) => failed(&stderr()),
        Err(error) => failed(&error.to_string()),
    }
}

/// Refuse a checkout whose `.git` is not a directory: a worktree's (or a
/// submodule's) `.git` file names a git dir outside the project, which
/// the project disk does not hold.
pub(crate) fn refuse_worktree(root: &Path) -> io::Result<()> {
    let dot_git = root.join(".git");
    match std::fs::symlink_metadata(&dot_git) {
        Ok(metadata) if !metadata.is_dir() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "worktree: {} is not a directory; run pinfold from the main checkout",
                dot_git.display()
            ),
        )),
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// `pinfold push` and `pinfold fetch`: cross this checkout's branches with
/// the box that attaches its project disk, or with a transient box on that
/// disk, without egress, when none does.
pub fn crossing(verb: &str, args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(cli::usage(verb, &format!("{verb} takes no arguments")));
    }
    let root = project_root(&env::current_dir()?)?;
    refuse_worktree(&root)?;
    let config = Config::load(&root)?;
    trust::check(&root, &config)?;
    let id = project_id(&root);
    let [_, disk] = state::disk_names(&id);
    let guest = utf8(&root, "project root")?.to_string();
    let (verb, remote) = if verb == "push" {
        ("push", "host")
    } else {
        ("fetch", "pinfold")
    };
    let box_runtime = runtime();
    let init = artifacts::init()?;
    let (repo, guest_repo) = (root.clone(), guest.clone());
    let cross = move |name: &str| {
        if verb == "push" {
            crossing::push(box_runtime, &init, name, &repo, &guest_repo, remote)
        } else {
            crossing::fetch(box_runtime, &init, name, &guest_repo, &repo, remote)
        }
    };
    match project_box(&disk)? {
        Holder::Ready(name) => {
            let result = cross(&name);
            return Ok(cli::report_crossing(verb, &name, remote, result));
        }
        Holder::Starting(name) => {
            let refusal = CrossingError::Refused(RefusalReason::DiskInUse, starting(&name, &disk));
            return Ok(cli::report_crossing(verb, &name, remote, Err(refusal)));
        }
        Holder::None => {}
    }
    if !box_runtime
        .list_disks()?
        .iter()
        .any(|listed| listed.name == disk)
        // The transient box names no `from`: claiming an unfilled disk
        // would empty it and mark it filled.
        || disk::unfilled(&disk)?
    {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "this project has no project disk yet; start it with `pinfold pi`",
        ));
    }
    let image = resolve_image(&config, &id);
    ensure_image(&config, &image)?;
    let plan = Plan {
        disks: vec![project_disk(disk, &root, None)?],
        ..box_plan(&config, &id, &image)
    };
    let init = artifacts::init()?;
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let result = runtime.block_on(async {
        let mut signals = Signals::new()?;
        let name = plan.name.clone();
        // Every start that ends without ready still prints one line.
        let unstarted = |error| Ok(cli::report_crossing(verb, &name, remote, Err(error)));
        let mut box_ = match Box::up(&plan, &init, &mut signals).await {
            Ok(box_) => box_,
            Err(UpError::Control) => {
                return unstarted(CrossingError::Failed(
                    "the transient box was stopped before ready".to_string(),
                ));
            }
            Err(UpError::Signal) => {
                return Ok(128 + signals.last().unwrap_or(nix::sys::signal::Signal::SIGTERM) as i32);
            }
            Err(UpError::Refused(refusal)) => {
                return unstarted(CrossingError::Refused(refusal.reason, refusal.detail));
            }
            Err(error) => {
                return unstarted(CrossingError::Failed(io::Error::from(error).to_string()));
            }
        };
        let crossed = tokio::task::spawn_blocking(move || {
            let result = cross(&name);
            cli::report_crossing(verb, &name, remote, result)
        });
        let code = tokio::select! {
            code = crossed => code.map_err(|error| io::Error::other(format!("{verb} failed: {error}"))),
            signal = signals.recv() => Ok(128 + signal as i32),
        };
        let down = box_.down(Some(Shutdown::Signal.terminal())).await;
        let code = code?;
        down?;
        Ok(code)
    });
    runtime.shutdown_background();
    result
}

/// What holds a project disk.
pub(crate) enum Holder {
    None,
    Ready(String),
    /// Claimed but not ready: a first start's `from` copy may be partial.
    Starting(String),
}

/// The box that attaches the project disk `disk`, and whether it is ready.
pub(crate) fn project_box(disk: &str) -> io::Result<Holder> {
    let Some((name, _)) = disk::attachments(runtime(), None)?
        .remove(disk)
        .and_then(|boxes| boxes.into_iter().next())
    else {
        return Ok(Holder::None);
    };
    let ready = match ownership::record(&name)? {
        Some(record) => runs::launched(&record.generation)?,
        // Listed by the runtime with no claim: a dead owner's box.
        None => false,
    };
    Ok(if ready {
        Holder::Ready(name)
    } else {
        Holder::Starting(name)
    })
}

/// The `disk-in-use` detail for a box that is not ready.
pub(crate) fn starting(name: &str, disk: &str) -> String {
    format!("box {name:?} attaches disk {disk:?} and is not ready")
}

/// The canonical project root for `cwd`: the git top level, else `cwd`.
/// Refuses a `cwd` inside a git directory, and one outside the root.
pub(crate) fn project_root(cwd: &Path) -> io::Result<PathBuf> {
    let cwd = canonical(cwd)?;
    // `--show-toplevel` fails inside `.git`, and the fallback would take
    // `.git` as the project, so refuse before anything is created.
    if git(&cwd, &["rev-parse", "--is-inside-git-dir"]).is_ok_and(|inside| inside == "true") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "project-in-git: refusing to run inside a git directory: {}",
                cwd.display()
            ),
        ));
    }
    let top = git(&cwd, &["rev-parse", "--show-toplevel"]).ok();
    let root = canonical(top.as_deref().map_or(&cwd, Path::new))?;
    if !cwd.starts_with(&root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "current directory {} is outside the project root {}",
                cwd.display(),
                root.display()
            ),
        ));
    }
    Ok(root)
}

/// The image ref the box runs. A project Containerfile runs the project's
/// image; otherwise the profile's. `doctor` reports it.
pub(crate) fn resolve_image(config: &Config, project: &str) -> String {
    if config.containerfile.is_some() {
        format!("localhost/pinfold/project-{project}:latest")
    } else {
        config.profile.image_ref()
    }
}

/// Refuse when the image has not been built, naming its build command, and
/// Warn when its Containerfile or its profile base changed. `doctor`
/// reports the same warnings.
pub(crate) fn ensure_image(config: &Config, image: &str) -> io::Result<()> {
    let profile_build = format!("pinfold build --profile {}", config.profile.name);
    let (base, build, containerfile) = match &config.containerfile {
        Some((_, bytes)) => (
            Some(config.profile.image_ref()),
            "pinfold build",
            bytes.as_slice(),
        ),
        None => (
            None,
            profile_build.as_str(),
            config.profile.containerfile.as_slice(),
        ),
    };
    let Ok(built) = runtime().resolve_image(image)? else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("image {image} is missing; run `{build}`"),
        ));
    };
    warn_containerfile(&built, containerfile, build);
    if let Some(base) = base {
        let recorded = built
            .labels
            .get(clean::BASE_LABEL)
            .filter(|digest| !digest.is_empty());
        let current = runtime().resolve_image(&base)?.ok();
        if let Some(profile) = &current {
            warn_containerfile(profile, &config.profile.containerfile, &profile_build);
        }
        let current = current.as_ref().map(|image| image.id.as_str());
        if recorded.map(String::as_str) != current {
            eprintln!(
                "pinfold: image-outdated: project image {image} was built from profile image {}; the current profile image is {}; run `pinfold build`",
                recorded.map_or("(none)", String::as_str),
                current.unwrap_or("(none)")
            );
        }
    }
    Ok(())
}

fn warn_containerfile(image: &ImageInfo, bytes: &[u8], build: &str) {
    let expected = sha256_hex(bytes);
    if image.labels.get(image::CONTAINERFILE_LABEL) != Some(&expected) {
        eprintln!(
            "pinfold: image-outdated: image {} has changed or unrecorded Containerfile inputs; run `{build}`",
            image.reference
        );
    }
}

fn build_plan(config: &Config, id: &str, root: &Path, image: &str) -> io::Result<Plan> {
    let mut env = BTreeMap::from([
        ("HOME".to_string(), Env::Exact(state::HOME.to_string())),
        ("HERDR_AGENT".to_string(), Env::Exact("pi".to_string())),
    ]);
    env.extend(config.env.iter().map(|name| {
        let from = format!("PINFOLD_ENV_{name}");
        (name.clone(), Env::From { from })
    }));
    let [home, project] = state::disk_names(id);
    Ok(Plan {
        profile: Some(config.profile.name.clone()),
        harness: Some(HARNESS.to_string()),
        disks: vec![
            Disk {
                name: home,
                guest: PathBuf::from(state::HOME),
                size: None,
                from: None,
                readonly: false,
            },
            project_disk(project, root, Some(root))?,
        ],
        env,
        egress: Some(Egress {
            allow: config.allow.clone(),
            routes: config.routes.clone(),
        }),
        ..box_plan(config, id, image)
    })
}

/// The plan both a pi box and a transient crossing box start from: the
/// image, the project label and resources, and nothing else.
fn box_plan(config: &Config, id: &str, image: &str) -> Plan {
    Plan {
        name: format!("pi-{id}-{}", std::process::id()),
        image: Some(image.to_string()),
        labels: BTreeMap::from([(clean::PROJECT_LABEL.to_string(), id.to_string())]),
        cpus: Some(config.cpus),
        memory: Some(config.memory.clone()),
        ..Plan::default()
    }
}

/// The project disk, writable at the project's own path. `from` copies the
/// checkout in when the disk is new.
fn project_disk(name: String, root: &Path, from: Option<&Path>) -> io::Result<Disk> {
    utf8(root, "project root")?;
    Ok(Disk {
        name,
        guest: root.to_path_buf(),
        size: None,
        from: from.map(Path::to_path_buf),
        readonly: false,
    })
}

/// `path` as a string, for a spec or report value; `what` names it in the
/// error. Both are JSON, so a non-UTF-8 path is refused, not made lossy.
pub(crate) fn utf8<'a>(path: &'a Path, what: &str) -> io::Result<&'a str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "path-not-utf8: {what} {} is not valid UTF-8",
                path.display()
            ),
        )
    })
}

fn run_box(plan: &Plan, root: &Path, cwd: &Path, args: &[String]) -> io::Result<i32> {
    let init = artifacts::init()?;
    let argv: Vec<String> = std::iter::once(format!("{}/pi", artifacts::guest(HARNESS).display()))
        .chain(args.iter().cloned())
        .collect();
    let (name, root, workdir) = (plan.name.clone(), root.to_path_buf(), cwd.to_path_buf());
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let box_runtime = runtime();
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let result = runtime.block_on(async {
        // Register the handlers before the box starts, so a closed terminal
        // during startup is caught and the box is removed once it is up.
        let mut signals = Signals::with_hangup()?;
        // The handlers above are the run's; `up` installs none of its own.
        let mut box_ = match Box::up(plan, &init, &mut signals).await {
            Ok(box_) => box_,
            Err(UpError::Control) => return Ok(0),
            Err(UpError::Signal) => {
                return Ok(128 + signals.last().unwrap_or(nix::sys::signal::Signal::SIGTERM) as i32);
            }
            Err(error) => return Err(error.into()),
        };
        // The sync runs here so that the signals and the box's end below
        // also cover it.
        let exec = tokio::task::spawn_blocking(move || {
            sync(&name, &init, &root);
            box_runtime.exec(&name, &init, tty, Some(&workdir), &argv)
        });
        let ended = tokio::select! {
            status = exec => status
                .map_err(|error| io::Error::other(format!("pi exec failed: {error}")))
                .and_then(|status| status)
                .map(|status| (Terminal { reason: "harness-exited", detail: Some(status.to_string()) }, cli::exit_code(status))),
            shutdown = box_.ended() => shutdown.map(|shutdown| {
                let code = match shutdown {
                    Shutdown::BoxExited(status) => cli::exit_code(status),
                    Shutdown::AuditLog => 1,
                    Shutdown::StdinEof | Shutdown::Signal => 0,
                };
                (shutdown.terminal(), code)
            }),
            signal = signals.recv() => Ok((Shutdown::Signal.terminal(), 128 + signal as i32)),
        };
        // Remove the box exactly once, whatever ended the run. A failed
        // removal must not hide the error that ended pi.
        let (terminal, code) = match ended {
            Ok((terminal, code)) => (Some(terminal), Ok(code)),
            Err(error) => (None, Err(error)),
        };
        let reason = terminal.as_ref().map(|terminal| terminal.reason);
        let down = box_.down(terminal).await;
        if let Some(reason) = reason.filter(|reason| !matches!(*reason, "harness-exited" | "signal")) {
            eprintln!(
                "pinfold pi: the box ended with {reason}; see `pinfold box inspect {}`",
                box_.run_id
            );
        }
        let code = code?;
        down?;
        Ok(code)
    });
    // The exec runs on the blocking pool. A removed box makes it return, but
    // a hung exec must not hold the process after teardown.
    runtime.shutdown_background();
    result
}

/// Run git in `dir` and return its stdout less one trailing newline.
fn git(dir: &Path, args: &[&str]) -> io::Result<String> {
    let command = format!("git -C {} {}", dir.display(), args.join(" "));
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("run {command}: {error}")))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut stdout = String::from_utf8(output.stdout).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{command} printed non-UTF-8 output: {error}"),
        )
    })?;
    if stdout.ends_with('\n') {
        stdout.pop();
    }
    Ok(stdout)
}
