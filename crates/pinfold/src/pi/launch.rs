//! `pinfold pi`: assemble one project's box and run pi in it.

use std::collections::BTreeMap;
use std::env;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use nix::sys::signal::Signal;
use tokio::runtime::Builder;
use tokio::signal::unix::{SignalKind, signal};

use crate::cli;
use crate::config::Config;
use crate::core::r#box::{Box, Signals};
use crate::core::clean;
use crate::core::plan::{Egress, Env, Mount, Plan};
use crate::core::runtime::{ImageInfo, runtime};
use crate::core::{artifacts, image, sha256_hex};
use crate::pi::git::{Git, git};
use crate::pi::state::{self, canonical, project_id};
use crate::trust;

/// The harness `pinfold pi` asks core for.
const HARNESS: &str = "pi";

/// Run a `pi`/`pinfold pi` invocation and return pi's exit code.
pub fn run(args: &[String]) -> io::Result<i32> {
    let cwd = canonical(&env::current_dir()?)?;
    let root = project_root(&cwd)?;
    let config = Config::load(&root)?;
    trust::check(&root, &config)?;
    let id = project_id(&root);
    let home = state::record_run(&root)?;
    let image = resolve_image(&config, &id);
    ensure_image(&config, &image)?;
    // The read-only mounts are prepared after trust, so a refused run leaves
    // no created directory behind.
    let git = Git::prepare(&root, &config.protect)?;
    let plan = build_plan(&config, &id, &home, &root, &image, &git)?;
    let code = run_box(&plan, &cwd, args);
    git.cleanup();
    code
}

/// The canonical project root for `cwd`: the git top level, else `cwd`.
/// Refuses a `cwd` inside a git directory, and one outside the root.
pub(crate) fn project_root(cwd: &Path) -> io::Result<PathBuf> {
    let cwd = canonical(cwd)?;
    // `--show-toplevel` fails inside `.git`, and the fallback would mount
    // `.git` writable as the project, so refuse before anything is created.
    if git(&cwd, &["rev-parse", "--is-inside-git-dir"]).is_ok_and(|inside| inside == "true") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to run inside a git directory: {}", cwd.display()),
        ));
    }
    let top = git(&cwd, &["rev-parse", "--show-toplevel"]).ok();
    let top = top.as_deref().filter(|top| !top.is_empty());
    let root = canonical(top.map_or(&cwd, Path::new))?;
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
        format!("pinfold/project-{project}:latest")
    } else {
        config.profile.image_ref()
    }
}

/// Refuse when the image has not been built, naming its build command, and
/// Warn when its Containerfile or its profile base changed. `doctor`
/// reports the same warnings.
pub(crate) fn ensure_image(config: &Config, image: &str) -> io::Result<()> {
    let (base, build, containerfile) = match &config.containerfile {
        Some((_, bytes)) => (
            Some(config.profile.image_ref()),
            "pinfold build".to_string(),
            bytes.as_slice(),
        ),
        None => (
            None,
            format!("pinfold build --profile {}", config.profile.name),
            config.profile.containerfile.as_slice(),
        ),
    };
    let Ok(built) = runtime().resolve_image(image)? else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("image {image} is missing; run `{build}`"),
        ));
    };
    warn_containerfile(&built, containerfile, &build);
    if let Some(base) = base {
        let recorded = built
            .labels
            .get(clean::BASE_LABEL)
            .filter(|digest| !digest.is_empty());
        let current = runtime().resolve_image(&base)?.ok();
        if let Some(profile) = &current {
            warn_containerfile(
                profile,
                &config.profile.containerfile,
                &format!("pinfold build --profile {}", config.profile.name),
            );
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

/// The complete box spec for one project.
fn build_plan(
    config: &Config,
    id: &str,
    home: &Path,
    root: &Path,
    image: &str,
    git: &Git,
) -> io::Result<Plan> {
    let mut labels = BTreeMap::new();
    labels.insert(clean::PROJECT_LABEL.to_string(), id.to_string());

    let mut env = BTreeMap::new();
    env.insert(
        "HOME".to_string(),
        Env::Exact(utf8(home, "project home")?.to_string()),
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
            host: home.to_path_buf(),
            guest: home.to_path_buf(),
            readonly: false,
        },
    ];
    // Nested after the writable project mount, so the read-only `.git` and
    // protected editor config shadow it.
    mounts.extend(git.readonly.iter().cloned());

    Ok(Plan {
        name: format!("pi-{id}-{}", std::process::id()),
        image: Some(image.to_string()),
        profile: Some(config.profile.name.clone()),
        harness: Some(HARNESS.to_string()),
        labels,
        mounts,
        env,
        egress: Some(Egress {
            allow: config.allow.clone(),
            routes: config.routes.clone(),
        }),
        cpus: Some(config.cpus),
        memory: Some(config.memory.clone()),
    })
}

/// `path` as a string, for a spec or report value; `what` names it in the
/// error. Both are JSON, so a non-UTF-8 path is refused, not made lossy.
pub(crate) fn utf8<'a>(path: &'a Path, what: &str) -> io::Result<&'a str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} {} is not valid UTF-8", path.display()),
        )
    })
}

fn run_box(plan: &Plan, cwd: &Path, args: &[String]) -> io::Result<i32> {
    let init = artifacts::init()?;
    let argv: Vec<String> = std::iter::once(format!("{}/pi", artifacts::guest(HARNESS).display()))
        .chain(args.iter().cloned())
        .collect();
    let (name, workdir) = (plan.name.clone(), cwd.to_path_buf());
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let box_runtime = runtime();
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let result = runtime.block_on(async {
        // Register the handlers before the box starts, so a closed terminal
        // during startup is caught and the box is removed once it is up.
        let mut signals = Signals::new()?;
        let mut hangup = signal(SignalKind::hangup())?;
        // The handlers above are the run's; `up` installs none of its own.
        let mut box_ = Box::up(plan, &init, None).await?;
        let exec = tokio::task::spawn_blocking(move || {
            box_runtime.exec(&name, &init, tty, Some(&workdir), &argv)
        });
        let code = tokio::select! {
            status = exec => status
                .map_err(|error| io::Error::other(format!("pi exec failed: {error}")))
                .and_then(|status| status.map(cli::exit_code)),
            signal = signals.recv() => Ok(128 + signal as i32),
            _ = hangup.recv() => Ok(128 + Signal::SIGHUP as i32),
        };
        // Remove the box exactly once, whatever ended the run. A failed
        // removal must not hide the error that ended pi.
        let down = box_.down().await;
        let code = code?;
        down?;
        Ok(code)
    });
    // The exec runs on the blocking pool. A removed box makes it return, but
    // a hung exec must not hold the process after teardown.
    runtime.shutdown_background();
    result
}
