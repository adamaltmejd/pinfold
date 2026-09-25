//! `pinfold box` and `pinfold image`: the JSON-on-stdio process interface
//! for programmatic callers, and `pinfold build`: the profile and project
//! image build.
//!
//! The verbs are parsed by hand: the set is small, and ARCHITECTURE.md's
//! dependency list has no argument parser.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Duration;

use crate::config::{Config, Origin};
use crate::core::artifacts;
use crate::core::r#box::{Box, Refusal, RefusalReason, Shutdown, UpError};
use crate::core::clean;
use crate::core::image::{self, Build, Context, ImageError, ImageRequest};
use crate::core::plan::{Plan, Route};
use crate::core::profile::{self, Profile};
use crate::core::runtime::{
    BoxInfo, BuildCache, ImageStatus, Runtime, exec_through_init, image_status, local_image_id,
    podman, runtime,
};
use crate::dirs;
use crate::trust;

const BOX_USAGE: &str = "usage: pinfold box up|exec BOX [--tty] [--workdir DIR] -- argv|stat BOX|down BOX|list --label k=v [--label k]|prune";
const BUILD_USAGE: &str = "usage: pinfold build [--profile NAME]";
const IMAGE_USAGE: &str = "usage: pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]... [--no-cache]";
const PROFILE_USAGE: &str =
    "usage: pinfold profile new NAME [--from PROFILE] [--from-project [PATH]]";
const ALLOW_USAGE: &str = "usage: pinfold allow";
const ATTACH_USAGE: &str = "usage: pinfold attach [--box NAME] [cmd...]";
const CLEAN_USAGE: &str = "usage: pinfold clean [--dry-run] [--unused AGE]";
const DOCTOR_USAGE: &str = "usage: pinfold doctor";
const ARTIFACTS_USAGE: &str = "usage: pinfold artifacts";
const CONFIG_USAGE: &str = "usage: pinfold config [ROOT]";

/// `doctor` suggests `pinfold clean` above this much measured disk use.
const CLEAN_SUGGESTION_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// A verb's process exit code: its own, or 1 after printing
/// `pinfold VERB: ERROR` on stderr.
pub fn report(verb: &str, result: io::Result<i32>) -> i32 {
    result.unwrap_or_else(|error| {
        eprintln!("pinfold {verb}: {error}");
        1
    })
}

/// A malformed invocation: `message`, then the verb's usage line.
fn usage(usage: &str, message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("{message}\n{usage}"))
}

/// `pinfold allow`: trust this project's `.pinfold.toml` and Containerfile.
pub fn allow(args: &[OsString]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage(ALLOW_USAGE, "allow takes no arguments"));
    }
    let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
    crate::trust::allow(&root)?;
    Ok(0)
}

/// `pinfold attach`: bash, or the given command, in this project's running
/// pi box. The box's owner keeps its lifetime; attach streams one exec and
/// adds none of its own.
pub fn attach(args: &[OsString]) -> io::Result<i32> {
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
    let init = init_path()?;
    let status = runtime.exec(&name, tty, Some(&cwd), &exec_through_init(&init, &argv))?;
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
                        .ok_or_else(|| usage(ATTACH_USAGE, "--box needs a box name"))?
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
                .ok_or_else(|| usage(ATTACH_USAGE, "attach arguments must be valid UTF-8"))?
                .to_string(),
        );
    }
    Ok((name, argv))
}

/// `pinfold box`: the process interface.
pub fn run(args: &[OsString]) -> io::Result<i32> {
    match args.first().and_then(|arg| arg.to_str()) {
        Some("up") => up(&args[1..]),
        Some("exec") => exec(&args[1..]),
        Some("stat") => stat(&args[1..]),
        Some("down") => {
            crate::core::r#box::down(&single_name(&args[1..], "down")?)?;
            Ok(0)
        }
        Some("list") => list(&args[1..]),
        Some("prune") => prune(&args[1..]),
        Some(verb) => Err(usage(BOX_USAGE, &format!("unknown box verb {verb:?}"))),
        None => Err(usage(BOX_USAGE, "a box verb is required")),
    }
}

