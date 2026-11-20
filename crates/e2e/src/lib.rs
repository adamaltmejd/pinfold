//! The end-to-end suite's harness. It drives the real `pinfold` binary
//! through a user's seams only: the CLI, environment variables, XDG roots
//! and the box spec. Nothing here imports pinfold.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;

/// A build of pinfold or of the default image, cold.
const BUILD: Duration = Duration::from_secs(1800);
/// Any other command a test runs on the host.
const COMMAND: Duration = Duration::from_secs(120);

/// The `pinfold` binary built from this checkout. Linux boxes run the CLI
/// itself as PID 1, so there it is the static musl release build.
pub fn pinfold() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
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
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| {
                name.starts_with("CARGO_PKG_") || name.starts_with("CARGO_MANIFEST_")
            }) {
                command.env_remove(name);
            }
        }
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

/// A finished host command.
#[derive(Debug)]
pub struct Exec {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run `command` with null stdin, capturing its output. Past `timeout` it
/// is killed and the test fails naming it; nothing waits unbounded.
pub fn run(command: &mut Command, timeout: Duration) -> Exec {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("spawn {command:?}: {error}"));
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
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for a host command") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{command:?} did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let [stdout, stderr] = streams.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    Exec {
        code: exit_code(status),
        stdout,
        stderr,
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
        for (name, _) in std::env::vars_os() {
            if name
                .to_str()
                .is_some_and(|name| name.starts_with("PINFOLD_"))
            {
                command.env_remove(name);
            }
        }
        command
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache);
        command
    }
}

/// One test's throwaway config and state roots, removed on drop unless a
/// box failed to end cleanly. The cache root is the suite run's own (see
/// [`suite_cache`]).
pub struct TestEnv {
    root: PathBuf,
    roots: Roots,
}

impl TestEnv {
    pub fn new(test: &str) -> TestEnv {
        let root = temp().join(format!("pinfold-e2e-{}-{test}", std::process::id()));
        let roots = Roots {
            config: root.join("config"),
            state: root.join("state"),
            cache: suite_cache().to_path_buf(),
            keep: Arc::default(),
        };
        fs::create_dir_all(&roots.config).unwrap();
        fs::create_dir_all(&roots.state).unwrap();
        TestEnv { root, roots }
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
        let name = spec["name"].as_str().expect("the spec names its box");
        let mut child = self
            .command()
            .args(["box", "up"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn pinfold box up");
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
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    return;
                }
            }
        });
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
        let ready: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("box up's first line {line:?}: {error}"));
        assert_eq!(ready["event"], "ready", "box up {name}: {ready}");
        up.ready = ready;
        up
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if self.roots.keep.load(Ordering::SeqCst) {
            eprintln!("e2e: kept {} for recovery", self.root.display());
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
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
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE.get_or_init(|| {
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
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let present = cfg!(target_os = "macos")
            && run(
                Command::new("container").args(["image", "inspect", IMAGE]),
                COMMAND,
            )
            .code
                == 0;
        if !present {
            let env = TestEnv::new("default-image");
            let build = run(env.command().args(["build", "--profile", "default"]), BUILD);
            assert_eq!(
                build.code, 0,
                "pinfold build --profile default: {}",
                build.stderr
            );
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
    /// The decisions in this run's egress log, at the path `ready` names.
    pub fn egress_log(&self) -> Vec<Value> {
        let path = self.ready["egress_log"]
            .as_str()
            .unwrap_or_else(|| panic!("ready names no egress log: {}", self.ready));
        fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {path}: {error}"))
            .lines()
            .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{line:?}: {e}")))
            .collect()
    }

    /// Read `up`'s stdout to EOF within `timeout`; whether it ended.
    fn drain(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(_) => {}
                Err(RecvTimeoutError::Disconnected) => return true,
                Err(RecvTimeoutError::Timeout) => return false,
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

impl BoxUp {
    fn teardown(&mut self) {
        // Never kill the owner: on Apple the guest would outlive it, and
        // only `box prune`, which acts host-wide, would remove it.
        drop(self.stdin.take());
        if !self.drain(Duration::from_secs(120)) {
            self.fail("stdin EOF did not end box up within 120 s; trying box down");
            let mut down = self.roots.command();
            down.args(["box", "down", &self.name]);
            let (send, done) = mpsc::channel();
            // `run` panics on its deadline; a Drop must not.
            thread::spawn(move || {
                let _ = send.send(run(&mut down, COMMAND).code);
            });
            match done.recv_timeout(COMMAND + Duration::from_secs(10)) {
                Ok(0) => {}
                Ok(code) => self.fail(&format!("box down exited {code}")),
                Err(_) => self.fail("box down did not finish"),
            }
            if !self.drain(Duration::from_secs(60)) {
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
}

pub fn exit_code(status: ExitStatus) -> i32 {
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
/// route. It answers every request with `status` and records it. It runs
/// in the test process, which on macOS holds the firewall exception
/// (scripts/run-mac-e2e.sh). Dropping it closes its listener.
pub struct Fixture {
    port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Fixture {
    pub fn start(status: u16) -> Fixture {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the fixture");
        let port = listener.local_addr().expect("fixture address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, stopped) = (Arc::clone(&requests), Arc::clone(&stop));
        let accept = thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                let seen = Arc::clone(&seen);
                thread::spawn(move || serve(stream, &seen, status));
            }
        });
        Fixture {
            port,
            requests,
            stop,
            accept: Some(accept),
        }
    }

    /// The `host:port` a route names to reach the fixture.
    pub fn route(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocked accept so the thread sees the flag and returns,
        // dropping the listener.
        if TcpStream::connect(("127.0.0.1", self.port)).is_ok()
            && let Some(accept) = self.accept.take()
        {
            let _ = accept.join();
        }
    }
}

/// Read one Content-Length-framed request, record it and answer it.
fn serve(stream: TcpStream, seen: &Mutex<Vec<Request>>, status: u16) {
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
    let _ = std::io::copy(&mut reader.by_ref().take(length), &mut std::io::sink());
    seen.lock().unwrap().push(Request {
        method,
        target,
        headers,
    });
    let _ = (&stream).write_all(
        format!("HTTP/1.1 {status} Fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    );
}
