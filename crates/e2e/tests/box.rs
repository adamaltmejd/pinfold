//! End-to-end tests for the guarantees in docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with rootless podman. The harness builds the `pinfold` binary, builds the
//! default profile image once, and drives pinfold as a user would: the CLI,
//! environment variables and the box spec are its only seams.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::{Mutex, OnceLock};

use e2e::{HttpFixture, TestEnv, pinfold};

/// The two owner-gone tests share one hazard: either one's removal can take
/// the other's dead box before the other expects it. Hold this from killing
/// an owner until that test's removal has run.
static DEAD_BOX_RACE: Mutex<()> = Mutex::new(());

/// `cleanup_removes_only_pinfolds_garbage` deletes the runtime's builder in
/// its `clean`, and a build racing that deletion fails. Every test that
/// builds holds this from before its first build through its last.
static BUILDER_RACE: Mutex<()> = Mutex::new(());

#[test]
fn box_lifecycle_works_for_a_caller() {
    // Guarantee 9: the lifecycle works for a caller.
    // Sabotage: make `box down` a no-op; the post-down list assertion fails.
    // Sabotage: drop the final `down` line; the last-line assertion fails.
    // Sabotage: make `box exec` drop the runtime's exit status and return 0;
    // the exit-3 assertion fails, and the zero-exit command below is the
    // positive control that the same path can succeed.
    // Sabotage: skip exec's existence check; the post-down exec returns the
    // runtime's 125 instead of 3.
    let binary = pinfold();
    let env = TestEnv::new("lifecycle");
    let name = format!("pinfold-e2e-{}-lifecycle", std::process::id());
    let label = "dev.example.test=lifecycle";
    let image = default_image(binary, &env);
    let spec = serde_json::json!({
        "name": name,
        "image": image,
        "labels": { "dev.example.test": "lifecycle" },
    });
    let mut up = box_up(binary, &env, &spec, &name);

    // `ready` carries the owner and the box's full label set, the image's
    // identity labels included.
    let build = runtime_images()
        .expect("list the runtime's images")
        .into_iter()
        .find(|known| {
            known
                .names
                .iter()
                .any(|name| name.strip_prefix("localhost/").unwrap_or(name) == image)
        })
        .and_then(|known| known.labels.get("dev.pinfold.build").cloned())
        .expect("the default image records dev.pinfold.build");
    assert_eq!(
        up.ready["owner"],
        up.pid(),
        "ready owner is not up's pid: {}",
        up.ready
    );
    assert_eq!(up.ready["labels"]["dev.example.test"], "lifecycle");
    assert_eq!(
        up.ready["labels"]["dev.pinfold.build"],
        build.as_str(),
        "ready lost the image's build label: {}",
        up.ready
    );

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

    // `exec` on the box `down` removed is pinfold's own absent-box failure:
    // exit 3, not the runtime's error and exit code.
    let absent = box_exec(binary, &env, &name, &["sh", "-c", "exit 0"]);
    assert_eq!(
        absent.code, 3,
        "exec on an absent box did not exit 3: {}",
        absent.stderr
    );

    // Closing stdin is `down`: the owner prints the final `down` line and
    // exits 0.
    let mut again = box_up(binary, &env, &spec, &name);
    let lines = again.close_stdin();
    let down = lines
        .last()
        .unwrap_or_else(|| panic!("up printed no down line: {lines:?}"));
    assert_eq!(
        down["event"], "down",
        "the last line was not down: {lines:?}"
    );
    assert_eq!(down["box"], name);
    assert_eq!(
        down["reason"], "stdin-closed",
        "wrong down reason: {lines:?}"
    );
    assert!(again.wait().success(), "up did not exit 0 for stdin-closed");
}

#[test]
fn up_refuses_before_it_creates() {
    // Guarantee 17: up refuses before it creates.
    // Sabotage: keep the name check after the state dir is created; the
    // first box's `pid` is overwritten and the second `up`'s cleanup takes
    // the first box down, so its `exec` fails.
    let binary = pinfold();
    let env = TestEnv::new("refuses");
    let name = format!("pinfold-e2e-{}-refuses", std::process::id());
    let label = "dev.example.test=refuses";

    // An image that was never built is refused as data, and the refusal
    // leaves no state dir and no box.
    let missing = serde_json::json!({
        "name": name,
        "image": "pinfold-e2e-missing:latest",
        "labels": { "dev.example.test": "refuses" },
    });
    let (code, refused) = box_up_refused(binary, &env, &missing);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["box"], name);
    assert_eq!(refused["reason"], "image-missing");
    let state = env.state.join("pinfold").join("boxes").join(&name);
    assert!(
        !state.exists(),
        "the refused up left a state dir: {}",
        state.display()
    );
    assert!(
        box_list(binary, &env, label).is_empty(),
        "the refused up left a box"
    );

    // A second `up` on a live name is refused without touching the first
    // box.
    let live = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.example.test": "refuses" },
    });
    let mut up = box_up(binary, &env, &live, &name);
    let (code, refused) = box_up_refused(binary, &env, &live);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["box"], name);
    assert_eq!(refused["reason"], "name-in-use");
    let ok = box_exec(binary, &env, &name, &["true"]);
    assert_eq!(
        ok.code, 0,
        "the first box did not survive the refused up: {}",
        ok.stderr
    );

    let _ = box_down(binary, &env, &name);
    assert!(up.wait().success(), "box up did not exit cleanly");
}

