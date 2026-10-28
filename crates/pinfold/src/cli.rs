use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use crate::config::Config;
use crate::core::artifacts;
use crate::core::r#box::{Box, Refusal, RefusalReason, Shutdown, Signals, UpError};
use crate::core::crossing::{self, CrossingError};
use crate::core::image::{self, Build, Context, ImageError, ImageRequest};
use crate::core::plan::{Plan, valid_name};
use crate::core::profile::{self, Profile};
use crate::core::runtime::{Runtime, runtime};
use crate::core::{clean, disk, runs};
use crate::dirs;
use crate::pi::launch::utf8;
use crate::trust;

/// One line per verb from the CLI table in docs/ARCHITECTURE.md but `init`,
/// which only the box runs, plus the options that answer before a verb is chosen: the syntax, three or more
/// spaces, the description. Printed on stdout by `--help`, on stderr for a
/// malformed invocation, and one line at a time by [`syntax`].
pub const USAGE: &str = "\
pinfold pi [pi args…]            pi in a box for this project; `pi` is a symlink to this
pinfold attach [cmd…]            bash (or cmd) in this project's running pi box
pinfold push                     this checkout's branches into its pi box as refs/remotes/host/*
pinfold fetch                    its pi box's branches into refs/remotes/pinfold/*
pinfold build [--profile NAME]   build this project's image, or a profile's; prints the ref
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]… [--no-cache]   a caller's image from its own context; one JSON line
pinfold image rm NAME            retire a caller image name; one JSON line
pinfold allow                    trust this project's .pinfold.toml and Containerfile
pinfold profile new NAME [--from PROFILE]   copy a profile to edit as files
pinfold clean [--dry-run] [--unused AGE]   reclaim unused disk space
pinfold doctor                   runtime, kernel, image, artifacts, trust, config, disk use
pinfold artifacts                pinned harnesses as JSON
pinfold apparmor-profile         the AppArmor profile for this executable (Linux)
pinfold update [--check]         check for a release, or download and install it
pinfold config [ROOT]            effective configuration and project facts as JSON
pinfold box …                    the process interface
pinfold disk list                list managed disks as JSON lines
pinfold disk rm NAME             remove an unattached disk; one JSON line
pinfold --version                print the version
pinfold --help                   print this usage";

/// The process interface's verbs, which the table above does not spell out.
const BOX_USAGE: &str = "\
pinfold box up
pinfold box exec BOX [--tty] -- argv
pinfold box stat BOX
pinfold box down BOX
pinfold box list --label KEY[=VALUE]…
pinfold box prune
pinfold box inspect RUN
pinfold box events RUN [--limit N] [--after CURSOR]
pinfold box fetch BOX --repo GUEST_REPO --into REPO";

pub fn report(verb: &str, result: io::Result<i32>) -> i32 {
    result.unwrap_or_else(|error| {
        eprintln!("pinfold {verb}: {error}");
        1
    })
}

/// A malformed invocation: `message`, then the verb's syntax from [`syntax`].
pub(crate) fn usage(verb: &str, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}\nusage: {}", syntax(verb)),
    )
}

/// `verb`'s syntax: [`BOX_USAGE`] for box, else its [`USAGE`] lines up to the
/// three-space gap. The one place [`usage`] and [`help`] read a syntax line.
fn syntax(verb: &str) -> String {
    match verb {
        "box" => return BOX_USAGE.to_string(),
        "init" => return "pinfold init".to_string(),
        _ => {}
    }
    let prefix = format!("pinfold {verb} ");
    USAGE
        .lines()
        .chain(BOX_USAGE.lines())
        .filter(|line| line.starts_with(&prefix) || *line == prefix.trim_end())
        .filter_map(|line| line.split("   ").next())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `args` hold a `--help`/`-h` before any `--`; if so, print
/// `verb`'s syntax from [`syntax`] on stdout, touching no runtime and no
/// state, as top-level `--help` does. `attach` counts one only as its
/// command's first word (`attach ls --help` gives `--help` to `ls`), and
/// `pi`'s arguments are all pi's.
pub fn help(verb: &str, args: &[String]) -> bool {
    let asks = match verb {
        "pi" => false,
        "attach" => matches!(args.first(), Some(first) if first == "--help" || first == "-h"),
        _ => args
            .iter()
            .take_while(|arg| arg.as_str() != "--")
            .any(|arg| arg == "--help" || arg == "-h"),
    };
    if asks {
        let specific = args
            .first()
            .map(|sub| syntax(&format!("{verb} {sub}")))
            .filter(|line| !line.is_empty());
        println!("{}", specific.unwrap_or_else(|| syntax(verb)));
    }
    asks
}

pub fn allow(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("allow", "allow takes no arguments"));
    }
    let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
    crate::trust::allow(&root)?;
    Ok(0)
}

