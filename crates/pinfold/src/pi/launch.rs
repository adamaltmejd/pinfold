//! `pinfold pi`: assemble one project's box and run pi in it.
//!
//! The spec is built here from the pi layer's config, state and pinned
//! artifacts; core owns the box lifecycle. No secret value reaches argv: a
//! `from` entry names a host variable, and the runtime reads its value from
//! this process's environment.

use std::collections::BTreeMap;
use std::env;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::Command;

use nix::sys::signal::Signal;
use tokio::runtime::Builder;
use tokio::signal::unix::{SignalKind, signal};

use crate::cli;
use crate::config::Config;
use crate::core::artifacts::{self, GUEST_PI};
use crate::core::r#box::{Box, RefusalReason, Signals, UpError};
use crate::core::clean;
use crate::core::plan::{Egress, Env, HARNESS_PI, Mount, Plan};
use crate::core::runtime::{ImageStatus, exec_through_init, image_status, runtime};
use crate::pi::git::Git;
use crate::pi::state::{ProjectState, canonical};
use crate::trust;

/// Run a `pi`/`pinfold pi` invocation and return pi's exit code.
pub fn run(args: &[String]) -> io::Result<i32> {
    let cwd = canonical(&env::current_dir()?)?;
    let root = project_root(&cwd)?;
    let config = Config::load(&root)?;
    trust::check(&root, &config)?;
    let state = ProjectState::load_or_create(&root)?;
    let image = resolve_image(&config, &state.id);
    ensure_image(&config, &image)?;
    let argv = pi_argv(args);
    // The read-only mounts are prepared after trust, so a refused run leaves
    // no created directory behind.
    let git = Git::prepare(&root, &config.protect)?;
    let plan = build_plan(&config, &state, &root, &image, &git)?;
    let code = run_box(&plan, &cwd, &argv);
    // The box is down; remove the protected directories this run created.
    git.cleanup();
    code
}

/// The canonical project root for `cwd`: the git top level, else `cwd`.
/// Refuses a `cwd` outside the root.
pub(crate) fn project_root(cwd: &Path) -> io::Result<PathBuf> {
    let cwd = canonical(cwd)?;
    let root = canonical(&top_level(&cwd)?)?;
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

/// The git top level, else the invoking directory.
fn top_level(cwd: &Path) -> io::Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output();
    if let Ok(output) = output
        && output.status.success()
        && let Ok(path) = std::str::from_utf8(&output.stdout)
    {
        let path = path.trim();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    Ok(cwd.to_path_buf())
}

/// The image ref the box runs. A project Containerfile runs the project's
/// image; otherwise the profile's. `doctor` reports it.
pub(crate) fn resolve_image(config: &Config, project: &str) -> String {
    if config.containerfile.is_some() {
        format!("pinfold/project-{project}:latest")
    } else {
        config.profile.image_ref()
    }
}

/// Refuse when the image has not been built, naming its build command, and
/// report when a project image's recorded profile image is no longer the
/// current one.
fn ensure_image(config: &Config, image: &str) -> io::Result<()> {
    let (base, build) = match config.containerfile {
        Some(_) => (
            Some(config.profile.image_ref()),
            "pinfold build".to_string(),
        ),
        None => (
            None,
            format!("pinfold build --profile {}", config.profile.name),
        ),
    };
    match image_status(runtime(), image, base.as_deref())? {
        ImageStatus::Missing => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("image {image} is missing; run `{build}`"),
        )),
        ImageStatus::Current => Ok(()),
        ImageStatus::Stale { recorded, current } => {
            eprintln!(
                "pinfold: project image {image} was built from profile image {}; the current profile image is {}; run `pinfold build`",
                recorded.as_deref().unwrap_or("(none)"),
                current.as_deref().unwrap_or("(none)")
            );
            Ok(())
        }
    }
}

/// pi's argv in the box: the mounted artifact and the caller's arguments,
/// unchanged.
fn pi_argv(args: &[String]) -> Vec<String> {
    std::iter::once(format!("{GUEST_PI}/pi"))
        .chain(args.iter().cloned())
        .collect()
}

