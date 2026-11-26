//! The end-to-end suite's harness. It drives the real `pinfold` binary
//! through a user's seams only: the CLI, environment variables, XDG roots
//! and the box spec. Nothing here imports pinfold.

use std::fs;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

/// A build of pinfold or of the default image, cold.
const BUILD: Duration = Duration::from_secs(1800);
/// Any other command a test runs on the host.
pub const COMMAND: Duration = Duration::from_secs(120);

/// The `pinfold` binary built from this checkout. Linux boxes run the CLI
/// itself as PID 1, so there it is the static musl release build.
pub fn pinfold() -> &'static Path {
    static BINARY: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    once(&BINARY, || {
        let mut command = Command::new(env!("CARGO"));
        command
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .args([
                "build",
                "-p",
                "pinfold",
                "--locked",
                "--message-format",
                "json-render-diagnostics",
            ]);
        // Cargo gives a test its own package's CARGO_PKG_* and
        // CARGO_MANIFEST_*; ring's build script reruns when those change.
        remove_vars(&mut command, "CARGO_PKG_");
        remove_vars(&mut command, "CARGO_MANIFEST_");
        if cfg!(target_os = "linux") {
            let triple = format!("{}-unknown-linux-musl", std::env::consts::ARCH);
            command.args(["--release", "--target", &triple]);
        }
        let output = run(&mut command, BUILD);
        assert_eq!(output.code, 0, "cargo build -p pinfold: {}", output.stderr);
        // The test executable may run from a copy (scripts/run-mac-e2e.sh),
        // so the path comes from Cargo, not from the test's own location.
        output
            .stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|message| {
                message["reason"] == "compiler-artifact"
                    && message["target"]["name"] == "pinfold"
                    && message["target"]["kind"] == serde_json::json!(["bin"])
            })
            .and_then(|message| message["executable"].as_str().map(PathBuf::from))
            .expect("cargo build reported the pinfold executable")
    })
}

/// Shared setup that runs once per suite run. A failure is kept, so every
/// later caller fails at once with the first error instead of retrying it.
fn once<T>(cell: &'static OnceLock<Result<T, String>>, init: impl FnOnce() -> T) -> &'static T {
    cell.get_or_init(|| {
        panic::catch_unwind(AssertUnwindSafe(init)).map_err(|payload| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
                .unwrap_or_default()
        })
    })
    .as_ref()
    .unwrap_or_else(|error| panic!("shared setup failed earlier: {error}"))
}

/// A finished host command.
#[derive(Debug)]
pub struct Exec {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Exec {
    /// Exit 0, its stdout trimmed; otherwise fail naming `what`.
    pub fn ok(self, what: impl std::fmt::Display) -> String {
        assert_eq!(self.code, 0, "{what}: {self:?}");
        self.stdout.trim().to_string()
    }
}

/// Run `command` with null stdin, capturing its output. Past `timeout` it
/// is killed and the test fails naming it; nothing waits unbounded.
pub fn run(command: &mut Command, timeout: Duration) -> Exec {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn(command);
    let (send, read) = mpsc::channel();
    for (index, mut pipe) in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ]
    .into_iter()
    .enumerate()
    {
        let send = send.clone();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = send.send((index, bytes));
        });
    }
    let deadline = Instant::now() + timeout;
    let mut streams = [Vec::new(), Vec::new()];
    for _ in 0..2 {
        let left = deadline.saturating_duration_since(Instant::now());
        match read.recv_timeout(left) {
            Ok((index, bytes)) => streams[index] = bytes,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{command:?} did not finish within {timeout:?}");
            }
        }
    }
    // The pipes are closed; the command itself gets the rest of the deadline.
    let status = wait_until(&mut child, deadline)
        .unwrap_or_else(|| panic!("{command:?} did not exit within {timeout:?}"));
    let [stdout, stderr] = streams.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    Exec {
        code: exit_code(status),
        stdout,
        stderr,
    }
}

/// Spawn `command`, one spawn at a time in this process. On macOS std
/// makes a child's stdio pipes with `pipe()` and sets CLOEXEC in separate
/// calls, so a spawn on another thread can inherit them: a refused `box up` then
/// exits, but the other `box up` holds its stdout open (#379). No user
/// login reaches it ([`scrub`]).
pub fn spawn(command: &mut Command) -> Child {
    scrub(command);
    static SPAWN: Mutex<()> = Mutex::new(());
    let _spawning = SPAWN.lock().unwrap_or_else(PoisonError::into_inner);
    command
        .spawn()
        .unwrap_or_else(|error| panic!("spawn {command:?}: {error}"))
}

/// Wait for `child` until `deadline`; past it, kill and reap it and return
/// `None`.
fn wait_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait().expect("wait for a host process") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// The XDG roots one test's `pinfold` commands use, and whether a box
/// failed to end cleanly, so its roots are kept as evidence.
#[derive(Clone)]
struct Roots {
    config: PathBuf,
    state: PathBuf,
    cache: PathBuf,
    keep: Arc<AtomicBool>,
}

impl Roots {
    fn command(&self) -> Command {
        let mut command = Command::new(pinfold());
        self.apply(&mut command);
        command
    }

    /// These roots and no inherited `PINFOLD_*`, for pinfold itself or a
    /// command that runs it, as host `git` runs the remote helper.
    fn apply(&self, command: &mut Command) {
        remove_vars(command, "PINFOLD_");
        command
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache);
    }
}

/// Remove every inherited variable whose name starts with `prefix`.
fn remove_vars(command: &mut Command, prefix: &str) {
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|name| name.starts_with(prefix)) {
            command.env_remove(name);
        }
    }
}