/// `pinfold attach`: bash, or the given command, in this project's running
/// pi box, the box that attaches its project disk. The box's owner keeps
/// its lifetime; attach streams one exec and adds none of its own.
pub fn attach(args: &[String]) -> io::Result<i32> {
    let mut argv = match args {
        [dashes, rest @ ..] if dashes == "--" => rest.to_vec(),
        _ => args.to_vec(),
    };
    if argv.is_empty() {
        argv.push("bash".to_string());
    }
    let cwd = fs::canonicalize(std::env::current_dir()?)?;
    let root = crate::pi::launch::project_root(&cwd)?;
    let [_, project] = crate::pi::state::disk_names(&crate::pi::state::project_id(&root));
    let name = match crate::pi::launch::project_box(&project)? {
        crate::pi::launch::Holder::Ready(name) => name,
        crate::pi::launch::Holder::Starting(name) => {
            return Err(io::Error::other(format!(
                "disk-in-use: {}",
                crate::pi::launch::starting(&name, &project)
            )));
        }
        crate::pi::launch::Holder::None => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no running pi box for this project; start one with `pinfold pi`",
            ));
        }
    };
    let runtime = runtime();
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let init = artifacts::init()?;
    let status = runtime.exec(&name, &init, tty, Some(&cwd), &argv)?;
    Ok(exit_code(status))
}

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
        Some("inspect") => inspect(&args[1..]),
        Some("events") => events(&args[1..]),
        Some("fetch") => fetch(&args[1..]),
        Some(verb) => Err(usage("box", &format!("unknown box verb {verb:?}"))),
        None => Err(usage("box", "a box verb is required")),
    }
}

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
            _ = signals.recv() => return Ok(down(None, None, Shutdown::Signal)),
            plan = tokio::task::spawn_blocking(|| {
                clean::maintain();
                Plan::from_reader(io::stdin())
            }) => match plan.map_err(io::Error::other)? {
                Ok(plan) => plan,
                Err(refusal) => return Ok(refused_box(&refusal)),
            }
        };
        // Every error from here on has removed what the start made; it ends
        // the stream as one `failed` line.
        match hold_up(&plan, signals).await {
            Ok(code) => Ok(code),
            Err(error) => {
                writeln!(
                    io::stdout(),
                    "{}",
                    serde_json::json!({
                        "event": "failed",
                        "box": &plan.name,
                        "detail": error.to_string(),
                    })
                )?;
                Ok(1)
            }
        }
    });
    // `Box::hold` watches stdin through tokio's blocking pool. A caller that
    // keeps stdin open leaves that read parked, and dropping the runtime
    // waits for it forever. Teardown is done, so leak the read and let the
    // process exit. The spec read above parks the same way when a signal
    // ends `up` first.
    runtime.shutdown_background();
    result
}

