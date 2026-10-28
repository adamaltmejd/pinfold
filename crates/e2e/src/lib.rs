//! Harness, host fixtures and helpers for the end-to-end suite in `tests/e2e/`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::thread;

/// The built `pinfold` binary, under Cargo's target directory even when the
/// test executable is copied elsewhere. Linux boxes use the CLI's own
/// executable as PID 1, so the static musl target is built and used.
pub fn pinfold() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let mut command = Command::new(env!("CARGO"));
        command.args([
            "build",
            "-p",
            "pinfold",
            "--locked",
            "--message-format",
            "json-render-diagnostics",
        ]);
        // Cargo gives tests their own package's CARGO_PKG_* and
        // CARGO_MANIFEST_*; ring's build script reruns when those change,
        // which rebuilt ring and pinfold at the start of every suite run.
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| {
                name.starts_with("CARGO_PKG_") || name.starts_with("CARGO_MANIFEST_")
            }) {
                command.env_remove(name);
            }
        }
        if cfg!(target_os = "linux") {
            let triple = format!("{}-unknown-linux-musl", std::env::consts::ARCH);
            command.args(["--target", &triple]);
        }
        let output = command
            .stderr(Stdio::inherit())
            .output()
            .expect("run cargo build -p pinfold");
        assert!(output.status.success(), "cargo build -p pinfold failed");
        // Cargo reports the bin's path, under the target triple's dir too.
        let binary = json_lines(&String::from_utf8_lossy(&output.stdout))
            .into_iter()
            .find(|message| {
                message["reason"] == "compiler-artifact"
                    && message["target"]["name"] == "pinfold"
                    && message["target"]["kind"] == serde_json::json!(["bin"])
            })
            .and_then(|message| message["executable"].as_str().map(PathBuf::from))
            .expect("cargo build reported the pinfold executable");
        assert!(binary.is_file(), "{} is missing", binary.display());
        binary
    })
}

/// Per-test XDG state and config, and a cache (the suite's shared one, or
/// the test's own; see `with_private_cache`), so a test never touches the
/// operator's. An empty config dir also means `default` resolves to the
/// embedded profile, not the operator's own copy of it.
pub struct TestEnv {
    /// The test's scratch root; projects and fixtures live under it.
    pub root: PathBuf,
    /// `XDG_STATE_HOME`; pinfold's state dir is `<state>/pinfold`.
    pub state: PathBuf,
    /// `XDG_CACHE_HOME`: the suite's shared cache, or the test's own.
    pub cache: PathBuf,
    pub config: PathBuf,
}

impl TestEnv {
    pub fn new(test: &str) -> TestEnv {
        // `/tmp` is a symlink on macOS; the runtime wants the real path.
        let temp = fs::canonicalize(std::env::temp_dir()).expect("canonicalize the temp dir");
        let root = temp.join(format!("pinfold-e2e-{}-{test}", std::process::id()));
        // The box's proxy socket lives under the state dir, and macOS caps
        // unix socket paths at 104 bytes. `$TMPDIR` is too long for that, so
        // the state dir gets its own short path under /tmp.
        let state = PathBuf::from("/tmp").join(format!("pf-e2e-{}-{test}", std::process::id()));
        // One cache for every test, so each pinned harness is fetched once.
        let cache = temp.join("pinfold-e2e-cache");
        let config = root.join("config");
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

    /// Like `new`, but this env's cache is its own under its root, for a
    /// scenario that damages the cache: the suite's shared cache is left
    /// alone.
    pub fn with_private_cache(test: &str) -> TestEnv {
        let mut env = TestEnv::new(test);
        env.cache = env.root.join("cache");
        fs::create_dir_all(&env.cache).unwrap();
        env
    }

    pub fn command(&self, binary: &Path) -> Command {
        let mut command = Command::new(binary);
        for (name, _) in std::env::vars_os() {
            if name
                .to_str()
                .is_some_and(|name| name.starts_with("PINFOLD_"))
            {
                command.env_remove(name);
            }
        }
        command.env("XDG_STATE_HOME", &self.state);
        command.env("XDG_CACHE_HOME", &self.cache);
        command.env("XDG_CONFIG_HOME", &self.config);
        command
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // Apple's disks are volumes outside the state root: remove this
        // root's. Linux disks live in the state root and go with it.
        // Best effort: a Drop during unwinding must not panic.
        if cfg!(target_os = "macos")
            && let Ok(listed) = self.command(pinfold()).args(["disk", "list"]).output()
        {
            for line in String::from_utf8_lossy(&listed.stdout).lines() {
                if let Ok(disk) = serde_json::from_str::<serde_json::Value>(line)
                    && let Some(name) = disk["name"].as_str()
                {
                    let _ = self.command(pinfold()).args(["disk", "rm", name]).output();
                }
            }
        }
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.state);
    }
}

