//! The verbs are parsed by hand: the set is small, and ARCHITECTURE.md's
//! dependency list has no argument parser.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use crate::config::Config;
use crate::core::artifacts;
use crate::core::r#box::{Box, Refusal, RefusalReason, Shutdown, Signals, UpError};
use crate::core::clean;
use crate::core::image::{self, Build, Context, ImageError, ImageRequest};
use crate::core::plan::Plan;
use crate::core::profile::{self, Profile};
use crate::core::runtime::{
    BoxInfo, ImageStatus, Runtime, image_status, local_image_id, podman, runtime,
};
use crate::dirs;
use crate::pi::launch::utf8;
use crate::trust;

/// One line per verb from the CLI table in docs/ARCHITECTURE.md, plus the
/// options that answer before a verb is chosen: the syntax, three or more
/// spaces, the description. Printed on stdout by `--help`, on stderr for a
/// malformed invocation, and one line at a time by [`syntax`].
pub const USAGE: &str = "\
pinfold pi [pi args…]            pi in a box for this project; `pi` is a symlink to this
pinfold attach [--box NAME] [cmd…]   bash (or cmd) in this project's running pi box
pinfold build [--profile NAME]   build this project's image, or a profile's; prints the ref
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]… [--no-cache]   a caller's image from its own context; one JSON line
pinfold image rm NAME            retire a caller image name; one JSON line
pinfold allow                    trust this project's .pinfold.toml and Containerfile
pinfold profile new NAME [--from PROFILE] [--from-project [PATH]]   copy a profile to edit as files
pinfold clean [--dry-run] [--unused AGE]   reclaim disk (see Maintenance)
pinfold doctor                   runtime, kernel, image, artifacts, trust, config, disk use
pinfold artifacts                the pinned harnesses as JSON: name, version, path, cached, assets
pinfold config [ROOT]            the effective configuration and project facts as JSON, for callers
pinfold box …                    the process interface
pinfold init                     PID 1 in the box (Linux builds)
pinfold --version                print the version
pinfold --help                   print this usage";

/// The box verbs, which the table above does not spell out.
const BOX_USAGE: &str = "pinfold box up|exec BOX [--tty] [--workdir DIR] -- argv|stat BOX|down BOX|list --label k=v [--label k]|prune";

/// A verb's process exit code: its own, or 1 after printing
/// `pinfold VERB: ERROR` on stderr.
pub fn report(verb: &str, result: io::Result<i32>) -> i32 {
    result.unwrap_or_else(|error| {
        eprintln!("pinfold {verb}: {error}");
        1
    })
}

/// A malformed invocation: `message`, then the verb's syntax from [`syntax`].
fn usage(verb: &str, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}\nusage: {}", syntax(verb)),
    )
}

/// `verb`'s syntax: [`BOX_USAGE`] for box, else its [`USAGE`] line up to the
/// three-space gap. The one place [`usage`] and [`help`] read a syntax line.
fn syntax(verb: &str) -> String {
    if verb == "box" {
        return BOX_USAGE.to_string();
    }
    let prefix = format!("pinfold {verb} ");
    USAGE
        .lines()
        .find(|line| line.starts_with(&prefix))
        .and_then(|line| line.split("   ").next())
        .unwrap_or_default()
        .to_string()
}

/// Whether `args` hold a `--help`/`-h` before any `--`; if so, print
/// `verb`'s syntax from [`syntax`] on stdout, touching no runtime and no
/// state, as top-level `--help` does. `attach` counts one only as its
/// command's first word (`attach ls --help` gives `--help` to `ls`), and
/// `pi`'s arguments are all pi's.
pub fn help(verb: &str, args: &[String]) -> bool {
    let asks = match verb {
        "pi" => false,
        "attach" => {
            matches!(parse_attach(args), Ok((_, [first, ..])) if first == "--help" || first == "-h")
        }
        _ => args
            .iter()
            .take_while(|arg| arg.as_str() != "--")
            .any(|arg| arg == "--help" || arg == "-h"),
    };
    if asks {
        println!("{}", syntax(verb));
    }
    asks
}