async fn hold_up(plan: &Plan, mut signals: Signals) -> io::Result<i32> {
    let init = artifacts::init()?;
    let mut box_ = match Box::up(plan, &init, &mut signals).await {
        Ok(box_) => box_,
        Err(UpError::Refused(refusal)) => return Ok(refused_box(&refusal)),
        Err(UpError::Signal | UpError::Control) => {
            return Ok(down(Some(&plan.name), None, Shutdown::Signal));
        }
        Err(UpError::Other(error)) => return Err(error),
    };
    let ready = (|| {
        let egress_log = box_
            .egress_log
            .as_deref()
            .map(|path| utf8(path, "egress log"))
            .transpose()?;
        let line = serde_json::to_string(&serde_json::json!({
            "event": "ready",
            "box": &plan.name,
            "run_id": &box_.run_id,
            "owner": std::process::id(),
            "labels": &box_.labels,
            "image": { "id": &box_.image_id, "ref": &box_.image_ref },
            "egress_log": egress_log,
        }))
        .map_err(io::Error::other)?;
        let mut stdout = io::stdout();
        writeln!(stdout, "{line}")?;
        stdout.flush()
    })();
    if let Err(error) = ready {
        // No ready reached the caller: a failed start retains no run.
        // Removed even when teardown fails, as `Box::up`'s abort does.
        let down = box_.down(None).await;
        if let Err(cleanup) = crate::core::runs::remove(&box_.run_id) {
            eprintln!("pinfold: startup cleanup: {cleanup}");
        }
        down?;
        return Err(error);
    }
    let shutdown = box_.hold(&mut signals).await?;
    Ok(down(Some(&plan.name), Some(&box_.run_id), shutdown))
}

/// `name` is null when a signal ends `up` before its spec parses, and
/// `run_id` when `up` ends before ready.
fn down(name: Option<&str>, run_id: Option<&str>, shutdown: Shutdown) -> i32 {
    let mut line = serde_json::json!({
        "event": "down",
        "box": name,
        "run_id": run_id,
        "reason": shutdown.as_str(),
    });
    if let Some(detail) = shutdown.detail() {
        line["detail"] = detail.into();
    }
    let code = match shutdown {
        Shutdown::BoxExited(_) | Shutdown::AuditLog => 1,
        Shutdown::StdinEof | Shutdown::Signal => 0,
    };
    println!("{line}");
    code
}

fn refused(kind: &str, name: Option<&str>, reason: RefusalReason, detail: &str) -> i32 {
    println!(
        "{}",
        serde_json::json!({
            "event": "refused",
            kind: name,
            "reason": reason,
            "detail": detail,
        })
    );
    1
}

fn refused_box(refusal: &Refusal) -> i32 {
    refused(
        "box",
        refusal.box_name.as_deref(),
        refusal.reason,
        &refusal.detail,
    )
}

fn exec(args: &[String]) -> io::Result<i32> {
    let (name, tty, argv) = parse_exec(args)?;
    let runtime = runtime();
    if !present(runtime, "exec", &name)? {
        return Ok(3);
    }
    // A TTY only makes sense when both ends are terminals; `--tty` forces it
    // for callers that drive pinfold through their own pty.
    let tty = tty || (io::stdin().is_terminal() && io::stdout().is_terminal());
    let init = artifacts::init()?;
    let status = runtime.exec(&name, &init, tty, None, &argv)?;
    Ok(exit_code(status))
}

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

/// `box fetch`: one JSON line, exit 0, 1 on a refusal or a broken
/// transfer, 3 on an absent box. The remote is the box's name.
fn fetch(args: &[String]) -> io::Result<i32> {
    let (name, repo, into) = parse_fetch(args)?;
    let runtime = runtime();
    if !present(runtime, "fetch", &name)? {
        return Ok(3);
    }
    let init = artifacts::init()?;
    let result = crossing::fetch(runtime, &init, &name, &repo, Path::new(&into), &name);
    Ok(report_crossing("fetch", &name, &name, result))
}

pub(crate) fn report_crossing(
    verb: &str,
    name: &str,
    remote: &str,
    result: Result<crossing::Refs, CrossingError>,
) -> i32 {
    match result {
        Ok(refs) => {
            println!(
                "{}",
                serde_json::json!({
                    "event": if verb == "push" { "pushed" } else { "fetched" },
                    "box": name,
                    "remote": remote,
                    "refs": refs,
                })
            );
            0
        }
        Err(CrossingError::Refused(reason, detail)) => refused("box", Some(name), reason, &detail),
        Err(CrossingError::Failed(detail)) => {
            println!(
                "{}",
                serde_json::json!({ "event": "failed", "box": name, "detail": detail })
            );
            1
        }
    }
}