/// Remove every inherited variable that may hold a credential or locate a
/// login store, so no user login reaches a command the harness runs. A
/// test's own variables must not match. `GIT_*` is left whole: removing
/// `GIT_CONFIG_KEY_n` alone would break Git, and project commands drop the
/// family (`host_git_env`).
fn scrub(command: &mut Command) {
    const MARKS: [&str; 6] = ["TOKEN", "KEY", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH"];
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|name| {
            let name = name.to_ascii_uppercase();
            !name.starts_with("GIT_")
                && (MARKS.iter().any(|mark| name.contains(mark)) || name == "CODEX_HOME")
        }) {
            command.env_remove(name);
        }
    }
}

/// pinfold's daily maintenance pass, which a state root's first working
/// command runs, prunes every dead box on the host. Each [`TestEnv`] spends
/// its root's pass under this lock, so a test holding it (from an owner's
/// SIGKILL to its own `box prune`) keeps its leftover until then. Never
/// create a `TestEnv` while holding it: the same thread would deadlock.
static MAINTENANCE: Mutex<()> = Mutex::new(());

pub fn hold_maintenance() -> MutexGuard<'static, ()> {
    MAINTENANCE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One test's throwaway config and state roots, removed on drop unless a
/// box failed to end cleanly. The cache root is the suite run's own (see
/// [`suite_cache`]) unless the test asks for its own. Every disk in its
/// state root, whose pathname no earlier run used, is removed by name on
/// drop, after the test's boxes, which are declared after it.
pub struct TestEnv {
    root: PathBuf,
    roots: Roots,
}

impl TestEnv {
    pub fn new(test: &str) -> TestEnv {
        TestEnv::with_cache(test, Some(suite_cache()))
    }

    /// A test with a cache root of its own, inside its directory. On Linux
    /// that is its own builder and image store: its builds neither wait
    /// for nor are cleaned with the suite's, and they start cold.
    pub fn own_cache(test: &str) -> TestEnv {
        TestEnv::with_cache(test, None)
    }

    fn with_cache(test: &str, cache: Option<&Path>) -> TestEnv {
        // A random component, so the root's pathname never recurs: on macOS
        // a state root's disks are keyed by it, and teardown removes every
        // disk under the key.
        let token = RandomState::new().build_hasher().finish();
        let root = temp().join(format!(
            "pinfold-e2e-{}-{token:016x}-{test}",
            std::process::id()
        ));
        let roots = Roots {
            config: root.join("config"),
            state: root.join("state"),
            cache: cache.map_or_else(|| root.join("cache"), Path::to_path_buf),
            keep: Arc::default(),
        };
        // A new directory, so cleanup never adopts an older run's state.
        fs::create_dir(&root).unwrap_or_else(|error| panic!("create {}: {error}", root.display()));
        let env = TestEnv { root, roots };
        fs::create_dir(&env.roots.config).unwrap();
        fs::create_dir(&env.roots.state).unwrap();
        fs::create_dir_all(&env.roots.cache).unwrap();
        // Spend this root's daily pass now (see MAINTENANCE).
        let _hold = hold_maintenance();
        run(env.command().args(["disk", "list", "--json"]), COMMAND).ok("pinfold disk list");
        env
    }

    /// The test's own directory, holding its roots; files a test makes go
    /// here too.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `XDG_STATE_HOME` this test's commands use.
    pub fn state_home(&self) -> &Path {
        &self.roots.state
    }

    /// The `XDG_CONFIG_HOME` this test's commands use.
    pub fn config_home(&self) -> &Path {
        &self.roots.config
    }

    /// The `XDG_CACHE_HOME` this test's commands use.
    pub fn cache_home(&self) -> &Path {
        &self.roots.cache
    }

    /// `pinfold` with this test's roots and no inherited `PINFOLD_*`.
    pub fn command(&self) -> Command {
        let mut command = self.roots.command();
        // Outside any project, so no command discovers this checkout.
        command.current_dir(&self.root);
        command
    }

    /// `pinfold box exec NAME -- ARGV`.
    pub fn exec(&self, name: &str, argv: &[&str]) -> Exec {
        run(
            self.command().args(["box", "exec", name, "--"]).args(argv),
            COMMAND,
        )
    }

    /// Spawn `box up` with `spec` and wait for its `ready` line.
    pub fn up(&self, spec: &Value) -> BoxUp {
        self.start(spec)
            .unwrap_or_else(|line| panic!("box up {}: {line}", spec["name"]))
    }

    /// Spawn `box up` with `spec`: the running box, or the line it printed
    /// instead of `ready` once `up` has exited 1.
    pub fn start(&self, spec: &Value) -> Result<BoxUp, Value> {
        let name = spec["name"].as_str().expect("the spec names its box");
        let mut child = spawn(
            self.command()
                .args(["box", "up"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit()),
        );
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("box up stdout");
        let (send, lines) = mpsc::channel();
        // The guard owns the child before anything else can fail.
        let mut up = BoxUp {
            name: name.to_string(),
            roots: self.roots.clone(),
            child: Some(child),
            stdin,
            lines,
            ready: Value::Null,
            failed: false,
        };
        forward_lines(stdout, send);
        up.stdin
            .as_mut()
            .expect("box up stdin")
            .write_all(spec.to_string().as_bytes())
            .expect("write the box spec");
        // A cold Linux cache fetches the pinned VMM and kernel first.
        let line = up
            .lines
            .recv_timeout(Duration::from_secs(600))
            .unwrap_or_else(|error| panic!("box up {name} printed no first line: {error}"));
        let first: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("box up's first line {line:?}: {error}"));
        if first["event"] == "ready" {
            up.ready = first;
            return Ok(up);
        }
        // Not ready: `up` ends on its own and exits 1 (Process interface).
        drop(up.stdin.take());
        assert!(
            up.drain(Instant::now() + COMMAND).is_some(),
            "box up {name} did not end after {first}"
        );
        let status = up.reap(COMMAND);
        assert_eq!(
            status.map(exit_code),
            Some(1),
            "box up {name}'s exit after {first}"
        );
        Err(first)
    }

