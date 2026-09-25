//! End-to-end tests that drive the `pinfold` binary from outside.
//!
//! The tests live in `tests/` and run on a macOS host with the Apple
//! `container` CLI, or a Linux host with rootless podman. `cargo test -p e2e`
//! builds the binary and runs them, so that one command is the whole host
//! gate.
//!
//! This crate also holds the host fixtures the tests reach through routes,
//! the built binary the test files drive, and the helpers they share.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::OnceLock;
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

/// Per-test XDG state and config, and the shared cache, so a test never
/// touches the operator's. An empty config dir also means `default`
/// resolves to the embedded profile, not the operator's own copy of it.
pub struct TestEnv {
    /// The test's scratch root; projects and fixtures live under it.
    pub root: PathBuf,
    /// `XDG_STATE_HOME`; pinfold's state dir is `<state>/pinfold`.
    pub state: PathBuf,
    /// `XDG_CACHE_HOME`, shared by every test; see `new`.
    pub cache: PathBuf,
    /// `XDG_CONFIG_HOME`, where profiles live. Empty, so `default` is the
    /// embedded one.
    pub config: PathBuf,
}

impl TestEnv {
    pub fn new(test: &str) -> TestEnv {
        // `/tmp` is a symlink on macOS; the runtime wants the real path.
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| std::env::temp_dir());
        let root = temp.join(format!("pinfold-e2e-{}-{test}", std::process::id()));
        // The box's proxy socket lives under the state dir, and macOS caps
        // unix socket paths at 104 bytes. `$TMPDIR` is too long for that, so
        // the state dir gets its own short path under /tmp.
        let state = PathBuf::from("/tmp").join(format!("pf-e2e-{}-{test}", std::process::id()));
        // One cache for every test and both test binaries, so
        // `artifacts::pi()` fetches the pinned release once.
        let cache = temp.join("pinfold-e2e-cache");
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

/// The image CLI this host uses: `podman` on Linux, Apple `container` on
/// macOS.
pub fn image_cli() -> &'static str {
    if cfg!(target_os = "linux") {
        "podman"
    } else {
        "container"
    }
}