/// Read one spec from stdin, start the box, report it ready, and hold it
/// until stdin closes, SIGTERM arrives, or the box exits.
fn up(args: &[OsString]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage(BOX_USAGE, "up takes no arguments"));
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
    // The owner label names this process to `list`. For a state dir other
    // than this one, it is also how the owner is judged alive.
    plan.labels
        .insert(clean::OWNER_LABEL.into(), std::process::id().to_string());
    // Every error from here on has removed what the start made; it ends the
    // stream as one `failed` line.
    Ok(match hold_up(&plan) {
        Ok(code) => code,
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({
                    "event": "failed",
                    "box": &plan.name,
                    "detail": error.to_string(),
                })
            );
            1
        }
    })
}

/// Start the validated box, report it ready, hold it, and print the `down`
/// line. A refusal prints its own line.
fn hold_up(plan: &Plan) -> io::Result<i32> {
    let init = init_path()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        // This process owns the box, so `up` takes SIGTERM and SIGINT.
        let mut box_ = match Box::up(plan, &init, true).await {
            Ok(box_) => box_,
            Err(UpError::Refused(refusal)) => return Ok(refused(refusal)),
            Err(UpError::Signal) => {
                println!("{}", down_line(&plan.name, Shutdown::Signal));
                io::stdout().flush()?;
                return Ok(0);
            }
            Err(UpError::Other(error)) => return Err(error),
        };
        println!(
            "{}",
            serde_json::json!({
                "event": "ready",
                "box": &plan.name,
                "owner": std::process::id(),
                "labels": &box_.labels,
                "image": { "id": &box_.image_id, "ref": &box_.image_ref },
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
    let init = init_path()?;
    let status = runtime.exec(
        &args.name,
        tty,
        args.workdir.as_deref(),
        &exec_through_init(&init, &args.argv),
    )?;
    Ok(exit_code(status))
}

/// Print one JSON object of a box's runtime facts: OOM kills, memory and
/// pids use and limits. `stat` is what tells a caller an OOM kill from a
/// failure.
fn stat(args: &[OsString]) -> io::Result<i32> {
    let name = single_name(args, "stat")?;
    let runtime = runtime()?;
    // An absent box is pinfold's own failure, told apart from a stat failure
    // by exit 3, like exec.
    if !runtime.list()?.iter().any(|box_| box_.id == name) {
        eprintln!("pinfold box stat: no box named {name:?}");
        return Ok(3);
    }
    let report = serde_json::to_string(&runtime.stat(&name)?).map_err(io::Error::other)?;
    println!("{report}");
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
                "image": { "id": box_.image_id, "ref": box_.image_ref },
                "owner": owner,
                "owner_alive": clean::box_owner_alive(&dirs::box_state_dir(&box_.id)?, owner),
                "created": box_.created,
                "state": if box_.running { "running" } else { "stopped" },
            })
        );
    }
    Ok(0)
}

/// Remove boxes pinfold labeled whose owning `box up` process is gone, and
/// print one line per removal.
fn prune(args: &[OsString]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage(BOX_USAGE, "prune takes no arguments"));
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