#[test]
fn box_shares_files_with_the_host() {
    // Sabotage: drop `readonly` from the adapter's bind mounts; the
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
    // fails. Sabotage: drop `--read-only` from the adapter's `run` argv;
    // the rootfs write then succeeds and its assertion fails.
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

    // On Linux, the podman seccomp profile must also block nested user
    // namespaces. Sabotage: prepend the ERRNO rules for clone and unshare
    // instead of removing `clone`, `clone3` and `unshare` from the default
    // profile's unconditional SCMP_ACT_ALLOW entry; the allow wins,
    // `unshare -U true` succeeds, and this assertion fails. Nested user
    // namespaces on Apple `container` are an open question in the spec, so
    // assert nothing there.
    if cfg!(target_os = "linux") {
        let unshare = box_exec(binary, &env, &name, &["unshare", "-U", "true"]);
        assert_ne!(unshare.code, 0, "unshare -U succeeded in the box");
        assert!(
            unshare.stderr.contains("Operation not permitted"),
            "unshare -U failed for another reason: {}",
            unshare.stderr
        );
        // Positive control: the same box still runs a plain child process.
        let child = box_exec(binary, &env, &name, &["true"]);
        assert_eq!(
            child.code, 0,
            "a plain child process failed: {}",
            child.stderr
        );
    }

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
fn the_proxy_refuses_the_tricks() {
    // Guarantee 3. Each trick has its control in the same box.
    //
    // Sabotage: delete the IP literal check; both 127.0.0.1 requests then
    // log "not allowlisted" (or reach the box's own loopback), so the
    // "ip literal" assertions fail.
    // Sabotage: delete the loopback address check; localhost resolves to
    // 127.0.0.1, the proxy dials it, and the 403 and "loopback" assertions
    // fail.
    // Sabotage: skip the SNI comparison; the --connect-to request finishes
    // its TLS handshake against api.github.com, so the exit-35 and "sni
    // mismatch" assertions fail.
    // Sabotage: check the allowlist before the routes; CONNECT to
    // fixture.internal logs "not allowlisted" and the "route" assertion
    // fails.
    // Sabotage: drop the duplicate-Content-Length check in parse_plain; the
    // raw request reaches api.github.com, so the 400 and "ambiguous
    // framing" assertions fail.
    let binary = pinfold();
    let env = TestEnv::new("tricks");
    let fixture = HttpFixture::start();
    let name = format!("pinfold-e2e-{}-tricks", std::process::id());
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "allow": ["api.github.com", "localhost"],
            "routes": { "fixture.internal": format!("127.0.0.1:{}", fixture.port()) },
        },
    });
    let up = box_up(binary, &env, &spec, &name);

    // Controls: the same paths work when the trick is not played.
    let connect = box_exec(
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
        connect.code, 0,
        "allowlisted CONNECT failed: {}",
        connect.stderr
    );
    let plain = box_exec(
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
            "http://api.github.com/",
        ],
    );
    assert_eq!(
        plain.code, 0,
        "allowlisted plain HTTP failed: {}",
        plain.stderr
    );
    let route = box_exec(
        binary,
        &env,
        &name,
        &["curl", "-sS", "--max-time", "5", "http://fixture.internal/"],
    );
    assert_eq!(route.code, 0, "route control failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "route control answered: {}",
        route.stdout
    );

    // An IP literal, in both request forms.
    let literal_connect = box_exec(
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
            "https://127.0.0.1/",
        ],
    );
    assert_ne!(
        literal_connect.code, 0,
        "CONNECT to an IP literal succeeded"
    );
    assert!(
        literal_connect.stderr.contains("403"),
        "CONNECT to an IP literal got no 403: {}",
        literal_connect.stderr
    );
    let literal_plain = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "-f",
            "--max-time",
            "30",
            "-o",
            "/dev/null",
            "http://127.0.0.1/",
        ],
    );
    assert_ne!(
        literal_plain.code, 0,
        "plain HTTP to an IP literal succeeded"
    );
    assert!(
        literal_plain.stderr.contains("403"),
        "plain HTTP to an IP literal got no 403: {}",
        literal_plain.stderr
    );

    // A name that resolves to loopback.
    let loopback = box_exec(
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
            "https://localhost/",
        ],
    );
    assert_ne!(loopback.code, 0, "a name resolving to loopback succeeded");
    assert!(
        loopback.stderr.contains("403"),
        "the loopback name got no 403: {}",
        loopback.stderr
    );

    // A ClientHello whose SNI names another host. The proxy answers the
    // CONNECT with 200 and then refuses on the ClientHello, so curl fails
    // the TLS handshake (35) rather than reading an HTTP status.
    let sni = box_exec(
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
            "--connect-to",
            "example.com:443:api.github.com:443",
            "https://example.com/",
        ],
    );
    assert_eq!(
        sni.code, 35,
        "the mismatched SNI did not fail the handshake: {}",
        sni.stderr
    );
    assert!(
        sni.stderr.contains("SSL"),
        "the mismatched SNI failed for another reason: {}",
        sni.stderr
    );

    // CONNECT to a route name.
    let route_connect = box_exec(
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
            "https://fixture.internal/",
        ],
    );
    assert_ne!(route_connect.code, 0, "CONNECT to a route succeeded");
    assert!(
        route_connect.stderr.contains("403"),
        "CONNECT to a route got no 403: {}",
        route_connect.stderr
    );

    // Ambiguous framing: two Content-Length headers, sent raw because curl
    // will not.
    let framing = box_exec(
        binary,
        &env,
        &name,
        &[
            "bash",
            "-c",
            "exec 3<>/dev/tcp/127.0.0.1/3128; \
             printf 'GET http://api.github.com/ HTTP/1.1\\r\\nHost: api.github.com\\r\\nContent-Length: 0\\r\\nContent-Length: 0\\r\\n\\r\\n' >&3; \
             cat <&3",
        ],
    );
    assert!(
        framing.stdout.contains("400"),
        "ambiguous framing got no 400: {}",
        framing.stdout
    );

    // Every refusal names its own reason, distinct from "not allowlisted".
    let lines = egress_log_lines(&env, &name);
    for (host, reason) in [
        ("127.0.0.1", "ip literal"),
        ("localhost", "loopback"),
        ("api.github.com", "sni mismatch"),
        ("fixture.internal", "route"),
    ] {
        assert!(
            lines.iter().any(|line| line["host"] == host
                && line["decision"] == "refused"
                && line["reason"] == reason),
            "no {reason} refusal for {host}: {lines:?}"
        );
    }
    assert!(
        lines
            .iter()
            .any(|line| line["decision"] == "refused" && line["reason"] == "ambiguous framing"),
        "no ambiguous framing refusal: {lines:?}"
    );

    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);
}