/// `pinfold allow`: trust this project's `.pinfold.toml` and Containerfile.
pub fn allow(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("allow", "allow takes no arguments"));
    }
    let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
    crate::trust::allow(&root)?;
    Ok(0)
}

/// `pinfold attach`: bash, or the given command, in this project's running
/// pi box. The box's owner keeps its lifetime; attach streams one exec and
/// adds none of its own.
pub fn attach(args: &[String]) -> io::Result<i32> {
    let (box_name, command) = parse_attach(args)?;
    let mut argv = match command {
        [dashes, rest @ ..] if dashes == "--" => rest.to_vec(),
        _ => command.to_vec(),
    };
    if argv.is_empty() {
        argv.push("bash".to_string());
    }
    let cwd = fs::canonicalize(std::env::current_dir()?)?;
    let root = crate::pi::launch::project_root(&cwd)?;
    let id = crate::pi::state::project_id(&root);
    let runtime = runtime();
    let boxes: Vec<BoxInfo> = runtime
        .list()?
        .into_iter()
        .filter(|box_| box_.labels.get(crate::core::clean::PROJECT_LABEL) == Some(&id))
        .collect();
    let name = select_box(&boxes, box_name)?;
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let init = artifacts::init()?;
    let status = runtime.exec(&name, &init, tty, Some(&cwd), &argv)?;
    Ok(exit_code(status))
}

/// The box to attach to: the named one, or the project's only one. Several
/// without a name is the caller's to resolve.
fn select_box(boxes: &[BoxInfo], requested: Option<&str>) -> io::Result<String> {
    let mut names: Vec<&str> = boxes
        .iter()
        .map(|box_| box_.id.as_str())
        .filter(|id| requested.is_none_or(|name| name == *id))
        .collect();
    names.sort_unstable();
    match (names.as_slice(), requested) {
        ([only], _) => Ok(only.to_string()),
        ([], Some(name)) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no running pi box named {name:?} for this project"),
        )),
        ([], None) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no running pi box for this project; start one with `pinfold pi`",
        )),
        (several, _) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "several pi boxes are running for this project; select one with `--box NAME`: {}",
                several.join(", ")
            ),
        )),
    }
}

/// `attach`'s `--box NAME` and the command after it, which keeps every
/// argument. A leading `--` stays, so [`help`] can tell `attach -- --help`
/// from `attach --help`.
fn parse_attach(args: &[String]) -> io::Result<(Option<&str>, &[String])> {
    match args {
        [flag, name, command @ ..] if flag == "--box" => Ok((Some(name), command)),
        [flag] if flag == "--box" => Err(usage("attach", "--box needs a box name")),
        command => Ok((None, command)),
    }
}

/// `pinfold box`: the process interface.
pub fn run(args: &[String]) -> io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("up") => up(&args[1..]),
        Some("exec") => exec(&args[1..]),
        Some("stat") => stat(&args[1..]),
        Some("down") => {
            crate::core::r#box::down(&single_name(&args[1..], "down")?)?;
            Ok(0)
        }
        Some("list") => list(&args[1..]),
        Some("prune") => prune(&args[1..]),
        Some(verb) => Err(usage("box", &format!("unknown box verb {verb:?}"))),
        None => Err(usage("box", "a box verb is required")),
    }
}

