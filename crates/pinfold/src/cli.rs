//! `pinfold box`: the JSON-on-stdio process interface Switchyard uses.
//!
//! The verbs are parsed by hand: the set is small, and ARCHITECTURE.md's
//! dependency list has no argument parser.

use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use crate::core::r#box::Box;
use crate::core::plan::Plan;
use crate::core::runtime::runtime;
use crate::dirs;

const USAGE: &str = "usage: pinfold box up|exec BOX [--tty] [--workdir DIR] -- argv|down BOX|list --label k=v|prune";

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
    let mut plan = Plan::from_reader(io::stdin()).map_err(io::Error::other)?;
    // The owner label is how `box prune` tells a live box from a leftover.
    plan.labels
        .insert("dev.pinfold.owner".into(), std::process::id().to_string());
    let init = init_path()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut box_ = Box::up(&plan, &init).await?;
        println!(
            "{}",
            serde_json::json!({ "event": "ready", "box": plan.name })
        );
        io::stdout().flush()?;
        box_.hold().await?;
        Ok(0)
    })
}

/// Run a command in a running box with this process's stdio and return its
/// exit code.
fn exec(args: &[OsString]) -> io::Result<i32> {
    let args = ExecArgs::parse(args)?;
    // A TTY only makes sense when both ends are terminals; `--tty` forces it
    // for callers that drive pinfold through their own pty.
    let tty = args.tty || (io::stdin().is_terminal() && io::stdout().is_terminal());
    let status = runtime()?.exec(&args.name, tty, args.workdir.as_deref(), &args.argv)?;
    Ok(exit_code(status))
}

/// Signal the owning `box up` process through the state dir and wait for it
/// to remove the box. A dead owner means remove the leftover directly.
fn down(args: &[OsString]) -> io::Result<i32> {
    let name = single_name(args, "down")?;
    let state = state_path(&name)?;
    if let Some(pid) = read_pid(&state)
        && alive(pid)
    {
        kill(Pid::from_raw(pid), Signal::SIGTERM).map_err(io::Error::other)?;
        for _ in 0..1000 {
            if !state.exists() {
                return Ok(0);
            }
            if !alive(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    runtime()?.down(&name)?;
    let _ = fs::remove_dir_all(&state);
    Ok(0)
}

/// Print one JSON line per box carrying the requested label.
fn list(args: &[OsString]) -> io::Result<i32> {
    let (key, value) = parse_label(args)?;
    for box_ in runtime()?.list()? {
        if box_.labels.get(&key).map(String::as_str) == Some(value.as_str()) {
            println!(
                "{}",
                serde_json::json!({ "name": box_.id, "labels": box_.labels })
            );
        }
    }
    Ok(0)
}

/// Remove boxes pinfold labeled whose owning `box up` process is gone.
fn prune(args: &[OsString]) -> io::Result<i32> {
    if !args.is_empty() {
        return Err(usage("prune takes no arguments"));
    }
    let runtime = runtime()?;
    for box_ in runtime.list()? {
        if !box_
            .labels
            .keys()
            .any(|key| key.starts_with("dev.pinfold."))
        {
            continue;
        }
        let owner = box_
            .labels
            .get("dev.pinfold.owner")
            .and_then(|pid| pid.parse::<i32>().ok());
        if owner.is_some_and(alive) {
            continue;
        }
        runtime.down(&box_.id)?;
        let _ = fs::remove_dir_all(state_path(&box_.id)?);
    }
    Ok(0)
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

fn parse_label(args: &[OsString]) -> io::Result<(String, String)> {
    let mut label = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg.to_str() != Some("--label") {
            return Err(usage("list takes only --label k=v"));
        }
        let value = args
            .next()
            .and_then(|value| value.to_str())
            .and_then(|value| value.split_once('='))
            .ok_or_else(|| usage("--label needs k=v"))?;
        label = Some((value.0.to_string(), value.1.to_string()));
    }
    label.ok_or_else(|| usage("list needs --label k=v"))
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

fn alive(pid: i32) -> bool {
    match kill(Pid::from_raw(pid), None) {
        Ok(()) | Err(nix::errno::Errno::EPERM) => true,
        Err(_) => false,
    }
}

fn exit_code(status: ExitStatus) -> i32 {
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
fn init_path() -> io::Result<PathBuf> {
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
    let dir = dirs::cache_dir()?
        .join("artifacts")
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