#[test]
fn losing_the_owner_fails_closed() {
    // Sabotage: make `box prune` skip boxes whose owner is gone; the box
    // survives prune and the post-prune list assertion fails. Sabotage:
    // report `owner_alive` as true whenever the label parses; the dead-owner
    // `owner_alive` assertion fails. Sabotage: start the proxy outside the
    // `box up` process; it survives the SIGKILL, logs the post-kill request
    // and lets it through, so the unchanged-log assertion fails. Curl's own
    // error is not asserted: Apple's forwarder sometimes hangs after the
    // owner dies rather than closing, so the request may end in a timeout
    // instead of a refusal.
    let binary = pinfold();
    let env = TestEnv::new("owner-gone");
    let name = format!("pinfold-e2e-{}-owner-gone", std::process::id());
    let label = "dev.example.test=owner-gone";
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.example.test": "owner-gone" },
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

    // Hold the race against the cleanup test's `clean`, which removes any
    // dead pinfold box, through this test's `box prune`.
    let _race = DEAD_BOX_RACE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
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

    // Pinfold's own liveness test reports the owner gone before prune acts:
    // the box is still listed, with `owner_alive` false.
    let owner = up.pid();
    let listed = box_list(binary, &env, label);
    let leftover = listed
        .iter()
        .find(|box_| box_["name"] == name)
        .unwrap_or_else(|| panic!("the dead owner's box is not listed: {listed:?}"));
    assert_eq!(
        leftover["owner_alive"], false,
        "the dead owner's box reports owner_alive true: {leftover:?}"
    );
    assert_eq!(
        leftover["owner"].as_u64(),
        Some(u64::from(owner)),
        "the dead owner's box names another owner: {leftover:?}"
    );

    // `box prune` removes the leftover by label and reports the removal.
    let output = box_prune(binary, &env);
    assert!(
        output.status.success(),
        "box prune failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pruned: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .expect("prune output is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("prune line is JSON"))
        .collect();
    let line = pruned
        .iter()
        .find(|line| line["box"] == name)
        .unwrap_or_else(|| panic!("prune did not report {name}: {pruned:?}"));
    assert_eq!(line["event"], "pruned", "not a pruned line: {line:?}");
    assert_eq!(
        line["owner"].as_u64(),
        Some(u64::from(owner)),
        "prune named another owner: {line:?}"
    );
    let listed = box_list(binary, &env, label);
    assert!(
        !listed.iter().any(|box_| box_["name"] == name),
        "prune left the box: {listed:?}"
    );
}

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    // Guarantee 15: cleanup removes only pinfold's garbage.
    //
    // Sabotage: make `keep_two_images` return before it removes anything;
    // three images remain and the two-image assertion fails. Sabotage: drop
    // the `dev.pinfold.profile` label from the build; no image matches and
    // the count is zero. Sabotage: make `clean` remove every image instead
    // of only pinfold's own; the unlabeled image assertion fails. Sabotage:
    // drop the live-box check from `clean`; the live project's marker is
    // deleted and the marker-survives assertion fails. Sabotage: make
    // `clean` remove every project state instead of only the stale ones;
    // the other project's state assertion fails. Sabotage: make `clean`
    // remove every box; the live box assertion fails. Sabotage: make
    // `clean` skip boxes whose owner is gone; the dead box assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("cleanup");
    // `clean` deletes the runtime's builder, so hold off the other tests'
    // builds through this test's `clean`: a build racing the deletion fails.
    // This also waits for the suite's shared default image, which must be
    // built before the deletion.
    let _builds = BUILDER_RACE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    default_image(binary, &env);
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

    // An unlabeled image: no `dev.pinfold` label, so `clean` must leave it.
    // Its tag shares the profile prefix, so the guard above deletes it.
    let context = containerfile.parent().unwrap();
    let unlabeled = format!("pinfold/profile-{profile}:unlabeled");
    let status = Command::new(image_cli())
        .args(["build", "--file"])
        .arg(&containerfile)
        .args(["--tag", &unlabeled])
        .arg(context)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .expect("run the runtime's build");
    assert!(status.success(), "building the unlabeled image failed");
    assert!(
        runtime_images()
            .expect("list images")
            .iter()
            .any(|image| image
                .names
                .iter()
                .any(|name| name.contains(unlabeled.as_str()))),
        "the unlabeled image is missing before clean"
    );

    // A project's state: a refused `pinfold pi` creates it before it names
    // the missing image. The profile is never built, so this part downloads
    // no pi artifact. Two projects get a cache marker: one runs a live box,
    // the other does not.
    let missing = format!("{profile}-missing");
    let missing_containerfile = env
        .config
        .join("pinfold")
        .join("profiles")
        .join(&missing)
        .join("Containerfile");
    fs::create_dir_all(missing_containerfile.parent().unwrap()).unwrap();
    fs::write(&missing_containerfile, b"FROM scratch\n").unwrap();

    let live_project = TestDir::new(&env, "live-project");
    let refused = env
        .command(binary)
        .args(["pi", "--version"])
        .env("PINFOLD_PROFILE", &missing)
        .current_dir(live_project.path())
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold pi");
    assert!(!refused.status.success(), "pi ran a profile with no image");
    let live_state = project_state_dir(&env, live_project.path());
    let live_id = live_state
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let live_marker = live_state.join("home").join(".cache").join("marker");
    fs::create_dir_all(live_marker.parent().unwrap()).unwrap();
    fs::write(&live_marker, b"live\n").unwrap();

    let other_project = TestDir::new(&env, "other-project");
    let refused = env
        .command(binary)
        .args(["pi", "--version"])
        .env("PINFOLD_PROFILE", &missing)
        .current_dir(other_project.path())
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold pi");
    assert!(!refused.status.success(), "pi ran a profile with no image");
    let other_state = project_state_dir(&env, other_project.path());
    let other_marker = other_state.join("home").join(".cache").join("marker");
    fs::create_dir_all(other_marker.parent().unwrap()).unwrap();
    fs::write(&other_marker, b"other\n").unwrap();

    // A live pinfold box for the live project, and a dead box that names
    // no project.
    let live_label = format!("dev.pinfold.project={live_id}");
    let live = format!("pinfold-e2e-{}-cleanup-live", std::process::id());
    let live_spec = serde_json::json!({
        "name": live,
        "image": format!("pinfold/profile-{profile}:latest"),
        "labels": { "dev.pinfold.project": live_id },
    });
    let _live = box_up(binary, &env, &live_spec, &live);

    let dead_label = "dev.pinfold.project=e2e-cleanup-dead";
    let dead = format!("pinfold-e2e-{}-cleanup-dead", std::process::id());
    let dead_spec = serde_json::json!({
        "name": dead,
        "image": format!("pinfold/profile-{profile}:latest"),
        "labels": { "dev.pinfold.project": "e2e-cleanup-dead" },
    });
    let mut dead_up = box_up(binary, &env, &dead_spec, &dead);
    // Hold the race against the owner-gone test's `box prune` through this
    // test's `clean`.
    let _race = DEAD_BOX_RACE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    dead_up.kill();
    dead_up.wait();

    // Positive controls: everything `clean` sorts out exists before it runs.
    // Sabotage: report `owner_alive` as false whenever the owner label
    // parses; the live-owner assertion fails.
    let listed = box_list(binary, &env, &live_label);
    let live_box = listed
        .iter()
        .find(|box_| box_["name"] == live)
        .unwrap_or_else(|| panic!("the live box is missing before clean: {listed:?}"));
    assert_eq!(
        live_box["owner_alive"], true,
        "the live box reports its owner dead before clean: {live_box:?}"
    );
    assert!(
        !box_list(binary, &env, dead_label).is_empty(),
        "the dead box is missing before clean"
    );
    assert!(
        live_marker.is_file(),
        "the live project's marker is missing before clean"
    );
    assert!(
        other_marker.is_file(),
        "the other project's marker is missing before clean"
    );

    // `--dry-run` only lists: nothing it lists may disappear.
    let dry = env
        .command(binary)
        .args(["clean", "--dry-run"])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold clean --dry-run");
    assert!(
        dry.status.success(),
        "pinfold clean --dry-run failed: {}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(
        !box_list(binary, &env, dead_label).is_empty(),
        "--dry-run removed the dead box"
    );
    assert!(live_state.is_dir(), "--dry-run removed the project state");
    assert!(
        live_marker.is_file(),
        "--dry-run removed the live project's marker"
    );
    assert!(other_marker.is_file(), "--dry-run removed a project cache");

    // The real clean removes the dead box and only the dead box.
    let clean = env
        .command(binary)
        .args(["clean"])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold clean");
    assert!(
        clean.status.success(),
        "pinfold clean failed: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
    assert!(
        runtime_images()
            .expect("list images")
            .iter()
            .any(|image| image
                .names
                .iter()
                .any(|name| name.contains(unlabeled.as_str()))),
        "clean removed an unlabeled image"
    );
    assert!(
        !box_list(binary, &env, &live_label).is_empty(),
        "clean removed a live box"
    );
    assert!(
        live_state.is_dir(),
        "clean removed a project state whose checkout exists"
    );
    assert!(
        other_state.is_dir(),
        "clean removed the other project's state"
    );
    assert!(
        live_marker.is_file(),
        "clean removed a live box's project cache"
    );
    assert!(
        !other_marker.exists(),
        "clean kept a project cache with no live box"
    );
    assert!(
        box_list(binary, &env, dead_label).is_empty(),
        "clean left a box whose owner is gone"
    );
}

