//! `pinfold box`: the JSON-on-stdio process interface for programmatic
//! callers, and `pinfold build`: the profile and project image build.
//!
//! The verbs are parsed by hand: the set is small, and ARCHITECTURE.md's
//! dependency list has no argument parser.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use crate::config::{Config, Containerfile, Origin};
use crate::core::artifacts;
use crate::core::r#box::{Box, Refusal, RefusalReason, Shutdown, UpError};
use crate::core::clean;
use crate::core::plan::Plan;
use crate::core::profile::{Profile, valid_name};
use crate::core::runtime::{BoxInfo, BuildRequest, Runtime, local_image_id, podman, runtime};
use crate::dirs;
use crate::trust;

const USAGE: &str = "usage: pinfold box up|exec BOX [--tty] [--workdir DIR] -- argv|down BOX|list --label k=v [--label k]|prune";
const BUILD_USAGE: &str = "usage: pinfold build [--profile NAME]";
const PROFILE_USAGE: &str = "usage: pinfold profile new NAME [--from PROFILE]";
const ALLOW_USAGE: &str = "usage: pinfold allow";
const ATTACH_USAGE: &str = "usage: pinfold attach [--box NAME] [cmd...]";
const CLEAN_USAGE: &str = "usage: pinfold clean [--dry-run] [--unused AGE]";
const DOCTOR_USAGE: &str = "usage: pinfold doctor";

/// `doctor` suggests `pinfold clean` above this much measured disk use.
const CLEAN_SUGGESTION_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// Run a `pinfold pi` invocation and return its process exit code.
pub fn pi(args: &[OsString]) -> i32 {
    crate::pi::launch::run(args)
}

/// Run a `pinfold allow` invocation and return its process exit code.
pub fn allow(args: &[OsString]) -> i32 {
    match run_allow(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("pinfold allow: {error}");
            1
        }
    }
}

fn run_allow(args: &[OsString]) -> io::Result<()> {
    if !args.is_empty() {
        return Err(allow_usage("allow takes no arguments"));
    }
    let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
    crate::trust::allow(&root)
}

/// Run a `pinfold attach` invocation: bash, or the given command, in this
/// project's running pi box. The box's owner keeps its lifetime; attach
/// streams one exec and adds none of its own.
pub fn attach(args: &[OsString]) -> i32 {
    match run_attach(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pinfold attach: {error}");
            1
        }
    }
}

fn run_attach(args: &[OsString]) -> io::Result<i32> {
    let (box_name, mut argv) = parse_attach(args)?;
    if argv.is_empty() {
        argv.push("bash".to_string());
    }
    let cwd = fs::canonicalize(std::env::current_dir()?)?;
    let root = crate::pi::launch::project_root(&cwd)?;
    let id = crate::pi::state::project_id(&root)?;
    let runtime = runtime()?;
    let boxes: Vec<BoxInfo> = runtime
        .list()?
        .into_iter()
        .filter(|box_| box_.labels.get(crate::core::clean::PROJECT_LABEL) == Some(&id))
        .collect();
    let name = select_box(&boxes, box_name.as_deref())?;
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let status = runtime.exec(&name, tty, Some(&cwd), &argv)?;
    Ok(exit_code(status))
}

/// The box to attach to: the named one, or the project's only one. Several
/// without a name is the caller's to resolve.
fn select_box(boxes: &[BoxInfo], requested: Option<&str>) -> io::Result<String> {
    if let Some(name) = requested {
        return boxes
            .iter()
            .find(|box_| box_.id == name)
            .map(|box_| box_.id.clone())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no running pi box named {name:?} for this project"),
                )
            });
    }
    match boxes {
        [] => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no running pi box for this project; start one with `pinfold pi`",
        )),
        [only] => Ok(only.id.clone()),
        several => {
            let mut names: Vec<&str> = several.iter().map(|box_| box_.id.as_str()).collect();
            names.sort_unstable();
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "several pi boxes are running for this project; select one with `--box NAME`: {}",
                    names.join(", ")
                ),
            ))
        }
    }
}

/// `attach`'s options and command. Options end at the first command word or
/// `--`, so a command keeps every argument after it.
fn parse_attach(args: &[OsString]) -> io::Result<(Option<String>, Vec<String>)> {
    let mut name = None;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.to_str() {
            Some("--box") => {
                name = Some(
                    args.get(index + 1)
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| attach_usage("--box needs a box name"))?
                        .to_string(),
                );
                index += 2;
            }
            Some("--") => {
                index += 1;
                break;
            }
            _ => break,
        }
    }
    let mut argv = Vec::new();
    for arg in &args[index..] {
        argv.push(
            arg.to_str()
                .ok_or_else(|| attach_usage("attach arguments must be valid UTF-8"))?
                .to_string(),
        );
    }
    Ok((name, argv))
}