/// Apple's native image and container CLI.
pub fn image_cli() -> &'static str {
    "container"
}

/// One image, observed in Apple's runtime or Linux's persisted catalog.
pub struct RuntimeImage {
    pub id: String,
    pub names: Vec<String>,
    pub labels: BTreeMap<String, String>,
}

fn suite_cache() -> PathBuf {
    fs::canonicalize(std::env::temp_dir())
        .expect("canonicalize the temp dir")
        .join("pinfold-e2e-cache")
}

/// Every image from the native runtime or the actual Linux store on the host.
pub fn runtime_images() -> Result<Vec<RuntimeImage>, String> {
    if cfg!(target_os = "linux") {
        return catalog_images(&suite_cache());
    }
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

fn catalog_images(cache: &Path) -> Result<Vec<RuntimeImage>, String> {
    let path = cache.join("pinfold/images/catalog.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let catalog: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    let references = catalog["references"]
        .as_object()
        .ok_or("the image catalog has no references")?;
    catalog["images"]
        .as_object()
        .ok_or("the image catalog has no images")?
        .iter()
        .map(|(id, image)| {
            Ok(RuntimeImage {
                id: id.clone(),
                names: references
                    .iter()
                    .filter(|(_, target)| target.as_str() == Some(id.as_str()))
                    .map(|(name, _)| name.clone())
                    .collect(),
                labels: serde_json::from_value(image["labels"].clone())
                    .map_err(|error| format!("image labels: {error}"))?,
            })
        })
        .collect()
}

/// One image list entry, whatever the runtime's schema.
fn normalize_image(image: &serde_json::Value) -> RuntimeImage {
    let labels = |value: &serde_json::Value| -> BTreeMap<String, String> {
        serde_json::from_value(value.clone()).unwrap_or_default()
    };
    // Apple `container`: build labels are OCI image config labels; a locally
    // built image also carries name annotations on its index descriptor.
    let mut all = labels(&image["configuration"]["descriptor"]["annotations"]);
    for variant in image["variants"].as_array().into_iter().flatten() {
        all.extend(labels(&variant["config"]["config"]["Labels"]));
    }
    RuntimeImage {
        id: image["id"].as_str().unwrap_or_default().to_string(),
        names: image["configuration"]["name"]
            .as_str()
            .map(|name| vec![name.to_string()])
            .unwrap_or_default(),
        labels: all,
    }
}

/// The id of the image the runtime lists under `reference`.
pub fn image_id(reference: &str) -> Option<String> {
    image_ids(|image| image.names.iter().any(|name| name == reference))
        .into_iter()
        .next()
}

/// The ids of every image tagged in `repository` (`<repository>:<tag>`),
/// from the runtime itself. A caller image name's builds are its tags.
pub fn tagged_images(repository: &str) -> Vec<String> {
    image_ids(|image| {
        image
            .names
            .iter()
            .any(|name| is_tagged_in(name, repository))
    })
}

/// Whether the reference `name` is a tag of `repository`.
fn is_tagged_in(name: &str, repository: &str) -> bool {
    name.contains(&format!("{repository}:"))
}

/// The sorted, distinct ids of the runtime's images that `keep` accepts.
pub fn image_ids(keep: impl Fn(&RuntimeImage) -> bool) -> Vec<String> {
    let mut ids: Vec<String> = runtime_images()
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .filter(|image| keep(image))
        .map(|image| image.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Best-effort public removal on drop. Linux can retire caller names only;
/// its managed families remain subject to normal retention.
pub struct ImageCleanup {
    pub repository: String,
}

impl Drop for ImageCleanup {
    fn drop(&mut self) {
        // Best effort: a Drop during unwinding must not panic.
        if cfg!(target_os = "linux") {
            if let Some(name) = self.repository.strip_prefix("localhost/pinfold/image-") {
                let env = TestEnv::new("image-cleanup");
                let _ = env.command(pinfold()).args(["image", "rm", name]).output();
            }
            // Managed families have no public removal command. Ordinary
            // retention owns those images; tests do not edit their catalog.
            return;
        }
        for image in runtime_images().unwrap_or_default() {
            for reference in image.names {
                if is_tagged_in(&reference, &self.repository) {
                    let _ = Command::new(image_cli())
                        .args(["image", "rm", &reference])
                        .output();
                }
            }
        }
    }
}

/// Build the candidate's default profile once for this suite.
pub fn default_image(env: &TestEnv) -> &'static str {
    static IMAGE: OnceLock<()> = OnceLock::new();
    static LINUX: OnceLock<Mutex<std::collections::BTreeSet<PathBuf>>> = OnceLock::new();
    const STABLE: &str = "localhost/pinfold/profile-default:latest";
    if cfg!(target_os = "linux") {
        let mut built = LINUX.get_or_init(Mutex::default).lock().unwrap();
        if !built.contains(&env.cache) {
            // A private image store needs a real build too. Populate only the
            // documented offline archive cache; the binary checks its pin.
            if env.cache != suite_cache() {
                let source = suite_cache().join("pinfold/artifacts/builder");
                for version in fs::read_dir(&source).expect("suite builder archive cache") {
                    let version = version.unwrap();
                    for artifact in fs::read_dir(version.path()).unwrap() {
                        let artifact = artifact.unwrap();
                        if artifact.file_type().unwrap().is_file()
                            && artifact
                                .path()
                                .extension()
                                .is_some_and(|suffix| suffix == "gz")
                        {
                            let target = env
                                .cache
                                .join("pinfold/artifacts/builder")
                                .join(version.file_name())
                                .join(artifact.file_name());
                            fs::create_dir_all(target.parent().unwrap()).unwrap();
                            fs::copy(artifact.path(), target).unwrap();
                        }
                    }
                }
            }
            build_profile(env, "default");
            built.insert(env.cache.clone());
        }
        return STABLE;
    }
    IMAGE.get_or_init(|| {
        build_profile(env, "default");
    });
    STABLE
}

/// Run `pinfold build --profile NAME` and assert it succeeded.
pub fn build_profile(env: &TestEnv, name: &str) {
    run_ok(env.command(pinfold()).args(["build", "--profile", name]));
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

/// `pinfold ARGS...` run in `project`.
pub fn pinfold_in(env: &TestEnv, project: &Path, args: &[&str]) -> Command {
    let mut command = env.command(pinfold());
    command.args(args).current_dir(project);
    command
}

/// Run `pinfold allow` in `project` and assert it succeeds.
pub fn allow(env: &TestEnv, project: &Path) {
    run_ok(&mut pinfold_in(env, project, &["allow"]));
}

/// Run host `git -C path ARGS...`, assert it succeeded, and return its
/// stdout. `path` must be an existing directory.
#[track_caller]
pub fn git(path: &Path, args: &[&str]) -> String {
    let output = run_ok(Command::new("git").arg("-C").arg(path).args(args));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Run `command`, assert it exited 0 with its stderr in the panic message,
/// and return its output.
#[track_caller]
pub fn run_ok(command: &mut Command) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("run {command:?}: {error}"));
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// One JSON value per line of `text`.
pub fn json_lines(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{line:?}: {error}")))
        .collect()
}

/// A new host directory under the test's root, removed with it.
pub fn test_dir(env: &TestEnv, name: &str) -> PathBuf {
    let path = env.root.join(name);
    fs::create_dir_all(&path).unwrap();
    path
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

pub fn box_exec(env: &TestEnv, name: &str, argv: &[&str]) -> ExecOutput {
    let output = env
        .command(pinfold())
        .args(["box", "exec", name, "--"])
        .args(argv)
        .output()
        .expect("run pinfold box exec");
    ExecOutput {
        code: exit_code(output.status),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Run `curl -sS --max-time SECONDS ARGS...` in the box.
pub fn curl(env: &TestEnv, name: &str, seconds: &str, args: &[&str]) -> ExecOutput {
    let mut argv = vec!["curl", "-sS", "--max-time", seconds];
    argv.extend_from_slice(args);
    box_exec(env, name, &argv)
}

pub fn box_list(env: &TestEnv, label: &str) -> Vec<serde_json::Value> {
    let output = run_ok(
        env.command(pinfold())
            .args(["box", "list", "--label", label]),
    );
    json_lines(&String::from_utf8_lossy(&output.stdout))
}

/// Run `box stat` on a live box and parse its one JSON object.
pub fn box_stat(env: &TestEnv, name: &str) -> serde_json::Value {
    let output = run_ok(env.command(pinfold()).args(["box", "stat", name]));
    serde_json::from_slice(&output.stdout).expect("stat output is one JSON object")
}

/// The text of the egress log a `ready` line names.
pub fn egress_log(ready: &serde_json::Value) -> String {
    let path = ready["egress_log"]
        .as_str()
        .unwrap_or_else(|| panic!("ready names no egress log: {ready}"));
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

/// A run's directory under pinfold's state dir, at the path the spec names.
pub fn run_dir(env: &TestEnv, run_id: &str) -> PathBuf {
    env.state.join("pinfold/runs").join(run_id)
}

/// Run `pinfold box inspect` or `box events` with `args` and return its exit
/// code and, on exit 0, its one JSON object (else null).
pub fn run_query(command: &mut Command) -> (i32, serde_json::Value) {
    let output = command.output().expect("run a pinfold run query");
    let code = exit_code(output.status);
    if code != 0 {
        return (code, serde_json::Value::Null);
    }
    let answer = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "a run query printed {:?}, not one JSON object: {error}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    (code, answer)
}

/// `box inspect RUN` in `env`, its exit code and object.
pub fn inspect(env: &TestEnv, run_id: &str) -> (i32, serde_json::Value) {
    run_query(env.command(pinfold()).args(["box", "inspect", run_id]))
}

pub fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// The project's id, as `pinfold config` reports it.
pub fn project_id(env: &TestEnv, project: &Path) -> String {
    let output = run_ok(env.command(pinfold()).arg("config").arg(project));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("pinfold config prints one JSON object");
    report["project"]["id"]
        .as_str()
        .expect("config names the project id")
        .to_string()
}

/// The project's state dir, at the path the spec names.
pub fn project_state_dir(env: &TestEnv, project: &Path) -> PathBuf {
    env.state
        .join("pinfold/projects")
        .join(project_id(env, project))
}

/// A host HTTP service, reachable from a box only through a route.
pub struct HttpFixture {
    port: u16,
    requests: Arc<Mutex<Vec<(String, Headers, String)>>>,
}

/// One request's header lines, as `(name, value)` in the order received.
pub type Headers = Vec<(String, String)>;

/// A fixed answer's content type and body.
pub type Answer = Option<(&'static str, &'static str)>;

impl HttpFixture {
    /// Bind on loopback and answer each request 200 with `answer`, or with
    /// the Host header it carried when `answer` is `None`, until the process
    /// exits.
    pub fn start(answer: Answer) -> HttpFixture {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the fixture");
        let port = listener.local_addr().expect("fixture address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
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

    /// The target, headers and body of each request the fixture has
    /// answered, in order.
    pub fn requests(&self) -> Vec<(String, Headers, String)> {
        self.requests.lock().expect("fixture requests").clone()
    }
}

/// Read one Content-Length-framed request, record its target, headers and
/// body, and answer it.
fn serve(stream: TcpStream, seen: &Mutex<Vec<(String, Headers, String)>>, answer: Answer) {
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let target = line.split(' ').nth(1).unwrap_or_default().to_string();
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
    seen.lock()
        .expect("fixture requests")
        .push((target, headers, body));
    let (content_type, body) = match answer {
        Some((content_type, body)) => (content_type, body.to_string()),
        None => ("text/plain", format!("fixture host={host}\n")),
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = (&stream).write_all(response.as_bytes());
}