    /// `pinfold box list` lines for the box whose generation is `run_id`.
    pub fn listed(&self, run_id: &str) -> Vec<Value> {
        json_lines(&list_generation(self.command(), run_id).ok("box list"))
    }
}

/// Remove every disk in `roots`' state root by name.
fn remove_disks(roots: &Roots) -> Result<(), String> {
    let command = |args: &[&str]| {
        let mut command = roots.command();
        command.current_dir(&roots.state).args(["disk"]).args(args);
        run(&mut command, COMMAND)
    };
    let listed = command(&["list", "--json"]);
    if listed.code != 0 {
        return Err(format!("disk list: {listed:?}"));
    }
    let mut failed = Vec::new();
    for disk in json_lines(&listed.stdout) {
        let name = disk["name"].as_str().unwrap_or_default();
        let removed = command(&["rm", name, "--json"]);
        if removed.code != 0 {
            failed.push(format!("disk rm {name}: {removed:?}"));
        }
    }
    match failed.is_empty() {
        true => Ok(()),
        false => Err(failed.join("; ")),
    }
}

/// Run `work` on its own thread, so a panic or a deadline in it never
/// panics a Drop.
pub fn in_drop(work: impl FnOnce() -> Result<(), String> + Send + 'static) -> Result<(), String> {
    let (send, done) = mpsc::channel();
    thread::spawn(move || {
        let _ = send.send(work());
    });
    done.recv_timeout(COMMAND * 3)
        .unwrap_or_else(|_| Err("did not finish".to_string()))
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let roots = self.roots.clone();
        if let Err(error) = in_drop(move || remove_disks(&roots)) {
            eprintln!("e2e: DISKS LEFT in {}: {error}", self.root.display());
            self.roots.keep.store(true, Ordering::SeqCst);
        }
        if self.roots.keep.load(Ordering::SeqCst) {
            eprintln!("e2e: kept {} for recovery", self.root.display());
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

/// A throwaway project folder holding a Git repository, run as a user runs
/// it: `pinfold` in its root with a non-terminal stdin, and host `git` with
/// `git-remote-pinfold` on PATH and none of the user's Git configuration.
/// Dropping it runs `pinfold stop` there; declare it after its [`TestEnv`],
/// whose drop then removes the project's disks.
pub struct Project<'a> {
    env: &'a TestEnv,
    root: PathBuf,
    config_dir: Option<PathBuf>,
}

impl<'a> Project<'a> {
    /// A new project, configured as on a first run ([`Project::configure`]).
    pub fn new(env: &'a TestEnv, name: &str) -> Project<'a> {
        let mut project = Project::unconfigured(env, name);
        project.configure();
        project
    }

    /// `git init` a new folder `name` in the test's directory, with no
    /// template and so no hooks directory. It has no configuration.
    pub fn unconfigured(env: &'a TestEnv, name: &str) -> Project<'a> {
        let root = env.root().join(name);
        fs::create_dir(&root).unwrap_or_else(|error| panic!("create {}: {error}", root.display()));
        host_git_setup(env);
        run(
            git_command(env, &root).args(["init", "-q", "--template="]),
            COMMAND,
        )
        .ok("git init");
        Project {
            env,
            root,
            config_dir: None,
        }
    }

    /// Create the project's configuration with `config edit`, as on a first
    /// run: a share on macOS, a disk on Linux.
    pub fn configure(&mut self) {
        let projects = self.env.config_home().join("pinfold/projects");
        let before = if projects.is_dir() {
            dir_names(&projects)
        } else {
            Vec::new()
        };
        self.pinfold(&["config", "edit"]).ok("config edit");
        let new: Vec<String> = dir_names(&projects)
            .into_iter()
            .filter(|name| !before.contains(name))
            .collect();
        assert_eq!(
            new.len(),
            1,
            "config edit's new project directories: {new:?}"
        );
        self.config_dir = Some(projects.join(&new[0]));
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project's configuration directory, the one `config edit` made
    /// under the config root.
    pub fn config_dir(&self) -> &Path {
        self.config_dir.as_deref().expect("a configured project")
    }

    /// `pinfold ARGS` in the project's root, with `VISUAL=true` so `config
    /// edit` exits at once.
    pub fn pinfold(&self, args: &[&str]) -> Exec {
        run(pinfold_command(self.env, &self.root).args(args), COMMAND)
    }

    /// `pinfold config --json`, parsed.
    pub fn config(&self) -> Value {
        let out = self.pinfold(&["config", "--json"]).ok("config --json");
        serde_json::from_str(&out).expect("config JSON")
    }

    /// `pinfold ARGS` in the project's root, to spawn.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = pinfold_command(self.env, &self.root);
        command.args(args);
        command
    }

    /// Host `git ARGS` in the project's root.
    pub fn git(&self, args: &[&str]) -> Exec {
        run(&mut self.git_command(args), COMMAND)
    }

    /// Host `git ARGS` in the project's root, to adjust or spawn.
    pub fn git_command(&self, args: &[&str]) -> Command {
        let mut command = git_command(self.env, &self.root);
        command.args(args);
        command
    }
}

impl Drop for Project<'_> {
    fn drop(&mut self) {
        let mut stop = pinfold_command(self.env, &self.root);
        stop.arg("stop");
        let mut toplevel = git_command(self.env, &self.root);
        toplevel.args(["rev-parse", "--show-toplevel"]);
        let root = self.root.clone();
        if let Err(error) = in_drop(move || {
            // `stop` acts on the project it discovers from here: were this
            // repository gone, that could be an enclosing one's.
            let found = run(&mut toplevel, COMMAND);
            if !root.join(".git").is_dir()
                || found.code != 0
                || Path::new(found.stdout.trim()) != root
            {
                return Err(format!(
                    "not stopped: no longer its own repository: {found:?}"
                ));
            }
            match run(&mut stop, COMMAND) {
                out if out.code == 0 => Ok(()),
                out => Err(format!("{out:?}")),
            }
        }) {
            eprintln!(
                "e2e: PINFOLD STOP FAILED in {}: {error}; its roots are kept",
                self.root.display()
            );
            self.env.roots.keep.store(true, Ordering::SeqCst);
            if !thread::panicking() {
                panic!(
                    "pinfold stop in {} failed (see stderr)",
                    self.root.display()
                );
            }
        }
    }
}

/// A session script for `sh -c`: it prints `started`, then runs each line it
/// reads with `sh -c`, stdin closed, and prints `done` after it, until
/// stdin ends.
pub const READER: &str =
    "echo started; while read -r line; do sh -c \"$line\" < /dev/null; echo done; done";

/// A spawned project client (`pinfold -- …`, from [`Project::command`]) whose
/// session a test holds open: stdin and stdout piped, stderr inherited.
/// Dropping it kills and reaps it.
pub struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Client {
    pub fn spawn(mut command: Command) -> Client {
        let mut child = spawn(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit()),
        );
        let (send, lines) = mpsc::channel();
        forward_lines(child.stdout.take().expect("client stdout"), send);
        Client {
            stdin: child.stdin.take(),
            child,
            lines,
        }
    }

    /// The command's next stdout line, waited for within the command
    /// timeout.
    pub fn line(&self) -> String {
        self.lines
            .recv_timeout(COMMAND)
            .unwrap_or_else(|error| panic!("client {}: no line: {error}", self.child.id()))
    }

    /// The client's pid.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// In a [`READER`] session: run `line`, and return the lines it printed.
    /// The whole exchange is bounded by the command timeout.
    pub fn ask(&mut self, line: &str) -> Vec<String> {
        let stdin = self.stdin.as_mut().expect("client stdin");
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .and_then(|()| stdin.flush())
            .expect("write to the client");
        let deadline = Instant::now() + COMMAND;
        let mut out = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(done) if done == "done" => return out,
                Ok(other) => out.push(other),
                Err(error) => panic!(
                    "client {}: {line:?} did not finish: {error}",
                    self.child.id()
                ),
            }
        }
    }

    /// Close stdin and wait, bounded, for the client's exit code.
    pub fn finish(mut self) -> i32 {
        drop(self.stdin.take());
        let status = wait_until(&mut self.child, Instant::now() + COMMAND)
            .unwrap_or_else(|| panic!("client {} did not exit", self.child.id()));
        exit_code(status)
    }

    /// SIGKILL the client and reap it.
    pub fn kill(mut self) {
        self.child.kill().expect("SIGKILL the client");
        self.child.wait().expect("reap the client");
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Send each line `stdout` prints, from a thread of its own.
fn forward_lines(stdout: impl Read + Send + 'static, send: mpsc::Sender<String>) {
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if send.send(line).is_err() {
                return;
            }
        }
    });
}