/// Run a `pinfold box` invocation and return its process exit code.
pub fn run(args: &[OsString]) -> i32 {
    match dispatch(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pinfold box: {error}");
            1
        }
    }
}

fn dispatch(args: &[OsString]) -> io::Result<i32> {
    match args.first().and_then(|arg| arg.to_str()) {
        Some("up") => up(&args[1..]),
        Some("exec") => exec(&args[1..]),
        Some("down") => down(&args[1..]),
        Some("list") => list(&args[1..]),
        Some("prune") => prune(&args[1..]),
        Some(verb) => Err(usage(&format!("unknown box verb {verb:?}"))),
        None => Err(usage("a box verb is required")),
    }
}

/// Read one spec from stdin, start the box, report it ready, and hold it
/// until stdin closes, SIGTERM arrives, or the box exits.
fn up(args: &[OsString]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("up takes no arguments"));
    }
    let mut plan = match Plan::from_reader(io::stdin()) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(refused(Refusal {
                box_name: None,
                reason: RefusalReason::Spec,
                detail: error.to_string(),
            }));
        }
    };
    if let Err(error) = plan.validate() {
        return Ok(refused(Refusal {
            box_name: Some(plan.name.clone()),
            reason: RefusalReason::Spec,
            detail: error.to_string(),
        }));
    }
    // The owner label is how `box prune` tells a live box from a leftover.
    plan.labels
        .insert(clean::OWNER_LABEL.into(), std::process::id().to_string());
    let init = init_path()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let mut box_ = match Box::up(&plan, &init).await {
            Ok(box_) => box_,
            Err(UpError::Refused(refusal)) => return Ok(refused(refusal)),
            Err(UpError::Other(error)) => return Err(error),
        };
        println!(
            "{}",
            serde_json::json!({
                "event": "ready",
                "box": &plan.name,
                "owner": std::process::id(),
                "labels": &box_.labels,
            })
        );
        io::stdout().flush()?;
        let shutdown = box_.hold().await?;
        // Teardown is done, so the down line names a box the caller can
        // start again. It ends the stream.
        println!("{}", down_line(&plan.name, shutdown));
        io::stdout().flush()?;
        Ok(match shutdown {
            Shutdown::StdinEof | Shutdown::Signal => 0,
            Shutdown::BoxExited(_) => 1,
        })
    });
    // `Box::hold` watches stdin through tokio's blocking pool. A caller that
    // keeps stdin open leaves that read parked, and dropping the runtime
    // waits for it forever. Teardown is done, so leak the read and let the
    // process exit.
    runtime.shutdown_background();
    result
}

/// The one `down` line that ends `up`'s stream: why the box ended, after
/// teardown. Only `exited` carries a detail, the init's exit status.
fn down_line(name: &str, shutdown: Shutdown) -> serde_json::Value {
    let reason = shutdown.as_str();
    match shutdown {
        Shutdown::BoxExited(status) => serde_json::json!({
            "event": "down",
            "box": name,
            "reason": reason,
            "detail": status.to_string(),
        }),
        Shutdown::StdinEof | Shutdown::Signal => serde_json::json!({
            "event": "down",
            "box": name,
            "reason": reason,
        }),
    }
}

/// Print one `refused` line on stdout and return exit code 1.
fn refused(refusal: Refusal) -> i32 {
    println!(
        "{}",
        serde_json::json!({
            "event": "refused",
            "box": refusal.box_name,
            "reason": refusal.reason.as_str(),
            "detail": refusal.detail,
        })
    );
    1
}

/// Run a command in a running box with this process's stdio and return its
/// exit code.
fn exec(args: &[OsString]) -> io::Result<i32> {
    let args = ExecArgs::parse(args)?;
    let runtime = runtime()?;
    // An absent box is pinfold's own failure, told apart from the command's
    // by exit 3; every other code is the command's.
    if !runtime.list()?.iter().any(|box_| box_.id == args.name) {
        eprintln!("pinfold box exec: no box named {:?}", args.name);
        return Ok(3);
    }
    // A TTY only makes sense when both ends are terminals; `--tty` forces it
    // for callers that drive pinfold through their own pty.
    let tty = args.tty || (io::stdin().is_terminal() && io::stdout().is_terminal());
    let status = runtime.exec(&args.name, tty, args.workdir.as_deref(), &args.argv)?;
    Ok(exit_code(status))
}