fn parse_fetch(args: &[String]) -> io::Result<(String, String, String)> {
    let (name, mut rest) = match args {
        [name, rest @ ..] if !name.starts_with('-') => (name.clone(), rest),
        _ => return Err(usage("box", "fetch needs a box name")),
    };
    let (mut repo, mut into) = (None, None);
    while let [option, value, tail @ ..] = rest {
        let slot = match option.as_str() {
            "--repo" => &mut repo,
            "--into" => &mut into,
            _ => return Err(usage("box", &format!("unknown fetch option {option:?}"))),
        };
        if slot.replace(value.clone()).is_some() {
            return Err(usage("box", &format!("fetch takes {option} once")));
        }
        rest = tail;
    }
    if let [option] = rest {
        return Err(usage(
            "box",
            &format!("fetch option {option:?} needs a value"),
        ));
    }
    match (repo, into) {
        (Some(repo), Some(into)) => Ok((name, repo, into)),
        _ => Err(usage("box", "fetch needs --repo and --into")),
    }
}

/// Whether the runtime lists pinfold box `name`. An absent box is pinfold's
/// own failure, which `exec` and `stat` tell apart from the command's by
/// exit 3.
fn present(runtime: &dyn Runtime, verb: &str, name: &str) -> io::Result<bool> {
    let found = runtime
        .list()?
        .iter()
        .any(|box_| box_.id == name && clean::pinfold_box(box_));
    if !found {
        eprintln!("pinfold box {verb}: no box named {name:?}");
    }
    Ok(found)
}

fn list(args: &[String]) -> io::Result<i32> {
    let filters = parse_labels(args)?;
    for box_ in runtime().list()? {
        if !clean::pinfold_box(&box_) {
            continue;
        }
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

fn prune(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("box", "prune takes no arguments"));
    }
    let runtime = runtime();
    let boxes = clean::boxes(runtime)?;
    let removed = clean::prune_boxes(runtime, &boxes)?;
    for dead in &removed {
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

fn inspect(args: &[String]) -> io::Result<i32> {
    let [run_id] = args else {
        return Err(usage("box", "inspect needs exactly one run id"));
    };
    let run_id = parse_run_id(run_id)?;
    answer("inspect", run_id, runs::inspect(run_id)?)
}

/// `box events RUN [--limit N] [--after CURSOR]`: one page of the run's
/// egress log. Every argument is checked before any path is built.
fn events(args: &[String]) -> io::Result<i32> {
    let Some((run_id, options)) = args.split_first() else {
        return Err(usage("box", "events needs a run id"));
    };
    let run_id = parse_run_id(run_id)?;
    let mut limit = runs::MAX_EVENTS;
    let mut after = None;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let value = options.next();
        match (option.as_str(), value) {
            ("--limit", Some(value)) => {
                limit = value
                    .parse()
                    .ok()
                    .filter(|limit| (1..=runs::MAX_EVENTS).contains(limit))
                    .ok_or_else(|| usage("box", &format!("--limit {value:?} is not 1 to 100")))?;
            }
            ("--after", Some(value)) => {
                after = Some(runs::Cursor::parse(run_id, value).ok_or_else(|| {
                    usage(
                        "box",
                        &format!("--after {value:?} is not a cursor of this run"),
                    )
                })?);
            }
            _ => return Err(usage("box", &format!("unknown events option {option:?}"))),
        }
    }
    answer("events", run_id, runs::events(run_id, limit, after)?)
}

fn parse_run_id(run_id: &str) -> io::Result<&str> {
    if runs::valid_id(run_id) {
        Ok(run_id)
    } else {
        Err(usage(
            "box",
            &format!("run id {run_id:?} is not 64 lowercase hex characters"),
        ))
    }
}

fn answer(verb: &str, run_id: &str, answer: Option<serde_json::Value>) -> io::Result<i32> {
    match answer {
        Some(answer) => {
            println!("{answer}");
            Ok(0)
        }
        None => {
            eprintln!("pinfold box {verb}: no run {run_id:?}");
            Ok(3)
        }
    }
}

pub fn disk(args: &[String]) -> io::Result<i32> {
    let runtime = runtime();
    match args {
        [verb] if verb == "list" => {
            let attached = disk::attachments(runtime, None)?;
            for found in runtime.list_disks()? {
                let boxes: Vec<_> = attached
                    .get(&found.name)
                    .into_iter()
                    .flatten()
                    .map(
                        |(name, readonly)| serde_json::json!({ "box": name, "readonly": readonly }),
                    )
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "name": found.name,
                        "size": found.size,
                        "allocated": found.allocated,
                        "attached": boxes,
                        "clean": !disk::unclean(&found.name)?,
                    })
                );
            }
            Ok(0)
        }
        [verb, name] if verb == "rm" => {
            if !valid_name(name) {
                return Ok(refused(
                    "disk",
                    Some(name),
                    RefusalReason::Spec,
                    &format!(
                        "disk name {name:?} must start alphanumeric and hold only [A-Za-z0-9._-]"
                    ),
                ));
            }
            match disk::remove(runtime, name)? {
                disk::Removal::InUse(holder) => Ok(refused(
                    "disk",
                    Some(name),
                    RefusalReason::DiskInUse,
                    &format!("box {holder:?} attaches disk {name:?}"),
                )),
                disk::Removal::Removed { existed } => {
                    println!(
                        "{}",
                        serde_json::json!({ "event": "removed", "disk": name, "existed": existed })
                    );
                    Ok(0)
                }
            }
        }
        _ => Err(usage("disk", "disk takes list, or rm and one name")),
    }
}