/// Read one spec from stdin, start the box, report it ready, and hold it
/// until stdin closes, SIGTERM arrives, or the box exits.
fn up(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("box", "up takes no arguments"));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        // This process owns the box, so it takes SIGTERM and SIGINT. The
        // handlers come before the daily pass and the spec read, so a signal
        // from the first instant ends the stream with one `down` line, its
        // box null until the spec parsed.
        let mut signals = Signals::new()?;
        let plan = tokio::select! {
            biased;
            _ = signals.recv() => return Ok(down(None, Shutdown::Signal)),
            plan = tokio::task::spawn_blocking(|| {
                clean::maintain();
                Plan::from_reader(io::stdin())
            }) => match plan.map_err(io::Error::other)? {
                Ok(plan) => plan,
                Err(refusal) => return Ok(refused(refusal)),
            }
        };
        // Every error from here on has removed what the start made; it ends
        // the stream as one `failed` line.
        Ok(hold_up(&plan, signals).await.unwrap_or_else(|error| {
            println!(
                "{}",
                serde_json::json!({
                    "event": "failed",
                    "box": &plan.name,
                    "detail": error.to_string(),
                })
            );
            1
        }))
    });
    // `Box::hold` watches stdin through tokio's blocking pool. A caller that
    // keeps stdin open leaves that read parked, and dropping the runtime
    // waits for it forever. Teardown is done, so leak the read and let the
    // process exit. The spec read above parks the same way when a signal
    // ends `up` first.
    runtime.shutdown_background();
    result
}

/// Start the validated box, report it ready, hold it, and print the `down`
/// line. A refusal prints its own line.
async fn hold_up(plan: &Plan, mut signals: Signals) -> io::Result<i32> {
    let init = artifacts::init()?;
    let mut box_ = match Box::up(plan, &init, Some(&mut signals)).await {
        Ok(box_) => box_,
        Err(UpError::Refused(refusal)) => return Ok(refused(refusal)),
        Err(UpError::Signal) => return Ok(down(Some(&plan.name), Shutdown::Signal)),
        Err(UpError::Other(error)) => return Err(error),
    };
    let egress_log = box_
        .egress_log
        .as_deref()
        .map(|path| utf8(path, "egress log"))
        .transpose()?;
    println!(
        "{}",
        serde_json::json!({
            "event": "ready",
            "box": &plan.name,
            "owner": std::process::id(),
            "labels": &box_.labels,
            "image": { "id": &box_.image_id, "ref": &box_.image_ref },
            "egress_log": egress_log,
        })
    );
    io::stdout().flush()?;
    let shutdown = box_.hold(&mut signals).await?;
    // Teardown is done, so the down line names a box the caller can
    // start again. It ends the stream.
    Ok(down(Some(&plan.name), shutdown))
}

