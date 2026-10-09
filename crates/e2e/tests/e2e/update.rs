//! Host self-update through a real HTTPS release fixture.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use e2e::{TestEnv, default_image, pinfold, run_ok};

use super::box_::{box_name, box_up};

#[test]
fn host_updates_are_verified_and_atomic() {
    let _runtime = crate::shared_runtime();
    // Guarantee 28. Sabotage: skip SHA256SUMS verification or its digest
    // comparison; the wrong digest under the requested asset name installs.
    // Compare a digest without selecting the asset name; the correct digest
    // under another name authorizes the release. Rename before
    // verification; the old inode changes on refusal. Write into the
    // executable instead of renaming; the final inode stays unchanged.
    // Remove the live owner guard; updating while a real box owner is alive
    // succeeds. Replace argv[0] rather than current_exe(); the invoked
    // command symlink is replaced and the installed binary's inode stays
    // unchanged. Remove the exclusive executable lock; the owner under a
    // different XDG root no longer prevents updating its executable. Drop
    // the check request; the fixture records none and reports no version.
    // Expected release bytes and digest, the fixture version and the
    // recorded check request come from the host fixture; inode identity
    // comes from the host filesystem.
    let env = TestEnv::new("update");
    let installed = env.root.join("pinfold");
    fs::copy(pinfold(), &installed).unwrap();
    let update_alias = env.root.join("pinfold-link");
    symlink("pinfold", &update_alias).unwrap();
    let mut fixture = ReleaseProxy::new(&env);
    let original = fs::read(&installed).unwrap();
    let original_inode = fs::metadata(&installed).unwrap().ino();
    let replacement = changed_profiles_binary(&env);
    let replacement_bytes = fs::read(&replacement).unwrap();
    assert_ne!(
        replacement_bytes, original,
        "release fixture must be a distinct real build"
    );
    fs::copy(replacement, fixture.root.join("release")).unwrap();

    let check = run_ok(
        fixture
            .command(&env, &installed)
            .args(["update", "--check"]),
    );
    assert!(
        String::from_utf8_lossy(&check.stdout).contains("999.0.0"),
        "check did not report the fixture version: {check:?}"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("checks"))
            .unwrap_or_default()
            .lines()
            .count(),
        1,
        "check-only did not reach the fixture"
    );
    assert_eq!(fs::metadata(&installed).unwrap().ino(), original_inode);
    assert_eq!(fs::read(&installed).unwrap(), original);

    let bad_checksum = fixture.root.join("bad-checksum");
    fs::write(&bad_checksum, b"").unwrap();
    let failure = fixture
        .command(&env, &installed)
        .arg("update")
        .output()
        .unwrap();
    assert!(
        !failure.status.success(),
        "wrong-name checksum authorized the release: {failure:?}"
    );
    assert!(
        String::from_utf8_lossy(&failure.stderr).contains("checksum-mismatch"),
        "wrong refusal: {failure:?}"
    );
    assert_eq!(fs::read(&installed).unwrap(), original);
    assert_eq!(fs::metadata(&installed).unwrap().ino(), original_inode);
    run_ok(env.command(&installed).arg("--version"));
    fs::remove_file(bad_checksum).unwrap();

    let name = box_name("update");
    let spec = serde_json::json!({ "name": name, "image": default_image(&env) });
    // box_up uses the suite's built binary, so its real live owner also
    // exercises the state-owner guard for boxes created before an update.
    let up = box_up(&env, &spec, &name);
    let failure = fixture
        .command(&env, &installed)
        .arg("update")
        .output()
        .unwrap();
    assert!(
        !failure.status.success(),
        "live box update succeeded: {failure:?}"
    );
    let stderr = String::from_utf8_lossy(&failure.stderr);
    assert!(
        stderr.contains("running-boxes"),
        "wrong refusal: {failure:?}"
    );
    assert_eq!(fs::metadata(&installed).unwrap().ino(), original_inode);
    drop(up);

    let other = TestEnv::new("update-owner");
    // Sabotage: hold the exclusive executable lock only before downloading.
    // A real owner starts after the fixture observes the asset request;
    // replacement must recheck that owner before its atomic rename.
    fs::write(fixture.root.join("hold-download"), b"").unwrap();
    let mut downloading = fixture.command(&env, &installed);
    let update = downloading
        .arg("update")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    fixture.wait_for_download();
    let owner = UpdateOwner::start(&other, &installed, &spec);
    fixture.release_download();
    let refusal = update.wait_with_output().unwrap();
    assert!(
        !refusal.status.success(),
        "owner starting during download was replaced"
    );
    assert!(
        String::from_utf8_lossy(&refusal.stderr).contains("update-busy"),
        "wrong race refusal: {refusal:?}"
    );
    assert_eq!(fs::read(&installed).unwrap(), original);
    assert_eq!(fs::metadata(&installed).unwrap().ino(), original_inode);
    drop(owner);
    fs::remove_file(fixture.root.join("hold-download")).unwrap();

    run_ok(fixture.command(&env, &update_alias).arg("update"));
    assert_eq!(fs::read(&installed).unwrap(), replacement_bytes);
    assert_ne!(fs::metadata(&installed).unwrap().ino(), original_inode);
    assert_eq!(fs::read_link(&update_alias).unwrap(), Path::new("pinfold"));
    run_ok(env.command(&installed).arg("--version"));
}