pub fn clean(args: &[String]) -> io::Result<i32> {
    let (dry_run, unused) = parse_clean(args)?;
    let runtime = runtime();
    let plan = CleanPlan::measure(runtime, unused)?;
    let total = plan.total();
    let action = if dry_run {
        format!("dry run; {total} B measured")
    } else {
        format!("{total} B measured; removing unused data")
    };
    println!("pinfold clean: {action}");
    plan.print();
    if !dry_run {
        plan.remove(runtime)?;
    }
    Ok(0)
}

/// Disk categories and paths for one `clean` pass, measured before removal.
/// `doctor` measures with it too, and removes nothing.
struct CleanPlan {
    /// The pass's box list from `measure`: dead boxes to remove, and live
    /// projects to protect.
    boxes: clean::Boxes,
    /// Unpinned artifact versions. Dead boxes' state dirs and leftover
    /// socket dirs count in `automatic_bytes` only.
    automatic: Vec<PathBuf>,
    /// Retired runs' directories.
    runs: Vec<PathBuf>,
    run_bytes: u64,
    /// Projects `--unused` selected and their disks, by project id.
    project_disks: BTreeMap<String, Vec<String>>,
    /// Every disk's allocated bytes, a category of its own.
    disk_bytes: u64,
    /// Recorded checkouts that are gone or moved; their state and disks
    /// stay until `--unused` or `disk rm`.
    moved: Vec<PathBuf>,
    automatic_bytes: u64,
    project_state: u64,
    build_cache_bytes: Option<u64>,
    unused: Option<Duration>,
}

impl CleanPlan {
    fn measure(runtime: &dyn Runtime, unused: Option<Duration>) -> io::Result<CleanPlan> {
        let boxes = clean::boxes(runtime)?;
        let box_paths: BTreeSet<PathBuf> = boxes
            .dead
            .iter()
            .map(|dead| dead.state_dir.clone())
            .chain(clean::leftover_socket_dirs()?)
            .collect();
        let automatic = artifacts::unpinned_versions()?;
        let runs = runs::retired()?;

        let disks = runtime.list_disks()?;
        let mut project_disks = BTreeMap::new();
        let mut project_state = 0;
        let mut moved = Vec::new();
        for project in crate::pi::state::state_dirs()? {
            if !project.root.exists() {
                moved.push(project.root.clone());
            }
            if Self::idle(&project, &boxes.live_projects, unused) {
                project_state += clean::path_bytes(&project.dir);
                let names: Vec<String> = crate::pi::state::disk_names(&project.id)
                    .into_iter()
                    .filter(|name| disks.iter().any(|disk| &disk.name == name))
                    .collect();
                project_disks.insert(project.id, names);
            }
        }

        Ok(CleanPlan {
            boxes,
            build_cache_bytes: image::service().build_cache_bytes()?,
            automatic_bytes: automatic
                .iter()
                .chain(&box_paths)
                .map(|path| clean::path_bytes(path))
                .sum(),
            project_state,
            automatic,
            run_bytes: runs.iter().map(|path| clean::path_bytes(path)).sum(),
            runs,
            project_disks,
            disk_bytes: disks.iter().map(|disk| disk.allocated).sum(),
            moved,
            unused,
        })
    }

