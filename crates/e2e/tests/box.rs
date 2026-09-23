//! End-to-end tests for guarantees 5, 9 and 10 in docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI. The harness
//! builds the `pinfold` binary, builds the default profile image once, and
//! drives pinfold as a user would: the CLI, environment variables and the
//! box spec are its only seams.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::OnceLock;

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
        "image": default_image(binary, &env),
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
    // Not a sabotage on Apple: running `box exec` as root. virtiofs reports
    // every host file as the host user's whatever the guest uid, so the owner
    // assertion still passes; the uid itself is guarantee 5's to check.
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
        "image": default_image(binary, &env),
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

#[test]
fn nothing_can_gain_privileges() {
    // Sabotage: drop `find / -xdev -perm /6000 -type f -exec chmod a-s {} +`
    // from profile/Containerfile; the setuid/setgid scan then lists files and
    // fails. Sabotage: drop `--read-only` from the Apple adapter's
    // `container run` argv; the rootfs write then succeeds and its assertion
    // fails.
    let binary = pinfold();
    let env = TestEnv::new("privileges");
    let image = default_image(binary, &env);
    let dir = TestDir::new(&env, "mount");
    let name = format!("pinfold-e2e-{}-privileges", std::process::id());
    let spec = serde_json::json!({
        "name": name,
        "image": image,
        "mounts": [{ "host": dir.path(), "guest": "/workspace" }],
    });
    let up = box_up(binary, &env, &spec, &name);

    // Exec'd work runs as the host uid:gid with an empty capability bounding
    // set. Read /proc: Apple's virtiofs reports host files as the host user's
    // whatever the guest uid, so ownership cannot show a root exec.
    let work = box_exec(binary, &env, &name, &["cat", "/proc/self/status"]);
    assert_eq!(
        work.code, 0,
        "reading /proc/self/status failed: {}",
        work.stderr
    );
    assert_eq!(
        status_field(&work.stdout, "CapBnd:"),
        "0000000000000000",
        "exec capability bound"
    );
    assert_process_ids(&work.stdout, "exec");

    // PID 1 is pinfold init, also as the host uid:gid.
    let init = box_exec(binary, &env, &name, &["cat", "/proc/1/status"]);
    assert_eq!(
        init.code, 0,
        "reading /proc/1/status failed: {}",
        init.stderr
    );
    assert_process_ids(&init.stdout, "PID 1");

    // No setuid or setgid files on the root filesystem. The marker proves the
    // scan ran even though unreadable directories make find exit nonzero.
    let setuid = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            "command -v find >/dev/null || exit 1; \
             find / -xdev -perm /6000 -type f 2>/dev/null; echo scan-complete",
        ],
    );
    assert_eq!(setuid.code, 0, "setuid scan failed: {}", setuid.stderr);
    assert_eq!(
        setuid.stdout, "scan-complete\n",
        "setuid or setgid files: {}",
        setuid.stdout
    );

    // The rootfs is read-only.
    let rootfs = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf x > /pinfold-root-write-test"],
    );
    assert_ne!(rootfs.code, 0, "the rootfs accepted a write");
    assert!(
        rootfs.stderr.contains("Read-only file system"),
        "the rootfs write failed for another reason: {}",
        rootfs.stderr
    );

    // Positive controls: the same write works on /tmp and the project mount.
    let tmp = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf x > /tmp/pinfold-write-test"],
    );
    assert_eq!(tmp.code, 0, "writing /tmp failed: {}", tmp.stderr);
    let workspace = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf x > /workspace/pinfold-write-test"],
    );
    assert_eq!(
        workspace.code, 0,
        "writing /workspace failed: {}",
        workspace.stderr
    );
    assert!(dir.path().join("pinfold-write-test").is_file());

    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);
}