/// The test's throwaway Git configuration and its `git-remote-pinfold`
/// link, made once per test.
fn host_git_setup(env: &TestEnv) {
    let bin = env.root().join("bin");
    if bin.exists() {
        return;
    }
    fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(pinfold(), bin.join("git-remote-pinfold")).unwrap();
    fs::write(
        env.root().join("gitconfig"),
        "[user]\n\tname = e2e\n\temail = e2e@example.invalid\n\
         [init]\n\tdefaultBranch = main\n\
         [commit]\n\tgpgSign = false\n[tag]\n\tgpgSign = false\n\
         [gc]\n\tauto = 0\n[maintenance]\n\tauto = false\n",
    )
    .unwrap();
    fs::write(env.root().join("gitconfig-system"), "").unwrap();
}

fn pinfold_command(env: &TestEnv, root: &Path) -> Command {
    let mut command = env.command();
    host_git_env(env, &mut command);
    command.current_dir(root).env("VISUAL", "true");
    command
}

fn git_command(env: &TestEnv, root: &Path) -> Command {
    let mut command = Command::new("git");
    host_git_env(env, &mut command);
    env.roots.apply(&mut command);
    command.current_dir(root);
    command
}

/// Host Git with the test's configuration only, and the test's helper link
/// first on PATH.
fn host_git_env(env: &TestEnv, command: &mut Command) {
    remove_vars(command, "GIT_");
    first_on_path(command, &env.root().join("bin"));
    command
        .env("GIT_CONFIG_GLOBAL", env.root().join("gitconfig"))
        .env("GIT_CONFIG_SYSTEM", env.root().join("gitconfig-system"));
}