    /// Measured disk use. Build-cache backing storage includes filesystem
    /// overhead and active records, so pruning may reclaim less.
    fn total(&self) -> u64 {
        self.automatic_bytes
            + self.run_bytes
            + self.project_state
            + self.disk_bytes
            + self.build_cache_bytes.unwrap_or(0)
    }

    fn print(&self) {
        println!("  automatic maintenance: {} B", self.automatic_bytes);
        println!("  run records: {} B", self.run_bytes);
        match self.build_cache_bytes {
            Some(bytes) => println!(
                "  build cache: {bytes} B; {}",
                image::service().build_cache()
            ),
            None => println!(
                "  build cache: unmeasured; {}",
                image::service().build_cache()
            ),
        }
        println!("  project state: {} B", self.project_state);
        println!("  disks: {} B", self.disk_bytes);
        for root in &self.moved {
            println!("  checkout gone, state and disks kept: {}", root.display());
        }
    }

    /// Remove everything the plan measured, then the builds past each
    /// source's newest two. Image sizes are unmeasured: the runtime's image
    /// list carries none.
    fn remove(self, runtime: &dyn Runtime) -> io::Result<()> {
        clean::prune_boxes(runtime, &self.boxes)?;
        clean::prune_leftover_states(runtime)?;
        image::service().purge_build_cache()?;
        clean::remove_paths(self.automatic)?;
        clean::remove_paths(self.runs)?;
        let mut locks = BTreeMap::new();
        for id in self.project_disks.keys() {
            if let Some(lock) = crate::core::ownership::try_project_lock(id)? {
                locks.insert(id.as_str(), lock);
            }
        }
        if !locks.is_empty() {
            // All candidates stay locked through this one fresh snapshot.
            let live = clean::boxes(runtime)?.live_projects;
            for project in crate::pi::state::state_dirs()? {
                if !locks.contains_key(project.id.as_str()) {
                    continue;
                }
                if Self::idle(&project, &live, self.unused) {
                    clean::remove_paths(vec![project.dir.clone()])?;
                    // A disk a box attaches stays; `disk::remove` checks.
                    for name in self.project_disks.get(&project.id).into_iter().flatten() {
                        disk::remove(runtime, name)?;
                    }
                }
            }
        }
        drop(locks);
        clean::keep_two_images(image::service(), None)
    }

    /// Whether `--unused` selects the project: not run for that long and no
    /// live box. A moved checkout alone keeps its state and disks.
    fn idle(
        project: &crate::pi::state::StateDir,
        live: &BTreeSet<String>,
        unused: Option<Duration>,
    ) -> bool {
        let idle = crate::core::now().saturating_sub(project.last_run);
        !live.contains(&project.id) && unused.is_some_and(|unused| idle >= unused.as_secs())
    }
}

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
    println!("runtime: {}", runtime.describe());
    match runtime.version() {
        Ok(version) => println!("  version: {version}"),
        Err(error) => println!("  version: unavailable: {error}"),
    }
    let runtime_ready = match runtime.preflight() {
        Ok(_) => {
            println!("  preflight: ok");
            true
        }
        Err(error) => {
            println!("  preflight: {error}");
            #[cfg(target_os = "linux")]
            for requirement in crate::core::runtime::firecracker::preflight::host_requirements() {
                println!("  {requirement}");
            }
            false
        }
    };

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
    if runtime_ready {
        match crate::pi::launch::ensure_image(&config, &image) {
            Ok(()) => println!("  exists"),
            Err(error) => println!("  {error}"),
        }
    } else {
        println!("  unavailable: runtime check failed");
    }

    match pins_json() {
        Ok(pins) => println!("artifacts: {pins}"),
        Err(error) => println!("artifacts: unavailable: {error}"),
    }

    println!("config: {}", config_report(&root, &config)?);

    println!("disk:");
    if !runtime_ready {
        println!("  unavailable: runtime check failed");
        return Ok(0);
    }
    match CleanPlan::measure(runtime, None) {
        Ok(plan) => {
            plan.print();
            let total = plan.total();
            if total > 20 << 30 {
                println!("  {total} B is over 20 GB; run `pinfold clean`");
            }
        }
        Err(error) => println!("  unavailable: {error}"),
    }
    Ok(0)
}