#[test]
fn every_build_reruns_its_steps() {
    // Every build reruns every step, so a rebuild picks up base updates
    // instead of replaying a cached `RUN` layer.
    //
    // Sabotage: drop `--no-cache` from Apple's build argv, or
    // `--layers=false` from podman's. The second build then serves `/stamp`
    // from the first build's layer, the two values match, and podman's
    // untagged-image assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("rerun");
    // The cleanup test's `clean` deletes the runtime's builder; a build
    // racing that deletion fails. Hold the same lock it does, and wait for
    // the suite's shared default image so the base is already pulled.
    let _builds = BUILDER_RACE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    default_image(binary, &env);

    let profile = format!("e2e-rerun-{}", std::process::id());
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
    // Random bytes at build time: a cached `RUN` layer replays the first
    // build's file, so the two builds agree only when the cache is reused.
    fs::write(
        &containerfile,
        b"FROM debian:trixie-slim\nRUN head -c8 /dev/urandom | od -An -tx1 > /stamp\n",
    )
    .unwrap();

    // podman's layer cache would show as untagged intermediate images. The
    // baseline is after the shared default build, so only these builds' own
    // leftovers are measured.
    let before = if cfg!(target_os = "linux") {
        Some(untagged_images())
    } else {
        None
    };

    let first = build_profile(binary, &env, &profile);
    let first_stamp = stamp_from_image(binary, &env, &first, "rerun-1");
    // Positive control: the first build ran the `RUN` step and wrote /stamp.
    assert!(
        !first_stamp.trim().is_empty(),
        "the first build left no /stamp"
    );

    let second = build_profile(binary, &env, &profile);
    let second_stamp = stamp_from_image(binary, &env, &second, "rerun-2");
    assert_ne!(
        first_stamp, second_stamp,
        "the second build reused the first build's RUN layer"
    );

    if let Some(before) = before {
        let after = untagged_images();
        assert!(
            after <= before,
            "the builds left {} untagged image(s) behind; podman held {before} before",
            after.saturating_sub(before)
        );
    }
}