#[test]
fn interactive_update_checks_are_bounded() {
    let _runtime = crate::shared_runtime();
    // Guarantee 29. Sabotage: remove terminal or opt-out checks; suppressed
    // launches reach the fixture. Ignore the daily stamp; the second launch
    // queries again. Increase notice_due's latest timeout from one to three
    // seconds; the fixture's observed connection lifetime exceeds its limit.
    // Print automatic-check errors; the offline launch differs from the
    // same terminal launch with checking disabled, violating silence.
    // Fixture request records and its release tag are outside expectations.
    let env = TestEnv::with_private_cache("update-notice");
    let installed = env.root.join("pinfold");
    fs::copy(pinfold(), &installed).unwrap();
    let mut fixture = ReleaseProxy::new(&env);
    let checks_path = fixture.root.join("checks");
    let checks = || {
        fs::read_to_string(&checks_path)
            .unwrap_or_default()
            .lines()
            .count()
    };
    let attach = ["attach", "--box", "missing"];
    fixture
        .command(&env, &installed)
        .args(attach)
        .output()
        .unwrap();
    let unchecked = fixture
        .terminal(&env, &installed, &attach)
        .env("PINFOLD_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    run_ok(&mut fixture.terminal(&env, &installed, &["--version"]));
    run_ok(&mut fixture.terminal(&env, &installed, &["attach", "--help"]));
    assert_eq!(checks(), 0, "suppressed launches checked for updates");

    let first = fixture
        .terminal(&env, &installed, &attach)
        .output()
        .unwrap();
    assert_eq!(checks(), 1);
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("999.0.0"),
        "fixture version absent: {first:?}"
    );
    fixture
        .terminal(&env, &installed, &attach)
        .output()
        .unwrap();
    assert_eq!(checks(), 1, "second launch queried within the same day");

    // A fresh host cache makes an offline check due without modifying
    // pinfold's cache record or waiting for the clock to advance.
    let offline = TestEnv::with_private_cache("update-offline");
    run_ok(
        offline
            .command(&installed)
            .args(["box", "list", "--label", "dev.pinfold.project"]),
    );
    fs::write(fixture.root.join("offline"), b"").unwrap();
    let result = fixture
        .terminal(&offline, &installed, &attach)
        .output()
        .unwrap();
    let lifetime = fixture.wait_for_stalled_check();
    assert!(
        lifetime <= Duration::from_millis(1750),
        "stalled HTTPS check lasted {lifetime:?}"
    );
    assert_eq!(checks(), 2, "offline check did not reach the fixture");
    assert_eq!(
        result.stdout, unchecked.stdout,
        "offline check changed command output"
    );
    fixture
        .terminal(&offline, &installed, &attach)
        .output()
        .unwrap();
    assert_eq!(checks(), 2, "offline launch retried within the same day");
}

