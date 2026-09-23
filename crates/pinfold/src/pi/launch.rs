//! `pinfold pi`: assemble one project's box and run pi in it.
//!
//! The spec is built here from the pi layer's config, state and pinned
//! artifacts; core owns the box lifecycle. No secret value reaches argv: a
//! `from` entry names a host variable, and the runtime reads its value from
//! this process's environment.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use nix::sys::signal::Signal;
use tokio::runtime::Builder;
use tokio::signal::unix::{SignalKind, signal};

use crate::cli;
use crate::config::{Config, Containerfile};
use crate::core::artifacts;
use crate::core::r#box::Box;
use crate::core::clean;
use crate::core::plan::{Egress, Env, Mount, Plan};
use crate::core::runtime::runtime;
use crate::pi::state::ProjectState;

/// The label naming a box's project.
pub const PROJECT_LABEL: &str = "dev.pinfold.project";

/// Where the pinned pi artifact is mounted in the box.
const GUEST_PI: &str = "/opt/pinfold/pi";

/// Run a `pi`/`pinfold pi` invocation and return pi's exit code.
pub fn run(args: &[OsString]) -> i32 {
    match launch(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pinfold pi: {error}");
            1
        }
    }
}

fn launch(args: &[OsString]) -> io::Result<i32> {
    let cwd = canonical(&env::current_dir()?)?;
    let root = canonical(&project_root(&cwd)?)?;
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
    let config = Config::load(&root)?;
    let state = ProjectState::load_or_create(&root)?;
    let image = resolve_image(&config);
    ensure_profile_image(&config, &image)?;
    let pi = artifacts::pi()?;
    let pi_dir = pi.parent().ok_or_else(|| {
        io::Error::other(format!("pi artifact {} has no directory", pi.display()))
    })?;
    let argv = pi_argv(args)?;
    let plan = build_plan(&config, &state, &image, pi_dir)?;
    run_box(&plan, &cwd, &argv)
}

/// The project root: the git top level, else the invoking directory.
fn project_root(cwd: &Path) -> io::Result<PathBuf> {
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

fn canonical(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("canonicalize {}: {error}", path.display()),
        )
    })
}

/// The stable ref of the selected profile's image.
fn profile_image(config: &Config) -> String {
    format!("pinfold/profile-{}:latest", config.profile.name)
}

/// The image ref the box runs. A project Containerfile is Y-9's; until then
/// the profile image runs in its place.
fn resolve_image(config: &Config) -> String {
    match &config.containerfile {
        Containerfile::Project(_) => profile_image(config),
        Containerfile::Profile(_) => config
            .image
            .clone()
            .unwrap_or_else(|| profile_image(config)),
    }
}

/// Refuse when the profile image has not been built. A named image ref is
/// the user's to provide.
fn ensure_profile_image(config: &Config, image: &str) -> io::Result<()> {
    if image != profile_image(config) {
        return Ok(());
    }
    let built = runtime()?
        .list_images()?
        .iter()
        .any(|info| info.reference == image);
    if built {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "image {image} is missing; run `pinfold build --profile {}`",
            config.profile.name
        ),
    ))
}

/// pi's argv in the box: the mounted artifact and the caller's arguments,
/// unchanged.
fn pi_argv(args: &[OsString]) -> io::Result<Vec<String>> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(format!("{GUEST_PI}/pi"));
    for arg in args {
        argv.push(
            arg.to_str()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "pi arguments must be valid UTF-8",
                    )
                })?
                .to_string(),
        );
    }
    Ok(argv)
}