/// Build `profile` and return the stable ref `pinfold build` printed.
fn build_profile(binary: &Path, env: &TestEnv, profile: &str) -> String {
    let output = env
        .command(binary)
        .args(["build", "--profile", profile])
        .output()
        .expect("run pinfold build");
    assert!(
        output.status.success(),
        "pinfold build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("build output is UTF-8")
        .trim()
        .to_string()
}

/// The `/stamp` file of the image `reference`, read through a box named
/// `name`.
fn stamp_from_image(binary: &Path, env: &TestEnv, reference: &str, name: &str) -> String {
    let name = format!("pinfold-e2e-{}-{name}", std::process::id());
    let spec = serde_json::json!({ "name": name, "image": reference });
    let up = box_up(binary, env, &spec, &name);
    let output = box_exec(binary, env, &name, &["cat", "/stamp"]);
    assert_eq!(output.code, 0, "reading /stamp failed: {}", output.stderr);
    let status = box_down(binary, env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);
    output.stdout
}

/// The number of untagged images `podman images -a` lists, including the
/// intermediate layers a cached build leaves behind. Linux only.
fn untagged_images() -> usize {
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

/// Removes the run's images from the runtime store on drop, so a failing run
/// does not leave them for the next run to count or for the operator's disk.
struct ImageCleanup {
    source: String,
}

impl Drop for ImageCleanup {
    fn drop(&mut self) {
        // Best effort: a Drop during unwinding must not panic.
        let Ok(images) = runtime_images() else {
            return;
        };
        // Every build tags the image `pinfold/profile-<source>:<build>`, and
        // the reference remains even when the label sabotage drops the source
        // label.
        let prefix = format!("pinfold/profile-{}:", self.source);
        for image in images {
            for reference in image.names {
                if reference.contains(&prefix) {
                    remove_runtime_image(&reference);
                }
            }
        }
    }
}

/// Remove one image by reference from the runtime, best effort.
fn remove_runtime_image(reference: &str) {
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

/// The image CLI this host uses: `podman` on Linux, Apple `container` on
/// macOS.
fn image_cli() -> &'static str {
    if cfg!(target_os = "linux") {
        "podman"
    } else {
        "container"
    }
}

/// One runtime image, normalized across podman and Apple `container`.
struct RuntimeImage {
    id: String,
    names: Vec<String>,
    labels: BTreeMap<String, String>,
}

/// Every image the runtime knows, from its own image list.
fn runtime_images() -> Result<Vec<RuntimeImage>, String> {
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

/// The `(digest, reference)` of every image carrying `label = value`, from
/// the runtime itself: its image list is the ground truth for what remains.
fn labeled_images(label: &str, value: &str) -> Vec<(String, String)> {
    let images = runtime_images().unwrap_or_else(|error| panic!("{error}"));
    images
        .into_iter()
        .filter(|image| image.labels.get(label).map(String::as_str) == Some(value))
        .flat_map(|image| {
            let id = image.id;
            let names = if image.names.is_empty() {
                vec![id.clone()]
            } else {
                image.names
            };
            names.into_iter().map(move |name| (id.clone(), name))
        })
        .collect()
}

/// The project state dir whose recorded root is `project`.
fn project_state_dir(env: &TestEnv, project: &Path) -> PathBuf {
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

#[test]
fn box_has_no_network_but_loopback() {
    // Sabotage: drop `--network none` from the adapter's `run` argv;
    // the box gains an interface and reaches `1.1.1.1`, so the
    // interface and unreachable assertions fail. The route to the fixture is
    // the positive control that the same box still has its one way out.
    let binary = pinfold();
    let env = TestEnv::new("network");
    let fixture = HttpFixture::start();
    let name = format!("pinfold-e2e-{}-network", std::process::id());
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "routes": { "fixture.internal": format!("127.0.0.1:{}", fixture.port()) },
        },
    });
    let up = box_up(binary, &env, &spec, &name);

    // Only loopback exists.
    let dev = box_exec(binary, &env, &name, &["cat", "/proc/net/dev"]);
    assert_eq!(dev.code, 0, "reading /proc/net/dev failed: {}", dev.stderr);
    let interfaces: Vec<&str> = dev
        .stdout
        .lines()
        .filter_map(|line| {
            let (name, _) = line.split_once(':')?;
            let name = name.trim();
            (!name.is_empty() && !name.contains('|')).then_some(name)
        })
        .collect();
    assert_eq!(interfaces, ["lo"], "the box has a network: {}", dev.stdout);

    // Named addresses fail at once with the kernel's reason, not a timeout.
    // The short --max-time turns a hang into a failure; -v carries the
    // kernel's reason, which curl's summary line omits.
    let public = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "-v",
            "--max-time",
            "5",
            "--noproxy",
            "*",
            "http://1.1.1.1/",
        ],
    );
    assert_eq!(public.code, 7, "1.1.1.1 answered: {}", public.stdout);
    assert!(
        public.stderr.contains("Network is unreachable"),
        "1.1.1.1 failed for another reason: {}",
        public.stderr
    );

    // An address on the host's network is unreachable too; with --network
    // none the box has no route to it.
    let gateway = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "-v",
            "--max-time",
            "5",
            "--noproxy",
            "*",
            "http://192.168.64.1/",
        ],
    );
    assert_eq!(
        gateway.code, 7,
        "the vmnet gateway answered: {}",
        gateway.stdout
    );
    assert!(
        gateway.stderr.contains("Network is unreachable"),
        "the vmnet gateway failed for another reason: {}",
        gateway.stderr
    );

    let dns = box_exec(
        binary,
        &env,
        &name,
        &["bash", "-c", "exec 3<>/dev/tcp/100.100.100.100/53"],
    );
    assert_eq!(dns.code, 1, "100.100.100.100:53 answered");
    assert!(
        dns.stderr.contains("Network is unreachable"),
        "100.100.100.100:53 failed for another reason: {}",
        dns.stderr
    );

    // Positive control: the route answers through the proxy while the box
    // has no network.
    let route = box_exec(
        binary,
        &env,
        &name,
        &["curl", "-sS", "--max-time", "5", "http://fixture.internal/"],
    );
    assert_eq!(route.code, 0, "the route failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "the route answered: {}",
        route.stdout
    );
    assert_eq!(
        fixture.requests(),
        1,
        "the fixture did not answer the route"
    );

    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);
}