/// Print the one `down` line that ends `up`'s stream, why the box ended
/// after teardown, and return `up`'s exit code: 1 for `exited`, else 0.
/// Only `exited` carries a detail, the init's exit status. `name` is null
/// when a signal ended `up` before its spec parsed.
fn down(name: Option<&str>, shutdown: Shutdown) -> i32 {
    let mut line = serde_json::json!({
        "event": "down",
        "box": name,
        "reason": shutdown.as_str(),
    });
    let code = if let Shutdown::BoxExited(status) = shutdown {
        line["detail"] = status.to_string().into();
        1
    } else {
        0
    };
    println!("{line}");
    code
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
fn exec(args: &[String]) -> io::Result<i32> {
    let (name, tty, workdir, argv) = parse_exec(args)?;
    let runtime = runtime();
    if !present(runtime, "exec", &name)? {
        return Ok(3);
    }
    // A TTY only makes sense when both ends are terminals; `--tty` forces it
    // for callers that drive pinfold through their own pty.
    let tty = tty || (io::stdin().is_terminal() && io::stdout().is_terminal());
    let init = artifacts::init()?;
    let status = runtime.exec(&name, &init, tty, workdir.as_deref(), &argv)?;
    Ok(exit_code(status))
}

/// Print one JSON object of a box's runtime facts: OOM kills, memory and
/// pids use and limits. `stat` is what tells a caller an OOM kill from a
/// failure.
fn stat(args: &[String]) -> io::Result<i32> {
    let name = single_name(args, "stat")?;
    let runtime = runtime();
    if !present(runtime, "stat", &name)? {
        return Ok(3);
    }
    let report = serde_json::to_string(&runtime.stat(&name)?).map_err(io::Error::other)?;
    println!("{report}");
    Ok(0)
}

/// Whether the runtime lists box `name`. An absent box is pinfold's own
/// failure, which `exec` and `stat` tell apart from the command's by exit 3.
fn present(runtime: &dyn Runtime, verb: &str, name: &str) -> io::Result<bool> {
    if runtime.list()?.iter().any(|box_| box_.id == name) {
        return Ok(true);
    }
    eprintln!("pinfold box {verb}: no box named {name:?}");
    Ok(false)
}

/// Print one JSON line per box matching every `--label` filter.
fn list(args: &[String]) -> io::Result<i32> {
    let filters = parse_labels(args)?;
    for box_ in runtime().list()? {
        let matches = filters.iter().all(|(key, value)| match value {
            Some(value) => box_.labels.get(key) == Some(value),
            None => box_.labels.contains_key(key),
        });
        if !matches {
            continue;
        }
        let (owner, owner_alive) = clean::owner(&box_)?;
        println!(
            "{}",
            serde_json::json!({
                "name": box_.id,
                "labels": box_.labels,
                "image": { "id": box_.image_id, "ref": box_.image_ref },
                "owner": owner,
                "owner_alive": owner_alive,
                "created": box_.created,
                "state": if box_.running { "running" } else { "stopped" },
            })
        );
    }
    Ok(0)
}

/// Remove boxes pinfold labeled whose owning `box up` process is gone, and
/// print one line per removal.
fn prune(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("box", "prune takes no arguments"));
    }
    for dead in clean::prune_boxes(runtime())? {
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
pub fn clean(args: &[String]) -> io::Result<i32> {
    let (dry_run, unused) = parse_clean(args)?;
    let runtime = runtime();
    let plan = CleanPlan::measure(runtime, unused)?;
    if dry_run {
        println!("pinfold clean: dry run; {} B reclaimable", plan.total());
    } else {
        println!("pinfold clean: reclaiming {} B", plan.total());
    }
    plan.print(runtime);
    if !dry_run {
        plan.remove(runtime)?;
    }
    Ok(0)
}

/// Everything one `clean` pass measures and would remove, measured before
/// anything is removed, so `remove` removes exactly what was printed.
/// `doctor` measures with it too, and removes nothing.
struct CleanPlan {
    dead: Vec<clean::DeadBox>,
    /// Dead boxes' state dirs, leftover socket dirs, unpinned artifact
    /// versions and old egress logs.
    automatic: BTreeSet<PathBuf>,
    caches: Vec<PathBuf>,
    stale: Vec<PathBuf>,
    automatic_bytes: u64,
    project_caches: u64,
    project_state: u64,
}

impl CleanPlan {
    /// Measure the categories without changing anything. `unused` ages
    /// project state as `clean --unused` does.
    fn measure(runtime: &dyn Runtime, unused: Option<Duration>) -> io::Result<CleanPlan> {
        let boxes = clean::boxes(runtime)?;
        let mut automatic: BTreeSet<PathBuf> = boxes
            .dead
            .iter()
            .map(|dead| dead.state_dir.clone())
            .collect();
        automatic.extend(clean::leftover_socket_dirs()?);
        automatic.extend(artifacts::unpinned_versions()?);
        automatic.extend(clean::old_egress_logs()?);

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
                let cache = project.dir.join("home/.cache");
                if cache.exists() {
                    caches.push(cache);
                }
            }
        }

        let automatic_bytes = clean::total_bytes(&automatic);
        let project_caches = clean::total_bytes(&caches);
        let project_state = clean::total_bytes(&stale);
        Ok(CleanPlan {
            dead: boxes.dead,
            automatic,
            caches,
            stale,
            automatic_bytes,
            project_caches,
            project_state,
        })
    }

    /// The bytes a real `clean` would reclaim, the unmeasured build cache
    /// aside.
    fn total(&self) -> u64 {
        self.automatic_bytes + self.project_caches + self.project_state
    }

    /// List the categories, as `clean` and `doctor` both show them.
    fn print(&self, runtime: &dyn Runtime) {
        println!("  automatic maintenance: {} B", self.automatic_bytes);
        println!("  build cache: {}", runtime.build_cache());
        println!("  project caches: {} B", self.project_caches);
        println!("  project state: {} B", self.project_state);
    }

    /// Remove everything the plan measured, then the builds past each
    /// source's newest two. Image sizes are unmeasured: the runtime's image
    /// list carries none.
    fn remove(self, runtime: &dyn Runtime) -> io::Result<()> {
        for dead in &self.dead {
            dead.remove(runtime)?;
        }
        runtime.purge_build_cache()?;
        clean::remove_paths(
            self.automatic
                .into_iter()
                .chain(self.caches)
                .chain(self.stale)
                .collect(),
        )?;
        clean::keep_two_images_per_source(runtime)
    }
}