#[test]
fn changed_bundled_profiles_are_reported_once() {
    let _runtime = crate::shared_runtime();
    // Guarantee 31. Sabotage: omit the embedded-profile comparison, or gate
    // it behind the daily network interval; the changed binary stays silent.
    // Do not save the new fingerprint; the next launch warns again. Reset
    // checked_at while saving it; the next launch queries again. Warn on
    // the first launch without history; the initial control fails. Resolve
    // --builtin through user profile lookup; the copied package misses the
    // changed full-profile package. Hash only the default profile; the full-
    // only change is missed. Fixture source bytes and the spec's
    // bundled-profiles-changed token give the
    // expectations. Both executables are built from production source.
    let env = TestEnv::with_private_cache("profile-notice");
    let installed = env.root.join("pinfold");
    fs::copy(pinfold(), &installed).unwrap();
    let fixture = ReleaseProxy::new(&env);
    let attach = ["attach", "--box", "missing"];
    let first = fixture
        .terminal(&env, &installed, &attach)
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&first.stdout).contains("bundled-profiles-changed"),
        "first launch claimed a change without history: {first:?}"
    );

    let changed = changed_profiles_binary(&env);
    let second = fixture.terminal(&env, &changed, &attach).output().unwrap();
    assert!(
        String::from_utf8_lossy(&second.stdout).contains("bundled-profiles-changed"),
        "changed profiles were not reported: {second:?}"
    );
    let third = fixture.terminal(&env, &changed, &attach).output().unwrap();
    assert!(
        !String::from_utf8_lossy(&third.stdout).contains("bundled-profiles-changed"),
        "unchanged profiles were reported again: {third:?}"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("checks"))
            .unwrap()
            .lines()
            .count(),
        1,
        "profile notice should not require another daily network check"
    );

    // The notice's suggested copy must expose the updated built-in defaults
    // even when a user profile shadows full, without replacing that copy.
    let override_dir = env.config.join("pinfold/profiles/full");
    let override_settings = override_dir.join("home/.pi/agent/settings.json");
    fs::create_dir_all(override_settings.parent().unwrap()).unwrap();
    fs::write(override_dir.join("Containerfile"), "FROM scratch\n").unwrap();
    let marker = b"{\"userOverride\":true}\n";
    fs::write(&override_settings, marker).unwrap();
    run_ok(env.command(&changed).args([
        "profile",
        "new",
        "updated-defaults",
        "--from",
        "full",
        "--builtin",
    ]));
    let copied: serde_json::Value = serde_json::from_slice(
        &fs::read(
            env.config
                .join("pinfold/profiles/updated-defaults/share/pi/package.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(copied["fixtureFullChanged"], true);
    assert_eq!(fs::read(override_settings).unwrap(), marker);
}

fn changed_profiles_binary(env: &TestEnv) -> PathBuf {
    // Both update guarantees need the same different production executable.
    // Retain bytes, not a path in another test's removable scratch tree.
    static BINARY: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let bytes = BINARY.get_or_init(|| build_changed_profiles_binary(env));
    let installed = env.root.join("changed-pinfold");
    fs::write(&installed, bytes).unwrap();
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o755)).unwrap();
    installed
}

fn build_changed_profiles_binary(env: &TestEnv) -> Vec<u8> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let source = env.root.join("changed-source");
    fs::create_dir_all(source.join("crates")).unwrap();
    for file in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"] {
        fs::copy(workspace.join(file), source.join(file)).unwrap();
    }
    copy_tree(
        &workspace.join("crates/pinfold"),
        &source.join("crates/pinfold"),
    );
    copy_tree(&workspace.join("profile"), &source.join("profile"));
    let package_path = source.join("profile/full/package.json");
    let mut package: serde_json::Value =
        serde_json::from_slice(&fs::read(&package_path).unwrap()).unwrap();
    package["fixtureFullChanged"] = serde_json::json!(true);
    fs::write(package_path, serde_json::to_vec(&package).unwrap()).unwrap();
    // A private target prevents replacing the suite's executable while other
    // tests run. Linux uses the same production musl target as e2e::pinfold.
    let target = env.root.join("changed-target");
    let mut build = Command::new(env!("CARGO"));
    build
        .args(["build", "-p", "pinfold", "--locked"])
        .current_dir(&source)
        .env("CARGO_TARGET_DIR", &target);
    let binary = if cfg!(target_os = "linux") {
        let triple = format!("{}-unknown-linux-musl", std::env::consts::ARCH);
        build.args(["--target", &triple]);
        target.join(triple).join("debug/pinfold")
    } else {
        target.join("debug/pinfold")
    };
    run_ok(&mut build);
    fs::read(binary).unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let destination = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

