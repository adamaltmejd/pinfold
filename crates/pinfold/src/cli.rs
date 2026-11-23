use crate::core::process::CommandGuard;
use std::collections::{BTreeMap, BTreeSet};

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::config::Config;
use crate::core::artifacts;
use crate::core::r#box::{Box, Refusal, RefusalReason, Shutdown, Signals, UpError};
use crate::core::image::{self, Build, Context, ImageError, ImageRequest};
use crate::core::plan::{Plan, Route, valid_name};
use crate::core::profile::{self, Profile};
use crate::core::runtime::{BoxInfo, Runtime, runtime};
use crate::core::{clean, disk, runs};
use crate::dirs;
use crate::formatting::terminal_text;
use crate::pi::launch::utf8;

/// Shadows std's `println!` here and where imported, which panics when the
/// reader of stdout has gone (#359). A report then dies of SIGPIPE, as a
/// Unix filter's writer does; a command that [`finishes_without_reader`]
/// drops the line and carries on. SIGPIPE stays ignored for everything
/// else: owners, relays, sessions and the Git remote helper handle EPIPE
/// on their own pipes and sockets.
macro_rules! println {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        $crate::cli::printed(writeln!(::std::io::stdout(), $($arg)*))
    }};
}
pub(crate) use println;

static FINISHES: AtomicBool = AtomicBool::new(false);

/// For commands that change state or answer the process interface: a
/// closed stdout must not cut short their work or change their status.
pub fn finishes_without_reader() {
    FINISHES.store(true, Ordering::Relaxed);
}

pub(crate) fn printed(result: io::Result<()>) {
    match result {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
            if FINISHES.load(Ordering::Relaxed) {
                return;
            }
            use nix::sys::signal::{SigHandler, Signal, raise, signal};
            // SAFETY: the default disposition installs no handler.
            let _ = unsafe { signal(Signal::SIGPIPE, SigHandler::SigDfl) };
            let _ = raise(Signal::SIGPIPE);
            // An inherited signal mask can block SIGPIPE.
            std::process::exit(141);
        }
        Err(error) => panic!("failed printing to stdout: {error}"),
    }
}

pub fn print_intro() {
    let mut unconfigured = false;
    match project_intro(&mut unconfigured) {
        Ok(Some(git_disk)) => {
            println!("\nRun a command    pinfold -- CMD\nKeep it running  pinfold start");
            if let Some(root) = git_disk {
                let remote = crate::pi::remote::remote_name(&root).unwrap_or_else(|| {
                    println!(
                        "Add Git remote   git remote add box {}",
                        crate::pi::remote::pinfold_url(&root)
                    );
                    "box".to_string()
                });
                println!(
                    "Send a branch    git push {remote} BRANCH\nBring work back  git fetch {remote}"
                );
            }
            println!("Change settings  pinfold config edit\nAll commands     pinfold --help");
        }
        Ok(None) => println!(
            "\nSet up and run     pinfold -- CMD     (e.g. pinfold -- claude)\nChange settings    pinfold config edit   (creates the settings file)\nAll commands       pinfold --help"
        ),
        Err(error) => {
            println!(
                "Project details unavailable. {}",
                terminal_text(&error.to_string())
            );
            println!("\nRun a command    pinfold -- CMD");
            if unconfigured {
                println!("Change settings  pinfold config edit   (creates the settings file)");
            }
            println!("All commands     pinfold --help");
        }
    }
}

/// `None` without a configuration; otherwise the root of a Git disk
/// project, whose branches cross through the remote `box`.
/// `unconfigured` is set once known, so a later failure's footer has it.
fn project_intro(unconfigured: &mut bool) -> io::Result<Option<Option<PathBuf>>> {
    use crate::pi::session::{ProjectStatus, project_status};

    let context = crate::pi::context::Context::discover(&std::env::current_dir()?)?;
    let root = &context.root;
    println!("Project   {}", terminal_text(&root.to_string_lossy()));
    let loaded = Config::load(root);
    let configured = !loaded.as_ref().is_err_and(|error| {
        error
            .get_ref()
            .is_some_and(|e| e.is::<crate::config::NoConfig>())
    });
    *unconfigured = !configured;
    let status = project_status(root);
    match &status {
        Ok(ProjectStatus::Stopped) if !configured => println!("Box       not set up"),
        Ok(ProjectStatus::Stopped) => println!("Box       stopped"),
        Ok(ProjectStatus::Starting) => println!("Box       starting"),
        Ok(ProjectStatus::Stopping) => println!("Box       stopping"),
        Ok(ProjectStatus::Running { profile, mount }) => {
            match profile {
                Some(profile) => {
                    println!("Box       running with profile {}", terminal_text(profile))
                }
                None => println!("Box       running"),
            }
            println!("Box files {}", project_files(*mount, root));
        }
        Ok(ProjectStatus::Unavailable) | Err(_) => println!("Box       status unavailable"),
    }
    if matches!(status, Ok(ProjectStatus::Running { .. })) {
        println!("\nThe next launch uses these settings.");
    }
    let config = if configured {
        loaded?
    } else {
        println!(
            "\nThe first run asks you to confirm{}:",
            if cfg!(target_os = "macos") {
                ", and lets you pick a disk instead"
            } else {
                ""
            }
        );
        Config::first_run_preview(root)?
    };
    context.check_metadata(config.mount)?;
    config.check_host_paths(root)?;
    // A preview has no saved profile for PINFOLD_PROFILE to override.
    let profile = if configured {
        profile_description(&config)
    } else {
        terminal_text(&config.profile.name)
    };
    println!("Profile   {profile}");
    println!("Files     {}", project_files(config.mount, root));
    let reference = crate::pi::launch::resolve_image(&config, &crate::pi::state::project_id(root));
    // Only a definite absence earns a line; an unknown answer stays quiet.
    if image_built(&image::service().resolve_image(&reference)).0 == Some(false) {
        match (&config.image, configured) {
            (Some(image), _) => println!(
                "Image     {} is not available locally. Make it available before running.",
                terminal_text(image)
            ),
            (None, false) => println!("Image     not built yet. The first run offers to build it."),
            (None, true) => println!("Image     not built yet. Build it with `pinfold build`."),
        }
    }
    if configured {
        println!(
            "Settings  {}",
            terminal_text(&config.path.to_string_lossy())
        );
    }
    let git_disk = config.mount == crate::config::ProjectMount::Disk && context.repository;
    Ok(configured.then(|| git_disk.then(|| root.clone())))
}