#[test]
fn a_route_reaches_exactly_one_host_service() {
    // Sabotage: forward the client's Host header unchanged; the evil-Host
    // request then reaches the fixture as evil.example and its assertion
    // fails. The route reaches the fixture through the proxy; the fixture's
    // loopback port is not reachable from the box without it.
    let binary = pinfold();
    let env = TestEnv::new("route");
    let fixture = HttpFixture::start();
    let name = format!("pinfold-e2e-{}-route", std::process::id());
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "routes": { "fixture.internal": format!("127.0.0.1:{}", fixture.port()) },
        },
    });
    let up = box_up(binary, &env, &spec, &name);

    // The route reaches the fixture.
    let route = box_exec(
        binary,
        &env,
        &name,
        &["curl", "-sS", "--max-time", "5", "http://fixture.internal/"],
    );
    assert_eq!(route.code, 0, "the route failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "the route answered: {}",
        route.stdout
    );

    // The Host header is rewritten from the absolute-form target, not passed
    // through from the client.
    let rewritten = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "--max-time",
            "5",
            "-H",
            "Host: evil.example",
            "http://fixture.internal/",
        ],
    );
    assert_eq!(rewritten.code, 0, "the route failed: {}", rewritten.stderr);
    assert!(
        rewritten.stdout.contains("fixture host=fixture.internal"),
        "the Host header was not rewritten: {}",
        rewritten.stdout
    );

    // The fixture's loopback port is unreachable without the route: the
    // box's own loopback has no listener.
    let direct = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "-v",
            "--max-time",
            "5",
            "--noproxy",
            "*",
            &format!("http://127.0.0.1:{}/", fixture.port()),
        ],
    );
    assert_eq!(
        direct.code, 7,
        "the fixture answered directly: {}",
        direct.stdout
    );
    assert!(
        direct.stderr.contains("Connection refused"),
        "the direct request failed for another reason: {}",
        direct.stderr
    );
    assert_eq!(
        fixture.requests(),
        2,
        "the fixture answered a direct request"
    );

    // Every decision is logged.
    let lines = egress_log_lines(&env, &name);
    assert!(
        lines.iter().any(|line| line["host"] == "fixture.internal"
            && line["decision"] == "allowed"
            && line["reason"] == "route"),
        "no route decision: {lines:?}"
    );

    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);
}