/// `pinfold clean`: list every category pinfold holds, then reclaim it
/// unless `--dry-run`.
pub fn clean(args: &[OsString]) -> io::Result<i32> {
    let (dry_run, unused) = parse_clean(args)?;
    let runtime = runtime()?;
    let plan = CleanPlan::measure(runtime, unused)?;
    if dry_run {
        println!("pinfold clean: dry run; {} B reclaimable", plan.total());
    } else {
        println!("pinfold clean: reclaiming {} B", plan.total());
    }
    plan.print();
    if !dry_run {
        plan.remove(runtime)?;
    }
    Ok(0)
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
    build_cache: BuildCache,
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
        let build_cache = runtime.build_cache()?;
        Ok(CleanPlan {
            boxes,
            caches,
            stale,
            automatic,
            project_caches,
            project_state,
            build_cache,
        })
    }

    /// The bytes a real `clean` would reclaim.
    fn total(&self) -> u64 {
        let build_cache = match self.build_cache {
            BuildCache::Bytes(bytes) => bytes,
            BuildCache::Named(_) => 0,
        };
        self.automatic + build_cache + self.project_caches + self.project_state
    }

    /// List the categories, as `clean` and `doctor` both show them.
    fn print(&self) {
        println!("  automatic maintenance: {} B", self.automatic);
        match self.build_cache {
            BuildCache::Bytes(bytes) => println!("  build cache: {bytes} B"),
            BuildCache::Named(name) => println!("  build cache: {name}"),
        }
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
                    .ok_or_else(|| usage(CLEAN_USAGE, "--unused needs an age like 30d"))?;
                unused = Some(parse_age(value)?);
            }
            Some(option) => {
                return Err(usage(
                    CLEAN_USAGE,
                    &format!("unknown clean option {option:?}"),
                ));
            }
            None => return Err(usage(CLEAN_USAGE, "clean options must be valid UTF-8")),
        }
    }
    Ok((dry_run, unused))
}

/// `AGE` is a whole number and one unit: `s`, `m`, `h` or `d`.
fn parse_age(value: &str) -> io::Result<Duration> {
    let mut chars = value.chars();
    let Some(unit) = chars.next_back() else {
        return Err(usage(CLEAN_USAGE, "--unused needs an age like 30d"));
    };
    let seconds = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => {
            return Err(usage(
                CLEAN_USAGE,
                &format!("age {value:?} needs a unit: s, m, h or d"),
            ));
        }
    };
    let number: u64 = chars.collect::<String>().parse().map_err(|_| {
        usage(
            CLEAN_USAGE,
            &format!("age {value:?} is not a whole number and a unit"),
        )
    })?;
    number
        .checked_mul(seconds)
        .map(Duration::from_secs)
        .ok_or_else(|| usage(CLEAN_USAGE, &format!("age {value:?} is too large")))
}

/// `pinfold doctor`: print one report of what `pinfold pi` depends on and
/// what state it is in, ending with the count of missing dependencies. Reads
/// only: nothing is created, downloaded or fixed.
pub fn doctor(args: &[OsString]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage(DOCTOR_USAGE, "doctor takes no arguments"));
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
                plan.print();
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

    if problems == 0 {
        println!("pinfold doctor: ok");
        Ok(0)
    } else {
        eprintln!("pinfold doctor: {problems} problem(s)");
        Ok(1)
    }
}

/// `pinfold artifacts`: print the pinned artifacts as one JSON array, an
/// object per pin. Reads only: nothing is downloaded.
pub fn artifacts(args: &[OsString]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage(ARTIFACTS_USAGE, "artifacts takes no arguments"));
    }
    let pins = serde_json::to_value(artifacts::pins()?).map_err(io::Error::other)?;
    println!("{pins}");
    Ok(0)
}