/// A stand-in for the 1Password CLI: `op` in a new directory `name` in the
/// test's directory, outside its XDG roots and projects. For `op read
/// --no-newline REFERENCE` it prints the value `values` gives REFERENCE,
/// with no newline. For anything else it prints `decoy` on stdout and
/// stderr and exits 1. It appends each call's reference to `calls` there
/// ([`op_calls`]). Values, references and the decoy are shell-safe words.
/// Returns the directory, to put first on PATH ([`first_on_path`]).
pub fn stand_in_op(env: &TestEnv, name: &str, values: &[(&str, &str)], decoy: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = env.root().join(name);
    fs::create_dir(&dir).unwrap_or_else(|error| panic!("create {}: {error}", dir.display()));
    let cases: String = values
        .iter()
        .map(|(reference, value)| format!("    '{reference}') printf %s '{value}'; exit 0 ;;\n"))
        .collect();
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$3\" >> '{}'\n\
         if [ \"$#\" = 3 ] && [ \"$1\" = read ] && [ \"$2\" = --no-newline ]; then\n  \
         case \"$3\" in\n{cases}  esac\nfi\n\
         printf %s '{decoy}'\nprintf %s '{decoy}' >&2\nexit 1\n",
        dir.join("calls").display()
    );
    let op = dir.join("op");
    fs::write(&op, script).unwrap();
    fs::set_permissions(&op, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// The references the stand-in `op` in `dir` was asked for, in order.
pub fn op_calls(dir: &Path) -> Vec<String> {
    match fs::read_to_string(dir.join("calls")) {
        Ok(text) => text.lines().map(str::to_string).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("read {}: {error}", dir.display()),
    }
}

/// Put `dir` first on `command`'s PATH: the one it sets, else this
/// process's.
pub fn first_on_path(command: &mut Command, dir: &Path) {
    let current = command
        .get_envs()
        .find(|(name, _)| *name == "PATH")
        .and_then(|(_, value)| value.map(std::ffi::OsStr::to_os_string))
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let mut path = vec![dir.to_path_buf()];
    path.extend(std::env::split_paths(&current));
    command.env("PATH", std::env::join_paths(path).expect("a PATH"));
}

/// Every path under `dir`, sorted, with its length and mtime, read without
/// following symlinks; none when `dir` is missing. Two equal trees hold the
/// same entries, unchanged as far as their metadata shows.
pub fn tree(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => panic!("read {}: {error}", dir.display()),
    };
    let mut found = Vec::new();
    for entry in entries {
        let path = entry.unwrap().path();
        let metadata = fs::symlink_metadata(&path).unwrap();
        found.push((path.clone(), metadata.len(), metadata.modified().unwrap()));
        if metadata.is_dir() {
            found.extend(tree(&path));
        }
    }
    found.sort();
    found
}

/// The names in `dir`, sorted.
pub fn dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// Each line of `text` as JSON.
pub fn json_lines(text: &str) -> Vec<Value> {
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{line:?}: {e}")))
        .collect()
}

/// The canonical temp dir: on macOS `/tmp` and `$TMPDIR` are symlinks, and
/// the runtime wants real paths.
fn temp() -> PathBuf {
    fs::canonicalize(std::env::temp_dir()).expect("canonicalize the temp dir")
}

/// One cache root for the whole suite run, so the pinned VMM, kernel and
/// (on Linux) the default image are fetched and built once. It lives with
/// the runner's temp files and is reused by later runs.
fn suite_cache() -> &'static Path {
    static CACHE: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    once(&CACHE, || {
        // A project discovers an enclosing repository; the suite's own
        // must have none.
        let mut git = Command::new("git");
        remove_vars(&mut git, "GIT_");
        let inside = run(
            git.current_dir(temp())
                .args(["rev-parse", "--show-toplevel"]),
            COMMAND,
        );
        assert_ne!(
            inside.code,
            0,
            "the temp dir {} is inside a Git work tree; set TMPDIR outside it",
            temp().display()
        );
        let cache = temp().join("pinfold-e2e-cache");
        fs::create_dir_all(&cache).unwrap();
        cache
    })
}

/// The built-in `default` profile's image, which has curl and perl. On
/// macOS images live in the runtime's host-wide store: an existing build is
/// used as it is, since a rebuild would move a shared tag. On Linux images
/// live in the suite cache, so the suite builds it once per run.
pub fn default_image() -> &'static str {
    const IMAGE: &str = "localhost/pinfold/profile-default:latest";
    static BUILT: OnceLock<Result<(), String>> = OnceLock::new();
    once(&BUILT, || {
        let present = cfg!(target_os = "macos")
            && run(
                Command::new("container").args(["image", "inspect", IMAGE]),
                COMMAND,
            )
            .code
                == 0;
        if !present {
            let env = TestEnv::new("default-image");
            run(env.command().args(["build", "--profile", "default"]), BUILD)
                .ok("pinfold build --profile default");
        }
    });
    IMAGE
}

/// A running box and the `box up` process that owns it. Dropping it closes
/// `up`'s stdin, which is `down`, and waits, bounded, for teardown.
pub struct BoxUp {
    name: String,
    roots: Roots,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    ready: Value,
    failed: bool,
}