/// Signal the owning `box up` process through the state dir and wait for it
/// to remove the box. A dead owner means remove the leftover directly.
fn down(args: &[OsString]) -> io::Result<i32> {
    let name = single_name(args, "down")?;
    let state = state_path(&name)?;
    if let Some(pid) = read_pid(&state)
        && clean::alive(pid)
    {
        kill(Pid::from_raw(pid), Signal::SIGTERM).map_err(io::Error::other)?;
        for _ in 0..1000 {
            if !state.exists() {
                return Ok(0);
            }
            if !clean::alive(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    runtime()?.down(&name)?;
    let _ = fs::remove_dir_all(&state);
    Ok(0)
}

/// Print one JSON line per box matching every `--label` filter.
fn list(args: &[OsString]) -> io::Result<i32> {
    let filters = parse_labels(args)?;
    for box_ in runtime()?.list()? {
        let matches = filters.iter().all(|(key, value)| match value {
            Some(value) => box_.labels.get(key) == Some(value),
            None => box_.labels.contains_key(key),
        });
        if !matches {
            continue;
        }
        let owner = box_
            .labels
            .get(clean::OWNER_LABEL)
            .and_then(|pid| pid.parse::<i32>().ok());
        println!(
            "{}",
            serde_json::json!({
                "name": box_.id,
                "labels": box_.labels,
                "owner": owner,
                "owner_alive": owner.is_some_and(clean::alive),
                "created": box_.created,
                "state": box_.state.as_str(),
            })
        );
    }
    Ok(0)
}

/// Remove boxes pinfold labeled whose owning `box up` process is gone, and
/// print one line per removal.
fn prune(args: &[OsString]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("prune takes no arguments"));
    }
    for dead in clean::prune_boxes(runtime()?)? {
        println!(
            "{}",
            serde_json::json!({
                "event": "pruned",
                "box": dead.id,
                "owner": dead.owner,
            })
        );
    }
    Ok(0)
}

/// Run a `pinfold clean` invocation and return its process exit code.
pub fn clean(args: &[OsString]) -> i32 {
    match run_clean(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("pinfold clean: {error}");
            1
        }
    }
}

/// List every category pinfold holds, then reclaim it unless `--dry-run`.
fn run_clean(args: &[OsString]) -> io::Result<()> {
    let (dry_run, unused) = parse_clean(args)?;
    let runtime = runtime()?;
    let plan = CleanPlan::measure(runtime, unused)?;
    if dry_run {
        println!("pinfold clean: dry run; {} B reclaimable", plan.total());
    } else {
        println!("pinfold clean: reclaiming {} B", plan.total());
    }
    plan.print(runtime);
    if dry_run {
        return Ok(());
    }
    plan.remove(runtime)
}

/// Everything one `clean` pass measures and would remove, measured before
/// anything is removed. `doctor` measures with it too, and removes nothing.
struct CleanPlan {
    boxes: clean::Boxes,
    caches: Vec<PathBuf>,
    stale: Vec<PathBuf>,
    automatic: u64,
    project_caches: u64,
    project_state: u64,
}

impl CleanPlan {
    /// Measure the categories without changing anything. `unused` ages
    /// project state as `clean --unused` does.
    fn measure(runtime: &dyn Runtime, unused: Option<Duration>) -> io::Result<CleanPlan> {
        let boxes = clean::boxes(runtime)?;
        let sockets = clean::leftover_socket_dirs()?;
        let mut box_dirs: BTreeSet<PathBuf> = boxes
            .dead
            .iter()
            .map(|dead| dead.state_dir.clone())
            .collect();
        box_dirs.extend(sockets);
        let artifacts = artifacts::unpinned_versions()?;
        let egress = clean::old_egress_logs()?;

        let projects = crate::pi::state::state_dirs()?;
        let mut caches = Vec::new();
        let mut stale = Vec::new();
        for project in &projects {
            // A live box holds this project's home; leave it all alone.
            if boxes.live_projects.contains(&project.id) {
                continue;
            }
            if project.stale(unused) {
                // The whole state dir goes; its cache is part of its size.
                stale.push(project.dir.clone());
            } else {
                let cache = project.home.join(".cache");
                if cache.exists() {
                    caches.push(cache);
                }
            }
        }

        let automatic = clean::total_bytes(&box_dirs)
            + clean::total_bytes(&artifacts)
            + clean::total_bytes(&egress);
        let project_caches = clean::total_bytes(&caches);
        let project_state = clean::total_bytes(&stale);
        Ok(CleanPlan {
            boxes,
            caches,
            stale,
            automatic,
            project_caches,
            project_state,
        })
    }

    /// The bytes a real `clean` would reclaim.
    fn total(&self) -> u64 {
        self.automatic + self.project_caches + self.project_state
    }