#[test]
fn only_allowlisted_hosts_get_through() {
    // Sabotage: make the proxy's allowlist check accept every host; example.com
    // then answers and the 403 and "not allowlisted" log assertions fail. The
    // api.github.com request is the positive control that the same path lets
    // an allowlisted host through.
    let binary = pinfold();
    let env = TestEnv::new("egress");
    let name = format!("pinfold-e2e-{}-egress", std::process::id());
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": { "allow": ["api.github.com"] },
    });
    let up = box_up(binary, &env, &spec, &name);

    let allowed = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "--max-time",
            "30",
            "-o",
            "/dev/null",
            "https://api.github.com/",
        ],
    );
    assert_eq!(
        allowed.code, 0,
        "allowlisted host failed: {}",
        allowed.stderr
    );

    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "--max-time",
            "30",
            "-o",
            "/dev/null",
            "https://example.com/",
        ],
    );
    assert_ne!(denied.code, 0, "example.com was allowed through");
    assert!(
        denied.stderr.contains("403"),
        "expected a proxy 403: {}",
        denied.stderr
    );

    // The log names the host, the decision and its reason.
    let log = fs::read_to_string(egress_log(&env, &name)).expect("read egress log");
    let lines: Vec<serde_json::Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).expect("log line is JSON"))
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line["host"] == "api.github.com" && line["decision"] == "allowed"),
        "no allowed decision for api.github.com: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line["host"] == "example.com"
            && line["decision"] == "refused"
            && line["reason"] == "not allowlisted"),
        "no not-allowlisted refusal for example.com: {lines:?}"
    );

    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);
}

