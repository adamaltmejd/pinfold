//! Host self-update through a real HTTPS release fixture.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use e2e::{TestEnv, default_image, pinfold, run_ok};

use super::box_::{box_name, box_up};

#[test]
fn host_updates_are_verified_and_atomic() {
    // Guarantee 28. Sabotage: skip SHA256SUMS verification; the damaged
    // release succeeds. Rename before verification; the old inode changes
    // on refusal. Write into the executable instead of renaming; the final
    // inode stays unchanged. Remove the live owner guard; updating
    // while a real box owner is alive succeeds. Replace argv[0] rather than
    // current_exe(); the host command symlink is replaced and the installed
    // binary's inode stays unchanged. Remove the
    // exclusive executable lock; the owner under a different XDG root no
    // longer prevents updating its executable.
    // Expected release bytes and digest come from the host fixture, and
    // inode identity comes from the host filesystem.
    let env = TestEnv::new("update");
    let installed = env.root.join("pinfold");
    fs::copy(pinfold(), &installed).unwrap();
    let alias = env.root.join("pi");
    symlink("pinfold", &alias).unwrap();
    let update_alias = env.root.join("pinfold-link");
    symlink("pinfold", &update_alias).unwrap();
    let fixture = ReleaseProxy::new(&env);
    let original = fs::read(&installed).unwrap();
    let original_inode = fs::metadata(&installed).unwrap().ino();

    let mut check = fixture.command(&env, &installed);
    run_ok(check.args(["update", "--check"]));
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
        "bad checksum installed: {failure:?}"
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
    let _owner = UpdateOwner::start(&other, &installed, &spec);
    let failure = fixture
        .command(&env, &installed)
        .arg("update")
        .output()
        .unwrap();
    assert!(
        !failure.status.success(),
        "active executable replaced: {failure:?}"
    );
    assert!(
        String::from_utf8_lossy(&failure.stderr).contains("update-busy"),
        "wrong refusal: {failure:?}"
    );
    assert_eq!(fs::metadata(&installed).unwrap().ino(), original_inode);
    drop(_owner);

    run_ok(fixture.command(&env, &update_alias).arg("update"));
    assert_eq!(fs::read(&installed).unwrap(), original);
    assert_ne!(fs::metadata(&installed).unwrap().ino(), original_inode);
    assert_eq!(fs::read_link(&alias).unwrap(), Path::new("pinfold"));
    run_ok(env.command(&installed).arg("--version"));
}

#[test]
fn interactive_update_checks_are_bounded() {
    // Guarantee 29. Sabotage: remove terminal or opt-out checks; suppressed
    // launches reach the fixture. Ignore the daily stamp; the second launch
    // queries again. Drop --max-time; the stalled fixture never finishes.
    // Print automatic-check errors; the offline launch differs from the
    // same terminal launch with checking disabled, violating silence.
    // Fixture request records and its release tag are outside expectations.
    let env = TestEnv::with_private_cache("update-notice");
    let installed = env.root.join("pinfold");
    fs::copy(pinfold(), &installed).unwrap();
    let fixture = ReleaseProxy::new(&env);
    let checks = || {
        fs::read_to_string(fixture.root.join("checks"))
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
    let start = Instant::now();
    let result = fixture
        .terminal(&offline, &installed, &attach)
        .output()
        .unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "offline launch exceeded its check budget"
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
fn changed_bundled_defaults_are_reported_once() {
    // Guarantee 31. Sabotage: omit the embedded-profile comparison, or gate
    // it behind the daily network interval; the changed binary stays silent.
    // Do not save the new fingerprint; the next launch warns again. Warn on
    // the first launch without history; the initial control fails. Resolve
    // --builtin through user profile lookup; the copied settings miss the
    // changed fixture default. Fixture source bytes and the spec's
    // default-profile-changed token give the
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
        !String::from_utf8_lossy(&first.stdout).contains("default-profile-changed"),
        "first launch claimed a change without history: {first:?}"
    );

    let changed = changed_defaults_binary(&env);
    let second = fixture.terminal(&env, &changed, &attach).output().unwrap();
    assert!(
        String::from_utf8_lossy(&second.stdout).contains("default-profile-changed"),
        "changed defaults were not reported: {second:?}"
    );
    let third = fixture.terminal(&env, &changed, &attach).output().unwrap();
    assert!(
        !String::from_utf8_lossy(&third.stdout).contains("default-profile-changed"),
        "unchanged defaults were reported again: {third:?}"
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
    // even when a user profile shadows default, without replacing that copy.
    let override_dir = env.config.join("pinfold/profiles/default");
    let override_settings = override_dir.join("home/.pi/agent/settings.json");
    fs::create_dir_all(override_settings.parent().unwrap()).unwrap();
    fs::write(override_dir.join("Containerfile"), "FROM scratch\n").unwrap();
    let marker = b"{\"userOverride\":true}\n";
    fs::write(&override_settings, marker).unwrap();
    run_ok(
        env.command(&changed)
            .args(["profile", "new", "updated-defaults", "--builtin"]),
    );
    let copied: serde_json::Value = serde_json::from_slice(
        &fs::read(
            env.config
                .join("pinfold/profiles/updated-defaults/home/.pi/agent/settings.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(copied["fixtureDefaultChanged"], true);
    assert_eq!(fs::read(override_settings).unwrap(), marker);
}

fn changed_defaults_binary(env: &TestEnv) -> PathBuf {
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
    let settings = source.join("profile/home/.pi/agent/settings.json");
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    defaults["fixtureDefaultChanged"] = serde_json::json!(true);
    fs::write(settings, serde_json::to_vec(&defaults).unwrap()).unwrap();
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
    let installed = env.root.join("changed-pinfold");
    fs::copy(binary, &installed).unwrap();
    installed
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
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .expect("read fixture readiness");
        let port = line.trim().parse().expect("fixture reports its port");
        Self { child, root, port }
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