    /// List the categories, as `clean` and `doctor` both show them.
    fn print(&self, runtime: &dyn Runtime) {
        println!("  automatic maintenance: {} B", self.automatic);
        println!("  build cache: {}", runtime.build_cache_description());
        println!("  project caches: {} B", self.project_caches);
        println!("  project state: {} B", self.project_state);
    }

    /// Remove everything the plan measured.
    fn remove(&self, runtime: &dyn Runtime) -> io::Result<()> {
        for dead in &self.boxes.dead {
            dead.remove(runtime)?;
        }
        clean::prune_sockets()?;
        artifacts::prune_unpinned()?;
        clean::prune_egress_logs()?;
        runtime.purge_build_cache()?;
        for cache in &self.caches {
            fs::remove_dir_all(cache)?;
        }
        for dir in &self.stale {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}

/// `clean`'s options: `--dry-run`, and `--unused AGE` for state not run for
/// that long.
fn parse_clean(args: &[OsString]) -> io::Result<(bool, Option<Duration>)> {
    let mut dry_run = false;
    let mut unused = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--dry-run") => dry_run = true,
            Some("--unused") => {
                let value = args
                    .next()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| clean_usage("--unused needs an age like 30d"))?;
                unused = Some(parse_age(value)?);
            }
            Some(option) => return Err(clean_usage(&format!("unknown clean option {option:?}"))),
            None => return Err(clean_usage("clean options must be valid UTF-8")),
        }
    }
    Ok((dry_run, unused))
}

/// `AGE` is a whole number and one unit: `s`, `m`, `h` or `d`.
fn parse_age(value: &str) -> io::Result<Duration> {
    let mut chars = value.chars();
    let Some(unit) = chars.next_back() else {
        return Err(clean_usage("--unused needs an age like 30d"));
    };
    let seconds = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => {
            return Err(clean_usage(&format!(
                "age {value:?} needs a unit: s, m, h or d"
            )));
        }
    };
    let number: u64 = chars
        .collect::<String>()
        .parse()
        .map_err(|_| clean_usage(&format!("age {value:?} is not a whole number and a unit")))?;
    number
        .checked_mul(seconds)
        .map(Duration::from_secs)
        .ok_or_else(|| clean_usage(&format!("age {value:?} is too large")))
}

fn clean_usage(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}\n{CLEAN_USAGE}"),
    )
}

/// Run a `pinfold doctor` invocation and return its process exit code.
pub fn doctor(args: &[OsString]) -> i32 {
    match run_doctor(args) {
        Ok(0) => {
            println!("pinfold doctor: ok");
            0
        }
        Ok(problems) => {
            eprintln!("pinfold doctor: {problems} problem(s)");
            1
        }
        Err(error) => {
            eprintln!("pinfold doctor: {error}");
            1
        }
    }
}

