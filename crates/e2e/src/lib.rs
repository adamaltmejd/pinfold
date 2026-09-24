//! End-to-end tests that drive the `pinfold` binary from outside.
//!
//! The tests live in `tests/` and run on a macOS host with the Apple
//! `container` CLI, or a Linux host with rootless podman. `cargo test -p e2e`
//! builds the binary and runs them, so that one command is the whole host
//! gate.
//!
//! This crate also holds the host fixtures the tests reach through routes,
//! and the built binary the test files drive.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// The built `pinfold` binary. The test executable lives in
/// `<target>/<profile>/deps`, so the binary is its sibling. On Linux the
/// box's PID 1 is the CLI's own executable, so the static musl target is
/// built and its binary used.
pub fn pinfold() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let triple = cfg!(target_os = "linux")
            .then(|| format!("{}-unknown-linux-musl", std::env::consts::ARCH));
        let mut command = Command::new(env!("CARGO"));
        command.args(["build", "-p", "pinfold", "--locked"]);
        if let Some(triple) = &triple {
            command.args(["--target", triple]);
        }
        let status = command.status().expect("run cargo build -p pinfold");
        assert!(status.success(), "cargo build -p pinfold failed");
        let exe = std::env::current_exe().expect("test executable path");
        let target = exe
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("target dir");
        let binary = match &triple {
            Some(triple) => target.join(triple).join("debug").join("pinfold"),
            None => target.join("debug").join("pinfold"),
        };
        assert!(binary.is_file(), "{} is missing", binary.display());
        binary
    })
}

/// Per-test XDG state, cache and config, so a test never touches the
/// operator's. An empty config dir also means `default` resolves to the
/// embedded profile, not the operator's own copy of it.
pub struct TestEnv {
    /// The test's scratch root; projects and fixtures live under it.
    pub root: PathBuf,
    /// `XDG_STATE_HOME`; pinfold's state dir is `<state>/pinfold`.
    pub state: PathBuf,
    /// `XDG_CACHE_HOME`, where built artifacts and the init land.
    pub cache: PathBuf,
    /// `XDG_CONFIG_HOME`, where profiles live. Empty, so `default` is the
    /// embedded one.
    pub config: PathBuf,
}

impl TestEnv {
    pub fn new(test: &str) -> TestEnv {
        // `/tmp` is a symlink on macOS; the runtime wants the real path.
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(format!("pinfold-e2e-{}-{test}", std::process::id()));
        // The box's proxy socket lives under the state dir, and macOS caps
        // unix socket paths at 104 bytes. `$TMPDIR` is too long for that, so
        // the state dir gets its own short path under /tmp.
        let state = PathBuf::from("/tmp").join(format!("pf-e2e-{}-{test}", std::process::id()));
        let cache = root.join("cache");
        let config = root.join("config");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&config).unwrap();
        TestEnv {
            root,
            state,
            cache,
            config,
        }
    }

    pub fn command(&self, binary: &Path) -> Command {
        let mut command = Command::new(binary);
        command.env("XDG_STATE_HOME", &self.state);
        command.env("XDG_CACHE_HOME", &self.cache);
        command.env("XDG_CONFIG_HOME", &self.config);
        command
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.state);
    }
}

/// A host HTTP service, reachable from a box only through a route.
pub struct HttpFixture {
    port: u16,
    requests: Arc<AtomicUsize>,
    headers: Arc<Mutex<Vec<Headers>>>,
}

/// One request's header lines, as `(name, value)` in the order received.
pub type Headers = Vec<(String, String)>;

impl HttpFixture {
    /// Bind on loopback and serve until the process exits.
    pub fn start() -> HttpFixture {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the fixture");
        let port = listener.local_addr().expect("fixture address").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let headers = Arc::new(Mutex::new(Vec::new()));
        let counter = Arc::clone(&requests);
        let seen = Arc::clone(&headers);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let counter = Arc::clone(&counter);
                let seen = Arc::clone(&seen);
                thread::spawn(move || serve(stream, &counter, &seen));
            }
        });
        HttpFixture {
            port,
            requests,
            headers,
        }
    }

    /// The port the fixture listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// How many requests the fixture has answered.
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// The headers of each request the fixture has answered, in order.
    pub fn headers(&self) -> Vec<Headers> {
        self.headers.lock().expect("fixture headers").clone()
    }
}

/// Answer one request with the Host header it carried, and record its
/// headers.
fn serve(mut stream: TcpStream, requests: &AtomicUsize, seen: &Mutex<Vec<Headers>>) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut host = String::new();
    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("host:") {
            host = value.trim().to_string();
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.to_string(), value.trim().to_string()));
        }
    }
    seen.lock().expect("fixture headers").push(headers);
    requests.fetch_add(1, Ordering::SeqCst);
    let body = format!("fixture host={host}\n");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}
