//! `pinfold box`: the JSON-on-stdio process interface Switchyard uses, and
//! `pinfold build`: the profile and project image build.
//!
//! The verbs are parsed by hand: the set is small, and ARCHITECTURE.md's
//! dependency list has no argument parser.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use crate::config::{Config, Containerfile};
use crate::core::artifacts;
use crate::core::r#box::Box;
use crate::core::clean;
use crate::core::plan::Plan;
use crate::core::profile::{Profile, valid_name};
use crate::core::runtime::{BoxInfo, BuildRequest, local_image_id, runtime};
use crate::dirs;
use crate::trust;

const USAGE: &str = "usage: pinfold box up|exec BOX [--tty] [--workdir DIR] -- argv|down BOX|list --label k=v|prune";
const BUILD_USAGE: &str = "usage: pinfold build [--profile NAME]";
const PROFILE_USAGE: &str = "usage: pinfold profile new NAME [--from PROFILE]";
const ALLOW_USAGE: &str = "usage: pinfold allow";
const ATTACH_USAGE: &str = "usage: pinfold attach [--box NAME] [cmd...]";
const CLEAN_USAGE: &str = "usage: pinfold clean [--dry-run] [--unused AGE]";

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
    let mut plan = Plan::from_reader(io::stdin()).map_err(io::Error::other)?;
    // The owner label is how `box prune` tells a live box from a leftover.
    plan.labels
        .insert(clean::OWNER_LABEL.into(), std::process::id().to_string());
    let init = init_path()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let mut box_ = Box::up(&plan, &init).await?;
        println!(
            "{}",
            serde_json::json!({ "event": "ready", "box": plan.name })
        );
        io::stdout().flush()?;
        box_.hold().await?;
        Ok(0)
    });
    // `Box::hold` watches stdin through tokio's blocking pool. A caller that
    // keeps stdin open leaves that read parked, and dropping the runtime
    // waits for it forever. Teardown is done, so leak the read and let the
    // process exit.
    runtime.shutdown_background();
    result
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
    clean::prune_boxes(runtime()?)?;
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

    // Measure everything before removing anything, so `--dry-run` lists the
    // sizes a real `clean` reclaims.
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
    let total = automatic + project_caches + project_state;
    if dry_run {
        println!("pinfold clean: dry run; {total} B reclaimable");
    } else {
        println!("pinfold clean: reclaiming {total} B");
    }
    println!("  automatic maintenance: {automatic} B");
    println!("  build cache: the runtime's builder container");
    println!("  project caches: {project_caches} B");
    println!("  project state: {project_state} B");

    if dry_run {
        return Ok(());
    }

    for dead in &boxes.dead {
        dead.remove(runtime)?;
    }
    clean::prune_sockets()?;
    artifacts::prune_unpinned()?;
    clean::prune_egress_logs()?;
    runtime.purge_build_cache()?;
    for cache in &caches {
        fs::remove_dir_all(cache)?;
    }
    for dir in &stale {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
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