/// Print one report of what `pinfold pi` depends on and what state it is in,
/// and return the number of missing dependencies. Reads only: nothing is
/// created, downloaded or fixed.
fn run_doctor(args: &[OsString]) -> io::Result<usize> {
    if !args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("doctor takes no arguments\n{DOCTOR_USAGE}"),
        ));
    }
    let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
    // The config must parse; a broken `.pinfold.toml` is itself the answer.
    let config = Config::load(&root)?;
    let project = crate::pi::state::project_id(&root)?;
    let profile = &config.profile.name;
    let mut problems = 0;

    println!("pinfold doctor: {}", root.display());

    let runtime = runtime();
    // A missing runtime binary is this host's one problem; the checks that
    // need the runtime fail with the same absence and are not counted again.
    let mut runtime_missing = false;
    match &runtime {
        Ok(runtime) => {
            let runtime = *runtime;
            println!("runtime: {} ({})", runtime.name(), runtime.isolation());
            match runtime.version() {
                Ok(version) => println!("  version: {version}"),
                Err(error) => {
                    println!("  version: unavailable: {error}");
                    problems += 1;
                    runtime_missing = error.kind() == io::ErrorKind::NotFound;
                }
            }
            if runtime.name() == "podman" {
                problems += report_podman();
            }
        }
        Err(error) => {
            println!("runtime: unavailable: {error}");
            problems += 1;
        }
    }

    match kernel() {
        Ok(kernel) => println!("kernel: {kernel}"),
        Err(error) => println!("kernel: unavailable: {error}"),
    }

    println!(
        "image: {}",
        crate::pi::launch::resolve_image(&config, &project)
    );
    match &runtime {
        Ok(runtime) => match report_image(&config, *runtime, &project) {
            Ok(missing) => problems += missing,
            Err(error) => {
                println!("  unchecked: {error}");
                problems += usize::from(!runtime_missing);
            }
        },
        Err(_) => println!("  unchecked: no runtime"),
    }

    match artifacts::pins() {
        Ok(pins) => {
            println!("artifacts:");
            for pin in pins {
                if pin.cached {
                    println!(
                        "  {} {}: cached at {}",
                        pin.name,
                        pin.version,
                        pin.path.display()
                    );
                } else {
                    println!(
                        "  {} {}: not cached; `pinfold pi` downloads it on first run ({})",
                        pin.name,
                        pin.version,
                        pin.path.display()
                    );
                }
            }
        }
        Err(error) => {
            println!("artifacts: unavailable: {error}");
            problems += 1;
        }
    }

    match trust::check(&root, &config) {
        Ok(()) => println!("trust: ok"),
        Err(error) => {
            println!("trust: {error}");
            problems += 1;
        }
    }

    println!("config:");
    println!(
        "  profile: {} ({})",
        profile,
        origin_label(config.origins.profile, profile)
    );
    println!(
        "  containerfile: {} ({})",
        config
            .containerfile_path
            .as_deref()
            .unwrap_or("(the profile image)"),
        origin_label(config.origins.containerfile, profile)
    );
    println!(
        "  cpus: {} ({})",
        config.cpus,
        origin_label(config.origins.cpus, profile)
    );
    println!(
        "  memory: {} ({})",
        config.memory,
        origin_label(config.origins.memory, profile)
    );
    print_list("allow", &config.allow, config.origins.allow, profile);
    print_routes(&config.routes, config.origins.routes, profile);
    print_list("protect", &config.protect, config.origins.protect, profile);
    println!("  env:");
    if config.env.is_empty() {
        println!("    (none)");
    }
    for name in &config.env {
        println!("    {name} (environment)");
    }

    println!("disk:");
    match &runtime {
        Ok(runtime) => match CleanPlan::measure(*runtime, None) {
            Ok(plan) => {
                plan.print(*runtime);
                let total = plan.total();
                if total > CLEAN_SUGGESTION_BYTES {
                    println!("  {total} B is over 20 GB; run `pinfold clean`");
                }
            }
            Err(error) => {
                println!("  unavailable: {error}");
                problems += usize::from(!runtime_missing);
            }
        },
        Err(_) => println!("  unavailable: no runtime"),
    }

    Ok(problems)
}

/// The podman lines in `doctor`'s runtime report, and the problems they add:
/// a host `preflight` refuses cannot start a box. Report only: `preflight`
/// is what refuses, and `doctor` changes nothing.
///
/// A missing podman is named here, not counted: `runtime.version()` has
/// already counted it, and a host is counted once.
fn report_podman() -> usize {
    let problems = match podman::detect() {
        Ok(podman::Detected::RootlessPodman(info)) => {
            println!("  detected: rootless podman");
            print_cgroup_manager(&info.cgroup_manager);
            usize::from(info.cgroup_manager != "systemd")
        }
        Ok(podman::Detected::RootfulPodman(info)) => {
            println!("  detected: rootful podman; pinfold requires rootless podman");
            print_cgroup_manager(&info.cgroup_manager);
            // Rootful alone is the problem; the cgroup manager is not
            // counted twice for one host.
            1
        }
        Ok(podman::Detected::Missing) => {
            println!("  podman: not installed or not on PATH; install rootless podman");
            if podman::docker_present() {
                println!("  docker is on PATH; pinfold does not use it");
            }
            0
        }
        Err(error) => {
            println!("  detected: unavailable: {error}");
            1
        }
    };
    match podman::linger() {
        Ok(true) => println!("  linger: enabled"),
        Ok(false) => println!(
            "  linger: disabled; long-lived boxes stop at logout; run `loginctl enable-linger`"
        ),
        Err(error) => println!("  linger: unavailable: {error}"),
    }
    problems
}

/// `doctor`'s cgroup-manager line. Under cgroupfs podman accepts `--cpus`
/// and `--memory` and silently ignores them; preflight refuses the host.
fn print_cgroup_manager(manager: &str) {
    if manager == "systemd" {
        println!("  cgroupManager: systemd");
    } else {
        println!("  cgroupManager: {manager}; --cpus and --memory are silently not enforced");
    }
}