/// `clean`'s options: `--dry-run`, and `--unused AGE` for state not run for
/// that long.
fn parse_clean(args: &[String]) -> io::Result<(bool, Option<Duration>)> {
    let mut dry_run = false;
    let mut unused = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--unused" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage("clean", "--unused needs an age like 30d"))?;
                unused = Some(parse_age(value)?);
            }
            option => {
                return Err(usage("clean", &format!("unknown clean option {option:?}")));
            }
        }
    }
    Ok((dry_run, unused))
}

/// `AGE` is a whole number and one unit: `s`, `m`, `h` or `d`.
fn parse_age(value: &str) -> io::Result<Duration> {
    let invalid = || {
        usage(
            "clean",
            &format!("invalid age {value:?}; an age is a whole number and a unit, s, m, h or d"),
        )
    };
    let mut chars = value.chars();
    let seconds = match chars.next_back() {
        Some('s') => 1,
        Some('m') => 60,
        Some('h') => 60 * 60,
        Some('d') => 24 * 60 * 60,
        _ => return Err(invalid()),
    };
    chars
        .as_str()
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(seconds))
        .map(Duration::from_secs)
        .ok_or_else(invalid)
}

/// `pinfold doctor`: print one report of what `pinfold pi` depends on and
/// what state it is in. Reads only: nothing is created, downloaded or fixed.
pub fn doctor(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("doctor", "doctor takes no arguments"));
    }
    let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
    // The config must parse; a broken `.pinfold.toml` is itself the answer.
    let config = Config::load(&root)?;
    let project = crate::pi::state::project_id(&root);

    println!("pinfold doctor: {}", root.display());

    let runtime = runtime();
    println!("runtime: {} ({})", runtime.name(), runtime.isolation());
    match runtime.version() {
        Ok(version) => println!("  version: {version}"),
        Err(error) => println!("  version: unavailable: {error}"),
    }
    if runtime.name() == "podman" {
        // preflight is what refuses a host; doctor only reports its verdict.
        match runtime.preflight() {
            Ok(_) => println!("  preflight: ok"),
            Err(error) => println!("  preflight: {error}"),
        }
        match podman::linger() {
            Ok(true) => println!("  linger: enabled"),
            Ok(false) => println!(
                "  linger: disabled; long-lived boxes stop at logout; run `loginctl enable-linger`"
            ),
            Err(error) => println!("  linger: unavailable: {error}"),
        }
    }

    match nix::sys::utsname::uname() {
        Ok(uname) => println!(
            "kernel: {} {}",
            uname.sysname().to_string_lossy(),
            uname.release().to_string_lossy()
        ),
        Err(error) => println!("kernel: unavailable: {error}"),
    }

    let image = crate::pi::launch::resolve_image(&config, &project);
    println!("image: {image}");
    match crate::pi::launch::ensure_image(&config, &image) {
        Ok(()) => println!("  exists"),
        Err(error) => println!("  {error}"),
    }

    match pins_json() {
        Ok(pins) => println!("artifacts: {pins}"),
        Err(error) => println!("artifacts: unavailable: {error}"),
    }

    println!("config: {}", config_report(&root, &config)?);

    println!("disk:");
    match CleanPlan::measure(runtime, None) {
        Ok(plan) => {
            plan.print(runtime);
            let total = plan.total();
            if total > 20 << 30 {
                println!("  {total} B is over 20 GB; run `pinfold clean`");
            }
        }
        Err(error) => println!("  unavailable: {error}"),
    }
    Ok(0)
}