#[test]
fn no_egress_means_no_way_out() {
    // Sabotage: start the proxy and relay even without `egress` in the spec;
    // the explicit-proxy request then succeeds, the fixture answers, and the
    // refusal assertions fail. The same request with the route in the spec is
    // the positive control.
    let binary = pinfold();
    let env = TestEnv::new("no-egress");
    let fixture = HttpFixture::start();
    let name = format!("pinfold-e2e-{}-no-egress", std::process::id());
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
    });
    let up = box_up(binary, &env, &spec, &name);

    // No proxy variables and no relay to use them.
    for variable in ["http_proxy", "HTTPS_PROXY"] {
        let found = box_exec(binary, &env, &name, &["printenv", variable]);
        assert_ne!(
            found.code, 0,
            "{variable} is set without egress: {}",
            found.stdout
        );
    }
    let proxied = box_exec(
        binary,
        &env,
        &name,
        &[
            "curl",
            "-sS",
            "-v",
            "--max-time",
            "5",
            "-x",
            "http://127.0.0.1:3128",
            "http://fixture.internal/",
        ],
    );
    assert_eq!(proxied.code, 7, "the box had a proxy: {}", proxied.stdout);
    assert!(
        proxied.stderr.contains("Connection refused"),
        "the proxied request failed for another reason: {}",
        proxied.stderr
    );

    // A route name is nothing without egress.
    let direct = box_exec(
        binary,
        &env,
        &name,
        &["curl", "-sS", "--max-time", "5", "http://fixture.internal/"],
    );
    assert_eq!(direct.code, 6, "the box reached a route: {}", direct.stdout);
    assert!(
        direct.stderr.contains("Could not resolve host"),
        "the direct request failed for another reason: {}",
        direct.stderr
    );
    assert_eq!(
        fixture.requests(),
        0,
        "the fixture answered a no-egress box"
    );

    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);

    // Positive control: the same request with the route in the spec answers.
    let name = format!("pinfold-e2e-{}-no-egress-control", std::process::id());
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "routes": { "fixture.internal": format!("127.0.0.1:{}", fixture.port()) },
        },
    });
    let up = box_up(binary, &env, &spec, &name);
    let route = box_exec(
        binary,
        &env,
        &name,
        &["curl", "-sS", "--max-time", "5", "http://fixture.internal/"],
    );
    assert_eq!(route.code, 0, "the control route failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "the control route answered: {}",
        route.stdout
    );
    assert_eq!(
        fixture.requests(),
        1,
        "the fixture did not answer the control"
    );

    let status = box_down(binary, &env, &name);
    assert!(status.success(), "box down failed: {status}");
    drop(up);
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
    /// The parsed `ready` line.
    ready: serde_json::Value,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
}