/// The complete box spec for one project.
fn build_plan(
    config: &Config,
    state: &ProjectState,
    root: &Path,
    image: &str,
    git: &Git,
) -> io::Result<Plan> {
    let home = utf8(&state.home, "project home")?;
    let mut labels = BTreeMap::new();
    labels.insert(clean::PROJECT_LABEL.to_string(), state.id.clone());

    let mut env = BTreeMap::new();
    env.insert("HOME".to_string(), Env::Exact(home.to_string()));
    env.insert("HERDR_AGENT".to_string(), Env::Exact("pi".to_string()));
    for name in &config.env {
        env.insert(
            name.clone(),
            Env::From {
                from: format!("PINFOLD_ENV_{name}"),
            },
        );
    }

    // Apple `container` shows the top directory of a mount as root-owned
    // inside the box, so git's ownership check refuses a mounted repository
    // with "detected dubious ownership". Listing the project root as
    // `safe.directory` through git's environment config skips the check. A
    // host `PINFOLD_ENV_GIT_CONFIG_COUNT` pass-through keeps its entries, so
    // the pi entry goes at its next index.
    let count = match env.get("GIT_CONFIG_COUNT") {
        Some(Env::From { from }) => env::var(from).ok().and_then(|value| value.parse().ok()),
        _ => None,
    }
    .unwrap_or(0);
    env.insert(
        "GIT_CONFIG_COUNT".to_string(),
        Env::Exact((count + 1).to_string()),
    );
    env.insert(
        format!("GIT_CONFIG_KEY_{count}"),
        Env::Exact("safe.directory".to_string()),
    );
    env.insert(
        format!("GIT_CONFIG_VALUE_{count}"),
        Env::Exact(utf8(root, "project root")?.to_string()),
    );

    let mut mounts = vec![
        Mount {
            host: root.to_path_buf(),
            guest: root.to_path_buf(),
            readonly: false,
        },
        Mount {
            host: state.home.clone(),
            guest: state.home.clone(),
            readonly: false,
        },
    ];
    // Nested after the writable project mount, so the read-only `.git` and
    // protected editor config shadow it.
    mounts.extend(git.mounts().iter().cloned());

    let plan = Plan {
        name: format!("pi-{}-{}", state.id, std::process::id()),
        image: Some(image.to_string()),
        profile: Some(config.profile.name.clone()),
        harness: Some(HARNESS_PI.to_string()),
        labels,
        mounts,
        user: None,
        env,
        egress: Some(Egress {
            allow: config.allow.clone(),
            routes: config.routes.clone(),
        }),
        cpus: Some(config.cpus),
        memory: Some(config.memory.clone()),
    };
    Ok(plan)
}

/// `path` as a string, for a spec value; `what` names it in the error.
fn utf8<'a>(path: &'a Path, what: &str) -> io::Result<&'a str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} {} is not valid UTF-8", path.display()),
        )
    })
}

fn run_box(plan: &Plan, cwd: &Path, argv: &[String]) -> io::Result<i32> {
    let init = artifacts::init()?;
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let result = runtime.block_on(async {
        // Register the handlers before the box starts, so a closed terminal
        // during startup is caught and the box is removed once it is up.
        let mut signals = Signals::new()?;
        let mut hangup = signal(SignalKind::hangup())?;
        // The handlers above are the run's; `up` installs none of its own.
        // A spec refusal reads as the pi layer's own input error.
        let mut box_ = match Box::up(plan, &init, None).await {
            Ok(box_) => box_,
            Err(UpError::Refused(refusal)) if refusal.reason == RefusalReason::Spec => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, refusal.detail));
            }
            Err(error) => return Err(error.into()),
        };
        let code = exec_pi(&plan.name, &init, tty, cwd, argv, &mut signals, &mut hangup).await;
        // Remove the box exactly once, whatever ended the run. A failed
        // removal must not hide the error that ended pi.
        let down = box_.down().await;
        match code {
            Ok(code) => {
                down?;
                Ok(code)
            }
            Err(error) => {
                let _ = down;
                Err(error)
            }
        }
    });
    // The exec runs on the blocking pool. A removed box makes it return, but
    // a hung exec must not hold the process after teardown.
    runtime.shutdown_background();
    result
}

/// Run pi with this process's stdio and return its exit code. The caller
/// removes the box; a signal that ends the run reports `128+n`.
async fn exec_pi(
    name: &str,
    init: &Path,
    tty: bool,
    cwd: &Path,
    argv: &[String],
    signals: &mut Signals,
    hangup: &mut tokio::signal::unix::Signal,
) -> io::Result<i32> {
    let runtime = runtime();
    let name = name.to_string();
    let init = init.to_path_buf();
    let workdir = cwd.to_path_buf();
    let argv = argv.to_vec();
    let exec = tokio::task::spawn_blocking(move || {
        runtime.exec(&name, tty, Some(&workdir), &exec_through_init(&init, &argv))
    });
    tokio::pin!(exec);
    Ok(tokio::select! {
        status = &mut exec => cli::exit_code(
            status.map_err(|error| io::Error::other(format!("pi exec failed: {error}")))??,
        ),
        signal = signals.recv() => 128 + signal as i32,
        _ = hangup.recv() => 128 + Signal::SIGHUP as i32,
    })
}