pub fn artifacts(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("artifacts", "artifacts takes no arguments"));
    }
    println!("{}", pins_json()?);
    Ok(0)
}

/// An unconfined profile that grants only `userns`, named for and attached
/// to this executable's path, so the VMM jail works where AppArmor restricts
/// unprivileged user namespaces.
pub fn apparmor_profile(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage(
            "apparmor-profile",
            "apparmor-profile takes no arguments",
        ));
    }
    if !cfg!(target_os = "linux") {
        return Err(io::Error::other("AppArmor profiles are for Linux hosts"));
    }
    let exe = std::env::current_exe()?;
    // Pattern characters would attach the profile to other paths too.
    let path = exe
        .to_str()
        .filter(|path| {
            !path
                .chars()
                .any(|c| c.is_control() || "\"\\*?[]{}^".contains(c))
        })
        .ok_or_else(|| {
            io::Error::other(format!(
                "{} cannot be named in an AppArmor profile",
                exe.display()
            ))
        })?;
    println!("abi <abi/4.0>,\n\n\"{path}\" flags=(unconfined) {{\n  userns,\n}}");
    Ok(0)
}

fn pins_json() -> io::Result<serde_json::Value> {
    serde_json::to_value(artifacts::pins()?).map_err(io::Error::other)
}

/// Read-only: do not materialize profiles or record project state.
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

fn config_report(root: &Path, config: &Config) -> io::Result<serde_json::Value> {
    let project = crate::pi::state::project_id(root);
    let [home_disk, project_disk] = crate::pi::state::disk_names(&project);
    let image = crate::pi::launch::resolve_image(config, &project);
    let (image_built, image_error) = match image::service().resolve_image(&image) {
        Ok(resolved) => (Some(resolved.is_ok()), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let trust = match trust::check(root, config) {
        Ok(()) => serde_json::json!({ "ok": true, "detail": "ok" }),
        Err(error) => serde_json::json!({ "ok": false, "detail": error.to_string() }),
    };
    Ok(serde_json::json!({
        "root": utf8(root, "project root")?,
        "profile": &config.profile.name,
        "image": image,
        "image_source": config.image_source(),
        "image_built": image_built,
        "image_error": image_error,
        "containerfile": if config.image.is_some() { None } else { Some(config.containerfile.as_ref().map_or("profile", |(path, _)| path.as_str())) },
        "egress": { "allow": &config.allow, "routes": &config.routes },
        "cpus": config.cpus,
        "memory": &config.memory,
        "env": &config.env,
        "project": {
            "id": project,
            "home": crate::pi::state::HOME,
            "disks": { "home": home_disk, "project": project_disk },
        },
        "trust": trust,
    }))
}

fn parse_exec(args: &[String]) -> io::Result<(String, bool, Vec<String>)> {
    let mut args = args.iter();
    let name = args
        .next()
        .ok_or_else(|| usage("box", "exec needs a box name"))?
        .clone();
    let mut tty = false;
    loop {
        match args.next().map(String::as_str) {
            Some("--") => break,
            Some("--tty") => tty = true,
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
    Ok((name, tty, argv))
}

fn single_name(args: &[String], verb: &str) -> io::Result<String> {
    match args {
        [name] => Ok(name.clone()),
        _ => Err(usage("box", &format!("{verb} needs exactly one box name"))),
    }
}

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
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// Containerfile-only contexts keep project builds covered by trust.
/// Other files in a profile directory are not build inputs.
pub fn build(args: &[String]) -> io::Result<i32> {
    let (profile, project) = match args {
        [flag, name] if flag == "--profile" => (Profile::load(name)?, None),
        [] => {
            let root = crate::pi::launch::project_root(&std::env::current_dir()?)?;
            let config = Config::load(&root)?;
            // A project build is a run of its config: an untrusted change
            // stops it like it stops `pinfold pi`.
            trust::check(&root, &config)?;
            if config.image.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "image-no-recipe: an explicit image is selected; there is nothing to build",
                ));
            }
            let project = config
                .containerfile
                .map(|(_, bytes)| (crate::pi::state::project_id(&root), bytes));
            (config.profile, project)
        }
        _ => return Err(usage("build", "build takes no arguments or --profile NAME")),
    };
    let profile_recipe = profile.containerfile.as_deref();
    if project.is_none() && profile_recipe.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "image-no-recipe: profile has no Containerfile; there is nothing to build",
        ));
    }
    let images = image::service();
    // A project image records the profile image it was built from, so
    // `pinfold pi` warns when the profile image moves past it; a profile
    // image records the image its FROM pulls.
    let (label, source, containerfile, base) = match &project {
        Some((id, bytes)) => (
            clean::PROJECT_LABEL,
            id,
            bytes.as_slice(),
            if profile_recipe.is_some() {
                images
                    .resolve_image(&profile.image_ref())?
                    .ok()
                    .map(|image| image.id)
            } else {
                image::base_digest(images, bytes)?
            },
        ),
        None => {
            let bytes = profile_recipe.expect("a profile build has a recipe");
            (
                clean::PROFILE_LABEL,
                &profile.name,
                bytes,
                image::base_digest(images, bytes)?,
            )
        }
    };
    let built = image::build(
        images,
        Build {
            label: Some(label),
            source,
            context: Context::Alone(containerfile),
            labels: base
                .map(|digest| (clean::BASE_LABEL.to_string(), digest))
                .into_iter()
                .collect(),
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
                images.program()
            )))
        }
    }
}