/// Report the image `pinfold pi` would run, and whether it was built from the
/// current profile image. Returns 1 when the image is missing.
fn report_image(config: &Config, runtime: &dyn Runtime, project: &str) -> io::Result<usize> {
    let image = crate::pi::launch::resolve_image(config, project);
    let images = runtime.list_images()?;
    let Some(found) = images.iter().find(|info| info.reference == image) else {
        println!("  missing; run `pinfold build`");
        return Ok(1);
    };
    println!("  exists");
    if matches!(config.containerfile, Containerfile::Project(_)) {
        let recorded = found.labels.get(clean::BASE_LABEL).map(String::as_str);
        let current = local_image_id(runtime, &config.profile.image_ref())?;
        if recorded == current.as_deref() {
            println!("  built from the current profile image");
        } else {
            println!(
                "  built from profile image {}; the current profile image is {}; run `pinfold build`",
                recorded.unwrap_or("(none)"),
                current.as_deref().unwrap_or("(none)")
            );
        }
    }
    Ok(0)
}

/// The host kernel, as `uname` reports it.
fn kernel() -> io::Result<String> {
    let output = Command::new("uname").arg("-sr").output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!("uname -sr: {}", output.status)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// One list key's effective entries, each with the layer the list came from.
fn print_list(key: &str, entries: &[String], origin: Origin, profile: &str) {
    println!("  {key}:");
    if entries.is_empty() {
        println!("    (none)");
    }
    for entry in entries {
        println!("    {entry} ({})", origin_label(origin, profile));
    }
}

/// The effective routes, each with the layer the list came from and target.
fn print_routes(routes: &BTreeMap<String, String>, origin: Origin, profile: &str) {
    println!("  routes:");
    if routes.is_empty() {
        println!("    (none)");
    }
    for (name, target) in routes {
        println!("    {name} -> {target} ({})", origin_label(origin, profile));
    }
}

/// The layer a value came from, naming the selected profile.
fn origin_label(origin: Origin, profile: &str) -> String {
    match origin {
        Origin::Profile => format!("profile {profile}"),
        other => other.name().to_string(),
    }
}

struct ExecArgs {
    name: String,
    tty: bool,
    workdir: Option<PathBuf>,
    argv: Vec<String>,
}

impl ExecArgs {
    fn parse(args: &[OsString]) -> io::Result<ExecArgs> {
        let mut args = args.iter();
        let name = args
            .next()
            .and_then(|arg| arg.to_str())
            .ok_or_else(|| usage("exec needs a box name"))?
            .to_string();
        let mut tty = false;
        let mut workdir = None;
        let mut argv = Vec::new();
        loop {
            match args.next().and_then(|arg| arg.to_str()) {
                Some("--") => {
                    for arg in args.by_ref() {
                        let arg = arg
                            .to_str()
                            .ok_or_else(|| usage("exec argv must be valid UTF-8"))?;
                        argv.push(arg.to_string());
                    }
                    break;
                }
                Some("--tty") => tty = true,
                Some("--workdir") => {
                    workdir = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| usage("--workdir needs a directory"))?,
                    ));
                }
                Some(option) => return Err(usage(&format!("unknown exec option {option:?}"))),
                None => return Err(usage("exec needs `-- argv`")),
            }
        }
        if argv.is_empty() {
            return Err(usage("exec needs a command after `--`"));
        }
        Ok(ExecArgs {
            name,
            tty,
            workdir,
            argv,
        })
    }
}

fn single_name(args: &[OsString], verb: &str) -> io::Result<String> {
    match args {
        [name] => name
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| usage("box name must be valid UTF-8")),
        _ => Err(usage(&format!("{verb} needs exactly one box name"))),
    }
}

/// `list`'s `--label` filters: each key, and the value to match when one was
/// given. Every filter must match.
fn parse_labels(args: &[OsString]) -> io::Result<Vec<(String, Option<String>)>> {
    let mut filters = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg.to_str() != Some("--label") {
            return Err(usage("list takes only --label k=v or --label k"));
        }
        let label = args
            .next()
            .and_then(|value| value.to_str())
            .ok_or_else(|| usage("--label needs k=v or k"))?;
        match label.split_once('=') {
            Some((key, value)) => filters.push((key.to_string(), Some(value.to_string()))),
            None => filters.push((label.to_string(), None)),
        }
    }
    if filters.is_empty() {
        return Err(usage("list needs at least one --label"));
    }
    Ok(filters)
}

fn state_path(name: &str) -> io::Result<PathBuf> {
    Ok(dirs::state_dir()?.join("boxes").join(name))
}