impl BoxUp {
    /// The `box up` process's pid, from the host.
    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("a running owner").id()
    }

    /// The `ready` line.
    pub fn ready(&self) -> &Value {
        &self.ready
    }

    /// Wait, bounded, for `up` to end on its own with stdin still open:
    /// its last line and its exit code.
    pub fn ended(mut self) -> (Value, i32) {
        let deadline = Instant::now() + COMMAND;
        let last = self
            .drain(deadline)
            .unwrap_or_else(|| panic!("box up {} did not end", self.name));
        // The owner stays in the guard until its `down` line is parsed, so a
        // panic here still gets recovery and keeps the roots.
        let status = loop {
            let child = self.child.as_mut().expect("a running owner");
            if let Some(status) = child.try_wait().expect("wait for box up") {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "box up {} was not reaped",
                self.name
            );
            thread::sleep(Duration::from_millis(10));
        };
        let last =
            last.unwrap_or_else(|| panic!("box up {} printed nothing after ready", self.name));
        let line: Value = serde_json::from_str(&last)
            .unwrap_or_else(|error| panic!("box up's last line {last:?}: {error}"));
        assert_eq!(line["event"], "down", "box up's last line {last:?}");
        self.child = None;
        (line, exit_code(status))
    }

    /// SIGKILL the owner and reap it. On Apple the guest outlives it. On
    /// Linux the VM ends with its owner: every host process below the
    /// owner, the VMM among them, is gone before this returns.
    pub fn kill_owner(mut self) -> Leftover {
        let mut child = self.child.take().expect("a running owner");
        let leftover = Leftover {
            name: self.name.clone(),
            run_id: self.ready["run_id"].as_str().expect("run_id").to_string(),
            owner: child.id(),
            roots: self.roots.clone(),
        };
        #[cfg(target_os = "linux")]
        let below = Below::of(child.id());
        child.kill().expect("SIGKILL box up");
        self.child = Some(child);
        let status = self.reap(COMMAND);
        assert_eq!(status.map(exit_code), Some(128 + 9), "box up after SIGKILL");
        #[cfg(target_os = "linux")]
        below.wait_gone();
        leftover
    }

    fn teardown(&mut self) {
        if self.child.is_none() {
            return;
        }
        // Never kill the owner: on Apple the guest would outlive it.
        drop(self.stdin.take());
        if self
            .drain(Instant::now() + Duration::from_secs(120))
            .is_none()
        {
            // Before `ready` the name may be another box's: a refused `up`
            // names a box that exists.
            let Some(run_id) = self.ready["run_id"].as_str().map(str::to_string) else {
                self.fail("stdin EOF did not end box up within 120 s, before ready; left as it is");
                return;
            };
            self.fail("stdin EOF did not end box up within 120 s; trying box down");
            let (name, roots) = (self.name.clone(), self.roots.clone());
            if let Err(error) = in_drop(move || down_generation(&roots, &name, &run_id)) {
                self.fail(&error);
            }
            if self
                .drain(Instant::now() + Duration::from_secs(60))
                .is_none()
            {
                self.fail("box up still running after box down; left as it is");
                return;
            }
        }
        match self.reap(Duration::from_secs(30)) {
            Some(status) if status.success() => {}
            Some(status) => self.fail(&format!("box up exited {status}")),
            None => self.fail("box up was not reaped within 30 s"),
        }
    }

    /// The egress log `ready` names, canonical, once checked to be a
    /// regular file under the test's own state root.
    pub fn egress_log_path(&self) -> PathBuf {
        let named = self.ready["egress_log"]
            .as_str()
            .unwrap_or_else(|| panic!("ready names no egress log: {}", self.ready));
        let path = fs::canonicalize(named).unwrap_or_else(|error| panic!("{named}: {error}"));
        let root = fs::canonicalize(&self.roots.state).expect("the state root");
        assert!(
            path.starts_with(&root) && fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()),
            "the egress log {named} is not a regular file under {}",
            root.display()
        );
        path
    }

    /// The decisions in this run's egress log.
    pub fn egress_log(&self) -> Vec<Value> {
        let path = self.egress_log_path();
        json_lines(
            &fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
        )
    }

    /// Read `up`'s stdout to EOF by `deadline`: the last line read, or
    /// `None` when it did not end.
    fn drain(&self, deadline: Instant) -> Option<Option<String>> {
        let mut last = None;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => last = Some(line),
                Err(RecvTimeoutError::Disconnected) => return Some(last),
                Err(RecvTimeoutError::Timeout) => return None,
            }
        }
    }

    /// Wait for the owner within `timeout`, without blocking past it.
    fn reap(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let mut child = self.child.take()?;
        let (send, status) = mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(child.wait());
        });
        status.recv_timeout(timeout).ok().and_then(Result::ok)
    }

    fn fail(&mut self, what: &str) {
        self.failed = true;
        eprintln!(
            "e2e: BOX {} DID NOT END CLEANLY: {what}; its roots are kept",
            self.name
        );
        self.roots.keep.store(true, Ordering::SeqCst);
    }
}

impl Drop for BoxUp {
    fn drop(&mut self) {
        self.teardown();
        if self.failed && !thread::panicking() {
            panic!("box {} did not end cleanly (see stderr)", self.name);
        }
    }
}

/// The host processes below an owner, the VMM among them, taken from
/// /proc before the owner is killed.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct Below(Vec<(u32, String)>);

#[cfg(target_os = "linux")]
impl Below {
    /// Every live process below `pid`, with its command name; at least one.
    pub fn of(pid: u32) -> Below {
        let parents: Vec<_> = live()
            .map(|(child, _, parent, _, name)| (child, parent, name))
            .collect();
        let mut found = Vec::new();
        let mut frontier = vec![pid];
        while let Some(parent) = frontier.pop() {
            for (child, _, name) in parents.iter().filter(|(_, p, _)| *p == parent) {
                found.push((*child, name.clone()));
                frontier.push(*child);
            }
        }
        assert!(!found.is_empty(), "no process below {pid}: no VMM");
        Below(found)
    }

    /// The pids whose command name is `name`.
    pub fn named(&self, name: &str) -> Vec<u32> {
        self.0
            .iter()
            .filter(|(_, comm)| comm == name)
            .map(|(pid, _)| *pid)
            .collect()
    }

