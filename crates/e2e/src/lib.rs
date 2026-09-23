//! End-to-end tests that drive the `pinfold` binary from outside.
//!
//! The tests live in `tests/` and run on a macOS host with the Apple
//! `container` CLI, or a Linux host with rootless podman. `cargo test -p e2e`
//! builds the binary and runs them, so that one command is the whole host
//! gate.
//!
//! This crate also holds the host fixtures the tests reach through routes,
//! and the built binary both test files drive.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// A host HTTP service, reachable from a box only through a route.
pub struct HttpFixture {
    port: u16,
    requests: Arc<AtomicUsize>,
}

impl HttpFixture {
    /// Bind on loopback and serve until the process exits.
    pub fn start() -> HttpFixture {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the fixture");
        let port = listener.local_addr().expect("fixture address").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let counter = Arc::clone(&counter);
                thread::spawn(move || serve(stream, &counter));
            }
        });
        HttpFixture { port, requests }
    }

    /// The port the fixture listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// How many requests the fixture has answered.
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// Answer one request with the Host header it carried.
fn serve(mut stream: TcpStream, requests: &AtomicUsize) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut host = String::new();
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
    }
    requests.fetch_add(1, Ordering::SeqCst);
    let body = format!("fixture host={host}\n");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}