/// The podman lines in `doctor`'s runtime report, and the problems they add:
/// a host `preflight` refuses cannot start a box. Report only: `preflight`
/// is what refuses, and `doctor` changes nothing. A missing tun device and a
/// short subordinate-id mapping do not refuse a box, but they fail
/// `pinfold build`, so they are counted for a rootless host.
///
/// A missing podman is named here, not counted: `runtime.version()` has
/// already counted it, and a host is counted once.
fn report_podman() -> usize {
    // Debian's `_apt` runs as gid 65534; the default image's apt step
    // `setegid`s to it.
    const APT_GID: u32 = 65534;
    let problems = match podman::detect() {
        Ok(podman::Detected::RootlessPodman(info)) => {
            println!("  detected: rootless podman");
            print_cgroup_manager(&info.host.cgroup_manager);
            let mut problems = usize::from(info.host.cgroup_manager != "systemd");
            if podman::tun_present() {
                println!("  tun: /dev/net/tun present");
            } else {
                println!(
                    "  tun: /dev/net/tun missing; pinfold build cannot give RUN steps a network (add the device, or set netns = \"host\" in containers.conf)"
                );
                problems += 1;
            }
            if podman::subordinate_ids_cover(&info, APT_GID) {
                println!("  subordinate ids: cover {APT_GID}");
            } else {
                println!(
                    "  subordinate ids: the mapping does not reach {APT_GID}; the default image's apt step fails (extend /etc/subuid and /etc/subgid, then podman system migrate)"
                );
                problems += 1;
            }
            problems
        }
        Ok(podman::Detected::RootfulPodman(info)) => {
            println!("  detected: rootful podman; pinfold requires rootless podman");
            print_cgroup_manager(&info.host.cgroup_manager);
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
    let base = config
        .containerfile
        .is_some()
        .then(|| config.profile.image_ref());
    let status = image_status(runtime, &image, base.as_deref())?;
    if let ImageStatus::Missing = status {
        println!("  missing; run `pinfold build`");
        return Ok(1);
    }
    println!("  exists");
    match status {
        ImageStatus::Stale { recorded, current } => println!(
            "  built from profile image {}; the current profile image is {}; run `pinfold build`",
            recorded.as_deref().unwrap_or("(none)"),
            current.as_deref().unwrap_or("(none)")
        ),
        _ if base.is_some() => println!("  built from the current profile image"),
        _ => {}
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
fn print_routes(routes: &BTreeMap<String, Route>, origin: Origin, profile: &str) {
    println!("  routes:");
    if routes.is_empty() {
        println!("    (none)");
    }
    for (name, target) in routes {
        let target = match target {
            Route::Address(address) => address,
            Route::Inject(inject) => &inject.to,
        };
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

/// `pinfold config`: print one JSON object, the effective configuration and
/// the project facts a caller needs to compose a box. `ROOT` defaults to the
/// project root `pinfold pi` would use from the current directory. Reads
/// only: nothing is created and nothing is recorded.
pub fn config(args: &[OsString]) -> io::Result<i32> {
    let root = match args {
        [] => crate::pi::launch::project_root(&std::env::current_dir()?)?,
        [root] => crate::pi::launch::project_root(Path::new(root))?,
        _ => {
            return Err(usage(CONFIG_USAGE, "config takes at most one project root"));
        }
    };
    let config = Config::load(&root)?;
    let project = crate::pi::state::project_id(&root)?;
    let home = crate::pi::state::project_home(&root)?;
    let image = crate::pi::launch::resolve_image(&config, &project);
    let image_built = !matches!(
        image_status(runtime()?, &image, None)?,
        ImageStatus::Missing
    );
    let trust = match trust::check(&root, &config) {
        Ok(()) => serde_json::json!({ "ok": true, "detail": "ok" }),
        Err(error) => serde_json::json!({ "ok": false, "detail": error.to_string() }),
    };
    let origin = |value: Origin| origin_label(value, &config.profile.name);
    let report = serde_json::json!({
        "root": path_string(&root)?,
        "profile": &config.profile.name,
        "image": image,
        "image_built": image_built,
        "containerfile": config.containerfile_path.as_deref().unwrap_or("profile"),
        "egress": { "allow": &config.allow, "routes": &config.routes },
        "protect": &config.protect,
        "cpus": config.cpus,
        "memory": &config.memory,
        "env": &config.env,
        "origins": {
            "profile": origin(config.origins.profile),
            "containerfile": origin(config.origins.containerfile),
            "cpus": origin(config.origins.cpus),
            "memory": origin(config.origins.memory),
            "allow": origin(config.origins.allow),
            "routes": origin(config.origins.routes),
            "protect": origin(config.origins.protect),
        },
        "project": { "id": project, "home": path_string(&home)? },
        "trust": trust,
    });
    println!("{report}");
    Ok(0)
}

/// A path as a string for the `config` report. The report is JSON, so a
/// non-UTF-8 path is a refusal, not a lossy value.
fn path_string(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path {} is not valid UTF-8", path.display()),
        )
    })
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
            .ok_or_else(|| usage(BOX_USAGE, "exec needs a box name"))?
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
                            .ok_or_else(|| usage(BOX_USAGE, "exec argv must be valid UTF-8"))?;
                        argv.push(arg.to_string());
                    }
                    break;
                }
                Some("--tty") => tty = true,
                Some("--workdir") => {
                    workdir =
                        Some(PathBuf::from(args.next().ok_or_else(|| {
                            usage(BOX_USAGE, "--workdir needs a directory")
                        })?));
                }
                Some(option) => {
                    return Err(usage(BOX_USAGE, &format!("unknown exec option {option:?}")));
                }
                None => return Err(usage(BOX_USAGE, "exec needs `-- argv`")),
            }
        }
        if argv.is_empty() {
            return Err(usage(BOX_USAGE, "exec needs a command after `--`"));
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
            .ok_or_else(|| usage(BOX_USAGE, "box name must be valid UTF-8")),
        _ => Err(usage(
            BOX_USAGE,
            &format!("{verb} needs exactly one box name"),
        )),
    }
}