fn read_pid(state: &Path) -> Option<i32> {
    fs::read_to_string(state.join("pid"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub(crate) fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    match status.code() {
        Some(code) => code,
        None => 128 + status.signal().unwrap_or(0),
    }
}

/// The Linux init binary to mount into a box.
///
/// On macOS the CLI embeds the `aarch64-unknown-linux-musl` build and
/// extracts it once per content into the artifact cache. On Linux the CLI
/// is itself a static Linux binary and mounts its own executable.
pub(crate) fn init_path() -> io::Result<PathBuf> {
    if cfg!(target_os = "macos") {
        embedded_init()
    } else {
        std::env::current_exe()
    }
}

fn embedded_init() -> io::Result<PathBuf> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::os::unix::fs::PermissionsExt;

    const INIT: &[u8] = include_bytes!(env!("PINFOLD_INIT"));
    let mut hasher = DefaultHasher::new();
    INIT.hash(&mut hasher);
    let dir = dirs::artifacts_dir()?
        .join("init")
        .join(format!("{:016x}", hasher.finish()))
        .join("linux-arm64");
    let path = dir.join("pinfold");
    if path.is_file() {
        return Ok(path);
    }
    fs::create_dir_all(&dir)?;
    // Write and chmod beside the destination, then rename, so a concurrent
    // `box up` never mounts a half-written init.
    let temp = dir.join(format!(".pinfold-init-{}", std::process::id()));
    fs::write(&temp, INIT)?;
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o755))?;
    fs::rename(&temp, &path)?;
    Ok(path)
}

fn usage(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("{message}\n{USAGE}"))
}

/// Run a `pinfold build` invocation and return its process exit code.
pub fn build(args: &[OsString]) -> i32 {
    match run_build(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("pinfold build: {error}");
            1
        }
    }
}

fn run_build(args: &[OsString]) -> io::Result<()> {
    match parse_profile(args)? {
        Some(name) => build_profile(&Profile::load(&name)?),
        None => build_configured(),
    }
}

/// `pinfold build` without `--profile`: build the project's own image when
/// its config names a Containerfile, else the configured profile's image.
fn build_configured() -> io::Result<()> {
    let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
    let config = Config::load(&root)?;
    // A project build is a run of its config: an untrusted change stops it
    // like it stops `pinfold pi`.
    trust::check(&root, &config)?;
    match &config.containerfile {
        Containerfile::Project(bytes) => build_project(&root, &config, bytes),
        Containerfile::Profile(_) => build_profile(&config.profile),
    }
}

/// Build `profile`'s image from an empty context holding its Containerfile.
fn build_profile(profile: &Profile) -> io::Result<()> {
    let id = build_id();
    // The build gets the Containerfile alone: a profile directory's other
    // files are not build inputs, and an embedded default has no directory.
    let context = dirs::cache_dir()?.join("build").join(&id);
    fs::create_dir_all(&context)?;
    let containerfile = context.join("Containerfile");
    fs::write(&containerfile, &profile.containerfile)?;
    let result = build_profile_image(profile, &context, &containerfile, &id);
    let _ = fs::remove_dir_all(&context);
    result
}

fn build_profile_image(
    profile: &Profile,
    context: &Path,
    containerfile: &Path,
    id: &str,
) -> io::Result<()> {
    let runtime = runtime()?;
    let mut labels = BTreeMap::new();
    labels.insert(clean::PROFILE_LABEL.to_string(), profile.name.clone());
    // The unique build label is what makes every build a distinct image even
    // when every layer is cached.
    labels.insert(clean::BUILD_LABEL.to_string(), id.to_string());
    if let Some(base) = profile.base_image()
        && let Some(digest) = runtime.image_digest(base)?
    {
        labels.insert(clean::BASE_LABEL.to_string(), digest);
    }
    let stable = profile.image_ref();
    let unique = format!("pinfold/profile-{}:{id}", profile.name);
    runtime.build(&BuildRequest {
        context,
        containerfile,
        tags: &[stable.clone(), unique],
        labels: &labels,
    })?;
    // Keep the two newest images of this source, the second for rollback.
    // A failure here is reported but never fails the build that succeeded.
    if let Err(error) = clean::keep_two_images(runtime, clean::PROFILE_LABEL, &profile.name) {
        eprintln!("pinfold build: maintenance: {error}");
    }
    println!("{stable}");
    Ok(())
}

/// Build this project's image from the Containerfile bytes trust checked.
fn build_project(root: &Path, config: &Config, bytes: &[u8]) -> io::Result<()> {
    let id = build_id();
    // The build gets the Containerfile alone, so trust covers every input.
    let context = dirs::cache_dir()?.join("build").join(&id);
    fs::create_dir_all(&context)?;
    let containerfile = context.join("Containerfile");
    fs::write(&containerfile, bytes)?;
    let result = build_project_image(root, config, &context, &containerfile, &id);
    let _ = fs::remove_dir_all(&context);
    result
}