    /// Wait, bounded, until each process has exited: gone from /proc, or a
    /// zombie no one has reaped yet.
    pub fn wait_gone(&self) {
        let deadline = Instant::now() + COMMAND;
        while self.0.iter().any(|(pid, _)| !exited(*pid)) {
            assert!(Instant::now() < deadline, "outlived their owner: {self:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

/// `/proc/PID/stat`'s pid, state, parent pid, session id and command name.
#[cfg(target_os = "linux")]
fn stat(pid: u32) -> Option<(u32, String, u32, u32, String)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `PID (COMM) STATE PPID PGRP SESSION ...`; COMM may hold spaces and
    // parentheses.
    let (head, tail) = stat.rsplit_once(')')?;
    let (_, name) = head.split_once('(')?;
    let mut fields = tail.split_whitespace();
    let state = fields.next()?.to_string();
    let parent = fields.next()?.parse().ok()?;
    let session = fields.nth(1)?.parse().ok()?;
    Some((pid, state, parent, session, name.to_string()))
}

/// The live direct children of `pid` that lead a session of their own
/// (`setsid`), from /proc: a client's detached owner, not the forks it
/// has yet to exec.
#[cfg(target_os = "linux")]
pub fn detached_children(pid: u32) -> Vec<u32> {
    live()
        .filter(|(child, _, parent, session, _)| *parent == pid && session == child)
        .map(|(child, ..)| child)
        .collect()
}

/// Every process in /proc that is not a zombie, as [`stat`] reads it.
#[cfg(target_os = "linux")]
fn live() -> impl Iterator<Item = (u32, String, u32, u32, String)> {
    fs::read_dir("/proc")
        .expect("read /proc")
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(stat)
        .filter(|(_, state, ..)| state != "Z")
}

/// Wait, bounded, until `pid` has exited (see [`exited`]).
#[cfg(target_os = "linux")]
pub fn wait_exited(pid: u32) {
    let deadline = Instant::now() + COMMAND;
    while !exited(pid) {
        assert!(Instant::now() < deadline, "{pid} did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether `pid` has exited: gone from /proc (ENOENT, or ESRCH while it
/// goes), or a zombie. A stat that cannot be read for another reason fails
/// the test.
#[cfg(target_os = "linux")]
fn exited(pid: u32) -> bool {
    const ESRCH: i32 = 3;
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.raw_os_error() == Some(ESRCH) =>
        {
            true
        }
        Err(error) => panic!("read /proc/{pid}/stat: {error}"),
        Ok(stat) => {
            let (_, tail) = stat.rsplit_once(')').expect("a stat line");
            tail.split_whitespace().next() == Some("Z")
        }
    }
}

/// A box whose owner a test killed. Dropping it removes the box by name
/// with `box down`, which removes only an orphan, if the runtime still
/// lists its generation. Drop it before reusing the name.
pub struct Leftover {
    name: String,
    run_id: String,
    owner: u32,
    roots: Roots,
}

impl Leftover {
    /// A leftover found by `box list`, whose owner the test killed itself.
    pub fn listed(env: &TestEnv, line: &Value) -> Leftover {
        Leftover {
            name: line["name"].as_str().expect("name").to_string(),
            run_id: line["labels"]["dev.pinfold.generation"]
                .as_str()
                .expect("generation")
                .to_string(),
            owner: line["owner"].as_u64().expect("owner") as u32,
            roots: env.roots.clone(),
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// The killed owner's pid.
    pub fn owner(&self) -> u32 {
        self.owner
    }

    /// SIGKILL the owner, once `box list` shows that pid still owning this
    /// generation: a pid from an earlier listing may since be another
    /// process's. A short window between the check and the kill remains.
    pub fn kill(&self) -> Exec {
        let out = list_generation(self.roots.command(), &self.run_id);
        let listed = json_lines(&out.stdout);
        assert!(
            out.code == 0
                && listed.len() == 1
                && listed[0]["owner"] == self.owner
                && listed[0]["owner_alive"] == true,
            "owner {} of generation {} before its SIGKILL: {out:?}",
            self.owner,
            self.run_id
        );
        run(
            Command::new("kill").args(["-KILL", &self.owner.to_string()]),
            COMMAND,
        )
    }
}

/// `box list` for generation `run_id`, run by `command`.
fn list_generation(mut command: Command, run_id: &str) -> Exec {
    let label = format!("dev.pinfold.generation={run_id}");
    run(command.args(["box", "list", "--label", &label]), COMMAND)
}

/// `box down NAME` only while the runtime lists generation `run_id`, so it
/// never acts on another box of that name; then the generation must be gone.
fn down_generation(roots: &Roots, name: &str, run_id: &str) -> Result<(), String> {
    let listed = || {
        let out = list_generation(roots.command(), run_id);
        match out.code {
            0 => Ok(!out.stdout.trim().is_empty()),
            _ => Err(format!("box list: {out:?}")),
        }
    };
    if !listed()? {
        return Ok(());
    }
    let down = run(roots.command().args(["box", "down", name]), COMMAND);
    if down.code != 0 || listed()? {
        return Err(format!("box down {name}: {down:?}"));
    }
    Ok(())
}

impl Drop for Leftover {
    fn drop(&mut self) {
        let (name, run_id, roots) = (self.name.clone(), self.run_id.clone(), self.roots.clone());
        if let Err(error) = in_drop(move || down_generation(&roots, &name, &run_id)) {
            eprintln!("e2e: LEFTOVER BOX {} NOT REMOVED: {error}", self.name);
            self.roots.keep.store(true, Ordering::SeqCst);
        }
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// A box name unique to this suite run and test.
pub fn box_name(test: &str) -> String {
    format!("e2e-{}-{test}", std::process::id())
}

/// One request a [`Fixture`] received.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub target: String,
    /// Header lines as received, `(name, value)`.
    pub headers: Vec<(String, String)>,
}

impl Request {
    /// The value of every header named `name`, ignoring ASCII case.
    pub fn header(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

/// A host HTTP service on loopback, reachable from a box only through a
/// route. It answers every request and records it. It runs in the test
/// process, which on macOS holds the firewall exception
/// (scripts/run-mac-e2e.sh). Its listener lives until the test process
/// exits.
pub struct Fixture {
    port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
}

/// What a [`Fixture`] answers.
#[derive(Clone)]
enum Answer {
    /// This status and no body, to every request.
    Status(u16),
    /// To a POST whose body holds the prompt (the second field), a
    /// streamed model reply holding the first: OpenAI's chat completion at
    /// `…/chat/completions`, Anthropic's message at `/v1/messages`, an
    /// empty 200 elsewhere. Anything else gets 400, so a request whose body
    /// was lost fails.
    Model(Arc<str>, Arc<str>),
    /// 200 to every request, as GitHub's API names further URLs: a `Link`
    /// to [`GITHUB_NEXT`] and a JSON body whose `upload_url` is
    /// [`GITHUB_UPLOAD`] with its URI template.
    Github,
}

/// The absolute next-page URL [`Fixture::github`] links to.
pub const GITHUB_NEXT: &str = "https://api.github.com/user?page=2";
/// The absolute upload URL [`Fixture::github`] returns, without its template.
pub const GITHUB_UPLOAD: &str = "https://uploads.github.com/repos/e2e/repo/releases/1/assets";

impl Fixture {
    /// Answer every request with `status` and no body.
    pub fn start(status: u16) -> Fixture {
        Fixture::serving(Answer::Status(status))
    }

    /// A fake model whose every completion is `reply`, given to a POST
    /// whose body holds `prompt`.
    pub fn model(reply: &str, prompt: &str) -> Fixture {
        Fixture::serving(Answer::Model(reply.into(), prompt.into()))
    }

    /// A GitHub API stand-in that names absolute URLs on GitHub's hosts.
    pub fn github() -> Fixture {
        Fixture::serving(Answer::Github)
    }

    fn serving(answer: Answer) -> Fixture {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the fixture");
        let port = listener.local_addr().expect("fixture address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (seen, answer) = (Arc::clone(&seen), answer.clone());
                thread::spawn(move || serve(stream, &seen, &answer));
            }
        });
        Fixture { port, requests }
    }

    /// The `host:port` a route names to reach the fixture.
    pub fn route(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

/// Read one Content-Length-framed request, record it and answer it.
fn serve(stream: TcpStream, seen: &Mutex<Vec<Request>>, answer: &Answer) {
    if stream.set_read_timeout(Some(COMMAND)).is_err() {
        return;
    }
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    let mut length = 0;
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let Some((name, value)) = line.trim_end().split_once(':') else {
            break;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = value.parse().unwrap_or(0);
        }
        headers.push((name.to_string(), value.to_string()));
    }
    let mut sent = Vec::new();
    let _ = reader.by_ref().take(length).read_to_end(&mut sent);
    let mut extra = String::new();
    let (status, body) = match answer {
        Answer::Status(status) => (*status, String::new()),
        Answer::Github => {
            extra = format!("Link: <{GITHUB_NEXT}>; rel=\"next\"\r\n");
            (
                200,
                format!("{{\"upload_url\":\"{GITHUB_UPLOAD}{{?name,label}}\"}}"),
            )
        }
        Answer::Model(reply, prompt)
            if method == "POST"
                && sent
                    .windows(prompt.len())
                    .any(|window| window == prompt.as_bytes()) =>
        {
            (200, model_reply(&target, reply))
        }
        Answer::Model(..) => (400, String::new()),
    };
    seen.lock().unwrap().push(Request {
        method,
        target,
        headers,
    });
    let _ = (&stream).write_all(
        format!(
            "HTTP/1.1 {status} Fixture\r\nContent-Type: text/event-stream\r\n{extra}\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    );
}

/// A streamed completion of `reply` in the wire format `target` names.
fn model_reply(target: &str, reply: &str) -> String {
    let path = target.split('?').next().unwrap_or_default();
    if path.ends_with("/chat/completions") {
        let chunk = |delta: Value, finish: Value| {
            serde_json::json!({
                "id": "e2e", "object": "chat.completion.chunk", "created": 0, "model": "e2e",
                "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
            })
        };
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            chunk(
                serde_json::json!({ "role": "assistant", "content": reply }),
                Value::Null,
            ),
            chunk(serde_json::json!({}), "stop".into()),
        )
    } else if path == "/v1/messages" {
        let message = serde_json::json!({
            "id": "msg_e2e", "type": "message", "role": "assistant", "model": "e2e",
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": 1, "output_tokens": 1 },
        });
        [
            ("message_start", serde_json::json!({ "message": message })),
            (
                "content_block_start",
                serde_json::json!({
                    "index": 0, "content_block": { "type": "text", "text": "" },
                }),
            ),
            (
                "content_block_delta",
                serde_json::json!({
                    "index": 0, "delta": { "type": "text_delta", "text": reply },
                }),
            ),
            ("content_block_stop", serde_json::json!({ "index": 0 })),
            (
                "message_delta",
                serde_json::json!({
                    "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                    "usage": { "output_tokens": 1 },
                }),
            ),
            ("message_stop", serde_json::json!({})),
        ]
        .into_iter()
        .map(|(event, mut data)| {
            data["type"] = event.into();
            format!("event: {event}\ndata: {data}\n\n")
        })
        .collect()
    } else {
        String::new()
    }
}
