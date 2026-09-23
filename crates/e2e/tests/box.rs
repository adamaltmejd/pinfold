//! End-to-end tests for guarantees 9 and 10 in docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI and the
//! `debian:trixie-slim` image. The harness builds the `pinfold` binary and
//! drives it as a user would: the CLI, environment variables and the box
//! spec are its only seams.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::OnceLock;

const IMAGE: &str =
    "debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a";

#[test]
fn box_lifecycle_works_for_a_caller() {
    // Sabotage: make `box down` a no-op; the post-down list assertion fails.
    // Sabotage: make `box exec` drop the runtime's exit status and return 0;
    // the exit-3 assertion fails, and the zero-exit command below is the
    // positive control that the same path can succeed.
    let binary = pinfold();
    let env = TestEnv::new("lifecycle");
    let name = format!("pinfold-e2e-{}-lifecycle", std::process::id());
    let label = "dev.yard.lane=e2e-lifecycle";
    let spec = serde_json::json!({
        "name": name,
        "image": IMAGE,
        "labels": { "dev.yard.lane": "e2e-lifecycle" },
    });
    let mut up = box_up(binary, &env, &spec, &name);

    // `exec` streams both streams and returns the process exit code.
    let failed = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "echo out; echo err >&2; exit 3"],
    );
    assert_eq!(failed.stdout, "out\n");
    assert_eq!(failed.stderr, "err\n");
    assert_eq!(failed.code, 3);

    // Positive control: the same command path passes a zero exit through.
    let ok = box_exec(binary, &env, &name, &["sh", "-c", "exit 0"]);
    assert_eq!(ok.code, 0);

    // `list` finds the box by the caller's label.
    let listed = box_list(binary, &env, label);
    assert!(
        listed
            .iter()
            .any(|box_| box_["name"].as_str() == Some(name.as_str())),
        "list did not find {name}: {listed:?}"
    );

    // `down` removes the box and the owner exits.
    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    let listed = box_list(binary, &env, label);
    assert!(
        !listed
            .iter()
            .any(|box_| box_["name"].as_str() == Some(name.as_str())),
        "box survived down: {listed:?}"
    );
    assert!(up.wait().success(), "box up did not exit cleanly");
}

#[test]
fn box_shares_files_with_the_host() {
    // Sabotage: drop `readonly` from the Apple adapter's bind mounts; the
    // write to /readonly/new then succeeds and its assertion fails. The
    // write to /workspace is the positive control that the same operation
    // works on a writable mount.
    // Sabotage: make `box exec` run as root instead of the run's user; the
    // box-created file is then not the host user's and the owner assertion
    // fails.
    let binary = pinfold();
    let env = TestEnv::new("shared-files");
    let dir = TestDir::new(&env, "mount");
    let name = format!("pinfold-e2e-{}-files", std::process::id());

    // Host files the box must be able to change.
    fs::write(dir.path().join("host-600"), b"before\n").unwrap();
    fs::set_permissions(
        dir.path().join("host-600"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fs::create_dir(dir.path().join("host-700")).unwrap();
    fs::set_permissions(
        dir.path().join("host-700"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let readonly = dir.path().join("readonly");
    fs::create_dir(&readonly).unwrap();
    fs::write(readonly.join("keep"), b"keep\n").unwrap();

    let spec = serde_json::json!({
        "name": name,
        "image": IMAGE,
        "mounts": [
            { "host": dir.path(), "guest": "/workspace" },
            { "host": readonly, "guest": "/readonly", "readonly": true },
        ],
    });
    let up = box_up(binary, &env, &spec, &name);

    // Box-created files: a 644 file, a 755 directory and an executable.
    let created = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            "printf created > /workspace/box-created && mkdir /workspace/box-dir \
             && printf '#!/bin/sh\\n' > /workspace/box-script \
             && chmod 755 /workspace/box-script",
        ],
    );
    assert_eq!(created.code, 0, "creating files failed: {}", created.stderr);

    // Host 0600 and 0700 files are writable in the box.
    let wrote = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            "printf changed >> /workspace/host-600 \
             && printf inside > /workspace/host-700/from-box",
        ],
    );
    assert_eq!(wrote.code, 0, "writing host files failed: {}", wrote.stderr);

    // The read-only mount rejects a write, and says why.
    let denied = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf nope > /readonly/new"],
    );
    assert_ne!(denied.code, 0, "the read-only mount accepted a write");
    assert!(
        denied.stderr.contains("Read-only file system"),
        "the write failed for another reason: {}",
        denied.stderr
    );

    // Host view of the same files.
    let host_600 = fs::metadata(dir.path().join("host-600")).unwrap();
    let box_created = fs::metadata(dir.path().join("box-created")).unwrap();
    assert_eq!(box_created.mode() & 0o777, 0o644, "box file mode");
    assert_eq!(box_created.uid(), host_600.uid(), "box file owner");
    assert_eq!(
        fs::read(dir.path().join("box-created")).unwrap(),
        b"created"
    );
    assert_eq!(
        fs::metadata(dir.path().join("box-dir")).unwrap().mode() & 0o777,
        0o755,
        "box directory mode"
    );
    assert_eq!(
        fs::metadata(dir.path().join("box-script")).unwrap().mode() & 0o777,
        0o755,
        "box script mode"
    );
    assert_eq!(
        fs::read(dir.path().join("host-600")).unwrap(),
        b"before\nchanged"
    );
    assert_eq!(
        fs::read(dir.path().join("host-700/from-box")).unwrap(),
        b"inside"
    );
    assert_eq!(fs::read(readonly.join("keep")).unwrap(), b"keep\n");

    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);
}