/// The project mount as the intro, a running box and `pinfold config` state it.
fn project_files(mount: crate::config::ProjectMount, root: &Path) -> String {
    let root = terminal_text(&root.to_string_lossy());
    match mount {
        crate::config::ProjectMount::Share => format!("Shared: the box edits {root}"),
        crate::config::ProjectMount::Disk => {
            format!(
                "Isolated disk: the box has its own files at {}, separate from {root}.",
                crate::pi::state::PROJECT
            )
        }
    }
}

pub fn print_help() {
    println!(
        "Usage: pinfold -- CMD [args…]
       pinfold COMMAND [options]

Project
  -- CMD          Run a command in the project box
  config          Show or edit this project's settings
  build           Build or rebuild the selected image
  start           Keep the box running between commands
  stop            Shut down the project box

Isolated disk: the box has its own files, separate from your folder; work crosses through Git
  git push box    Send host branches (as host/*) and tags into the box
                  In the box: git switch -c BRANCH host/BRANCH
  git fetch box   Bring guest branches back as box/*, and new tags
                  Once first: git remote add box pinfold::ROOT
                  (needs git-remote-pinfold on PATH)
  disk export     Copy a stopped disk's files into a new directory

Maintenance
  doctor          Check the runtime, project setup and disk use
  clean           Reclaim unused storage
  update          Check for or install a release

Advanced
  profile         List profiles, or copy one to customize
  disk            List, export or remove managed disks"
    );
    #[cfg(target_os = "linux")]
    println!("  apparmor-profile  Print the AppArmor policy for installation");
    println!(
        "  box             Process interface for integrators
  image           Build or remove a caller image

Run pinfold COMMAND --help for options. Use --version to show the version."
    );
}

const SYNTAX: &str = "\
pinfold init
pinfold -- CMD [args…]
pinfold start
pinfold stop
pinfold build [--profile NAME] [--no-cache] [--allow HOST]…
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]… [--no-cache] [--allow HOST]… [--json]
pinfold image rm NAME [--json]
pinfold profile
pinfold profile list
pinfold profile new NAME [--from PROFILE]
pinfold clean [--dry-run] [--unused AGE]
pinfold doctor [--artifacts] [--json]
pinfold apparmor-profile
pinfold update [--check]
pinfold config [ROOT] [--json]
pinfold config edit
pinfold disk list [--json]
pinfold disk rm NAME|--all [--json]
pinfold disk export NAME DEST [--image IMAGE] [--json]
pinfold box up < spec.json
pinfold box exec BOX [--tty] -- argv…
pinfold box stat BOX
pinfold box down BOX
pinfold box list --label KEY[=VALUE]…
pinfold box prune
pinfold box inspect RUN
pinfold box events RUN [--limit N] [--after CURSOR]";

pub fn report(verb: &str, result: io::Result<i32>) -> i32 {
    result.unwrap_or_else(|error| {
        if verb == "__project-owner" {
            // The client forwards the owner's stderr by line, so an error's
            // external text must not add one.
            let text = crate::formatting::terminal_text(&error.to_string());
            let _ = writeln!(io::stderr(), "pinfold {verb}: {text}");
        } else if verb == "--" {
            let _ = writeln!(io::stderr(), "pinfold: {error}");
        } else {
            let _ = writeln!(io::stderr(), "pinfold {verb}: {error}");
        }
        if matches!(verb, "--" | "start" | "stop" | "__project-owner") {
            125
        } else {
            1
        }
    })
}

/// A malformed invocation: `message`, then the verb's syntax from [`syntax`].
pub(crate) fn usage(verb: &str, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}\nusage: {}", syntax(verb)),
    )
}

fn syntax(verb: &str) -> String {
    let prefix = format!("pinfold {verb}");
    SYNTAX
        .lines()
        .filter(|line| {
            line.strip_prefix(&prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Print help for host verbs without interpreting project-command argv.
pub fn help(verb: &str, args: &[String]) -> bool {
    let asks = verb != "--"
        && args
            .iter()
            .take_while(|arg| arg.as_str() != "--")
            .any(|arg| arg == "--help" || arg == "-h");
    if asks {
        let specific = args
            .first()
            .map(|sub| syntax(&format!("{verb} {sub}")))
            .filter(|line| !line.is_empty());
        println!("{}", specific.unwrap_or_else(|| syntax(verb)));
    }
    asks
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
        Some(verb) => Err(usage("box", &format!("unknown box verb {verb:?}"))),
        None => Err(usage("box", "a box verb is required")),
    }
}

fn up(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("box up", "up takes no arguments"));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        // This process owns the box, so it takes SIGTERM and SIGINT. The
        // handlers come before the spec read, so a signal
        // from the first instant ends the stream with one `down` line, its
        // box null until the spec parsed.
        let mut signals = Signals::new()?;
        let plan = tokio::select! {
            biased;
            _ = signals.recv() => return Ok(down(None, None, Shutdown::Signal)),
            plan = tokio::task::spawn_blocking(|| {
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
                let written = writeln!(
                    io::stdout(),
                    "{}",
                    serde_json::json!({
                        "event": "failed",
                        "box": &plan.name,
                        "detail": error.to_string(),
                    })
                );
                match written {
                    Err(error) if error.kind() != io::ErrorKind::BrokenPipe => Err(error),
                    _ => Ok(1),
                }
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
    let mut box_ = match Box::up(plan, &mut signals).await {
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
    let Some(_dispatch) = exec_admission(runtime, "exec", &name)? else {
        return Ok(3);
    };
    // A TTY only makes sense when both ends are terminals; `--tty` forces it
    // for callers that drive pinfold through their own pty.
    let tty = tty || (io::stdin().is_terminal() && io::stdout().is_terminal());
    let init = artifacts::init()?;
    let status = runtime.exec(
        &name,
        &init,
        &crate::core::runtime::ExecContext {
            tty,
            ..Default::default()
        },
        &argv,
    )?;
    Ok(exit_code(status))
}

fn stat(args: &[String]) -> io::Result<i32> {
    let name = single_name(args, "stat")?;
    let runtime = runtime();
    if found(runtime, "stat", &name)?.is_none() {
        return Ok(3);
    }
    let report = serde_json::to_string(&runtime.stat(&name)?).map_err(io::Error::other)?;
    println!("{report}");
    Ok(0)
}

/// The runtime's pinfold box `name`. An absent box is pinfold's own failure,
/// which `exec` and `stat` tell apart from the command's by exit 3.
fn found(runtime: &dyn Runtime, verb: &str, name: &str) -> io::Result<Option<BoxInfo>> {
    let found = runtime.inspect(name)?.filter(clean::pinfold_box);
    if found.is_none() {
        eprintln!("pinfold box {verb}: no box named {name:?}");
    }
    Ok(found)
}

fn exec_admission(
    runtime: &dyn Runtime,
    verb: &str,
    name: &str,
) -> io::Result<Option<nix::fcntl::Flock<fs::File>>> {
    let Some(box_) = found(runtime, verb, name)? else {
        return Ok(None);
    };
    let lock = match box_.labels.get(crate::core::ownership::GENERATION_LABEL) {
        Some(generation) => crate::core::ownership::exec_lock(name, generation)?,
        None => None,
    };
    if lock.is_none() {
        eprintln!("pinfold box {verb}: box {name:?} has not completed startup");
    }
    Ok(lock)
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
                "state": if box_.running { "running" } else if box_.stopped { "stopped" } else { "unknown" },
            })
        );
    }
    Ok(0)
}

fn prune(args: &[String]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("box prune", "prune takes no arguments"));
    }
    let runtime = runtime();
    let boxes = match clean::boxes(runtime) {
        Ok(boxes) => boxes,
        Err(error) => {
            clean::prune_leftover_states(runtime)?;
            return Err(error);
        }
    };
    let removed = clean::prune_boxes(runtime, &boxes)?;
    clean::prune_leftover_states(runtime)?;
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
        return Err(usage("box inspect", "inspect needs exactly one run id"));
    };
    let run_id = parse_run_id("box inspect", run_id)?;
    answer("inspect", run_id, runs::inspect(run_id)?)
}

/// `box events RUN [--limit N] [--after CURSOR]`: one page of the run's
/// egress log. Every argument is checked before any path is built.
fn events(args: &[String]) -> io::Result<i32> {
    let Some((run_id, options)) = args.split_first() else {
        return Err(usage("box events", "events needs a run id"));
    };
    let run_id = parse_run_id("box events", run_id)?;
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
                    .ok_or_else(|| {
                        usage("box events", &format!("--limit {value:?} is not 1 to 100"))
                    })?;
            }
            ("--after", Some(value)) => {
                after = Some(runs::Cursor::parse(run_id, value).ok_or_else(|| {
                    usage(
                        "box events",
                        &format!("--after {value:?} is not a cursor of this run"),
                    )
                })?);
            }
            _ => {
                return Err(usage(
                    "box events",
                    &format!("unknown events option {option:?}"),
                ));
            }
        }
    }
    answer("events", run_id, runs::events(run_id, limit, after)?)
}

fn parse_run_id<'a>(verb: &str, run_id: &'a str) -> io::Result<&'a str> {
    if runs::valid_id(run_id) {
        Ok(run_id)
    } else {
        Err(usage(
            verb,
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
    let json = args.last().is_some_and(|arg| arg == "--json");
    let args = if json { &args[..args.len() - 1] } else { args };
    if let [verb, name, destination, rest @ ..] = args
        && verb == "export"
    {
        if !valid_name(name) {
            return Err(usage("disk export", "spec: invalid disk name"));
        }
        let image = match rest {
            [] => None,
            [flag, image] if flag == "--image" && !image.is_empty() => Some(image.as_str()),
            _ => return Err(usage("disk export", "expected NAME DEST [--image IMAGE]")),
        };
        let status = crate::core::disk_export::run(name, Path::new(destination), image)?;
        if status == 0 {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"event":"exported", "disk":name, "destination":destination})
                );
            } else {
                println!(
                    "Exported {} to {} (untrusted files).",
                    terminal_text(name),
                    terminal_text(destination)
                );
            }
        }
        return Ok(status);
    }
    let runtime = runtime();
    match args {
        [verb] if verb == "list" => {
            let attached = disk::attachments(None)?;
            let origins = if json {
                BTreeMap::new()
            } else {
                crate::pi::state::disk_origins()?
            };
            let mut disks = runtime.list_disks()?;
            let mut rows = vec![
                [
                    "NAME",
                    "ALLOCATED",
                    "CAPACITY",
                    "STATE",
                    "ATTACHED",
                    "ORIGIN",
                ]
                .map(String::from)
                .to_vec(),
            ];
            if !json {
                disks.sort_by(|left, right| left.name.cmp(&right.name));
            }
            for found in disks {
                let clean = !disk::unclean(&found.name)?;
                if !json {
                    let holders = attached
                        .get(&found.name)
                        .into_iter()
                        .flatten()
                        .map(|(name, readonly)| {
                            format!(
                                "{} ({})",
                                terminal_text(name),
                                if *readonly { "ro" } else { "rw" }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let origin = origins
                        .get(&found.name)
                        .map(|root| terminal_text(&root.to_string_lossy()))
                        .unwrap_or_else(|| "—".to_string());
                    rows.push(vec![
                        terminal_text(&found.name),
                        human_size(found.allocated),
                        human_size(found.size),
                        if clean { "clean" } else { "unclean" }.to_string(),
                        if holders.is_empty() {
                            "—".to_string()
                        } else {
                            holders
                        },
                        origin,
                    ]);
                    continue;
                }
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
                        "clean": clean,
                    })
                );
            }
            if !json {
                print_table(&rows);
            }
            Ok(0)
        }
        [verb, name] if verb == "rm" && name == "--all" => {
            let mut names: Vec<String> = runtime
                .list_disks()?
                .into_iter()
                .map(|disk| disk.name)
                .collect();
            names.sort();
            let mut status = 0;
            for name in names {
                match remove_disk(runtime, &name, json) {
                    Ok(code) => status |= code,
                    Err(error) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({"event":"failed", "disk":name, "detail":error.to_string()})
                            );
                        } else {
                            eprintln!(
                                "pinfold disk rm: {}: {}",
                                terminal_text(&name),
                                terminal_text(&error.to_string())
                            );
                        }
                        status = 1;
                    }
                }
            }
            Ok(status)
        }
        [verb, name] if verb == "rm" => remove_disk(runtime, name, json),
        [verb, ..] if matches!(verb.as_str(), "list" | "export" | "rm") => {
            Err(usage(&format!("disk {verb}"), "invalid arguments"))
        }
        _ => Err(usage(
            "disk",
            "expected list, rm NAME|--all, or export NAME DEST [--image IMAGE], with optional --json",
        )),
    }
}

fn remove_disk(runtime: &dyn Runtime, name: &str, json: bool) -> io::Result<i32> {
    if !valid_name(name) {
        let detail =
            format!("disk name {name:?} must start alphanumeric and hold only [A-Za-z0-9._-]");
        return Ok(if json {
            refused("disk", Some(name), RefusalReason::Spec, &detail)
        } else {
            eprintln!("pinfold disk rm: spec: {}", terminal_text(&detail));
            1
        });
    }
    match disk::remove(runtime, name)? {
        disk::Removal::InUse(holder) => Ok(if json {
            refused(
                "disk",
                Some(name),
                RefusalReason::DiskInUse,
                &format!("box {holder:?} attaches disk {name:?}"),
            )
        } else {
            eprintln!(
                "pinfold disk rm: disk-in-use: {} is attached by {}",
                terminal_text(name),
                terminal_text(&holder)
            );
            1
        }),
        disk::Removal::Removed { existed } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "event": "removed", "disk": name, "existed": existed })
                );
            } else {
                println!(
                    "{} {}",
                    if existed {
                        "Removed"
                    } else {
                        "Already absent:"
                    },
                    terminal_text(name)
                );
            }
            Ok(0)
        }
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn clean(args: &[String]) -> io::Result<i32> {
    let (dry_run, unused) = parse_clean(args)?;
    let runtime = runtime();
    let plan = CleanPlan::measure(runtime, unused)?;
    let total = human_size(plan.total());
    let action = if dry_run {
        format!("dry run; {total} measured")
    } else {
        format!("{total} measured; removing unused data")
    };
    println!("pinfold clean: {action}");
    plan.print();
    if !dry_run {
        let (disk_count, unused) = (plan.disk_count, plan.unused);
        let removed = plan.remove(runtime)?;
        if disk_count > 0 {
            if unused.is_none() {
                println!("pinfold clean: disks kept (pinfold disk list)");
            } else if removed == 0 {
                println!(
                    "pinfold clean: no idle project's disks removed; disks kept (pinfold disk list)"
                );
            } else {
                println!(
                    "pinfold clean: removed {} of idle projects; others kept (pinfold disk list)",
                    disks_text(removed)
                );
            }
        }
    }
    Ok(0)
}

fn disks_text(count: usize) -> String {
    format!("{count} disk{}", if count == 1 { "" } else { "s" })
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
    disk_count: usize,
    /// Recorded checkouts that are gone or moved; their state and disks
    /// stay until `--unused` or `disk rm`.
    moved: Vec<PathBuf>,
    automatic_bytes: u64,
    project_state: u64,
    build_cache_bytes: u64,
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
        let runs = match runs::retired() {
            Ok(runs) => runs,
            Err(error) => {
                let _ = writeln!(
                    io::stderr(),
                    "pinfold: run retention: {error}; keeping run records"
                );
                Vec::new()
            }
        };
        let run_bytes = dirs::entries(&dirs::runs_dir()?)?
            .iter()
            .map(|entry| clean::path_bytes(&entry.path()))
            .sum();

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
            run_bytes,
            runs,
            project_disks,
            disk_bytes: disks.iter().map(|disk| disk.allocated).sum(),
            disk_count: disks.len(),
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
            + self.build_cache_bytes
    }

    fn print(&self) {
        println!(
            "  reclaimable by daily cleanup: {}",
            human_size(self.automatic_bytes)
        );
        println!("  run records: {}", human_size(self.run_bytes));
        println!(
            "  build cache: {} allocated; clean frees its unused part",
            human_size(self.build_cache_bytes)
        );
        println!("  project state: {}", human_size(self.project_state));
        let disks = human_size(self.disk_bytes);
        if self.disk_count == 0 {
            println!("  disks: {disks}");
        } else if self.unused.is_some() {
            println!(
                "  disks: {disks} in {}; --unused AGE removes those of projects not run for AGE (pinfold disk list)",
                disks_text(self.disk_count)
            );
            println!("    remove others with `pinfold disk rm NAME|--all`");
        } else {
            println!(
                "  disks: {disks} in {}; clean keeps them (pinfold disk list)",
                disks_text(self.disk_count)
            );
            println!(
                "    remove with `pinfold clean --unused AGE` (projects not run for AGE) or `pinfold disk rm NAME|--all`"
            );
        }
        // Under `--unused` an idle project's state and disks go too.
        let moved = if self.unused.is_some() {
            "checkout gone"
        } else {
            "checkout gone, state and disks kept"
        };
        for root in &self.moved {
            println!("  {moved}: {}", terminal_text(&root.to_string_lossy()));
        }
    }

    /// Remove everything the plan measured, then the builds past each
    /// source's newest two. Image sizes are unmeasured: the runtime's image
    /// list carries none. Returns how many disks it removed.
    fn remove(self, runtime: &dyn Runtime) -> io::Result<usize> {
        clean::prune_boxes(runtime, &self.boxes)?;
        clean::prune_leftover_states(runtime)?;
        if let Err(error) = image::service().purge_build_cache() {
            let _ = writeln!(io::stderr(), "pinfold clean: build cache: {error}; kept");
        }
        clean::remove_paths(self.automatic)?;
        clean::remove_paths(self.runs)?;
        let mut locks = BTreeMap::new();
        for id in self.project_disks.keys() {
            if let Some(lock) = crate::core::ownership::try_project_lock(id)? {
                locks.insert(id.as_str(), lock);
            }
        }
        let mut disk_failure = None;
        let mut removed = 0;
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
                        match disk::remove(runtime, name) {
                            Ok(disk::Removal::Removed { existed: true }) => removed += 1,
                            Ok(_) => {}
                            Err(error) => {
                                let _ = writeln!(
                                    io::stderr(),
                                    "pinfold clean: disk {name:?}: {error}; kept"
                                );
                                disk_failure.get_or_insert(error);
                            }
                        }
                    }
                }
            }
        }
        drop(locks);
        if let Err(error) = clean::keep_two_images(image::service(), None) {
            let _ = writeln!(io::stderr(), "pinfold clean: images: {error}; kept");
        }
        disk_failure.map_or(Ok(removed), Err)
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

/// `pinfold doctor`: print one report of what project commands depend on and
/// what state it is in. Reads only: nothing is created, downloaded or fixed.
pub fn doctor(args: &[String]) -> io::Result<i32> {
    let mut json = false;
    let mut only_pins = false;
    for arg in args {
        match arg.as_str() {
            "--json" if !json => json = true,
            "--artifacts" if !only_pins => only_pins = true,
            _ => return Err(usage("doctor", &format!("unexpected argument {arg:?}"))),
        }
    }
    if only_pins {
        let pins = artifacts::pins()?;
        if json {
            println!("{}", serde_json::to_string(&pins)?);
        } else {
            for pin in pins {
                let status = if pin.cached { "cached" } else { "not cached" };
                println!("{}  {}  ({status})", pin.name, pin.version);
                println!("  {}", terminal_text(&pin.path.to_string_lossy()));
            }
        }
        return Ok(0);
    }

    let cwd = std::env::current_dir()?;
    let mut findings = Vec::new();
    let project = project_config(&cwd).and_then(|(root, config)| {
        config.check_host_paths(&root)?;
        Ok((root, config))
    });
    let root = project.as_ref().map_or(&cwd, |(root, _)| root);
    let runtime = runtime();
    let version = runtime
        .version()
        .map_err(|e| findings.push(format!("Runtime version: {e}")))
        .ok();
    let ready = match runtime.preflight() {
        Ok(()) => true,
        Err(error) => {
            findings.push(format!("Runtime: {error}"));
            #[cfg(target_os = "linux")]
            findings.extend(crate::core::runtime::firecracker::preflight::host_requirements());
            false
        }
    };
    let kernel = nix::sys::utsname::uname()
        .map(|uname| {
            format!(
                "{} {}",
                uname.sysname().to_string_lossy(),
                uname.release().to_string_lossy()
            )
        })
        .map_err(|e| findings.push(format!("Kernel: {e}")))
        .ok();
    let pins = artifacts::pins()
        .map_err(|e| findings.push(format!("Artifacts: {e}")))
        .ok();
    let mut image = None;
    let mut configuration = None;
    // Plain output names an outdated chain once, as its lowest image.
    let mut outdated = None;
    match &project {
        Ok((root, config)) => {
            let reference =
                crate::pi::launch::resolve_image(config, &crate::pi::state::project_id(root));
            let resolved = (ready || cfg!(target_os = "linux"))
                .then(|| image::service().resolve_image(&reference));
            let found = match &resolved {
                Some(Ok(found)) => crate::pi::launch::outdated_images(config, &reference, found),
                Some(Err(error)) => Err(io::Error::other(error.to_string())),
                None => Ok(Default::default()),
            };
            match found {
                Ok(found) => {
                    let warnings = found.warnings();
                    outdated = found
                        .summary()
                        .map(|line| (findings.len()..findings.len() + warnings.len(), line));
                    findings.extend(warnings);
                }
                Err(error) => findings.push(format!("Image: {error}")),
            }
            if json {
                configuration = Some(config_report(root, config, &reference, resolved.as_ref())?);
            }
            image = Some(reference);
            if config.mount == crate::config::ProjectMount::Disk {
                findings.extend(helper_finding());
            }
        }
        Err(error) => findings.push(error.to_string()),
    }

    let disk = if ready || cfg!(target_os = "linux") {
        CleanPlan::measure(runtime, None)
            .map_err(|e| findings.push(format!("Disk usage: {e}")))
            .ok()
    } else {
        None
    };
    if let Some(plan) = &disk
        && plan.total() > 20 << 30
    {
        findings.push(format!(
            "Pinfold uses {}. Run `pinfold clean --dry-run` to inspect reclaimable space.",
            human_size(plan.total())
        ));
    }
    if json {
        // `json!` panics on a path that is not UTF-8.
        let pins = serde_json::to_value(&pins)?;
        let what = if project.is_ok() {
            "project root"
        } else {
            "working directory"
        };
        let disk = match &disk {
            Some(plan) => Some(serde_json::json!({
                "automatic_bytes": plan.automatic_bytes, "run_bytes": plan.run_bytes,
                "build_cache_bytes": plan.build_cache_bytes, "project_state_bytes": plan.project_state,
                "disk_bytes": plan.disk_bytes, "moved": serde_json::to_value(&plan.moved)?
            })),
            None => None,
        };
        println!(
            "{}",
            serde_json::json!({
                "root": utf8(root, what)?, "runtime": runtime.describe(), "runtime_ready": ready,
                "version": version, "kernel": kernel, "config": configuration, "image": image,
                "artifacts": pins, "findings": findings, "disk": disk
            })
        );
        return Ok(0);
    }

    if let Some((chain, line)) = outdated {
        findings.splice(chain, [line]);
    }
    if findings.is_empty() {
        println!("No issues found.");
    } else {
        for finding in findings {
            println!("- {}", terminal_text(&finding));
        }
    }
    println!("\nProject   {}", terminal_text(&root.to_string_lossy()));
    println!(
        "Runtime   {} ({})",
        runtime.describe(),
        terminal_text(version.as_deref().unwrap_or("version unknown"))
    );
    if let Some(kernel) = kernel {
        println!("Kernel    {}", terminal_text(&kernel));
    }
    if let Ok((_, config)) = &project {
        print_profile(config, 10)?;
    }
    if let Some(image) = image {
        println!("Image     {}", terminal_text(&image));
    }
    if let Ok((_, config)) = &project {
        println!("Logins    {}", logins_description(config));
    }
    if let Some(plan) = disk {
        println!("Disk usage");
        plan.print();
    }
    Ok(0)
}

/// A disk project's Git remote needs `git-remote-pinfold` on PATH, and
/// Git runs whichever pinfold that one resolves to.
fn helper_finding() -> Option<String> {
    let exe = crate::update::installed().ok()?;
    let Some(helper) = crate::update::helper_on_path() else {
        return Some(format!(
            "Git remote: no git-remote-pinfold on PATH; {}",
            crate::update::link_remedy(&exe)
        ));
    };
    (fs::canonicalize(&helper).ok()? != exe).then(|| {
        format!(
            "Git remote: {} is not this pinfold ({})",
            helper.to_string_lossy(),
            exe.to_string_lossy()
        )
    })
}

fn profile_description(config: &Config) -> String {
    terminal_text(&config.profile.name) + &profile_override(config)
}

fn profile_override(config: &Config) -> String {
    match config.project_profile.as_deref() {
        Some(saved)
            if std::env::var_os("PINFOLD_PROFILE").is_some() && saved != config.profile.name =>
        {
            format!(" (PINFOLD_PROFILE overrides {})", terminal_text(saved))
        }
        _ => String::new(),
    }
}

/// The selected profile's chain, top first, where the profile lives, and
/// the settings file over it; labels padded to `width`.
fn print_profile(config: &Config, width: usize) -> io::Result<()> {
    let profile = &config.profile;
    let chain: Vec<String> = profile
        .links()
        .iter()
        .rev()
        .map(|link| terminal_text(&link.name))
        .collect();
    println!(
        "{:width$}{}{}",
        "Profile",
        chain.join(" → "),
        profile_override(config)
    );
    let place = match profile.dir()? {
        Some(dir) => {
            let files: Vec<&str> = ["pinfold.toml", "Containerfile", "home", "share"]
                .into_iter()
                .filter(|file| dir.join(file).exists())
                .collect();
            let dir = terminal_text(&dir.to_string_lossy());
            if files.is_empty() {
                format!("{dir} (editable)")
            } else {
                format!("{dir} (editable: {})", files.join(", "))
            }
        }
        None => format!(
            "built in; copy it with `pinfold profile new NAME --from {}` to change it",
            terminal_text(&profile.name)
        ),
    };
    println!("{:width$}{place}", "");
    println!(
        "{:width$}{} (this project only; overrides the profile)",
        "Settings",
        terminal_text(&config.path.to_string_lossy())
    );
    Ok(())
}

/// `rows` with each column but the last padded to its widest cell.
fn print_table(rows: &[Vec<String>]) {
    let mut widths = Vec::new();
    for row in rows {
        widths.resize(widths.len().max(row.len()), 0);
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    for row in rows {
        let mut line = String::new();
        for (index, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if index + 1 < row.len() {
                let pad = widths[index] - cell.chars().count() + 2;
                line.extend(std::iter::repeat_n(' ', pad));
            }
        }
        println!("{line}");
    }
}

/// The login routes, whose credentials stay on the host, from configuration
/// alone: no value, and not whether a variable is set.
fn logins_description(config: &Config) -> String {
    let logins: Vec<String> = config
        .routes
        .iter()
        .filter_map(|(name, route)| match route {
            Route::Login(login) => Some(format!(
                "{} ({})",
                terminal_text(name),
                terminal_text(&match &login.provider {
                    Some(provider) => format!("{} {provider}", login.login),
                    None => login.login.clone(),
                })
            )),
            _ => None,
        })
        .collect();
    if logins.is_empty() {
        "none; agents log in inside the box".to_string()
    } else {
        logins.join(", ")
    }
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
                terminal_text(&exe.to_string_lossy())
            ))
        })?;
    println!("abi <abi/4.0>,\n\n\"{path}\" flags=(unconfined) {{\n  userns,\n}}");
    Ok(0)
}

fn project_config(cwd: &Path) -> io::Result<(PathBuf, Config)> {
    let context = crate::pi::context::Context::discover(cwd)?;
    let config = Config::load(&context.root)?;
    context.check_metadata(config.mount)?;
    Ok((context.root, config))
}

/// Read-only: do not materialize profiles or record project state.
pub fn config(args: &[String]) -> io::Result<i32> {
    if args.first().is_some_and(|a| a == "edit") {
        return if args.len() == 1 {
            config_edit()
        } else {
            Err(usage("config edit", "edit takes no arguments"))
        };
    }
    let mut json = false;
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "--json" if !json => json = true,
            arg if arg != "--json" && path.is_none() => path = Some(Path::new(arg)),
            _ => {
                return Err(usage(
                    "config",
                    "expected one optional project root and --json",
                ));
            }
        }
    }
    let (root, config) = match path {
        Some(path) => project_config(path)?,
        None => project_config(&std::env::current_dir()?)?,
    };
    let reference = crate::pi::launch::resolve_image(&config, &crate::pi::state::project_id(&root));
    let resolved = image::service().resolve_image(&reference);
    let report = config_report(&root, &config, &reference, Some(&resolved))?;
    if json {
        println!("{report}");
        return Ok(0);
    }
    println!("Project   {}", terminal_text(&root.to_string_lossy()));
    println!("Files     {}", project_files(config.mount, &root));
    print_profile(&config, 10)?;
    let availability = match report["image_built"].as_bool() {
        Some(true) => "available locally",
        Some(false) => "missing",
        None => "availability unknown",
    };
    println!(
        "Image     {} ({availability})",
        terminal_text(report["image"].as_str().expect("image reference"))
    );
    if let Some(error) = report["image_error"].as_str() {
        println!("          {}", terminal_text(error));
    }
    if path.is_none() {
        println!("\nEdit these settings with `pinfold config edit`.");
    }
    Ok(0)
}

/// `pinfold config edit`: write the project's configuration file when it is
/// missing, then open it in the user's editor and return its exit code.
fn config_edit() -> io::Result<i32> {
    let project = crate::pi::context::Context::discover(&std::env::current_dir()?)?;
    if project.is_worktree() {
        let mount = crate::config::project_mount(&project.root)
            .ok()
            .flatten()
            .unwrap_or(crate::config::ProjectMount::Share);
        project.check_worktree(mount)?;
    }
    let (path, created) = crate::config::create(&project.root, None)?;
    let path_text = terminal_text(&path.to_string_lossy());
    if created {
        let mount = match crate::config::ProjectMount::default_kind() {
            crate::config::ProjectMount::Share => "shared folder",
            crate::config::ProjectMount::Disk => "isolated disk",
        };
        eprintln!("Created {path_text} with the default settings ({mount}).");
    } else {
        eprintln!("Opening {path_text}");
    }
    let editor = ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "vi".into());
    // The editor value may carry arguments (`code -w`), as shells expect.
    let mut script = editor;
    script.push(" \"$1\"");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(&path)
        .status_guarded()?;
    let code = exit_code(status);
    if code != 0 {
        eprintln!(
            "pinfold: editor exited with status {code}. Open the file above directly or set VISUAL to a working editor and rerun `pinfold config edit`."
        );
    }
    Ok(code)
}

/// Whether an image lookup found the image, and why when it cannot tell.
fn image_built(
    resolved: &io::Result<Result<image::ImageInfo, String>>,
) -> (Option<bool>, Option<String>) {
    match resolved {
        Ok(Ok(_)) => (Some(true), None),
        #[cfg(target_os = "macos")]
        Ok(Err(message)) if !message.starts_with("Error: image not found: ") => {
            (None, Some(message.clone()))
        }
        Ok(Err(_)) => (Some(false), None),
        Err(error) => (None, Some(error.to_string())),
    }
}

/// `resolved` is the lookup of `image`, the config's image, or `None` when it
/// was skipped.
fn config_report(
    root: &Path,
    config: &Config,
    image: &str,
    resolved: Option<&io::Result<Result<image::ImageInfo, String>>>,
) -> io::Result<serde_json::Value> {
    let project = crate::pi::state::project_id(root);
    let [home_disk, project_disk] = crate::pi::state::disk_names(&project);
    let (image_built, image_error) = resolved.map_or((None, None), image_built);
    let mut chain = Vec::new();
    for link in config.profile.links().into_iter().rev() {
        let path = link.dir()?;
        let path = path
            .as_deref()
            .map(|dir| utf8(dir, "profile directory"))
            .transpose()?;
        chain.push(serde_json::json!({ "name": &link.name, "path": path }));
    }
    Ok(serde_json::json!({
        "root": utf8(root, "project root")?,
        "config": utf8(&config.path, "configuration file")?,
        "profile": &config.profile.name,
        "profile_chain": chain,
        "image": image,
        "image_source": config.image_source(),
        "image_built": image_built,
        "image_error": image_error,
        "containerfile": if config.image.is_some() { None } else { Some(match &config.containerfile { Some((path, _)) => utf8(path, "project Containerfile")?, None => "profile" }) },
        "egress": { "allow": &config.allow, "routes": &config.routes },
        "cpus": config.cpus,
        "memory": &config.memory,
        // The project box ignores a configured HOME.
        "env": config.env.iter().filter(|(name, _)| *name != "HOME").collect::<std::collections::BTreeMap<_, _>>(),
        "project": {
            "id": project,
            "home": crate::pi::state::HOME,
            "mount": { "kind": config.mount, "host": utf8(root, "project root")?, "guest": utf8(crate::pi::state::guest_root(config.mount, root), "project root")? },
            "disks": { "home": home_disk, "project": if config.mount == crate::config::ProjectMount::Disk { Some(project_disk) } else { None } },
        },
    }))
}

fn parse_exec(args: &[String]) -> io::Result<(String, bool, Vec<String>)> {
    let mut args = args.iter();
    let name = args
        .next()
        .ok_or_else(|| usage("box exec", "exec needs a box name"))?
        .clone();
    let mut tty = false;
    loop {
        match args.next().map(String::as_str) {
            Some("--") => break,
            Some("--tty") => tty = true,
            Some(option) => {
                return Err(usage(
                    "box exec",
                    &format!("unknown exec option {option:?}"),
                ));
            }
            None => return Err(usage("box exec", "exec needs `-- argv`")),
        }
    }
    let argv: Vec<String> = args.cloned().collect();
    if argv.is_empty() {
        return Err(usage("box exec", "exec needs a command after `--`"));
    }
    Ok((name, tty, argv))
}

fn single_name(args: &[String], verb: &str) -> io::Result<String> {
    match args {
        [name] => Ok(name.clone()),
        _ => Err(usage(
            &format!("box {verb}"),
            &format!("{verb} needs exactly one box name"),
        )),
    }
}

fn parse_labels(args: &[String]) -> io::Result<Vec<(String, Option<String>)>> {
    let mut filters = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg != "--label" {
            return Err(usage(
                "box list",
                "list takes only --label k=v or --label k",
            ));
        }
        let label = args
            .next()
            .ok_or_else(|| usage("box list", "--label needs k=v or k"))?;
        match label.split_once('=') {
            Some((key, value)) => filters.push((key.to_string(), Some(value.to_string()))),
            None => filters.push((label.to_string(), None)),
        }
    }
    if filters.is_empty() {
        return Err(usage("box list", "list needs at least one --label"));
    }
    Ok(filters)
}

pub(crate) fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// A build's context is its Containerfile alone. Other files in a profile
/// or project configuration directory are not build inputs.
pub fn build(args: &[String]) -> io::Result<i32> {
    let mut cache = true;
    let mut profile_name = None;
    let mut allow = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-cache" => cache = false,
            "--profile" if profile_name.is_none() => {
                profile_name = Some(
                    args.next()
                        .ok_or_else(|| usage("build", "--profile needs a name"))?,
                );
            }
            "--allow" => allow.push(
                args.next()
                    .ok_or_else(|| usage("build", "--allow needs a host"))?
                    .clone(),
            ),
            option => {
                return Err(usage("build", &format!("unknown build option {option:?}")));
            }
        }
    }
    image::check_build_allow(&allow).map_err(|error| usage("build", &error))?;
    let (profile, project) = match profile_name {
        Some(name) => (Profile::load(name)?, None),
        None => {
            let (root, config) = project_config(&std::env::current_dir()?)?;
            config.check_host_paths(&root)?;
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
    };
    if project.is_none() && profile.recipe().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "image-no-recipe: profile has no Containerfile; there is nothing to build",
        ));
    }
    let images = image::service();
    let build = |label, source: &str, bytes: &[u8], base: Option<String>| {
        image::build(
            images,
            Build {
                label: Some(label),
                source,
                context: Context::Alone(bytes),
                cache,
                labels: base
                    .map(|digest| (clean::BASE_LABEL.to_string(), digest))
                    .into_iter()
                    .collect(),
                allow: allow.clone(),
            },
        )?
        .map_err(|output| {
            eprint!("{output}");
            io::Error::other(format!("the {} build failed", images.program()))
        })
    };
    // Root first: each image records the identity of the one just built
    // beneath it, and Linux also binds its source to it. Each is published
    // on its own, so a later failure leaves the lower ones published.
    let recipes: Vec<_> = profile
        .links()
        .into_iter()
        .filter_map(|link| {
            Some((
                clean::PROFILE_LABEL,
                link.name.clone(),
                link.containerfile.clone()?,
            ))
        })
        .chain(project.map(|(id, bytes)| (clean::PROJECT_LABEL, id, bytes)))
        .collect();
    // The project's source is its opaque id, so progress names it in words.
    let names: Vec<&str> = recipes
        .iter()
        .map(|(label, source, _)| match *label {
            clean::PROJECT_LABEL => "the project image",
            _ => source,
        })
        .collect();
    let count = names.len();
    // A chain that ends in the project's image is the project's.
    let selected = match recipes.last() {
        Some((clean::PROJECT_LABEL, ..)) => "this project",
        _ => &profile.name,
    };
    if count > 1 {
        let order = names.join(", ");
        eprintln!("pinfold: {selected} needs {count} images, built in order: {order}");
    }
    let mut below: Option<image::Built> = None;
    for (index, (label, source, bytes)) in recipes.iter().enumerate() {
        let name = names[index];
        match count {
            1 if name == selected || *label == clean::PROJECT_LABEL => {
                eprintln!("pinfold: building {name}")
            }
            1 => eprintln!("pinfold: building {name} for {selected}"),
            _ => eprintln!("pinfold: building {name} ({}/{count})", index + 1),
        }
        let base = below.as_ref().and_then(|built| built.id.clone());
        #[cfg(target_os = "linux")]
        if below.is_some() && base.is_none() {
            return Err(io::Error::other(
                "image-base-changed: rebuilt profile identity unavailable",
            ));
        }
        below = Some(build(label, source, bytes, base)?);
    }
    println!("{}", below.expect("a build has a recipe").latest);
    Ok(0)
}

pub fn image(args: &[String]) -> io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("build") => Ok(image_build(&args[1..])),
        Some("rm") => {
            let json = args.last().is_some_and(|arg| arg == "--json");
            image_rm(&args[1..args.len() - usize::from(json)], json)
        }
        Some(verb) => Err(usage("image", &format!("unknown image verb {verb:?}"))),
        None => Err(usage("image", "an image verb is required")),
    }
}

fn image_build(args: &[String]) -> i32 {
    let name = args
        .first()
        .map(String::as_str)
        .filter(|arg| !arg.starts_with("--"));
    let options = &args[usize::from(name.is_some())..];
    // A malformed invocation still reports in the form it asked for.
    let (json, parsed) = parse_image_build(name, options);
    let result = parsed
        .map_err(|detail| ImageError::Refused(RefusalReason::Spec, detail))
        .and_then(image::build_image);
    let (line, code) = match result {
        Ok(built) if !json => {
            println!("Built {}", terminal_text(&built.latest));
            return 0;
        }
        Ok(built) => (
            serde_json::json!({
                "event": "built",
                "image": name,
                "ref": built.reference,
                "latest": built.latest,
                "id": built.id,
                "labels": built.labels,
            }),
            0,
        ),
        Err(ImageError::Failed(output)) if !json => {
            eprintln!("pinfold image build: build failed");
            for line in output.lines() {
                eprintln!("{}", terminal_text(line));
            }
            return 1;
        }
        Err(ImageError::Failed(output)) => (
            serde_json::json!({ "event": "failed", "image": name, "log": output.lines().collect::<Vec<_>>() }),
            1,
        ),
        Err(ImageError::Refused(reason, detail)) => {
            return image_refused(json, "build", name, reason, &detail);
        }
    };
    println!("{line}");
    code
}

fn image_refused(
    json: bool,
    verb: &str,
    name: Option<&str>,
    reason: RefusalReason,
    detail: &str,
) -> i32 {
    if json {
        return refused("image", name, reason, detail);
    }
    let reason = serde_json::json!(reason);
    eprintln!(
        "pinfold image {verb}: {}: {}",
        reason.as_str().unwrap_or("refused"),
        terminal_text(detail)
    );
    1
}

fn image_rm(args: &[String], json: bool) -> io::Result<i32> {
    let refused =
        |name, detail: String| image_refused(json, "rm", name, RefusalReason::Spec, &detail);
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
    if !json {
        if removed.ids.is_empty() && removed.in_use.is_empty() {
            println!("No local image named {name}.");
        }
        for id in removed.ids {
            println!("Removed {name} tags from {}", terminal_text(&id));
        }
        for id in removed.in_use {
            println!(
                "Kept the last tag on {} because a box uses it.",
                terminal_text(&id)
            );
        }
        return Ok(0);
    }
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

/// Returns whether `--json` appeared in an option position, even when the
/// invocation is malformed, so an option's value is never read as the flag.
fn parse_image_build(name: Option<&str>, args: &[String]) -> (bool, Result<ImageRequest, String>) {
    let mut json = false;
    let mut error = None;
    let mut containerfile = None;
    let mut context = None;
    let mut labels = BTreeMap::new();
    let mut cache = true;
    let mut allow = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut option = || -> Result<(), String> {
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
                "--allow" => allow.push(args.next().ok_or("--allow needs a host")?.clone()),
                "--json" if !json => json = true,
                option => return Err(format!("unknown image build option {option:?}")),
            }
            Ok(())
        };
        if let Err(problem) = option() {
            error.get_or_insert(problem);
        }
    }
    let request = (|| {
        let name = name.ok_or("image build needs an image name")?.to_string();
        if let Some(error) = error {
            return Err(error);
        }
        Ok(ImageRequest {
            name,
            containerfile: containerfile.ok_or("image build needs --containerfile PATH")?,
            context: context.ok_or("image build needs --context DIR")?,
            labels,
            cache,
            allow,
        })
    })();
    (json, request)
}

pub fn profile(args: &[String]) -> io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("new") => profile_new(&args[1..]).map(|()| 0),
        None => profile_list(),
        Some("list") if args.len() == 1 => profile_list(),
        Some("list") => Err(usage("profile", "list takes no arguments")),
        Some(verb) => Err(usage("profile", &format!("unknown profile verb {verb:?}"))),
    }
}

/// Every profile, built in or the user's, with its base and where it
/// lives. Inside a configured project, `*` marks the selected one.
fn profile_list() -> io::Result<i32> {
    let mut names: BTreeSet<String> = profile::BUILT_IN.map(String::from).into();
    for entry in dirs::entries(&profile::user_dir()?)? {
        if let Some(name) = entry.file_name().to_str()
            && entry.path().is_dir()
            && profile::check_name("profile", name).is_ok()
        {
            names.insert(name.to_string());
        }
    }
    let selected = std::env::current_dir()
        .and_then(|cwd| project_config(&cwd))
        .ok()
        .map(|(_, config)| config.profile.name);
    let mut rows = vec![
        ["", "PROFILE", "BUILDS ON", "WHERE"]
            .map(String::from)
            .to_vec(),
    ];
    for name in names {
        let mark = if selected.as_ref() == Some(&name) {
            "*"
        } else {
            ""
        };
        // A user directory wins whole, as `Profile::link` decides, so a
        // profile that fails to load still shows where to fix it.
        let dir = profile::user_dir()?.join(&name);
        let mut place = if dir.is_dir() {
            format!("{} (editable)", terminal_text(&dir.to_string_lossy()))
        } else {
            "built in".to_string()
        };
        let base = match Profile::link(&name).and_then(|link| profile::base_name(&link)) {
            Ok(base) => base.map_or("—".to_string(), |base| terminal_text(&base)),
            Err(error) => {
                // A TOML error spans lines; its first names the place.
                let error = error.to_string();
                let error = error.lines().next().unwrap_or_default();
                place = format!("{place}; {}", terminal_text(error));
                "?".to_string()
            }
        };
        rows.push(vec![mark.to_string(), terminal_text(&name), base, place]);
    }
    print_table(&rows);
    Ok(0)
}

fn profile_new(args: &[String]) -> io::Result<()> {
    let (name, from) = parse_profile_new(args)?;
    profile::check_name("profile", &name)?;
    let source = Profile::link(&from);
    let profiles = profile::user_dir()?;
    fs::create_dir_all(&profiles)?;
    let target = profiles.join(&name);
    fs::create_dir(&target).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "profile {name:?} at {}: {error}",
                terminal_text(&target.to_string_lossy())
            ),
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
