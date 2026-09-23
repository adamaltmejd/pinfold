//! End-to-end tests for guarantees 6, 12 and 13 in docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI. The harness
//! isolates the XDG dirs, builds the default profile image once, and drives
//! `pinfold pi` as a user would. No model is needed: `pi --mode rpc` answers
//! `get_state` while the box runs, and `pi --version` exits on its own.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::OnceLock;

#[test]
fn the_environment_is_exactly_the_spec() {
    // Sabotage: pass the PINFOLD_ENV_* value on the command line (for
    // example `container exec --env SECRET=shhh`) instead of through the
    // child's environment; `shhh` then appears in host `ps` while the box
    // runs and the ps assertion fails. Sabotage: let the box inherit the
    // host environment; the unprefixed variable is then present and its
    // assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("pi-env");
    default_image(binary, &env);
    let project = TestDir::new(&env, "project");
    git_init(project.path());

    // The run creates the project state; the id names its directory.
    let run = PiRpc::start(binary, &env, project.path());
    let id = project_id(&env, project.path());
    let name = run.ready_box(binary, &env, &id);

    // PINFOLD_ENV_SECRET arrives as SECRET; the prefix stays on the host.
    let secret = box_exec(binary, &env, &name, &["sh", "-c", "printf %s \"$SECRET\""]);
    assert_eq!(secret.code, 0, "reading SECRET failed: {}", secret.stderr);
    assert_eq!(secret.stdout, "shhh", "SECRET did not arrive");
    let prefixed = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$PINFOLD_ENV_SECRET\""],
    );
    assert_eq!(
        prefixed.stdout, "",
        "the PINFOLD_ENV_ prefix reached the box"
    );

    // An unprefixed host variable is absent.
    let host_only = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$E2E_HOST_ONLY\""],
    );
    assert_eq!(host_only.stdout, "", "an unprefixed host variable leaked");

    // The fixed environment is the spec's.
    for (variable, want) in [
        ("PI_TELEMETRY", "0"),
        ("PI_SKIP_VERSION_CHECK", "1"),
        ("HERDR_AGENT", "pi"),
    ] {
        let got = box_exec(
            binary,
            &env,
            &name,
            &["sh", "-c", &format!("printf %s \"${variable}\"")],
        );
        assert_eq!(got.stdout, want, "{variable} in the box");
    }

    // The secret never shows in host `ps` while the box runs.
    let ps = Command::new("ps")
        .args(["-ww", "-A", "-o", "command"])
        .output()
        .expect("run ps");
    let ps = String::from_utf8_lossy(&ps.stdout);
    assert!(
        !ps.contains("shhh"),
        "the secret is in host ps output:\n{ps}"
    );

    // Normal pi exit removes the box.
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
    let listed = box_list(binary, &env, &format!("dev.pinfold.project={id}"));
    assert!(listed.is_empty(), "the box survived pi exit: {listed:?}");
}

#[test]
fn project_state_persists_and_stays_separate() {
    // Sabotage: derive the project id from the directory name alone (drop
    // the root hash in state.rs::id_for); the two checkouts named `checkout`
    // then share a home, and the assert_ne "two checkouts share a home"
    // fails. Sabotage: seed $HOME on every run instead of only when missing;
    // the edited marker is overwritten and the survives-a-run assertion
    // fails.
    let binary = pinfold();
    let env = TestEnv::new("pi-state");
    default_image(binary, &env);
    let a = TestDir::new(&env, "a/checkout");
    let b = TestDir::new(&env, "b/checkout");
    git_init(a.path());
    git_init(b.path());

    // The first run seeds the default profile's settings.json.
    pi_version(binary, &env, a.path());
    let settings_a = project_home(&env, a.path()).join(".pi/agent/settings.json");
    let seeded = fs::read_to_string(&settings_a).expect("read seeded settings.json");
    assert!(
        seeded.contains("defaultProjectTrust"),
        "seed content: {seeded}"
    );

    // An edit survives the next run: a seed is copied only when missing.
    let marker = "{\"marker\":\"project-a\"}\n";
    fs::write(&settings_a, marker).expect("edit settings.json");
    pi_version(binary, &env, a.path());
    assert_eq!(
        fs::read_to_string(&settings_a).expect("read edited settings.json"),
        marker,
        "a run reseeded an edited file"
    );

    // A second checkout with the same directory name gets its own home and
    // does not see the first project's marker. Take the first home before
    // the second run: the sabotage overwrites the shared state.json's root.
    let home_a = project_home(&env, a.path());
    pi_version(binary, &env, b.path());
    let home_b = project_home(&env, b.path());
    assert_ne!(home_a, home_b, "two checkouts share a home");
    let seeded_b =
        fs::read_to_string(home_b.join(".pi/agent/settings.json")).expect("read second seed");
    assert!(
        seeded_b.contains("defaultProjectTrust"),
        "second seed content: {seeded_b}"
    );
    assert!(
        !seeded_b.contains("project-a"),
        "the second project sees the first's marker"
    );

    // A deleted seed comes back on the next run.
    fs::remove_file(&settings_a).expect("delete settings.json");
    pi_version(binary, &env, a.path());
    let reseeded = fs::read_to_string(&settings_a).expect("read reseeded settings.json");
    assert!(
        reseeded.contains("defaultProjectTrust"),
        "reseed content: {reseeded}"
    );
    assert!(!reseeded.contains("project-a"), "the marker came back");
}