struct UpdateOwner {
    child: Child,
    _stdout: BufReader<ChildStdout>,
    down: Command,
}

impl UpdateOwner {
    fn start(env: &TestEnv, installed: &Path, spec: &serde_json::Value) -> Self {
        let mut child = env
            .command(installed)
            .args(["box", "up"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(serde_json::to_string(spec).unwrap().as_bytes())
            .unwrap();
        let mut line = String::new();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        stdout.read_line(&mut line).unwrap();
        let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["event"], "ready", "box owner failed: {ready}");
        let mut down = env.command(installed);
        down.args(["box", "down", spec["name"].as_str().unwrap()]);
        Self {
            child,
            down,
            _stdout: stdout,
        }
    }
}

impl Drop for UpdateOwner {
    fn drop(&mut self) {
        let _ = self.down.status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct ReleaseProxy {
    child: Child,
    events: BufReader<ChildStdout>,
    root: PathBuf,
    port: u16,
}

impl ReleaseProxy {
    fn new(env: &TestEnv) -> Self {
        let root = env.root.join("release-fixture");
        fs::create_dir(&root).unwrap();
        fs::copy(pinfold(), root.join("release")).unwrap();
        run_ok(Command::new("openssl").current_dir(&root).args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=api.github.com",
            "-addext",
            "subjectAltName=DNS:api.github.com,DNS:github.com",
            "-keyout",
            "key.pem",
            "-out",
            "cert.pem",
        ]));
        let target = if cfg!(target_os = "macos") {
            "aarch64-apple-darwin".to_string()
        } else {
            format!("{}-unknown-linux-musl", std::env::consts::ARCH)
        };
        let mut child = Command::new("python3")
            .args(["-c", include_str!("release_proxy.py")])
            .arg(&root)
            .arg(format!("pinfold-999.0.0-{target}"))
            .stdout(Stdio::piped())
            .spawn()
            .expect("start host HTTPS proxy");
        let mut line = String::new();
        let mut events = BufReader::new(child.stdout.take().unwrap());
        events.read_line(&mut line).expect("read fixture readiness");
        let port = line.trim().parse().expect("fixture reports its port");
        Self {
            child,
            events,
            root,
            port,
        }
    }

    fn wait_for_download(&mut self) {
        let mut line = String::new();
        self.events.read_line(&mut line).unwrap();
        assert_eq!(
            line.trim(),
            "download",
            "release fixture did not observe an asset request"
        );
    }

    fn release_download(&self) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.write_all(b"RELEASE\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert_eq!(reply, "released\n");
    }

    fn wait_for_stalled_check(&mut self) -> Duration {
        let mut line = String::new();
        self.events.read_line(&mut line).unwrap();
        let event: serde_json::Value =
            serde_json::from_str(&line).expect("fixture check completion");
        assert_eq!(event["event"], "stalled-check-closed");
        assert_eq!(
            event["closed"], true,
            "curl did not close its stalled request: {event}"
        );
        Duration::from_secs_f64(event["seconds"].as_f64().expect("fixture lifetime"))
    }

    fn command(&self, env: &TestEnv, binary: &Path) -> Command {
        let mut command = env.command(binary);
        command
            .env("HTTPS_PROXY", format!("http://127.0.0.1:{}", self.port))
            .env("CURL_CA_BUNDLE", self.root.join("cert.pem"))
            .env_remove("https_proxy")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy");
        command
    }

    fn terminal(&self, env: &TestEnv, binary: &Path, args: &[&str]) -> Command {
        let mut command = self.command(env, Path::new("python3"));
        command
            .args([
                "-c",
                r#"
import errno, os, pty, select, subprocess, sys, time
master, slave = pty.openpty()
child = subprocess.Popen(sys.argv[1:], stdin=slave, stdout=slave, stderr=slave)
os.close(slave)
deadline = time.monotonic() + 10
try:
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([master], [], [], remaining)[0]:
            child.kill()
            child.wait()
            sys.exit(124)
        try:
            chunk = os.read(master, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
        if not chunk:
            break
        sys.stdout.buffer.write(chunk)
finally:
    os.close(master)
sys.exit(child.wait())
"#,
            ])
            .arg(binary)
            .args(args)
            .current_dir(&env.root);
        command
    }
}

impl Drop for ReleaseProxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