/// The complete box spec for one project.
fn build_plan(
    config: &Config,
    state: &ProjectState,
    image: &str,
    pi_dir: &Path,
) -> io::Result<Plan> {
    let home = state.home.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("project home {} is not valid UTF-8", state.home.display()),
        )
    })?;
    let mut labels = BTreeMap::new();
    labels.insert(PROJECT_LABEL.to_string(), state.id.clone());
    // Maintenance prunes boxes whose owning process is gone; without the
    // owner label a live pi box would look like a leftover.
    labels.insert(
        clean::OWNER_LABEL.to_string(),
        std::process::id().to_string(),
    );

    let mut env = BTreeMap::new();
    env.insert("HOME".to_string(), Env::Exact(home.to_string()));
    env.insert("PI_TELEMETRY".to_string(), Env::Exact("0".to_string()));
    env.insert(
        "PI_SKIP_VERSION_CHECK".to_string(),
        Env::Exact("1".to_string()),
    );
    env.insert(
        "PINFOLD_ALLOW".to_string(),
        Env::Exact(config.allow.join(",")),
    );
    env.insert("HERDR_AGENT".to_string(), Env::Exact("pi".to_string()));
    for name in &config.env {
        env.insert(
            name.clone(),
            Env::From {
                from: format!("PINFOLD_ENV_{name}"),
            },
        );
    }

    Ok(Plan {
        name: format!("pi-{}-{}", state.id, std::process::id()),
        image: Some(image.to_string()),
        profile: Some(config.profile.name.clone()),
        labels,
        mounts: vec![
            Mount {
                host: state.root.clone(),
                guest: state.root.clone(),
                readonly: false,
            },
            Mount {
                host: state.home.clone(),
                guest: state.home.clone(),
                readonly: false,
            },
            Mount {
                host: pi_dir.to_path_buf(),
                guest: PathBuf::from(GUEST_PI),
                readonly: true,
            },
        ],
        user: None,
        env,
        egress: Some(Egress {
            allow: config.allow.clone(),
            routes: config.routes.clone(),
        }),
        cpus: Some(config.cpus),
        memory: Some(config.memory.clone()),
    })
}

fn run_box(plan: &Plan, cwd: &Path, argv: &[String]) -> io::Result<i32> {
    let init = cli::init_path()?;
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let result = runtime.block_on(async {
        // Register the handlers before the box starts, so a closed terminal
        // during startup is caught and the box is removed once it is up.
        let mut shutdown = Shutdown::new(tty)?;
        let mut box_ = Box::up(plan, &init).await?;
        let code = exec_pi(&plan.name, tty, cwd, argv, &mut shutdown).await;
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

/// How the pi run ended.
enum Stop {
    Exited(ExitStatus),
    Signal(Signal),
}

/// Run pi with this process's stdio and return its exit code. The caller
/// removes the box; a signal that ends the run reports `128+n`.
async fn exec_pi(
    name: &str,
    tty: bool,
    cwd: &Path,
    argv: &[String],
    shutdown: &mut Shutdown,
) -> io::Result<i32> {
    let runtime = runtime()?;
    let name = name.to_string();
    let workdir = cwd.to_path_buf();
    let argv = argv.to_vec();
    let exec = tokio::task::spawn_blocking(move || runtime.exec(&name, tty, Some(&workdir), &argv));
    tokio::pin!(exec);
    let stop = tokio::select! {
        status = &mut exec => Stop::Exited(
            status
                .map_err(|error| io::Error::other(format!("pi exec failed: {error}")))??,
        ),
        signal = shutdown.recv() => Stop::Signal(signal),
    };
    Ok(match stop {
        Stop::Exited(status) => cli::exit_code(status),
        Stop::Signal(signal) => 128 + signal as i32,
    })
}

/// The signals that end a run. With a TTY, Ctrl-C and Ctrl-\ are bytes on
/// the terminal and never signals here; SIGHUP and SIGTERM still remove the
/// box. The handlers are installed before the box starts.
struct Shutdown {
    hangup: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
    interrupt: Option<tokio::signal::unix::Signal>,
}

impl Shutdown {
    fn new(tty: bool) -> io::Result<Shutdown> {
        Ok(Shutdown {
            hangup: signal(SignalKind::hangup())?,
            terminate: signal(SignalKind::terminate())?,
            interrupt: if tty {
                None
            } else {
                Some(signal(SignalKind::interrupt())?)
            },
        })
    }

    async fn recv(&mut self) -> Signal {
        match self.interrupt.as_mut() {
            Some(interrupt) => tokio::select! {
                _ = self.hangup.recv() => Signal::SIGHUP,
                _ = self.terminate.recv() => Signal::SIGTERM,
                _ = interrupt.recv() => Signal::SIGINT,
            },
            None => tokio::select! {
                _ = self.hangup.recv() => Signal::SIGHUP,
                _ = self.terminate.recv() => Signal::SIGTERM,
            },
        }
    }
}