/// `list`'s `--label` filters: each key, and the value to match when one was
/// given. Every filter must match.
fn parse_labels(args: &[OsString]) -> io::Result<Vec<(String, Option<String>)>> {
    let mut filters = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg.to_str() != Some("--label") {
            return Err(usage(BOX_USAGE, "list takes only --label k=v or --label k"));
        }
        let label = args
            .next()
            .and_then(|value| value.to_str())
            .ok_or_else(|| usage(BOX_USAGE, "--label needs k=v or k"))?;
        match label.split_once('=') {
            Some((key, value)) => filters.push((key.to_string(), Some(value.to_string()))),
            None => filters.push((label.to_string(), None)),
        }
    }
    if filters.is_empty() {
        return Err(usage(BOX_USAGE, "list needs at least one --label"));
    }
    Ok(filters)
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

/// `pinfold build`: `--profile NAME`'s image; else the project's own image
/// when its config names a Containerfile, else the configured profile's.
/// Either builds from an empty context holding only the Containerfile, so
/// trust covers every input of a project build, and a profile directory's
/// other files are not build inputs. Prints the stable ref, or the build's
/// captured output on stderr when it failed.
pub fn build(args: &[OsString]) -> io::Result<i32> {
    let (profile, project) = match parse_profile(args)? {
        Some(name) => (Profile::load(&name)?, None),
        None => {
            let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
            let config = Config::load(&root)?;
            // A project build is a run of its config: an untrusted change
            // stops it like it stops `pinfold pi`.
            trust::check(&root, &config)?;
            let project = config
                .containerfile
                .map(|bytes| crate::pi::state::project_id(&root).map(|id| (id, bytes)))
                .transpose()?;
            (config.profile, project)
        }
    };
    let runtime = runtime()?;
    // A project image records the profile image it was built from, so
    // `pinfold pi` warns when the profile image moves past it; a profile
    // image records the image its FROM pulls.
    let (repository, label, source, containerfile, base) = match &project {
        Some((id, bytes)) => (
            format!("pinfold/project-{id}"),
            clean::PROJECT_LABEL,
            id,
            bytes,
            local_image_id(runtime, &profile.image_ref())?,
        ),
        None => (
            format!("pinfold/profile-{}", profile.name),
            clean::PROFILE_LABEL,
            &profile.name,
            &profile.containerfile,
            profile::base_image(&profile.containerfile)
                .map(|base| runtime.image_digest(base))
                .transpose()?
                .flatten(),
        ),
    };
    let built = image::build(
        runtime,
        Build {
            repository,
            label,
            source,
            context: Context::Alone(containerfile),
            labels: base
                .map(|digest| (clean::BASE_LABEL.to_string(), digest))
                .into_iter()
                .collect(),
            cache: false,
        },
    )?;
    match built {
        Ok(built) => {
            println!("{}", built.latest);
            Ok(0)
        }
        Err(output) => {
            eprint!("{output}");
            Err(io::Error::other(format!(
                "the {} build failed",
                runtime.name()
            )))
        }
    }
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
                        .ok_or_else(|| usage(BUILD_USAGE, "--profile needs a name"))?
                        .to_string(),
                );
            }
            Some(option) => {
                return Err(usage(
                    BUILD_USAGE,
                    &format!("unknown build option {option:?}"),
                ));
            }
            None => return Err(usage(BUILD_USAGE, "build options must be valid UTF-8")),
        }
    }
    Ok(name)
}