/// Remove one image by reference from the runtime, best effort.
pub fn remove_runtime_image(reference: &str) {
    let mut command = Command::new(image_cli());
    if cfg!(target_os = "linux") {
        command.args(["image", "rm", reference]);
    } else {
        command.args(["image", "delete", reference]);
    }
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// One runtime image, normalized across podman and Apple `container`.
pub struct RuntimeImage {
    pub id: String,
    pub names: Vec<String>,
    pub labels: BTreeMap<String, String>,
}

/// Every image the runtime knows, from its own image list.
pub fn runtime_images() -> Result<Vec<RuntimeImage>, String> {
    let output = Command::new(image_cli())
        .args(["image", "list", "--format", "json"])
        .output()
        .map_err(|error| format!("run {} image list: {error}", image_cli()))?;
    if !output.status.success() {
        return Err(format!(
            "{} image list failed: {}",
            image_cli(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let images: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("image list is not JSON: {error}"))?;
    Ok(images.iter().map(normalize_image).collect())
}

/// One image list entry, whatever the runtime's schema.
fn normalize_image(image: &serde_json::Value) -> RuntimeImage {
    // podman: `Id`, `Names` and `Labels`, the last two null when empty.
    if let Some(id) = image["Id"].as_str() {
        return RuntimeImage {
            id: id.to_string(),
            names: image["Names"]
                .as_array()
                .map(|names| {
                    names
                        .iter()
                        .filter_map(|name| Some(name.as_str()?.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            labels: image["Labels"]
                .as_object()
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default(),
        };
    }
    // Apple `container`: build labels are OCI image config labels; a locally
    // built image also carries name annotations on its index descriptor.
    let mut labels: BTreeMap<String, String> = image["configuration"]["descriptor"]["annotations"]
        .as_object()
        .map(|labels| {
            labels
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    if let Some(variants) = image["variants"].as_array() {
        for variant in variants {
            if let Some(config) = variant["config"]["config"]["Labels"].as_object() {
                for (key, value) in config {
                    if let Some(value) = value.as_str() {
                        labels.insert(key.clone(), value.to_string());
                    }
                }
            }
        }
    }
    RuntimeImage {
        id: image["id"].as_str().unwrap_or_default().to_string(),
        names: image["configuration"]["name"]
            .as_str()
            .map(|name| vec![name.to_string()])
            .unwrap_or_default(),
        labels,
    }
}

/// Whether the runtime lists an image under `reference`, ignoring podman's
/// `localhost/` prefix.
pub fn image_named(reference: &str) -> bool {
    image_id(reference).is_some()
}

/// The id of the image the runtime lists under `reference`, ignoring
/// podman's `localhost/` prefix.
pub fn image_id(reference: &str) -> Option<String> {
    runtime_image(reference).map(|image| image.id)
}

/// The image the runtime lists under `reference`, ignoring podman's
/// `localhost/` prefix.
pub fn runtime_image(reference: &str) -> Option<RuntimeImage> {
    runtime_images()
        .expect("list the runtime's images")
        .into_iter()
        .find(|image| {
            image
                .names
                .iter()
                .any(|name| name.strip_prefix("localhost/").unwrap_or(name) == reference)
        })
}

/// The ids of every image carrying `label = value`, from the runtime
/// itself: its image list is the ground truth for what remains.
pub fn labeled_images(label: &str, value: &str) -> Vec<String> {
    let mut ids: Vec<String> = runtime_images()
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .filter(|image| image.labels.get(label).map(String::as_str) == Some(value))
        .map(|image| image.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Removes every image tagged `<repository>:<tag>` from the runtime store on
/// drop, so a failing run does not leave them for the next run to count or
/// for the operator's disk. Every build tags its image so, and the tag
/// remains even when a label sabotage drops the source label.
/// Sabotage: hardcode `container`; `podman image ls` keeps the leak.
pub struct ImageCleanup {
    pub repository: String,
}

impl Drop for ImageCleanup {
    fn drop(&mut self) {
        // Best effort: a Drop during unwinding must not panic.
        let prefix = format!("{}:", self.repository);
        for image in runtime_images().unwrap_or_default() {
            for reference in image.names {
                if reference.contains(&prefix) {
                    remove_runtime_image(&reference);
                }
            }
        }
    }
}

/// The digest the runtime's inspect reports for `reference`: podman's
/// manifest digest, which is not the image id; Apple's descriptor digest,
/// which is.
pub fn image_digest(reference: &str) -> String {
    let output = Command::new(image_cli())
        .args(["image", "inspect", reference])
        .output()
        .unwrap_or_else(|error| panic!("run {} image inspect: {error}", image_cli()));
    assert!(
        output.status.success(),
        "{} image inspect {reference} failed: {}",
        image_cli(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let images: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("image inspect is JSON");
    let image = images.first().expect("image inspect returned an image");
    image["Digest"]
        .as_str()
        .or_else(|| image["configuration"]["descriptor"]["digest"].as_str())
        .unwrap_or_else(|| panic!("{reference} inspect carries no digest: {image}"))
        .to_string()
}

/// The number of untagged images `podman images -a` lists, including the
/// intermediate layers a cached build leaves behind. Linux only.
pub fn untagged_images() -> usize {
    let output = Command::new("podman")
        .args(["images", "-a"])
        .output()
        .expect("run podman images -a");
    assert!(
        output.status.success(),
        "podman images -a failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.split_whitespace().next() == Some("<none>"))
        .count()
}

/// The stable ref of the built-in default profile's image, built once per
/// suite run. Every box and `pinfold pi` run starts from it.
pub fn default_image(binary: &Path, env: &TestEnv) -> &'static str {
    static IMAGE: OnceLock<()> = OnceLock::new();
    // The runtime store is shared by both test binaries; a stale image is
    // fine, the tests read its labels and run boxes from it.
    const STABLE: &str = "pinfold/profile-default:latest";
    IMAGE.get_or_init(|| {
        if !image_named(STABLE) {
            build_profile(binary, env, "default");
        }
    });
    STABLE
}

/// Run `pinfold build --profile NAME`, assert it succeeded, and return its
/// output: the stable ref on stdout, any maintenance line on stderr.
pub fn build_profile(binary: &Path, env: &TestEnv, name: &str) -> Output {
    let output = env
        .command(binary)
        .args(["build", "--profile", name])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold build --profile");
    assert!(
        output.status.success(),
        "pinfold build --profile {name} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Write a user profile's Containerfile under the test's config dir and
/// return its path.
pub fn profile_containerfile(env: &TestEnv, profile: &str, contents: &str) -> PathBuf {
    let path = env
        .config
        .join("pinfold")
        .join("profiles")
        .join(profile)
        .join("Containerfile");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, contents).unwrap();
    path
}

/// Create a git repository at `path`, so launch finds the project root.
pub fn git_init(path: &Path) {
    let status = Command::new("git")
        .args(["init", "-q"])
        .arg(path)
        .status()
        .expect("run git init");
    assert!(status.success(), "git init failed in {}", path.display());
}

/// Host `git status --porcelain` in `path`, asserting it ran.
pub fn git_status(path: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain"])
        .output()
        .expect("run host git status");
    assert!(
        output.status.success(),
        "host git status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// One JSON value per line of `text`.
pub fn json_lines(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{line:?}: {error}")))
        .collect()
}

/// A host directory under the test's root, removed with it.
pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new(env: &TestEnv, name: &str) -> TestDir {
        let path = env.root.join(name);
        fs::create_dir_all(&path).unwrap();
        TestDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub struct ExecOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Assert that `output` failed and that its stderr names `reason`.
#[track_caller]
pub fn assert_denied(output: &ExecOutput, reason: &str, what: &str) {
    assert_ne!(output.code, 0, "{what} succeeded");
    assert!(
        output.stderr.contains(reason),
        "{what} failed without {reason}: {}",
        output.stderr
    );
}

/// Assert that `output` exited 0, with its stderr in the panic message.
#[track_caller]
pub fn assert_ok(output: &ExecOutput, what: &str) {
    assert_eq!(output.code, 0, "{what} failed: {}", output.stderr);
}

pub fn box_exec(binary: &Path, env: &TestEnv, name: &str, argv: &[&str]) -> ExecOutput {
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

/// Run `curl -sS --max-time SECONDS ARGS...` in the box.
pub fn curl(binary: &Path, env: &TestEnv, name: &str, seconds: &str, args: &[&str]) -> ExecOutput {
    let mut argv = vec!["curl", "-sS", "--max-time", seconds];
    argv.extend_from_slice(args);
    box_exec(binary, env, name, &argv)
}

pub fn box_list(binary: &Path, env: &TestEnv, label: &str) -> Vec<serde_json::Value> {
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
    json_lines(&String::from_utf8_lossy(&output.stdout))
}

/// Run `box stat` on a live box and parse its one JSON object.
pub fn box_stat(binary: &Path, env: &TestEnv, name: &str) -> serde_json::Value {
    let output = env
        .command(binary)
        .args(["box", "stat", name])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold box stat");
    assert!(
        output.status.success(),
        "box stat failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stat output is one JSON object")
}

/// The box's egress log, at the fixed path under pinfold's state dir.
pub fn egress_log(env: &TestEnv, name: &str) -> PathBuf {
    env.state
        .join("pinfold")
        .join("egress")
        .join(format!("{name}.jsonl"))
}

/// The parsed decision lines of a box's egress log.
pub fn egress_log_lines(env: &TestEnv, name: &str) -> Vec<serde_json::Value> {
    json_lines(&fs::read_to_string(egress_log(env, name)).expect("read egress log"))
}

pub fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// The project state dir whose recorded root is `project`.
pub fn project_state_dir(env: &TestEnv, project: &Path) -> PathBuf {
    let root = fs::canonicalize(project).expect("canonicalize project");
    let projects = env.state.join("pinfold").join("projects");
    for entry in fs::read_dir(&projects).expect("read projects dir") {
        let dir = entry.expect("project entry").path();
        let state = fs::read_to_string(dir.join("state.json")).expect("read state.json");
        let state: serde_json::Value = serde_json::from_str(&state).expect("state.json is JSON");
        if state["root"].as_str() == root.to_str() {
            return dir;
        }
    }
    panic!("no project state for {}", project.display());
}

/// The project's home, in its state dir.
pub fn project_home(env: &TestEnv, project: &Path) -> PathBuf {
    project_state_dir(env, project).join("home")
}

/// The project's id, which names its state dir.
pub fn project_id(env: &TestEnv, project: &Path) -> String {
    project_state_dir(env, project)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the project id is UTF-8")
        .to_string()
}

/// A host HTTP service, reachable from a box only through a route.
pub struct HttpFixture {
    port: u16,
    requests: Arc<Mutex<Vec<(Headers, String)>>>,
}

/// One request's header lines, as `(name, value)` in the order received.
pub type Headers = Vec<(String, String)>;

/// A fixed answer's content type and body.
type Answer = Option<(&'static str, &'static str)>;

impl HttpFixture {
    /// Bind on loopback and answer every request with the Host header it
    /// carried, until the process exits.
    pub fn start() -> HttpFixture {
        HttpFixture::listen(None)
    }

    /// Bind on loopback and answer every request with `body`, until the
    /// process exits.
    pub fn answering(content_type: &'static str, body: &'static str) -> HttpFixture {
        HttpFixture::listen(Some((content_type, body)))
    }

    fn listen(answer: Answer) -> HttpFixture {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the fixture");
        let port = listener.local_addr().expect("fixture address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let seen = Arc::clone(&seen);
                thread::spawn(move || serve(stream, &seen, answer));
            }
        });
        HttpFixture { port, requests }
    }

    /// The `host:port` a route names to reach the fixture.
    pub fn route(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// How many requests the fixture has answered.
    pub fn requests(&self) -> usize {
        self.headers().len()
    }

    /// The headers of each request the fixture has answered, in order.
    pub fn headers(&self) -> Vec<Headers> {
        let requests = self.requests.lock().expect("fixture requests");
        requests
            .iter()
            .map(|(headers, _)| headers.clone())
            .collect()
    }

    /// The body of each request the fixture has answered, in order.
    pub fn bodies(&self) -> Vec<String> {
        let requests = self.requests.lock().expect("fixture requests");
        requests.iter().map(|(_, body)| body.clone()).collect()
    }
}

/// Read one Content-Length-framed request, record its headers and body, and
/// answer it.
fn serve(mut stream: TcpStream, seen: &Mutex<Vec<(Headers, String)>>, answer: Answer) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut host = String::new();
    let mut length = 0u64;
    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("host") {
                host = value.to_ascii_lowercase();
            }
            if name.eq_ignore_ascii_case("content-length") {
                length = value.parse().unwrap_or(0);
            }
            headers.push((name.to_string(), value.to_string()));
        }
    }
    let mut body = Vec::new();
    let _ = reader.by_ref().take(length).read_to_end(&mut body);
    let body = String::from_utf8_lossy(&body).into_owned();
    seen.lock().expect("fixture requests").push((headers, body));
    let (content_type, body) = match answer {
        Some((content_type, body)) => (content_type, body.to_string()),
        None => ("text/plain", format!("fixture host={host}\n")),
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}