pub fn image(args: &[String]) -> io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("build") => Ok(image_build(&args[1..])),
        Some("rm") => image_rm(&args[1..]),
        Some(verb) => Err(usage("image", &format!("unknown image verb {verb:?}"))),
        None => Err(usage("image", "an image verb is required")),
    }
}

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
        Err(ImageError::Failed(output)) => (
            serde_json::json!({ "event": "failed", "image": name, "log": output.lines().collect::<Vec<_>>() }),
            1,
        ),
        Err(ImageError::Refused(reason, detail)) => {
            return refused("image", name, reason, &detail);
        }
    };
    println!("{line}");
    code
}

fn image_rm(args: &[String]) -> io::Result<i32> {
    let refused = |name, detail: String| refused("image", name, RefusalReason::Spec, &detail);
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
    let removed = clean::remove_images(image::service(), name)?;
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

pub fn profile(args: &[String]) -> io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("new") => profile_new(&args[1..]).map(|()| 0),
        Some(verb) => Err(usage("profile", &format!("unknown profile verb {verb:?}"))),
        None => Err(usage("profile", "a profile verb is required")),
    }
}

fn profile_new(args: &[String]) -> io::Result<()> {
    let (name, from) = parse_profile_new(args)?;
    profile::check_name("profile", &name)?;
    let source = Profile::load(&from);
    let profiles = dirs::config_dir()?.join("profiles");
    fs::create_dir_all(&profiles)?;
    let target = profiles.join(&name);
    fs::create_dir(&target).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("profile {name:?} at {}: {error}", target.display()),
        )
    })?;
    let result = source.and_then(|source| profile::write_profile(&source, &target));
    if result.is_err() {
        // Do not leave a half-written profile behind to load or block a retry.
        let _ = fs::remove_dir_all(&target);
    }
    result
}

fn parse_profile_new(args: &[String]) -> io::Result<(String, String)> {
    let mut name = None;
    let mut from = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--from" => {
                from = Some(
                    args.next()
                        .ok_or_else(|| usage("profile", "--from needs a profile name"))?
                        .clone(),
                );
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
    Ok((name, from.unwrap_or_else(|| "default".to_string())))
}