/// `pinfold image`: a caller's image build.
pub fn image(args: &[OsString]) -> io::Result<i32> {
    match args.first().and_then(|arg| arg.to_str()) {
        Some("build") => Ok(image_build(&args[1..])),
        Some(verb) => Err(usage(IMAGE_USAGE, &format!("unknown image verb {verb:?}"))),
        None => Err(usage(IMAGE_USAGE, "an image verb is required")),
    }
}

/// The most lines of a failed build's output the `failed` line carries.
const FAILED_LOG_LINES: usize = 40;

/// Build a caller's image and print one `built`, `failed` or `refused` line.
fn image_build(args: &[OsString]) -> i32 {
    let name = args
        .first()
        .and_then(|arg| arg.to_str())
        .filter(|arg| !arg.starts_with("--"));
    let result = parse_image_build(args)
        .map_err(|detail| ImageError::Refused(RefusalReason::Spec, detail))
        .and_then(image::build_image);
    let (line, code) = match result {
        Ok(built) => (
            serde_json::json!({
                "event": "built",
                "image": name,
                "ref": built.reference,
                "latest": built.latest,
                "base": built
                    .labels
                    .get(clean::BASE_LABEL)
                    .filter(|base| !base.is_empty()),
                "labels": built.labels,
            }),
            0,
        ),
        Err(ImageError::Failed(output)) => {
            let lines: Vec<&str> = output.lines().collect();
            let tail = &lines[lines.len().saturating_sub(FAILED_LOG_LINES)..];
            (
                serde_json::json!({ "event": "failed", "image": name, "log": tail }),
                1,
            )
        }
        Err(ImageError::Refused(reason, detail)) => (
            serde_json::json!({
                "event": "refused",
                "image": name,
                "reason": reason.as_str(),
                "detail": detail,
            }),
            1,
        ),
    };
    println!("{line}");
    code
}

/// `image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]...
/// [--no-cache]`. An `Err` is the refusal's detail.
fn parse_image_build(args: &[OsString]) -> Result<ImageRequest, String> {
    let mut args = args.iter();
    let name = args
        .next()
        .and_then(|arg| arg.to_str())
        .filter(|arg| !arg.starts_with("--"))
        .ok_or("image build needs an image name")?
        .to_string();
    let mut containerfile = None;
    let mut context = None;
    let mut labels = BTreeMap::new();
    let mut cache = true;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--containerfile") => {
                containerfile = Some(PathBuf::from(
                    args.next().ok_or("--containerfile needs a path")?,
                ));
            }
            Some("--context") => {
                context = Some(PathBuf::from(
                    args.next().ok_or("--context needs a directory")?,
                ));
            }
            Some("--label") => {
                let (key, value) = args
                    .next()
                    .and_then(|label| label.to_str())
                    .and_then(|label| label.split_once('='))
                    .filter(|(key, _)| !key.is_empty())
                    .ok_or("--label needs KEY=VALUE")?;
                labels.insert(key.to_string(), value.to_string());
            }
            Some("--no-cache") => cache = false,
            Some(option) => return Err(format!("unknown image build option {option:?}")),
            None => return Err("image build options must be valid UTF-8".to_string()),
        }
    }
    Ok(ImageRequest {
        name,
        containerfile: containerfile.ok_or("image build needs --containerfile PATH")?,
        context: context.ok_or("image build needs --context DIR")?,
        labels,
        cache,
    })
}