fn build_project_image(
    root: &Path,
    config: &Config,
    context: &Path,
    containerfile: &Path,
    id: &str,
) -> io::Result<()> {
    let runtime = runtime()?;
    let project = crate::pi::state::project_id(root)?;
    let mut labels = BTreeMap::new();
    labels.insert(clean::PROJECT_LABEL.to_string(), project.clone());
    labels.insert(clean::BUILD_LABEL.to_string(), id.to_string());
    // The digest of the profile image this project image was built from;
    // `pinfold pi` warns when the profile image moves past it.
    if let Some(digest) = local_image_id(runtime, &config.profile.image_ref())? {
        labels.insert(clean::BASE_LABEL.to_string(), digest);
    }
    let prefix = format!("pinfold/project-{project}");
    let stable = format!("{prefix}:latest");
    let unique = format!("{prefix}:{id}");
    runtime.build(&BuildRequest {
        context,
        containerfile,
        tags: &[stable.clone(), unique],
        labels: &labels,
    })?;
    // Keep the two newest images of this project, the second for rollback.
    // A failure here is reported but never fails the build that succeeded.
    if let Err(error) = clean::keep_two_images(runtime, clean::PROJECT_LABEL, &project) {
        eprintln!("pinfold build: maintenance: {error}");
    }
    println!("{stable}");
    Ok(())
}

fn parse_profile(args: &[OsString]) -> io::Result<Option<String>> {
    let mut name = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--profile") => {
                name = Some(
                    args.next()
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| build_usage("--profile needs a name"))?
                        .to_string(),
                );
            }
            Some(option) => return Err(build_usage(&format!("unknown build option {option:?}"))),
            None => return Err(build_usage("build options must be valid UTF-8")),
        }
    }
    Ok(name)
}

fn build_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{}", std::process::id())
}

fn build_usage(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}\n{BUILD_USAGE}"),
    )
}

/// Run a `pinfold profile` invocation and return its process exit code.
pub fn profile(args: &[OsString]) -> i32 {
    match run_profile(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("pinfold profile: {error}");
            1
        }
    }
}

fn run_profile(args: &[OsString]) -> io::Result<()> {
    match args.first().and_then(|arg| arg.to_str()) {
        Some("new") => profile_new(&args[1..]),
        Some(verb) => Err(profile_usage(&format!("unknown profile verb {verb:?}"))),
        None => Err(profile_usage("a profile verb is required")),
    }
}

/// Copy a profile's files to `~/.config/pinfold/profiles/NAME/`, refusing to
/// overwrite an existing profile.
fn profile_new(args: &[OsString]) -> io::Result<()> {
    let (name, from) = parse_profile_new(args)?;
    if !valid_name(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("profile name {name:?} must start alphanumeric and hold only [a-z0-9._-]"),
        ));
    }
    let source = Profile::load(&from);
    let target = dirs::config_dir()?.join("profiles").join(&name);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&target).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("profile {name:?} already exists at {}", target.display()),
            )
        } else {
            error
        }
    })?;
    let result = source.and_then(|source| write_profile(&source, &target));
    if result.is_err() {
        // Do not leave a half-written profile behind to load or block a retry.
        let _ = fs::remove_dir_all(&target);
    }
    result
}

fn write_profile(source: &Profile, target: &Path) -> io::Result<()> {
    fs::write(target.join("Containerfile"), &source.containerfile)?;
    fs::write(target.join("pinfold.toml"), &source.config)?;
    for seed in &source.home {
        let path = target.join("home").join(&seed.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &seed.contents)?;
    }
    if let Some(share) = &source.share {
        copy_tree(share, &target.join("share"))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn parse_profile_new(args: &[OsString]) -> io::Result<(String, String)> {
    let mut name = None;
    let mut from = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--from") => {
                from = Some(
                    args.next()
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| profile_usage("--from needs a profile name"))?
                        .to_string(),
                );
            }
            Some(option) if option.starts_with("--") => {
                return Err(profile_usage(&format!("unknown profile option {option:?}")));
            }
            Some(value) if name.is_none() => name = Some(value.to_string()),
            Some(_) => return Err(profile_usage("profile new takes one name")),
            None => return Err(profile_usage("profile names must be valid UTF-8")),
        }
    }
    let name = name.ok_or_else(|| profile_usage("profile new needs a name"))?;
    Ok((name, from.unwrap_or_else(|| "default".to_string())))
}

fn profile_usage(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}\n{PROFILE_USAGE}"),
    )
}

fn allow_usage(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}\n{ALLOW_USAGE}"),
    )
}

fn attach_usage(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}\n{ATTACH_USAGE}"),
    )
}