/// The built `pinfold` binary. The test executable lives in
/// `<target>/<profile>/deps`, so the binary is its sibling.
fn pinfold() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "pinfold", "--locked"])
            .status()
            .expect("run cargo build -p pinfold");
        assert!(status.success(), "cargo build -p pinfold failed");
        let exe = std::env::current_exe().expect("test executable path");
        let target = exe
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("target dir");
        let binary = target.join("debug").join("pinfold");
        assert!(binary.is_file(), "{} is missing", binary.display());
        binary
    })
}

/// Per-test XDG state and cache, so a test never touches the operator's.
struct TestEnv {
    root: PathBuf,
    state: PathBuf,
    cache: PathBuf,
}

impl TestEnv {
    fn new(test: &str) -> TestEnv {
        // `/tmp` is a symlink on macOS; the runtime wants the real path.
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap_or(std::env::temp_dir())
            .join(format!("pinfold-e2e-{}-{test}", std::process::id()));
        let state = root.join("state");
        let cache = root.join("cache");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&cache).unwrap();
        TestEnv { root, state, cache }
    }

    fn command(&self, binary: &Path) -> Command {
        let mut command = Command::new(binary);
        command.env("XDG_STATE_HOME", &self.state);
        command.env("XDG_CACHE_HOME", &self.cache);
        command
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A host directory the test mounts into the box.
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(env: &TestEnv, name: &str) -> TestDir {
        let path = env.root.join(name);
        fs::create_dir_all(&path).unwrap();
        TestDir { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A `box up` process and its ready box.
struct Up {
    binary: PathBuf,
    state: PathBuf,
    cache: PathBuf,
    name: String,
    child: Child,
    _stdin: ChildStdin,
}

impl Up {
    fn wait(&mut self) -> ExitStatus {
        self.child.wait().expect("wait for box up")
    }
}

impl Drop for Up {
    fn drop(&mut self) {
        // Best effort, so a panicking test does not leak a box.
        let _ = Command::new(&self.binary)
            .args(["box", "down", &self.name])
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn box_up(binary: &Path, env: &TestEnv, spec: &serde_json::Value, name: &str) -> Up {
    let mut child = env
        .command(binary)
        .args(["box", "up"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn pinfold box up");
    let mut stdin = child.stdin.take().expect("box up stdin");
    let spec = serde_json::to_string(spec).expect("serialize spec");
    stdin.write_all(spec.as_bytes()).expect("write spec");
    stdin.flush().expect("flush spec");

    let stdout = child.stdout.take().expect("box up stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read ready line");
    let ready: serde_json::Value = serde_json::from_str(line.trim()).expect("ready JSON");
    assert_eq!(ready["event"], "ready", "first line was {line:?}");
    assert_eq!(ready["box"], name, "ready named another box: {line:?}");

    // Keep draining stdout so a chatty box cannot block on the pipe.
    std::thread::spawn(move || {
        let mut sink = String::new();
        while reader.read_line(&mut sink).unwrap_or(0) > 0 {
            sink.clear();
        }
    });

    Up {
        binary: binary.to_path_buf(),
        state: env.state.clone(),
        cache: env.cache.clone(),
        name: name.to_string(),
        child,
        _stdin: stdin,
    }
}

struct ExecOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn box_exec(binary: &Path, env: &TestEnv, name: &str, argv: &[&str]) -> ExecOutput {
    let output = env
        .command(binary)
        .args(["box", "exec", name, "--"])
        .args(argv)
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold box exec");
    ExecOutput {
        code: exit_code(output.status),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn box_list(binary: &Path, env: &TestEnv, label: &str) -> Vec<serde_json::Value> {
    let output = env
        .command(binary)
        .args(["box", "list", "--label", label])
        .output()
        .expect("run pinfold box list");
    assert!(
        output.status.success(),
        "box list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("list output is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("list line is JSON"))
        .collect()
}

fn box_down(binary: &Path, env: &TestEnv, name: &str) -> ExitStatus {
    env.command(binary)
        .args(["box", "down", name])
        .stdin(Stdio::null())
        .status()
        .expect("run pinfold box down")
}

fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}