/// `pinfold profile`: copy a profile to edit as files.
pub fn profile(args: &[OsString]) -> io::Result<i32> {
    match args.first().and_then(|arg| arg.to_str()) {
        Some("new") => profile_new(&args[1..]).map(|()| 0),
        Some(verb) => Err(usage(
            PROFILE_USAGE,
            &format!("unknown profile verb {verb:?}"),
        )),
        None => Err(usage(PROFILE_USAGE, "a profile verb is required")),
    }
}

/// Copy a profile's files to `~/.config/pinfold/profiles/NAME/`, refusing to
/// overwrite an existing profile. `--from-project` merges a project's pi
/// agent config into the new profile after the copy.
fn profile_new(args: &[OsString]) -> io::Result<()> {
    let (name, from, from_project) = parse_profile_new(args)?;
    profile::check_name(&name)?;
    let project_agent = from_project.as_deref().map(project_agent_dir).transpose()?;
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
    let result = source
        .and_then(|source| write_profile(&source, &target))
        .and_then(|()| match &project_agent {
            Some(agent) => merge_agent_tree(agent, &target.join("home/.pi/agent")),
            None => Ok(()),
        });
    if result.is_err() {
        // Do not leave a half-written profile behind to load or block a retry.
        let _ = fs::remove_dir_all(&target);
    }
    result
}

/// The pi agent dir of the project rooted at `path` (any directory inside
/// the project works). A project that never started has no state: an error
/// naming the missing path.
fn project_agent_dir(path: &Path) -> io::Result<PathBuf> {
    let root = crate::pi::launch::project_root(path)?;
    let agent = crate::pi::state::project_home(&root)?.join(".pi/agent");
    if !agent.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "project {} has no state at {}",
                root.display(),
                agent.display()
            ),
        ));
    }
    Ok(agent)
}

/// Merge a project's `home/.pi/agent/` into a profile's, replacing the
/// seeds of the same path. pi's login, session history, npm install and
/// caches stay behind; nothing else from the project home is copied.
fn merge_agent_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if agent_entry_excluded(&name) {
            continue;
        }
        let target = to.join(&name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            merge_agent_tree(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Entries that never enter a profile: pi's credentials, session history,
/// package install and cache directories.
fn agent_entry_excluded(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some("auth.json" | "sessions" | "npm" | "cache" | ".cache")
    )
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

fn parse_profile_new(args: &[OsString]) -> io::Result<(String, String, Option<PathBuf>)> {
    let mut name = None;
    let mut from = None;
    let mut from_project = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--from") => {
                from = Some(
                    args.next()
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| usage(PROFILE_USAGE, "--from needs a profile name"))?
                        .to_string(),
                );
            }
            Some("--from-project") => {
                // The path is optional; without one, or before another
                // option, the project root of the current directory.
                let path = match args.clone().next().and_then(|value| value.to_str()) {
                    Some(value) if !value.starts_with("--") => {
                        args.next();
                        PathBuf::from(value)
                    }
                    _ => PathBuf::from("."),
                };
                from_project = Some(path);
            }
            Some(option) if option.starts_with("--") => {
                return Err(usage(
                    PROFILE_USAGE,
                    &format!("unknown profile option {option:?}"),
                ));
            }
            Some(value) if name.is_none() => name = Some(value.to_string()),
            Some(_) => return Err(usage(PROFILE_USAGE, "profile new takes one name")),
            None => return Err(usage(PROFILE_USAGE, "profile names must be valid UTF-8")),
        }
    }
    let name = name.ok_or_else(|| usage(PROFILE_USAGE, "profile new needs a name"))?;
    Ok((
        name,
        from.unwrap_or_else(|| "default".to_string()),
        from_project,
    ))
}