/// `pinfold artifacts`: print the pinned harnesses as one JSON array, an
/// object per harness. Reads only: nothing is downloaded.
pub fn artifacts(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("artifacts", "artifacts takes no arguments"));
    }
    println!("{}", pins_json()?);
    Ok(0)
}

/// The pinned harnesses as one JSON array, an object per harness.
fn pins_json() -> io::Result<serde_json::Value> {
    serde_json::to_value(artifacts::pins()?).map_err(io::Error::other)
}

/// `pinfold config`: print one JSON object, the effective configuration and
/// the project facts a caller needs to compose a box. `ROOT` defaults to the
/// project root `pinfold pi` would use from the current directory. Reads
/// only: nothing is created and nothing is recorded.
pub fn config(args: &[String]) -> io::Result<i32> {
    let root = match args {
        [] => crate::pi::launch::project_root(&std::env::current_dir()?)?,
        [root] => crate::pi::launch::project_root(Path::new(root))?,
        _ => {
            return Err(usage("config", "config takes at most one project root"));
        }
    };
    let config = Config::load(&root)?;
    println!("{}", config_report(&root, &config)?);
    Ok(0)
}

/// The `pinfold config` object for the project rooted at `root`, with its
/// `config` loaded. `doctor` prints the same object.
fn config_report(root: &Path, config: &Config) -> io::Result<serde_json::Value> {
    let project = crate::pi::state::project_id(root);
    let home = crate::pi::state::project_home(root)?;
    let image = crate::pi::launch::resolve_image(config, &project);
    let image_built = !matches!(image_status(runtime(), &image, None)?, ImageStatus::Missing);
    let trust = match trust::check(root, config) {
        Ok(()) => serde_json::json!({ "ok": true, "detail": "ok" }),
        Err(error) => serde_json::json!({ "ok": false, "detail": error.to_string() }),
    };
    Ok(serde_json::json!({
        "root": utf8(root, "project root")?,
        "profile": &config.profile.name,
        "image": image,
        "image_built": image_built,
        "containerfile": config.containerfile.as_ref().map_or("profile", |(path, _)| path.as_str()),
        "egress": { "allow": &config.allow, "routes": &config.routes },
        "protect": &config.protect,
        "cpus": config.cpus,
        "memory": &config.memory,
        "env": &config.env,
        "project": { "id": project, "home": utf8(&home, "project home")? },
        "trust": trust,
    }))
}

/// `exec`'s box name, `--tty`, `--workdir` and the command after `--`.
fn parse_exec(args: &[String]) -> io::Result<(String, bool, Option<PathBuf>, Vec<String>)> {
    let mut args = args.iter();
    let name = args
        .next()
        .ok_or_else(|| usage("box", "exec needs a box name"))?
        .clone();
    let mut tty = false;
    let mut workdir = None;
    loop {
        match args.next().map(String::as_str) {
            Some("--") => break,
            Some("--tty") => tty = true,
            Some("--workdir") => {
                workdir =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        usage("box", "--workdir needs a directory")
                    })?));
            }
            Some(option) => {
                return Err(usage("box", &format!("unknown exec option {option:?}")));
            }
            None => return Err(usage("box", "exec needs `-- argv`")),
        }
    }
    let argv: Vec<String> = args.cloned().collect();
    if argv.is_empty() {
        return Err(usage("box", "exec needs a command after `--`"));
    }
    Ok((name, tty, workdir, argv))
}

fn single_name(args: &[String], verb: &str) -> io::Result<String> {
    match args {
        [name] => Ok(name.clone()),
        _ => Err(usage("box", &format!("{verb} needs exactly one box name"))),
    }
}