#[test]
fn a_changed_project_file_stops_the_run() {
    // Sabotage: drop the trust::check call from pi::launch; the created and
    // the changed `.pinfold.toml` then run and both refusal assertions fail.
    // Sabotage: hash an absent `.pinfold.toml` as the empty file instead of
    // recording its absence; the first run, with no `.pinfold.toml` and no
    // record, refuses as "not trusted" and the bare-run control fails.
    let binary = pinfold();
    let env = TestEnv::new("pi-trust");
    default_image(binary, &env);
    let project = TestDir::new(&env, "project");
    git_init(project.path());
    let config = project.path().join(".pinfold.toml");

    // A project with no `.pinfold.toml` has nothing to trust: it runs
    // without `pinfold allow`.
    pi_version(binary, &env, project.path());

    // `pinfold allow` records the absence, so the file the agent creates in
    // the live box is a change.
    allow(binary, &env, project.path());
    let run = PiRpc::start(binary, &env, project.path());
    let id = project_id(&env, project.path());
    let name = run.ready_box(binary, &env, &id);
    let created = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf 'allow = [\"example.com\"]\\n' > {}",
                config.display()
            ),
        ],
    );
    assert_eq!(
        created.code, 0,
        "writing .pinfold.toml in the box failed: {}",
        created.stderr
    );
    assert!(run.finish().success(), "the bare run did not exit cleanly");

    // The file that appeared stops the run until `pinfold allow` records it.
    let refused = pi_version_output(binary, &env, project.path());
    assert!(!refused.status.success(), "the new .pinfold.toml ran");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("pinfold allow"),
        "the refusal did not name `pinfold allow`: {stderr}"
    );
    allow(binary, &env, project.path());
    pi_version(binary, &env, project.path());

    // The agent adds a domain to the now-trusted file in a live box.
    let run = PiRpc::start(binary, &env, project.path());
    let name = run.ready_box(binary, &env, &id);
    let changed = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf 'allow = [\"example.com\", \"api.github.com\"]\\n' > {}",
                config.display()
            ),
        ],
    );
    assert_eq!(
        changed.code, 0,
        "changing .pinfold.toml in the box failed: {}",
        changed.stderr
    );
    assert!(
        run.finish().success(),
        "the trusted run did not exit cleanly"
    );

    // The change stops the run again, and `pinfold allow` clears it.
    let refused = pi_version_output(binary, &env, project.path());
    assert!(!refused.status.success(), "the changed .pinfold.toml ran");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("pinfold allow"),
        "the refusal did not name `pinfold allow`: {stderr}"
    );
    allow(binary, &env, project.path());
    pi_version(binary, &env, project.path());
}

/// A `pinfold pi --mode rpc` process with a live box.
struct PiRpc {
    child: Child,
    stdin: Option<ChildStdin>,
    _reader: BufReader<ChildStdout>,
}