impl Up {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn wait(&mut self) -> ExitStatus {
        self.child.wait().expect("wait for box up")
    }

    fn kill(&mut self) {
        self.child.kill().expect("kill box up");
    }

    /// Close `up`'s stdin and read the rest of its stdout. `up` ends its
    /// stream once teardown is done, so reading to EOF waits for the `down`
    /// line without a sleep.
    fn close_stdin(&mut self) -> Vec<serde_json::Value> {
        drop(self.stdin.take());
        let stdout = self.stdout.take().expect("box up stdout");
        stdout
            .lines()
            .map(|line| {
                serde_json::from_str(&line.expect("read box up output"))
                    .expect("box up line is JSON")
            })
            .collect()
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

    Up {
        binary: binary.to_path_buf(),
        state: env.state.clone(),
        cache: env.cache.clone(),
        name: name.to_string(),
        ready,
        child,
        stdin: Some(stdin),
        stdout: Some(reader),
    }
}

/// Run `box up` with a spec and return its exit code and first stdout line.
/// For an `up` that refuses before it holds; close the spec stdin so the
/// child cannot park.
fn box_up_refused(
    binary: &Path,
    env: &TestEnv,
    spec: &serde_json::Value,
) -> (i32, serde_json::Value) {
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
    drop(stdin);

    let output = child.wait_with_output().expect("wait for box up");
    let stdout = String::from_utf8(output.stdout).expect("box up output is UTF-8");
    let line = stdout.lines().next().expect("a refused up prints a line");
    (
        exit_code(output.status),
        serde_json::from_str(line).expect("refusal line is JSON"),
    )
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

fn box_prune(binary: &Path, env: &TestEnv) -> Output {
    env.command(binary)
        .args(["box", "prune"])
        .stdin(Stdio::null())
        .output()
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