/// `list`'s `--label` filters: each key, and the value to match when one was
/// given. Every filter must match.
fn parse_labels(args: &[String]) -> io::Result<Vec<(String, Option<String>)>> {
    let mut filters = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg != "--label" {
            return Err(usage("box", "list takes only --label k=v or --label k"));
        }
        let label = args
            .next()
            .ok_or_else(|| usage("box", "--label needs k=v or k"))?;
        match label.split_once('=') {
            Some((key, value)) => filters.push((key.to_string(), Some(value.to_string()))),
            None => filters.push((label.to_string(), None)),
        }
    }
    if filters.is_empty() {
        return Err(usage("box", "list needs at least one --label"));
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

/// `pinfold build`: `--profile NAME`'s image; else the project's own image
/// when its config names a Containerfile, else the configured profile's.
/// Either builds from an empty context holding only the Containerfile, so
/// trust covers every input of a project build, and a profile directory's
/// other files are not build inputs. Prints the stable ref, or the build's
/// captured output on stderr when it failed.
pub fn build(args: &[String]) -> io::Result<i32> {
    let (profile, project) = match args {
        [flag, name] if flag == "--profile" => (Profile::load(name)?, None),
        [] => {
            let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
            let config = Config::load(&root)?;
            // A project build is a run of its config: an untrusted change
            // stops it like it stops `pinfold pi`.
            trust::check(&root, &config)?;
            let project = config
                .containerfile
                .map(|(_, bytes)| (crate::pi::state::project_id(&root), bytes));
            (config.profile, project)
        }
        _ => return Err(usage("build", "build takes no arguments or --profile NAME")),
    };
    let runtime = runtime();
    // A project image records the profile image it was built from, so
    // `pinfold pi` warns when the profile image moves past it; a profile
    // image records the image its FROM pulls.
    let (label, source, containerfile, base) = match &project {
        Some((id, bytes)) => (
            clean::PROJECT_LABEL,
            id,
            bytes,
            local_image_id(runtime, &profile.image_ref())?,
        ),
        None => (
            clean::PROFILE_LABEL,
            &profile.name,
            &profile.containerfile,
            image::base_digest(runtime, &profile.containerfile)?,
        ),
    };
    let built = image::build(
        runtime,
        Build {
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

/// `pinfold image`: a caller's image build or removal.
pub fn image(args: &[String]) -> io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("build") => Ok(image_build(&args[1..])),
        Some("rm") => image_rm(&args[1..]),
        Some(verb) => Err(usage("image", &format!("unknown image verb {verb:?}"))),
        None => Err(usage("image", "an image verb is required")),
    }
}

/// The most lines of a failed build's output the `failed` line carries.
const FAILED_LOG_LINES: usize = 40;

/// Build a caller's image and print one `built`, `failed` or `refused` line.
fn image_build(args: &[String]) -> i32 {
    let name = args
        .first()
        .map(String::as_str)
        .filter(|arg| !arg.starts_with("--"));
    let result = parse_image_build(name, &args[usize::from(name.is_some())..])
        .map_err(|detail| ImageError::Refused(RefusalReason::Spec, detail))
        .and_then(image::build_image);
    let (line, code) = match result {
        Ok(built) => (
            serde_json::json!({
                "event": "built",
                "image": name,
                "ref": built.reference,
                "latest": built.latest,
                "id": built.id,
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

/// `pinfold image rm NAME`: retire a caller image name: remove every image
/// of NAME that no listed box uses, all its tags, and print the ids removed
/// and the ids a box still uses. A bad name or argument is refused as
/// `spec`, like a build's.
fn image_rm(args: &[String]) -> io::Result<i32> {
    let refused = |name: Option<&str>, detail: String| {
        println!(
            "{}",
            serde_json::json!({
                "event": "refused",
                "image": name,
                "reason": RefusalReason::Spec.as_str(),
                "detail": detail,
            })
        );
        1
    };
    let name = args.first().map(String::as_str);
    let Some(name) = name.filter(|arg| !arg.starts_with("--")) else {
        return Ok(refused(name, "image rm needs an image name".to_string()));
    };
    if args.len() > 1 {
        return Ok(refused(
            Some(name),
            format!("unknown image rm argument {:?}", args[1]),
        ));
    }
    if let Err(error) = profile::check_name("image", name) {
        return Ok(refused(Some(name), error.to_string()));
    }
    let removed = clean::remove_images(runtime(), clean::IMAGE_LABEL, name)?;
    println!(
        "{}",
        serde_json::json!({
            "event": "removed",
            "image": name,
            "ids": removed.ids,
            "in_use": removed.in_use,
        })
    );
    Ok(0)
}

/// `image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]...
/// [--no-cache]`: the name, then the options after it. An `Err` is the
/// refusal's detail.
fn parse_image_build(name: Option<&str>, args: &[String]) -> Result<ImageRequest, String> {
    let name = name.ok_or("image build needs an image name")?.to_string();
    let mut containerfile = None;
    let mut context = None;
    let mut labels = BTreeMap::new();
    let mut cache = true;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--containerfile" => {
                containerfile = Some(PathBuf::from(
                    args.next().ok_or("--containerfile needs a path")?,
                ));
            }
            "--context" => {
                context = Some(PathBuf::from(
                    args.next().ok_or("--context needs a directory")?,
                ));
            }
            "--label" => {
                let (key, value) = args
                    .next()
                    .and_then(|label| label.split_once('='))
                    .filter(|(key, _)| !key.is_empty())
                    .ok_or("--label needs KEY=VALUE")?;
                labels.insert(key.to_string(), value.to_string());
            }
            "--no-cache" => cache = false,
            option => return Err(format!("unknown image build option {option:?}")),
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
pub fn profile(args: &[String]) -> io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("new") => profile_new(&args[1..]).map(|()| 0),
        Some(verb) => Err(usage("profile", &format!("unknown profile verb {verb:?}"))),
        None => Err(usage("profile", "a profile verb is required")),
    }
}

/// Copy a profile's files to `~/.config/pinfold/profiles/NAME/`, refusing to
/// overwrite an existing profile. `--from-project` merges a project's pi
/// agent config into the new profile after the copy.
fn profile_new(args: &[String]) -> io::Result<()> {
    let (name, from, from_project) = parse_profile_new(args)?;
    profile::check_name("profile", &name)?;
    let project_agent = from_project.as_deref().map(project_agent_dir).transpose()?;
    let source = Profile::load(&from);
    let profiles = dirs::config_dir()?.join("profiles");
    fs::create_dir_all(&profiles)?;
    let target = profiles.join(&name);
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
            // The project's `home/.pi/agent/` replaces the seeds of the same
            // path; pi's login, session history, npm install and caches stay
            // behind, and nothing else from the project home is copied.
            Some(agent) => copy_tree(agent, &target.join("home/.pi/agent"), |name| {
                matches!(
                    name.to_str(),
                    Some("auth.json" | "sessions" | "npm" | "cache" | ".cache")
                )
            }),
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
        copy_tree(share, &target.join("share"), |_| false)?;
    }
    Ok(())
}

/// Copy the directories and regular files under `from` into `to`, over
/// what is there, skipping every entry name `skip` accepts at any depth.
fn copy_tree(from: &Path, to: &Path, skip: fn(&OsStr) -> bool) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if skip(&entry.file_name()) {
            continue;
        }
        let target = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target, skip)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn parse_profile_new(args: &[String]) -> io::Result<(String, String, Option<PathBuf>)> {
    let mut name = None;
    let mut from = None;
    let mut from_project = None;
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--from" => {
                from = Some(
                    args.next()
                        .ok_or_else(|| usage("profile", "--from needs a profile name"))?
                        .clone(),
                );
            }
            "--from-project" => {
                // The path is optional; without one, or before another
                // option, the project root of the current directory.
                let path = args.next_if(|value| !value.starts_with("--"));
                from_project = Some(PathBuf::from(path.map_or(".", String::as_str)));
            }
            option if option.starts_with("--") => {
                return Err(usage(
                    "profile",
                    &format!("unknown profile option {option:?}"),
                ));
            }
            value if name.is_none() => name = Some(value.to_string()),
            _ => return Err(usage("profile", "profile new takes one name")),
        }
    }
    let name = name.ok_or_else(|| usage("profile", "profile new needs a name"))?;
    Ok((
        name,
        from.unwrap_or_else(|| "default".to_string()),
        from_project,
    ))
}