impl PiRpc {
    fn start(binary: &Path, env: &TestEnv, project: &Path) -> PiRpc {
        let mut child = env
            .command(binary)
            .args(["pi", "--mode", "rpc"])
            .current_dir(project)
            .env("PINFOLD_ENV_SECRET", "shhh")
            .env("E2E_HOST_ONLY", "host-value")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn pinfold pi");
        let mut stdin = child.stdin.take().expect("pi stdin");
        stdin
            .write_all(b"{\"type\":\"get_state\",\"id\":\"1\"}\n")
            .expect("write get_state");
        stdin.flush().expect("flush get_state");
        let stdout = child.stdout.take().expect("pi stdout");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let read = reader.read_line(&mut line).expect("read pi reply");
            assert!(read > 0, "pi exited before answering get_state");
            if let Ok(reply) = serde_json::from_str::<serde_json::Value>(line.trim())
                && reply["id"] == "1"
            {
                break;
            }
        }
        PiRpc {
            child,
            stdin: Some(stdin),
            _reader: reader,
        }
    }

    /// The one box this run owns, once pi has answered.
    fn ready_box(&self, binary: &Path, env: &TestEnv, id: &str) -> String {
        let listed = box_list(binary, env, &format!("dev.pinfold.project={id}"));
        assert_eq!(listed.len(), 1, "expected one pi box for {id}: {listed:?}");
        listed[0]["name"]
            .as_str()
            .expect("box name is a string")
            .to_string()
    }

    /// Close pi's stdin, wait for it, and return its exit status.
    fn finish(mut self) -> ExitStatus {
        drop(self.stdin.take());
        self.child.wait().expect("wait for pinfold pi")
    }
}

impl Drop for PiRpc {
    fn drop(&mut self) {
        // A panicking test must not leave a live `pinfold pi` behind; the
        // next pinfold command prunes the box once its owner is gone.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `pinfold pi --version` in `project` and assert it exits cleanly.
fn pi_version(binary: &Path, env: &TestEnv, project: &Path) {
    let output = pi_version_output(binary, env, project);
    assert!(
        output.status.success(),
        "pinfold pi --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run `pinfold pi --version` in `project` and return its output.
fn pi_version_output(binary: &Path, env: &TestEnv, project: &Path) -> Output {
    env.command(binary)
        .args(["pi", "--version"])
        .current_dir(project)
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold pi --version")
}

/// Run `pinfold allow` in `project` and assert it succeeds.
fn allow(binary: &Path, env: &TestEnv, project: &Path) {
    let output = env
        .command(binary)
        .arg("allow")
        .current_dir(project)
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold allow");
    assert!(
        output.status.success(),
        "pinfold allow failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The project id of the state dir whose recorded root is `project`.
fn project_id(env: &TestEnv, project: &Path) -> String {
    project_home(env, project)
        .parent()
        .expect("the home has a project dir")
        .file_name()
        .expect("the project dir has a name")
        .to_str()
        .expect("the project id is UTF-8")
        .to_string()
}

/// The project's home, found from the state dir's `state.json`.
fn project_home(env: &TestEnv, project: &Path) -> PathBuf {
    let root = fs::canonicalize(project).expect("canonicalize project");
    let projects = env.state.join("pinfold").join("projects");
    for entry in fs::read_dir(&projects).expect("read projects dir") {
        let dir = entry.expect("project entry").path();
        let state = fs::read_to_string(dir.join("state.json")).expect("read state.json");
        let state: serde_json::Value = serde_json::from_str(&state).expect("state.json is JSON");
        if state["root"].as_str() == root.to_str() {
            return dir.join("home");
        }
    }
    panic!("no project state for {}", project.display());
}

/// Create a git repository at `path`, so launch finds the project root.
fn git_init(path: &Path) {
    let status = Command::new("git")
        .args(["init", "-q"])
        .arg(path)
        .status()
        .expect("run git init");
    assert!(status.success(), "git init failed in {}", path.display());
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

/// Build the default profile image once per suite run; `pinfold pi` refuses
/// without it.
fn default_image(binary: &Path, env: &TestEnv) {
    static IMAGE: OnceLock<()> = OnceLock::new();
    IMAGE.get_or_init(|| {
        let output = env
            .command(binary)
            .args(["build", "--profile", "default"])
            .output()
            .expect("run pinfold build --profile default");
        assert!(
            output.status.success(),
            "pinfold build --profile default failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    });
}

/// Per-test XDG state, cache and config, so a test never touches the
/// operator's. An empty config dir also means `default` resolves to the
/// embedded profile, not the operator's own copy of it.
struct TestEnv {
    root: PathBuf,
    state: PathBuf,
    cache: PathBuf,
    config: PathBuf,
}

impl TestEnv {
    fn new(test: &str) -> TestEnv {
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

    fn command(&self, binary: &Path) -> Command {
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

/// A host directory the tests run projects in.
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

fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}