#[test]
fn losing_the_owner_fails_closed() {
    // Sabotage: make `box prune` skip boxes whose owner is gone; the box
    // survives prune and the post-prune list assertion fails. Sabotage: start
    // the proxy outside the `box up` process; it survives the SIGKILL, logs
    // the post-kill request and lets it through, so the unchanged-log
    // assertion fails. Curl's own error is not asserted: Apple's forwarder
    // sometimes hangs after the owner dies rather than closing, so the
    // request may end in a timeout instead of a refusal.
    let binary = pinfold();
    let env = TestEnv::new("owner-gone");
    let name = format!("pinfold-e2e-{}-owner-gone", std::process::id());
    let label = "dev.yard.lane=e2e-owner-gone";
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.yard.lane": "e2e-owner-gone" },
        "egress": { "allow": ["api.github.com"] },
    });
    let mut up = box_up(binary, &env, &spec, &name);

    // Positive control: the box has egress while its owner lives.
    let allowed = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "--max-time",
            "30",
            "-o",
            "/dev/null",
            "https://api.github.com/",
        ],
    );
    assert_eq!(
        allowed.code, 0,
        "positive control failed: {}",
        allowed.stderr
    );

    up.kill();
    up.wait();

    // The positive control left its decision in the log. A live proxy
    // anywhere would log before it dials, so no new line means no proxy saw
    // the request; the request itself may hang, because Apple's forwarder
    // does not reliably close after the owner dies.
    let before = egress_log_lines(&env, &name);
    assert!(
        !before.is_empty(),
        "the positive control left no egress log line"
    );
    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "--max-time",
            "5",
            "-o",
            "/dev/null",
            "https://api.github.com/",
        ],
    );
    assert_ne!(
        denied.code, 0,
        "the box still had egress after the owner died"
    );
    let after = egress_log_lines(&env, &name);
    assert_eq!(
        after.len(),
        before.len(),
        "the egress log gained a line after the owner died: {after:?}"
    );

    // `box prune` removes the leftover by label.
    let status = box_prune(binary, &env);
    assert!(status.success(), "box prune failed: {status}");
    let listed = box_list(binary, &env, label);
    assert!(
        !listed.iter().any(|box_| box_["name"] == name),
        "prune left the box: {listed:?}"
    );
}

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    // Guarantee 15's after-build half: after three builds of one source, two
    // images remain. The `pinfold clean` half of this test lands with the
    // clean command.
    //
    // Sabotage: make `keep_two_images` return before it removes anything;
    // three images remain and the two-image assertion fails. Sabotage: drop
    // the `dev.pinfold.profile` label from the build; no image matches and
    // the count is zero.
    let binary = pinfold();
    let env = TestEnv::new("cleanup");
    // A profile of this test's own, named for this run, so the operator's
    // default profile images, the other tests and a failed run's leftovers
    // cannot share the source.
    let profile = format!("e2e-maintenance-{}", std::process::id());
    let _images = ImageCleanup {
        source: profile.clone(),
    };
    let containerfile = env
        .config
        .join("pinfold")
        .join("profiles")
        .join(&profile)
        .join("Containerfile");
    fs::create_dir_all(containerfile.parent().unwrap()).unwrap();
    // `FROM scratch` keeps the test off the network and fast.
    fs::write(&containerfile, b"FROM scratch\n").unwrap();

    for _ in 0..3 {
        let output = env
            .command(binary)
            .args(["build", "--profile", &profile])
            .output()
            .expect("run pinfold build");
        assert!(
            output.status.success(),
            "pinfold build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let images = labeled_images("dev.pinfold.profile", &profile);
    let mut digests: Vec<&str> = images.iter().map(|(digest, _)| digest.as_str()).collect();
    digests.sort_unstable();
    digests.dedup();
    assert_eq!(
        digests.len(),
        2,
        "after three builds of one source, two images should remain: {images:?}"
    );
}

/// Removes the run's images from the runtime store on drop, so a failing run
/// does not leave them for the next run to count or for the operator's disk.
struct ImageCleanup {
    source: String,
}

impl Drop for ImageCleanup {
    fn drop(&mut self) {
        // Best effort: a Drop during unwinding must not panic.
        let Ok(output) = Command::new("container")
            .args(["image", "list", "--quiet"])
            .output()
        else {
            return;
        };
        if !output.status.success() {
            return;
        }
        // Every build tags the image `pinfold/profile-<source>:<build>`, and
        // the reference remains even when the label sabotage drops the source
        // label.
        let prefix = format!("pinfold/profile-{}:", self.source);
        let references = String::from_utf8_lossy(&output.stdout);
        for reference in references.lines() {
            if !reference.contains(&prefix) {
                continue;
            }
            let _ = Command::new("container")
                .args(["image", "delete", reference])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// The `(digest, reference)` of every image carrying `label = value`, from
/// the runtime itself: `container image list` is the ground truth for what
/// remains.
fn labeled_images(label: &str, value: &str) -> Vec<(String, String)> {
    let output = Command::new("container")
        .args(["image", "list", "--format", "json"])
        .output()
        .expect("run container image list");
    assert!(
        output.status.success(),
        "container image list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let images: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("image list is JSON");
    images
        .into_iter()
        .filter(|image| image_has_label(image, label, value))
        .filter_map(|image| {
            Some((
                image["id"].as_str()?.to_string(),
                image["configuration"]["name"].as_str()?.to_string(),
            ))
        })
        .collect()
}

/// Whether one `container image list` entry carries `label = value`, as an
/// OCI image config label or an index descriptor annotation.
fn image_has_label(image: &serde_json::Value, label: &str, value: &str) -> bool {
    if image["configuration"]["descriptor"]["annotations"][label] == value {
        return true;
    }
    image["variants"].as_array().is_some_and(|variants| {
        variants
            .iter()
            .any(|variant| variant["config"]["config"]["Labels"][label] == value)
    })
}

/// One `/proc/<pid>/status` field's value.
fn status_field(status: &str, key: &str) -> String {
    status
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .unwrap_or_else(|| panic!("{key} missing from:\n{status}"))
        .trim()
        .to_string()
}

/// Assert a `/proc/<pid>/status` dump's real, effective, saved and fs uid and
/// gid are all the host's.
fn assert_process_ids(status: &str, who: &str) {
    for (key, want) in [("Uid:", host_id("-u")), ("Gid:", host_id("-g"))] {
        for got in status_field(status, key).split_whitespace() {
            assert_eq!(got, want.as_str(), "{who} {key}");
        }
    }
}

fn host_id(flag: &str) -> String {
    let output = Command::new("id").arg(flag).output().expect("run id");
    assert!(output.status.success(), "id {flag} failed");
    String::from_utf8(output.stdout)
        .expect("id output is UTF-8")
        .trim()
        .to_string()
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

/// The stable ref of the built-in default profile's image, built once per
/// suite run. Every test's boxes run on it.
fn default_image(binary: &Path, env: &TestEnv) -> &'static str {
    static IMAGE: OnceLock<String> = OnceLock::new();
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
        String::from_utf8(output.stdout)
            .expect("build output is UTF-8")
            .trim()
            .to_string()
    })
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
            .unwrap_or(std::env::temp_dir())
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

    fn kill(&mut self) {
        self.child.kill().expect("kill box up");
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

fn box_prune(binary: &Path, env: &TestEnv) -> ExitStatus {
    env.command(binary)
        .args(["box", "prune"])
        .stdin(Stdio::null())
        .status()
        .expect("run pinfold box prune")
}

/// The box's egress log, at the fixed path under pinfold's state dir.
fn egress_log(env: &TestEnv, name: &str) -> PathBuf {
    env.state
        .join("pinfold")
        .join("egress")
        .join(format!("{name}.jsonl"))
}

/// The parsed decision lines of a box's egress log.
fn egress_log_lines(env: &TestEnv, name: &str) -> Vec<serde_json::Value> {
    fs::read_to_string(egress_log(env, name))
        .expect("read egress log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("log line is JSON"))
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
