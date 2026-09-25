//! End-to-end tests for the guarantees in docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with rootless podman. The harness builds the `pinfold` binary, builds the
//! default profile image once, and drives pinfold as a user would: the CLI,
//! environment variables and the box spec are its only seams.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use e2e::{
    HttpFixture, ImageCleanup, TestDir, TestEnv, assert_denied, assert_ok, box_exec, box_list,
    box_stat, build_profile, curl, default_image, egress_log, egress_log_lines, exit_code,
    git_init, git_status, image_cli, image_digest, image_id, image_named, json_lines,
    labeled_images, pinfold, profile_containerfile, project_id, project_state_dir, runtime_images,
    untagged_images,
};

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
    // Sabotage: drop init's SIGCHLD SIG_IGN; the orphaned `true` stays a
    // zombie of PID 1 and the no-zombie assertion fails.
    // Sabotage: skip exec's existence check; the post-down exec returns the
    // runtime's 125 instead of 3.
    // Sabotage: install `up`'s SIGTERM and SIGINT handlers after `ready`, as
    // before; the SIGTERM sent once `pid` exists lands before `ready` and
    // kills `up` by the default action, so there is no `down` line, `up`
    // ends by signal 15 instead of exit 0, and the box stays listed.
    // Sabotage: make `ready` print the merged spec set (`plan.labels`) again;
    // on podman `list` also carries the image's `io.buildah.version`, so the
    // labels equality assertion fails. This one fails only on Linux: on
    // Apple the two sets already agree.
    // Sabotage: let Apple's `down` pass the runtime's stderr through again;
    // on macOS the second `down` prints the runtime's not-found error and the
    // empty-stderr assertion fails.
    // Sabotage: drop `-f` from podman's `rm` argv in `down`; podman then refuses
    // `-t`, so `down` treats the non-zero `rm` status as an error and on
    // Linux the first `box down` fails the status assertion.
    // Sabotage: find the image in `Box::up` by exact string match against
    // `list_images()` references again; the tag-less `again` box is refused
    // `image-missing` on both runtimes, so `box_up` panics on a first line
    // that is not `ready`. On Linux the box named by id is refused the same
    // way.
    // Sabotage: key the state dir by the name again; the 60-character box
    // fails at `up` with the socket-length error, so the ready read panics.
    let binary = pinfold();
    let env = TestEnv::new("lifecycle");
    // A caller's names can be long. The state dir is keyed by the name's
    // hash, so 60 characters, past the old socket-path budget, still come
    // up.
    let name = format!("{:-<60}", box_name("lifecycle"));
    let label = "dev.example.test=lifecycle";
    let image = default_image(binary, &env);
    let spec = serde_json::json!({
        "name": name,
        "image": image,
        "labels": { "dev.example.test": "lifecycle" },
    });
    let mut up = box_up(binary, &env, &spec, &name);

    // `ready` carries the owner, the box's full label set, the image's
    // identity labels included, and the image it runs.
    let default = runtime_images()
        .expect("list the runtime's images")
        .into_iter()
        .find(|known| {
            known
                .names
                .iter()
                .any(|name| name.strip_prefix("localhost/").unwrap_or(name) == image)
        })
        .expect("the runtime lists the default image");
    let image_id = default.id;
    let build = default
        .labels
        .get("dev.pinfold.build")
        .cloned()
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
    assert_eq!(
        up.ready["image"]["id"],
        image_id.as_str(),
        "ready names the wrong image id: {}",
        up.ready
    );
    assert_eq!(
        up.ready["image"]["ref"], image,
        "ready's image ref is not the spec's: {}",
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

    // A child orphaned by an exec session is reparented to init and reaped:
    // after it, no process in the box is a zombie. The image has no `ps`, so
    // the kernel's own state line is read.
    let orphan = box_exec(binary, &env, &name, &["sh", "-c", "true & exit 0"]);
    assert_eq!(orphan.code, 0, "orphaning exec failed: {}", orphan.stderr);
    let states = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "grep -Hs '^State:' /proc/[0-9]*/status; exit 0"],
    );
    assert!(
        states.stdout.contains("State:"),
        "read no process states: {}",
        states.stderr
    );
    assert!(
        !states.stdout.contains("(zombie)"),
        "a zombie outlived its exec session:\n{}",
        states.stdout
    );

    // `list` finds the box by the caller's label, with the labels `ready`
    // reported and the same image id.
    let listed = box_list(binary, &env, label);
    let line = listed
        .iter()
        .find(|box_| box_["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| panic!("list did not find {name}: {listed:?}"));
    assert_eq!(
        line["labels"], up.ready["labels"],
        "list's labels differ from ready's: {line} vs {}",
        up.ready
    );
    assert_eq!(
        line["image"]["id"],
        image_id.as_str(),
        "list names the wrong image id: {line}"
    );

    // `down` removes the box and the owner exits.
    up.down(binary, &env);
    let listed = box_list(binary, &env, label);
    assert!(
        !listed
            .iter()
            .any(|box_| box_["name"].as_str() == Some(name.as_str())),
        "box survived down: {listed:?}"
    );
    assert!(up.wait().success(), "box up did not exit cleanly");

    // `down` is idempotent: on the box already gone it exits 0 and prints
    // nothing on either stream.
    let second = env
        .command(binary)
        .args(["box", "down", &name])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold box down");
    assert!(
        second.status.success(),
        "down on an absent box failed: {}: {}",
        second.status,
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        second.stdout.is_empty(),
        "down on an absent box printed on stdout: {}",
        String::from_utf8_lossy(&second.stdout)
    );
    assert!(
        second.stderr.is_empty(),
        "down on an absent box printed on stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    // `exec` on the box `down` removed is pinfold's own absent-box failure:
    // exit 3, not the runtime's error and exit code.
    let absent = box_exec(binary, &env, &name, &["sh", "-c", "exit 0"]);
    assert_eq!(
        absent.code, 3,
        "exec on an absent box did not exit 3: {}",
        absent.stderr
    );

    // Closing stdin is `down`: the owner prints the final `down` line and
    // exits 0. This box names the image without its tag, which the runtime
    // resolves to the same image.
    let untagged = image
        .strip_suffix(":latest")
        .unwrap_or_else(|| panic!("the default image {image} is not tagged latest"));
    let untagged_spec = serde_json::json!({
        "name": name,
        "image": untagged,
        "labels": { "dev.example.test": "lifecycle" },
    });
    let mut again = box_up(binary, &env, &untagged_spec, &name);
    assert_eq!(
        again.ready["image"]["id"],
        image_id.as_str(),
        "the tag-less image resolved to another id: {}",
        again.ready
    );
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

    // podman also resolves the image by its id and by its `localhost/` name.
    // Apple's inspect resolves neither id nor digest; macOS asserts nothing
    // about these forms.
    if cfg!(target_os = "linux") {
        let localhost = format!("localhost/{image}");
        for reference in [image_id.as_str(), localhost.as_str()] {
            let spec = serde_json::json!({
                "name": name,
                "image": reference,
                "labels": { "dev.example.test": "lifecycle" },
            });
            let mut up = box_up(binary, &env, &spec, &name);
            assert_eq!(
                up.ready["image"]["id"],
                image_id.as_str(),
                "{reference} resolved to another id: {}",
                up.ready
            );
            assert_eq!(
                up.ready["labels"]["dev.pinfold.build"],
                build.as_str(),
                "{reference} lost the image's build label: {}",
                up.ready
            );
            up.close_stdin();
            assert!(up.wait().success(), "up did not exit 0 for stdin-closed");
        }
    }

    // SIGTERM before ready tears down whatever exists and still ends with
    // `down`. `up` writes its `pid` file right after its claim, so after its
    // handlers; a SIGTERM sent right after spawn could meet the default
    // action instead. State dirs are keyed by a hash of the name, so wait on
    // the owner pid the test knows rather than a path.
    let mut starting = box_up_start(binary, &env, &spec, &[]);
    let owner = starting.child.id();
    let deadline = Instant::now() + Duration::from_secs(60);
    let state = loop {
        if let Some(state) = find_box_state_dir(&env, owner) {
            break state;
        }
        assert!(Instant::now() < deadline, "up never wrote its pid");
        std::thread::yield_now();
    };
    let kill = Command::new("kill")
        .args(["-TERM", &starting.child.id().to_string()])
        .status()
        .expect("run kill");
    assert!(kill.success(), "kill failed: {kill}");
    let lines = starting.rest();
    let status = starting.child.wait().expect("wait for box up");
    assert!(
        status.success(),
        "up did not exit 0 for a SIGTERM before ready: {status}"
    );
    let (down, earlier) = lines
        .split_last()
        .unwrap_or_else(|| panic!("up printed no down line after SIGTERM"));
    assert_eq!(
        down["event"], "down",
        "the last line was not down: {lines:?}"
    );
    assert_eq!(down["box"], name);
    assert_eq!(down["reason"], "signal", "wrong down reason: {lines:?}");
    assert!(
        earlier.iter().all(|line| line["event"] == "ready"),
        "up printed more than ready and down: {lines:?}"
    );
    drop(starting.stdin);
    let listed = box_list(binary, &env, label);
    assert!(
        !listed
            .iter()
            .any(|box_| box_["name"].as_str() == Some(name.as_str())),
        "the box survived a SIGTERM before ready: {listed:?}"
    );
    assert!(
        !state.exists(),
        "a SIGTERM before ready left the state dir: {}",
        state.display()
    );
}

#[test]
fn up_refuses_before_it_creates() {
    // Guarantee 17: up refuses before it creates.
    // Sabotage: keep the name check after the state dir is created; the
    // first box's `pid` is overwritten and the second `up`'s cleanup takes
    // the first box down, so its `exec` fails.
    // Sabotage: drop `deny_unknown_fields` from `Mount`; the misspelled
    // mount spec then comes up `ready` and the refusal assertion fails.
    // Sabotage: drop the character-class check from the env-name validation,
    // keeping only the old empty-or-`=` test; the wildcard spec then comes up
    // `ready`, so the refusal assertion fails, and on podman the box's
    // environment also holds `HOSTSECRET_TOKEN=leaked`.
    // Sabotage: drop the comma and control-character check from the
    // mount-path validation; the comma path then reaches the runtime, whose
    // option parser reads the rest as mount options, so the refusal assertion
    // fails.
    // Sabotage: make the claim treat an existing state dir as success, as
    // `create_dir_all` does, and make the name checks always pass; the
    // concurrent loser is never refused `name-in-use`, and its failure path
    // removes the winner's box, so the winner's `exec` fails too.
    // Sabotage: return a failure after the claim as prose on stderr, as
    // before; the absent-mount `up` prints no line and the first-line read
    // fails.
    let binary = pinfold();
    let env = TestEnv::new("refuses");
    let name = box_name("refuses");
    let label = "dev.example.test=refuses";

    // An image that was never built is refused as data, and the refusal
    // leaves no state dir and no box.
    let missing = serde_json::json!({
        "name": name,
        "image": "pinfold-e2e-missing:latest",
        "labels": { "dev.example.test": "refuses" },
    });
    let (code, refused) = box_up_refused(binary, &env, &missing, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["box"], name);
    assert_eq!(refused["reason"], "image-missing");
    assert_left_nothing(binary, &env, &name, label, "refused");

    // A spec whose mount misspells `readonly` as `read_only` is refused as
    // data, naming the key, and leaves no state dir and no box. The positive
    // control, a spelled `readonly` that rejects writes, is guarantee 10's
    // `box_shares_files_with_the_host`.
    let misspelled = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": env.root, "guest": "/workspace", "read_only": true }],
    });
    let (code, refused) = box_up_refused(binary, &env, &misspelled, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("read_only"),
        "the refusal did not name the key: {refused}"
    );
    assert_left_nothing(binary, &env, &name, label, "refused");

    // A spec whose env name is not a POSIX name is refused as data, naming
    // the key, and leaves no state dir and no box. `HOSTSECRET_*` would make
    // podman import every `HOSTSECRET_` variable from `up`'s own
    // environment, which is the caller's. The positive control, a valid name
    // arriving in the box, is guarantee 6's `the_environment_is_exactly_the_spec`.
    let wildcard = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.example.test": "refuses" },
        "env": { "HOSTSECRET_*": "x" },
    });
    let (code, refused) =
        box_up_refused(binary, &env, &wildcard, &[("HOSTSECRET_TOKEN", "leaked")]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("HOSTSECRET_*"),
        "the refusal did not name the key: {refused}"
    );
    assert_left_nothing(binary, &env, &name, label, "refused");

    // A spec whose mount path holds a comma is refused as data, naming the
    // path, and leaves no state dir and no box: the bind value is built by
    // concatenation, so the runtime reads the rest as mount options. The
    // positive control, a plain path mounting, is guarantee 10's
    // `box_shares_files_with_the_host`.
    let comma = env.root.join("a,b");
    fs::create_dir_all(&comma).unwrap();
    let comma_mount = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": comma, "guest": "/workspace", "readonly": false }],
    });
    let (code, refused) = box_up_refused(binary, &env, &comma_mount, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("a,b"),
        "the refusal did not name the path: {refused}"
    );
    assert_left_nothing(binary, &env, &name, label, "refused");

    // A mount whose absolute host path does not exist passes validation, so
    // the runtime rejects the run after the claim. `up` removes what it made
    // and ends with one `failed` line. The positive control is the live
    // name below, which comes up `ready` from the same image.
    let absent = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": env.root.join("absent"), "guest": "/workspace", "readonly": false }],
    });
    let (code, failed) = box_up_refused(binary, &env, &absent, &[]);
    assert_eq!(code, 1, "a failed up exits 1: {failed}");
    assert_eq!(failed["event"], "failed", "not a failed line: {failed}");
    assert_eq!(failed["box"], name);
    assert!(
        !failed["detail"].as_str().unwrap_or_default().is_empty(),
        "the failed line has no detail: {failed}"
    );
    assert_left_nothing(binary, &env, &name, label, "failed");

    // A second `up` on a live name is refused without touching the first
    // box.
    let live = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.example.test": "refuses" },
    });
    let mut up = box_up(binary, &env, &live, &name);
    let (code, refused) = box_up_refused(binary, &env, &live, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["box"], name);
    assert_eq!(refused["reason"], "name-in-use");
    let ok = box_exec(binary, &env, &name, &["true"]);
    assert_ok(&ok, "exec in the first box after the refused up");

    let _ = box_down(binary, &env, &name);
    assert!(up.wait().success(), "box up did not exit cleanly");
    drop(up);

    // Two `up`s on one free name at once: the claim lets exactly one hold
    // it, and the loser touches nothing of the winner's. Both are spawned
    // before either's first line is read.
    let mut starts = [
        box_up_start(binary, &env, &live, &[]),
        box_up_start(binary, &env, &live, &[]),
    ];
    let lines = starts.each_mut().map(Starting::first_line);
    let [first, second] = starts;
    let [first_line, second_line] = lines;
    let (winner, ready, mut loser, refused) = if first_line["event"] == "ready" {
        (first, first_line, second, second_line)
    } else {
        (second, second_line, first, first_line)
    };
    assert_eq!(ready["event"], "ready", "neither up came ready: {ready}");
    assert_eq!(ready["box"], name);
    let mut winner = winner.into_up(binary, &env, &name, ready);
    assert_eq!(
        refused["event"], "refused",
        "the loser was not refused: {refused}"
    );
    assert_eq!(refused["box"], name);
    assert_eq!(refused["reason"], "name-in-use", "wrong reason: {refused}");
    drop(loser.stdin);
    let status = loser.child.wait().expect("wait for the losing up");
    assert_eq!(exit_code(status), 1, "the losing up did not exit 1");
    let ok = box_exec(binary, &env, &name, &["true"]);
    assert_ok(&ok, "exec in the winner after the losing up");
    let _ = box_down(binary, &env, &name);
    assert!(
        winner.wait().success(),
        "the winning up did not exit cleanly"
    );
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
    let name = box_name("files");

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
    assert_denied(&denied, "Read-only file system", "the /readonly write");

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

    up.down(binary, &env);
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
    let name = box_name("privileges");
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
    assert_ok(&work, "reading /proc/self/status");
    assert_eq!(
        status_field(&work.stdout, "CapBnd:"),
        "0000000000000000",
        "exec capability bound"
    );
    assert_process_ids(&work.stdout, "exec");

    // PID 1 is pinfold init, also as the host uid:gid.
    let init = box_exec(binary, &env, &name, &["cat", "/proc/1/status"]);
    assert_ok(&init, "reading /proc/1/status");
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
    assert_denied(&rootfs, "Read-only file system", "a rootfs write");

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
    assert_ok(&workspace, "writing /workspace");
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
        assert_denied(&unshare, "Operation not permitted", "unshare -U");
        // Positive control: the same box still runs a plain child process.
        let child = box_exec(binary, &env, &name, &["true"]);
        assert_ok(&child, "a plain child process");
    }

    up.down(binary, &env);
}

#[test]
fn only_allowlisted_hosts_get_through() {
    // Sabotage: make the proxy's allowlist check accept every host; example.com
    // then answers and the 403 and "not allowlisted" log assertions fail. The
    // api.github.com request is the positive control that the same path lets
    // an allowlisted host through.
    // Sabotage: drop `time` from `record`; the refusal's time is absent and
    // the window assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("egress");
    let name = box_name("egress");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": { "allow": ["api.github.com"] },
    });
    let up = box_up(binary, &env, &spec, &name);

    let allowed = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "allowlisted host");

    // The host clock in the log's own shape, so the window compares as
    // strings.
    let utc = || {
        let output = Command::new("date")
            .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
            .output()
            .expect("run date");
        String::from_utf8(output.stdout)
            .expect("date writes UTF-8")
            .trim()
            .to_owned()
    };
    let before = utc();
    let denied = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://example.com/"],
    );
    let after = utc();
    assert_denied(&denied, "403", "a request to example.com");

    // The log names the time, the host, the decision and its reason.
    let lines = egress_log_lines(&env, &name);
    assert!(
        lines
            .iter()
            .any(|line| line["host"] == "api.github.com" && line["decision"] == "allowed"),
        "no allowed decision for api.github.com: {lines:?}"
    );
    let refused = lines
        .iter()
        .find(|line| {
            line["host"] == "example.com"
                && line["decision"] == "refused"
                && line["reason"] == "not allowlisted"
        })
        .unwrap_or_else(|| panic!("no not-allowlisted refusal for example.com: {lines:?}"));
    let time = refused["time"].as_str().unwrap_or_default();
    assert!(
        time >= before.as_str() && time <= after.as_str(),
        "the refusal at {time:?} is outside {before}..{after}: {refused}"
    );

    up.down(binary, &env);
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
    // Sabotage: answer the malformed CONNECT with 400 but no record; the
    // new refusal-line assertion for it fails.
    let binary = pinfold();
    let env = TestEnv::new("tricks");
    let fixture = HttpFixture::start();
    let name = box_name("tricks");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "allow": ["api.github.com", "localhost"],
            "routes": { "fixture.internal": fixture.route() },
        },
    });
    let up = box_up(binary, &env, &spec, &name);

    // Controls: the same paths work when the trick is not played.
    let connect = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&connect, "allowlisted CONNECT");
    let plain = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "http://api.github.com/"],
    );
    assert_ok(&plain, "allowlisted plain HTTP");
    let route = curl(binary, &env, &name, "5", &["http://fixture.internal/"]);
    assert_eq!(route.code, 0, "route control failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "route control answered: {}",
        route.stdout
    );

    // An IP literal, in both request forms.
    let literal_connect = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://127.0.0.1/"],
    );
    assert_denied(&literal_connect, "403", "CONNECT to an IP literal");
    let literal_plain = curl(
        binary,
        &env,
        &name,
        "30",
        &["-f", "-o", "/dev/null", "http://127.0.0.1/"],
    );
    assert_denied(&literal_plain, "403", "plain HTTP to an IP literal");

    // A name that resolves to loopback.
    let loopback = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://localhost/"],
    );
    assert_denied(&loopback, "403", "a name resolving to loopback");

    // A ClientHello whose SNI names another host. The proxy answers the
    // CONNECT with 200 and then refuses on the ClientHello, so curl fails
    // the TLS handshake (35) rather than reading an HTTP status.
    let sni = curl(
        binary,
        &env,
        &name,
        "30",
        &[
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
    let route_connect = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://fixture.internal/"],
    );
    assert_denied(&route_connect, "403", "CONNECT to a route");

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

    // A CONNECT whose authority has no host is malformed: the proxy answers
    // 400 and, like every decision, writes one refusal line.
    let before = egress_log_lines(&env, &name);
    let malformed_connect = box_exec(
        binary,
        &env,
        &name,
        &[
            "bash",
            "-c",
            "exec 3<>/dev/tcp/127.0.0.1/3128; \
             printf 'CONNECT :443 HTTP/1.1\\r\\nHost: example.com\\r\\n\\r\\n' >&3; \
             cat <&3",
        ],
    );
    assert!(
        malformed_connect.stdout.contains("400"),
        "malformed CONNECT got no 400: {}",
        malformed_connect.stdout
    );
    let after = egress_log_lines(&env, &name);
    assert!(
        after.len() == before.len() + 1
            && after.last().is_some_and(
                |line| line["decision"] == "refused" && line["reason"] == "malformed request"
            ),
        "the malformed CONNECT left no refusal line: {after:?}"
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

    up.down(binary, &env);
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
    // instead of a refusal. Sabotage: make the owner liveness test read the
    // pid from the `pid` file and return `kill(pid, 0)` as `Ok` or `EPERM`;
    // pid 1 then counts as alive, so `list` reports `owner_alive` true and
    // `prune` skips the box, and both assertions fail. The positive control
    // is the live box in `cleanup_removes_only_pinfolds_garbage`, whose
    // `owner_alive` is true through the same test.
    let binary = pinfold();
    let env = TestEnv::new("owner-gone");
    let name = box_name("owner-gone");
    let label = "dev.example.test=owner-gone";
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "labels": { "dev.example.test": "owner-gone" },
        "egress": { "allow": ["api.github.com"] },
    });
    let mut up = box_up(binary, &env, &spec, &name);

    // Positive control: the box has egress while its owner lives.
    let allowed = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "positive control");

    // Hold the race against the cleanup test's `clean`, which removes any
    // dead pinfold box, through this test's `box prune`.
    let _race = DEAD_BOX_RACE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    // The owner is reaped only after `box prune` has reported the box. Until
    // then it is a zombie: `kill(pid, 0)` succeeds on it, so every other
    // test's state root, which judges this box by its label pid, leaves it
    // alone, while this root sees the lock released the moment the process
    // died. Reaped earlier, another test's first command runs the daily pass
    // and prunes the box before this test's `box prune` can report it.
    up.kill();

    // The positive control left its decision in the log. A live proxy
    // anywhere would log before it dials, so no new line means no proxy saw
    // the request; the request itself may hang, because Apple's forwarder
    // does not reliably close after the owner dies.
    // The log-line count is the assertion; the curl exit is only a
    // precondition, since a hang and a refusal both exit non-zero.
    let before = egress_log_lines(&env, &name);
    assert!(
        !before.is_empty(),
        "the positive control left no egress log line"
    );
    let denied = curl(
        binary,
        &env,
        &name,
        "5",
        &["-o", "/dev/null", "https://api.github.com/"],
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

    // The state dir's `pid` now names pid 1, which is alive and which this
    // test cannot signal: pid reuse and EPERM in one write. Only the lock
    // tells the owner is gone. State dirs are keyed by a hash of the name,
    // so the test finds one by the owner it records.
    let owner = up.pid();
    fs::write(
        find_box_state_dir(&env, owner)
            .expect("find the dead owner's state dir")
            .join("pid"),
        "1",
    )
    .expect("overwrite the dead owner's pid");

    // Pinfold's own liveness test reports the owner gone before prune acts:
    // the box is still listed, with `owner_alive` false.
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
    up.wait();
    let listed = box_list(binary, &env, label);
    assert!(
        !listed.iter().any(|box_| box_["name"] == name),
        "prune left the box: {listed:?}"
    );

    // The name is free again: a fresh `up` comes up `ready`, not refused
    // `name-in-use`, and goes down cleanly.
    let mut fresh = box_up(binary, &env, &spec, &name);
    fresh.down(binary, &env);
    assert!(fresh.wait().success(), "the fresh up did not exit cleanly");
}

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    // Guarantee 15: cleanup removes only pinfold's garbage.
    //
    // Sabotage: make `keep_two_images` return before it removes anything;
    // three images remain and the two-image assertion fails. Sabotage: drop
    // the in-use skip from `keep_two_images` and restore the `?` on
    // `remove_image`; on Apple build 4 deletes b2 under box A and the "b2 is
    // still listed" assertion fails, and on podman build 5 returns at the
    // refused b3 before reaching the freed b2, so the "b2 is gone" assertion
    // fails. Sabotage: drop
    // the `dev.pinfold.profile` label from the build; no image matches and
    // the count is zero. Sabotage: make `clean` remove every image instead
    // of only pinfold's own; the unlabeled image assertion fails. Sabotage:
    // drop the live-box check from `clean`; the live project's marker is
    // deleted and the marker-survives assertion fails. Sabotage: make
    // `clean` remove every project state instead of only the stale ones;
    // the other project's state assertion fails. Sabotage: make `clean`
    // remove every box; the live box assertion fails. Sabotage: make
    // `clean` skip boxes whose owner is gone; the dead box assertion fails.
    // Sabotage: set only the build's own family label, as before; build 4
    // then counts x1 and x2 as P's newest images and removes b3, so the "b3
    // is still listed" assertion fails, and x's base label carries P's own
    // base, so the base assertion fails. Sabotage: make `--unused` skip the
    // live-box check (drop `live_projects` from `CleanPlan::measure`'s
    // stale test); the live project's state goes and its assertion fails.
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
        repository: format!("pinfold/profile-{profile}"),
    };
    // `FROM scratch` keeps the test off the network and fast.
    let containerfile = profile_containerfile(&env, &profile, "FROM scratch\n");

    // Record each build's unique tag. The `:latest` tag moves along, so a
    // box pins the image it started from by this tag. After three builds b1
    // is gone; b2 and b3 remain and b2 is the next removal candidate.
    let mut builds: Vec<String> = Vec::new();
    for _ in 0..3 {
        build_profile(binary, &env, &profile);
        builds.push(built_unique_ref(&profile));
    }
    let b2 = builds[1].clone();
    let b3 = builds[2].clone();

    let images = labeled_images("dev.pinfold.profile", &profile);
    assert_eq!(
        images.len(),
        2,
        "after three builds of one source, two images should remain: {images:?}"
    );
    assert!(
        image_named(&b2) && image_named(&b3),
        "the two images left are not b2 and b3: {images:?}"
    );

    // A caller image built on the profile's `:latest`. The runtime copies
    // the base image's labels onto it, so without the explicit empty family
    // labels it would count as the profile's image in build 4's retention.
    let x_name = format!("{profile}-x");
    let _x_images = ImageCleanup {
        repository: format!("pinfold/image-{x_name}"),
    };
    let profile_ref = format!("pinfold/profile-{profile}:latest");
    let profile_digest = image_digest(&profile_ref);
    let x_context = env.root.join("x-context");
    fs::create_dir_all(&x_context).unwrap();
    let x_containerfile = x_context.join("Containerfile");
    fs::write(
        &x_containerfile,
        format!("FROM {profile_ref}\nCOPY marker.txt /marker.txt\n"),
    )
    .unwrap();
    fs::write(x_context.join("marker.txt"), "x\n").unwrap();
    let mut x_refs: Vec<String> = Vec::new();
    for i in 0..2 {
        let (code, built) = image_build(binary, &env, &x_name, &x_containerfile, &x_context);
        assert_eq!(built["event"], "built", "x build {}: {built}", i + 1);
        assert_eq!(code, 0, "x build {} exited {code}", i + 1);
        assert_eq!(
            built["labels"]["dev.pinfold.image"],
            x_name.as_str(),
            "x is not its own family: {built}"
        );
        assert_eq!(
            built["labels"]["dev.pinfold.profile"], "",
            "x carries the profile's family label: {built}"
        );
        // The base is the runtime's digest of the profile ref: x's own,
        // never the one the profile carries.
        assert_eq!(
            built["labels"]["dev.pinfold.base"],
            profile_digest.as_str(),
            "x build {} does not carry the profile image's digest as its base: {built}",
            i + 1
        );
        assert_eq!(
            built["base"],
            built["labels"]["dev.pinfold.base"],
            "x build {} base is not the recorded base label: {built}",
            i + 1
        );
        x_refs.push(
            built["ref"]
                .as_str()
                .unwrap_or_else(|| panic!("x build {} carries no ref: {built}", i + 1))
                .to_string(),
        );
    }

    // Box A pins b2, the older of the two, so the next build's retention
    // meets a pinned image first. `FROM scratch` has no program for `exec`,
    // so `box list` is how the test sees the box survive.
    let pinned_label = "dev.example.test=cleanup-pinned";
    let box_a = box_name("cleanup-a");
    let pin_a = serde_json::json!({
        "name": box_a,
        "image": b2.as_str(),
        "labels": { "dev.example.test": "cleanup-pinned" },
    });
    let mut up_a = box_up(binary, &env, &pin_a, &box_a);

    // Build 4: b2 cannot go while box A holds it. The build still exits 0
    // and its one maintenance line names b2.
    let output = build_profile(binary, &env, &profile);
    builds.push(built_unique_ref(&profile));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        image_named(&b2),
        "the image box A pins is gone after build 4"
    );
    assert!(
        stderr.contains(&b2),
        "build 4's maintenance line did not name the pinned b2: {stderr}"
    );
    // Build 4 counts only the profile's own images, so the caller's two
    // survive and b3 remains the profile's older rollback image.
    assert!(
        x_refs.iter().all(|reference| image_named(reference)),
        "build 4 removed a caller image built on the profile: {x_refs:?}"
    );
    assert!(
        image_named(&b3),
        "build 4 removed the profile's b3 for its caller images"
    );

    // Box B pins b3, then box A goes down and frees b2. Build 5's retention
    // meets pinned b3 first and must still remove the free b2 behind it.
    let box_b = box_name("cleanup-b");
    let pin_b = serde_json::json!({
        "name": box_b,
        "image": b3.as_str(),
        "labels": { "dev.example.test": "cleanup-pinned" },
    });
    let mut up_b = box_up(binary, &env, &pin_b, &box_b);
    up_a.down(binary, &env);
    assert!(up_a.wait().success(), "box A's up did not exit cleanly");

    let output = build_profile(binary, &env, &profile);
    builds.push(built_unique_ref(&profile));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !image_named(&b2),
        "build 5 kept the freed b2 because the pinned b3 failed first"
    );
    assert!(image_named(&b3), "build 5 removed the pinned b3");
    assert!(
        stderr.contains(&b3),
        "build 5's maintenance line did not name the pinned b3: {stderr}"
    );
    assert!(
        !stderr.contains(&b2),
        "build 5's maintenance line named the freed b2: {stderr}"
    );
    let listed = box_list(binary, &env, pinned_label);
    let pinned = listed
        .iter()
        .find(|box_| box_["name"] == box_b)
        .unwrap_or_else(|| panic!("box B is not listed after build 5: {listed:?}"));
    assert_eq!(
        pinned["state"], "running",
        "box B is not running after build 5: {pinned:?}"
    );
    assert_eq!(
        pinned["owner_alive"], true,
        "box B's owner is reported dead after build 5: {pinned:?}"
    );

    // Box B goes down before the rest of the test, which starts its own
    // boxes from the profile's `:latest`.
    up_b.down(binary, &env);
    assert!(up_b.wait().success(), "box B's up did not exit cleanly");

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
        image_named(&unlabeled),
        "the unlabeled image is missing before clean"
    );

    // A project's state: a refused `pinfold pi` creates it before it names
    // the missing image. The profile is never built, so this part downloads
    // no pi artifact. Two projects get a cache marker: one runs a live box,
    // the other does not.
    let missing = format!("{profile}-missing");
    profile_containerfile(&env, &missing, "FROM scratch\n");
    let seed_project = |project: &TestDir, text: &[u8]| {
        let refused = env
            .command(binary)
            .args(["pi", "--version"])
            .env("PINFOLD_PROFILE", &missing)
            .current_dir(project.path())
            .stdin(Stdio::null())
            .output()
            .expect("run pinfold pi");
        assert!(!refused.status.success(), "pi ran a profile with no image");
        let marker = project_state_dir(&env, project.path())
            .join("home")
            .join(".cache")
            .join("marker");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, text).unwrap();
        marker
    };

    let live_project = TestDir::new(&env, "live-project");
    let live_marker = seed_project(&live_project, b"live\n");
    let live_state = project_state_dir(&env, live_project.path());
    let live_id = project_id(&env, live_project.path());

    let other_project = TestDir::new(&env, "other-project");
    let other_marker = seed_project(&other_project, b"other\n");
    let other_state = project_state_dir(&env, other_project.path());

    // A live pinfold box for the live project, and a dead box that names
    // no project.
    let live_label = format!("dev.pinfold.project={live_id}");
    let live = box_name("cleanup-live");
    let live_spec = serde_json::json!({
        "name": live,
        "image": format!("pinfold/profile-{profile}:latest"),
        "labels": { "dev.pinfold.project": live_id },
    });
    let _live = box_up(binary, &env, &live_spec, &live);

    let dead_label = "dev.pinfold.project=e2e-cleanup-dead";
    let dead = box_name("cleanup-dead");
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
    assert!(image_named(&unlabeled), "clean removed an unlabeled image");
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

    // `--unused AGE` removes the state of projects not run for that long,
    // except one with a live box. Both projects last ran seconds ago, so
    // `0s` ages them out and only the live one survives.
    let unused = env
        .command(binary)
        .args(["clean", "--unused", "0s"])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold clean --unused");
    assert!(
        unused.status.success(),
        "pinfold clean --unused failed: {}",
        String::from_utf8_lossy(&unused.stderr)
    );
    assert!(
        !other_state.exists(),
        "clean --unused kept the state of a project that has not run"
    );
    assert!(
        live_state.is_dir(),
        "clean --unused removed the state of a project with a live box"
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
        repository: format!("pinfold/profile-{profile}"),
    };
    // Random bytes at build time: a cached `RUN` layer replays the first
    // build's file, so the two builds agree only when the cache is reused.
    profile_containerfile(
        &env,
        &profile,
        "FROM debian:trixie-slim\nRUN head -c8 /dev/urandom | od -An -tx1 > /stamp\n",
    );

    // podman's layer cache would show as untagged intermediate images. The
    // baseline is after the shared default build, so only these builds' own
    // leftovers are measured.
    let before = if cfg!(target_os = "linux") {
        Some(untagged_images())
    } else {
        None
    };

    build_profile(binary, &env, &profile);
    let first = built_unique_ref(&profile);
    let first_stamp = file_from_image(binary, &env, &first, "rerun-1", "/stamp");
    // Positive control: the first build ran the `RUN` step and wrote /stamp.
    assert!(
        !first_stamp.trim().is_empty(),
        "the first build left no /stamp"
    );

    build_profile(binary, &env, &profile);
    let second = built_unique_ref(&profile);
    let second_stamp = file_from_image(binary, &env, &second, "rerun-2", "/stamp");
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

#[test]
fn a_caller_builds_an_image_from_its_own_tree() {
    // Guarantee 23: a caller builds an image from its own tree.
    //
    // Sabotage: tag the build but skip the `--context` argument, so the
    // runtime gets no context holding marker.txt; the COPY fails and the
    // first `built` assertion fails. Sabotage: set the caller image window
    // to zero; the third build removes the first build's ref, so its
    // existence assertion fails and the ref's `up` is refused
    // `image-missing`.
    let binary = pinfold();
    let env = TestEnv::new("image-build");
    // The cleanup test's `clean` deletes the runtime's builder; a build
    // racing that deletion fails. Hold the same lock it does, and wait for
    // the suite's shared default image, which this image builds on.
    let _builds = BUILDER_RACE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let base = default_image(binary, &env);

    let name = format!("pinfold-e2e-{}", std::process::id());
    let repository = format!("pinfold/image-{name}");
    let latest = format!("{repository}:latest");
    let _images = ImageCleanup {
        repository: repository.clone(),
    };
    // The Containerfile lies outside the context, as a caller's may.
    let context = env.root.join("image-context");
    fs::create_dir_all(&context).unwrap();
    let containerfile = env.root.join("Containerfile");
    fs::write(
        &containerfile,
        format!("FROM {base}\nCOPY marker.txt /marker.txt\n"),
    )
    .unwrap();

    let mut refs: Vec<String> = Vec::new();
    for marker in ["first", "second", "third"] {
        fs::write(context.join("marker.txt"), format!("{marker}\n")).unwrap();
        let (code, built) = image_build(binary, &env, &name, &containerfile, &context);
        assert_eq!(built["event"], "built", "the {marker} build: {built}");
        assert_eq!(code, 0, "the {marker} build exited {code}");
        assert_eq!(built["image"], name.as_str(), "built named another image");
        let reference = built["ref"]
            .as_str()
            .unwrap_or_else(|| panic!("built carries no ref: {built}"))
            .to_string();
        let build = reference
            .strip_prefix(&format!("{repository}:"))
            .unwrap_or_else(|| panic!("ref {reference} is not {repository}:<build>"));
        assert_eq!(built["latest"], latest.as_str(), "built: {built}");
        assert_eq!(built["labels"]["dev.pinfold.image"], name.as_str());
        assert_eq!(built["labels"]["dev.pinfold.build"], build);
        assert_eq!(built["labels"]["dev.example.test"], "image");
        assert_eq!(
            built["base"], built["labels"]["dev.pinfold.base"],
            "base is not the recorded base label: {built}"
        );
        if refs.is_empty() {
            // The COPYed file reaches a box started from the unique ref.
            let read = file_from_image(binary, &env, &reference, "image", "/marker.txt");
            assert_eq!(read, "first\n", "the box read another marker");
        }
        assert_eq!(
            image_id(&latest),
            image_id(&reference),
            "{latest} does not name the {marker} build"
        );
        refs.push(reference);
    }

    let names_ids = || labeled_images("dev.pinfold.image", &name);
    let kept = names_ids();
    // A caller's `built` ref is a handle for later, so later builds of the
    // name do not reclaim it: the first build's ref still names an image.
    assert!(
        image_named(&refs[0]),
        "the first build's ref is gone after three builds: {refs:?}"
    );
    // And it still comes up, as the caller's later `box up` uses it.
    let again = box_name("image-again");
    let spec = serde_json::json!({ "name": again, "image": refs[0] });
    let mut up = box_up(binary, &env, &spec, &again);
    up.down(binary, &env);
    assert!(
        up.wait().success(),
        "the first ref's box up did not exit cleanly"
    );

    // A failed build carries its log and makes no image.
    let failing = env.root.join("Containerfile.fail");
    fs::write(&failing, format!("FROM {base}\nRUN false\n")).unwrap();
    let (code, failed) = image_build(binary, &env, &name, &failing, &context);
    assert_eq!(failed["event"], "failed", "a failing build: {failed}");
    assert_eq!(code, 1, "a failed build exited {code}");
    assert_eq!(failed["image"], name.as_str(), "failed named another image");
    assert!(
        failed["log"].as_array().is_some_and(|log| !log.is_empty()),
        "the failed line carries no log: {failed}"
    );
    assert_eq!(
        names_ids(),
        kept,
        "the failed build changed the name's images"
    );
    assert_eq!(
        image_id(&latest),
        image_id(&refs[2]),
        "the failed build moved {latest}"
    );
}

/// Run `pinfold image build NAME` with the caller label `dev.example.test`,
/// and return its exit code and its one stdout line, parsed.
fn image_build(
    binary: &Path,
    env: &TestEnv,
    name: &str,
    containerfile: &Path,
    context: &Path,
) -> (i32, serde_json::Value) {
    let output = env
        .command(binary)
        .args(["image", "build", name, "--containerfile"])
        .arg(containerfile)
        .arg("--context")
        .arg(context)
        .args(["--label", "dev.example.test=image"])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold image build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "image build printed {} stdout lines: {stdout}\nstderr: {}",
        lines.len(),
        String::from_utf8_lossy(&output.stderr)
    );
    let line = serde_json::from_str(lines[0]).expect("the image build line is JSON");
    (exit_code(output.status), line)
}

/// The file at `path` in the image `reference`, read through a box named
/// `name`.
fn file_from_image(
    binary: &Path,
    env: &TestEnv,
    reference: &str,
    name: &str,
    path: &str,
) -> String {
    let name = box_name(name);
    let spec = serde_json::json!({ "name": name, "image": reference });
    let up = box_up(binary, env, &spec, &name);
    let output = box_exec(binary, env, &name, &["cat", path]);
    assert_eq!(output.code, 0, "reading {path} failed: {}", output.stderr);
    up.down(binary, env);
    output.stdout
}

/// The unique tag of the image that `pinfold/profile-<profile>:latest` names
/// now, with podman's `localhost/` prefix stripped. Each build tags its image
/// `:latest` and one unique tag; the stable tag moves to every new build, so
/// a box pins an image by the unique tag. The unique tag is collected from
/// every runtime image entry sharing the `:latest` image's id, because a
/// runtime may list one entry per tag.
fn built_unique_ref(profile: &str) -> String {
    let latest = format!("pinfold/profile-{profile}:latest");
    let id = image_id(&latest).unwrap_or_else(|| panic!("no image names {latest}"));
    runtime_images()
        .expect("list the runtime's images")
        .iter()
        .filter(|image| image.id == id)
        .flat_map(|image| image.names.iter())
        .find_map(|name| {
            let name = name.strip_prefix("localhost/").unwrap_or(name);
            (name != latest).then(|| name.to_string())
        })
        .unwrap_or_else(|| panic!("the image {latest} names has no unique tag"))
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
    let name = box_name("network");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "routes": { "fixture.internal": fixture.route() },
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
    let public = curl(
        binary,
        &env,
        &name,
        "5",
        &["-v", "--noproxy", "*", "http://1.1.1.1/"],
    );
    assert_eq!(public.code, 7, "1.1.1.1 answered: {}", public.stdout);
    assert!(
        public.stderr.contains("Network is unreachable"),
        "1.1.1.1 failed for another reason: {}",
        public.stderr
    );

    // An address on the host's network is unreachable too; with --network
    // none the box has no route to it.
    let gateway = curl(
        binary,
        &env,
        &name,
        "5",
        &["-v", "--noproxy", "*", "http://192.168.64.1/"],
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
    let route = curl(binary, &env, &name, "5", &["http://fixture.internal/"]);
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

    up.down(binary, &env);
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
    let name = box_name("route");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "routes": { "fixture.internal": fixture.route() },
        },
    });
    let up = box_up(binary, &env, &spec, &name);

    // The route reaches the fixture.
    let route = curl(binary, &env, &name, "5", &["http://fixture.internal/"]);
    assert_eq!(route.code, 0, "the route failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "the route answered: {}",
        route.stdout
    );

    // The Host header is rewritten from the absolute-form target, not passed
    // through from the client.
    let rewritten = curl(
        binary,
        &env,
        &name,
        "5",
        &["-H", "Host: evil.example", "http://fixture.internal/"],
    );
    assert_eq!(rewritten.code, 0, "the route failed: {}", rewritten.stderr);
    assert!(
        rewritten.stdout.contains("fixture host=fixture.internal"),
        "the Host header was not rewritten: {}",
        rewritten.stdout
    );

    // The fixture's loopback port is unreachable without the route: the
    // box's own loopback has no listener.
    let direct = curl(
        binary,
        &env,
        &name,
        "5",
        &[
            "-v",
            "--noproxy",
            "*",
            &format!("http://{}/", fixture.route()),
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

    up.down(binary, &env);
}

#[test]
fn an_injecting_route_keeps_the_credential_on_the_host() {
    // Guarantee 21. Sabotage: pass the header value through the box's
    // environment as well (spec env `ROUTE_KEY: {from:
    // PINFOLD_E2E_ROUTE_KEY}`); the environment assertion fails. The value
    // lives only in `up`'s environment; the box sends its own Authorization,
    // which the proxy replaces.
    let binary = pinfold();
    let env = TestEnv::new("inject");
    let fixture = HttpFixture::start();
    let secret = format!("pf-secret-{}", std::process::id());
    let name = box_name("inject");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "routes": {
                "key.internal": {
                    "to": format!("http://{}", fixture.route()),
                    "headers": {
                        "Authorization": { "from": "PINFOLD_E2E_ROUTE_KEY", "prefix": "Bearer " },
                    },
                },
                "gh": { "to": "https://api.github.com" },
            },
        },
    });
    let up = box_up_with_env(
        binary,
        &env,
        &spec,
        &name,
        &[("PINFOLD_E2E_ROUTE_KEY", &secret)],
    );

    // The fixture receives the header from the host, not the box's own.
    let route = curl(
        binary,
        &env,
        &name,
        "5",
        &[
            "-H",
            "Authorization: Bearer from-box",
            "http://key.internal/",
        ],
    );
    assert_eq!(route.code, 0, "the route failed: {}", route.stderr);
    assert!(
        route
            .stdout
            .contains(&format!("fixture host={}", fixture.route())),
        "the route answered: {}",
        route.stdout
    );
    let requests = fixture.headers();
    assert_eq!(requests.len(), 1, "the fixture saw {requests:?}");
    let authorization: Vec<&str> = requests[0]
        .iter()
        .filter(|(header, _)| header.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_str())
        .collect();
    assert_eq!(
        authorization,
        [format!("Bearer {secret}")],
        "the fixture's Authorization headers"
    );

    // The box's environment never holds the value.
    let environment = box_exec(binary, &env, &name, &["env"]);
    assert_eq!(environment.code, 0, "env failed: {}", environment.stderr);
    assert!(
        !environment.stdout.contains(&secret),
        "the box's environment holds the value"
    );

    // An https route reaches api.github.com over TLS. GitHub's answer is the
    // proof, whatever its status: its runners are sometimes rate-limited to
    // a 403, and a failed TLS dial would be the proxy's 502 with no GitHub
    // header at all.
    let github = curl(
        binary,
        &env,
        &name,
        "20",
        &["-o", "/dev/null", "-D", "-", "http://gh/"],
    );
    assert_eq!(github.code, 0, "the https route failed: {}", github.stderr);
    assert!(
        github
            .stdout
            .to_ascii_lowercase()
            .contains("x-github-request-id"),
        "the https route did not reach GitHub: {}",
        github.stdout
    );

    // The log names the routes and never the value.
    let log = fs::read_to_string(egress_log(&env, &name)).expect("read egress log");
    assert!(!log.contains(&secret), "the egress log holds the value");
    let lines = egress_log_lines(&env, &name);
    for route in ["key.internal", "gh"] {
        assert!(
            lines.iter().any(|line| line["host"] == route
                && line["decision"] == "allowed"
                && line["reason"] == "route"),
            "no route decision for {route}: {lines:?}"
        );
    }

    up.down(binary, &env);
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
    let name = box_name("no-egress");
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
    let proxied = curl(
        binary,
        &env,
        &name,
        "5",
        &[
            "-v",
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

    // A route name is nothing without egress. The DNS error is the
    // guaranteed behavior here, not a pass by failure: no egress means no
    // resolver.
    let direct = curl(binary, &env, &name, "5", &["http://fixture.internal/"]);
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

    up.down(binary, &env);
    drop(up);

    // Positive control: the same request with the route in the spec answers.
    let name = box_name("no-egress-control");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "egress": {
            "routes": { "fixture.internal": fixture.route() },
        },
    });
    let up = box_up(binary, &env, &spec, &name);
    let route = curl(binary, &env, &name, "5", &["http://fixture.internal/"]);
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

    up.down(binary, &env);
}

#[test]
fn a_caller_can_tell_an_oom_kill_from_a_failure() {
    // Guarantee 20: a caller can tell an OOM kill from a failure.
    // Sabotage: read `memory.events` but report `high` instead of `oom_kill`;
    // with no memory.high set the count stays 0 and the post-exec assertion
    // fails.
    // Sabotage: skip the oom_score_adj write in init's exec wrapper; `cat`
    // then prints 0 and the oom_score_adj assertion fails. (The original
    // flake is not usable: it needs a kernel that picks init, which the
    // macOS host and Debian do not.)
    let binary = pinfold();
    let env = TestEnv::new("oom");
    let name = box_name("oom");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(binary, &env),
        "memory": "256M",
    });
    let up = box_up(binary, &env, &spec, &name);

    // Every key is present, and the limit is the one in force: 256 MiB. A
    // field the runtime cannot answer is null, not absent.
    let before = box_stat(binary, &env, &name);
    assert_eq!(before["box"], name);
    assert_eq!(
        before["memory"]["limit"].as_u64(),
        Some(256 * 1024 * 1024),
        "stat: {before}"
    );
    for key in ["box", "oom_kills", "memory", "pids"] {
        assert!(before.get(key).is_some(), "stat omitted {key}: {before}");
    }
    for key in ["current", "peak", "limit"] {
        assert!(
            before["memory"].get(key).is_some(),
            "stat omitted memory.{key}: {before}"
        );
    }
    for key in ["current", "limit"] {
        assert!(
            before["pids"].get(key).is_some(),
            "stat omitted pids.{key}: {before}"
        );
    }

    // Every process `exec` starts runs through the box's init, which raises
    // its oom_score_adj to 1000 before exec'ing the command. The kernel's
    // OOM killer then takes a box process before init, so the box survives
    // the hog on every kernel.
    let score = box_exec(binary, &env, &name, &["cat", "/proc/self/oom_score_adj"]);
    assert_ok(&score, "cat /proc/self/oom_score_adj");
    assert_eq!(
        score.stdout.trim(),
        "1000",
        "exec'd process oom_score_adj: {}",
        score.stdout
    );

    if cfg!(target_os = "linux") {
        // A fresh box's cgroup has killed nothing; the caller's baseline.
        assert_eq!(before["oom_kills"].as_u64(), Some(0), "stat: {before}");
        // A command that allocates past the limit is killed; the box stays
        // up and the count rises. The command's non-zero exit must be read
        // together with stat, not as a failure.
        let killed = box_exec(
            binary,
            &env,
            &name,
            &["sh", "-c", "head -c 1G /dev/zero | tail"],
        );
        assert_ne!(killed.code, 0, "the memory hog exited 0");
        let after = box_stat(binary, &env, &name);
        let kills = after["oom_kills"]
            .as_u64()
            .unwrap_or_else(|| panic!("stat lost oom_kills: {after}"));
        assert!(kills >= 1, "stat reports no OOM kill after one: {after}");
    } else {
        // The VM cannot see kills or use; the field is present and null, so
        // a caller can tell "cannot know" from "no kill".
        assert!(before["oom_kills"].is_null(), "stat: {before}");
        assert!(before["memory"]["current"].is_null(), "stat: {before}");
        assert!(before["memory"]["peak"].is_null(), "stat: {before}");
        assert!(before["pids"]["current"].is_null(), "stat: {before}");
        assert!(before["pids"]["limit"].is_null(), "stat: {before}");
    }

    // An absent box is pinfold's own failure, exit 3, like exec.
    up.down(binary, &env);
    let absent = env
        .command(binary)
        .args(["box", "stat", &name])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold box stat");
    assert_eq!(
        exit_code(absent.status),
        3,
        "stat on an absent box did not exit 3: {}",
        String::from_utf8_lossy(&absent.stderr)
    );

    drop(up);
}

#[test]
fn a_caller_owned_box_launches_the_pinned_harness() {
    // Guarantee 19: a caller-owned box launches the pinned harness.
    // Sabotage: drop the harness mount (or mount the wrong directory); the
    // `/opt/pinfold/pi/pi --version` assertion fails. Sabotage: mount pi but
    // leave `PINFOLD_ALLOW` unset; the second assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("harness");
    let name = box_name("harness");
    let home = TestDir::new(&env, "home");
    default_image(binary, &env);

    // The version `pinfold artifacts` pins. The caller records which pi it
    // ran from the same report.
    let output = env
        .command(binary)
        .arg("artifacts")
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold artifacts");
    assert!(
        output.status.success(),
        "pinfold artifacts failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pins: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("pinfold artifacts output is JSON");
    let pi = pins
        .iter()
        .find(|pin| pin["name"] == "pi")
        .unwrap_or_else(|| panic!("pinfold artifacts does not name pi: {pins:?}"));
    let version = pi["version"].as_str().expect("pin version is a string");

    // A caller-owned box: the spec names the harness and the allow list; the
    // profile supplies the image and home seeds.
    let spec = serde_json::json!({
        "name": name,
        "profile": "default",
        "harness": "pi",
        "mounts": [{ "host": home.path(), "guest": "/home/harness" }],
        "env": { "HOME": "/home/harness" },
        "egress": { "allow": ["api.github.com"] },
    });
    let mut up = box_up(binary, &env, &spec, &name);

    // The pinned pi is in the box and runs at the pinned version.
    let ran = box_exec(binary, &env, &name, &["/opt/pinfold/pi/pi", "--version"]);
    assert_ok(&ran, "/opt/pinfold/pi/pi --version");
    assert!(
        ran.stdout.contains(version),
        "the box's pi is not the pinned {version}: {}",
        ran.stdout
    );

    // The harness environment is the spec's allow list, exactly.
    let allow = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$PINFOLD_ALLOW\""],
    );
    assert_eq!(
        allow.stdout, "api.github.com",
        "the box's PINFOLD_ALLOW is not the spec's allow list"
    );

    up.down(binary, &env);
    assert!(up.wait().success(), "box up did not exit cleanly");
}

#[test]
fn a_caller_owned_box_cannot_write_git() {
    // Guarantee 22: a caller-owned box cannot write `.git`.
    // Sabotage: mount `.git` writable; the hook, commit and rename
    // assertions fail.
    let binary = pinfold();
    let env = TestEnv::new("caller-git");
    let name = box_name("caller-git");
    let image = default_image(binary, &env);
    let repo = TestDir::new(&env, "repo");
    let root = repo.path().to_string_lossy().into_owned();
    let dot_git = repo.path().join(".git");
    // On Apple the top directory of every mount is root:root inside the
    // box while the files under it keep the host ids: `stat` printed 0:0
    // for REPO and REPO/.git and 501:20 for REPO/f.txt, so git refuses any
    // mounted repository until `safe.directory` names it.
    let safe = format!("safe.directory={root}");

    // One host commit, so the box has history to read.
    git_init(repo.path());
    fs::write(repo.path().join("committed.txt"), b"one\n").expect("write committed.txt");
    let status = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["add", "committed.txt"])
        .status()
        .expect("run host git add");
    assert!(status.success(), "host git add failed");
    let status = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args([
            "-c",
            "user.name=a",
            "-c",
            "user.email=a@b",
            "commit",
            "-q",
            "-m",
            "one",
        ])
        .status()
        .expect("run host git commit");
    assert!(status.success(), "host git commit failed");

    // The `.git` mount first and read-only, the repository second and
    // writable: a runtime that applied mounts in spec order would let the
    // parent shadow `.git`, so this checks that both runtimes apply a
    // nested mount by path.
    let spec = serde_json::json!({
        "name": name,
        "image": image,
        "labels": { "dev.example.test": "caller-git" },
        "mounts": [
            { "host": dot_git, "guest": dot_git, "readonly": true },
            { "host": repo.path(), "guest": repo.path(), "readonly": false },
        ],
    });
    let mut up = box_up(binary, &env, &spec, &name);

    // The box reads history and status, and writes the worktree.
    let log = box_exec(
        binary,
        &env,
        &name,
        &["git", "-C", &root, "-c", &safe, "log", "--oneline"],
    );
    assert_eq!(log.code, 0, "box git log failed: {}", log.stderr);
    assert!(
        log.stdout.contains("one"),
        "box git log lost the commit: {}",
        log.stdout
    );
    let status = box_exec(
        binary,
        &env,
        &name,
        &["git", "-C", &root, "-c", &safe, "status", "--porcelain"],
    );
    assert_eq!(status.code, 0, "box git status failed: {}", status.stderr);
    let wrote = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", &format!("echo x > '{root}/new.txt'")],
    );
    assert_ok(&wrote, "writing the worktree");

    // Every write into `.git` fails.
    let hook = dot_git.join("hooks/pre-commit");
    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("printf '#!/bin/sh\\n' > '{}'", hook.display()),
        ],
    );
    assert_denied(&denied, "Read-only file system", "the hook write");
    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "git",
            "-C",
            &root,
            "-c",
            "user.name=a",
            "-c",
            "user.email=a@b",
            "-c",
            &safe,
            "commit",
            "-qam",
            "x",
        ],
    );
    assert_denied(&denied, "Read-only file system", "the commit");
    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("mv '{}' '{}-moved'", dot_git.display(), dot_git.display()),
        ],
    );
    assert_denied(&denied, "Device or resource busy", "renaming .git");

    up.down(binary, &env);
    assert!(up.wait().success(), "box up did not exit cleanly");

    // Host git works on the clone afterwards and runs nothing the box wrote.
    let porcelain = git_status(repo.path());
    assert!(
        porcelain.contains("new.txt"),
        "host git status lost the worktree change: {porcelain}"
    );
    assert!(!hook.exists(), "the box planted a hook");
    assert!(
        !repo.path().join(".git-moved").exists(),
        "the box renamed .git"
    );
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

/// A `box up` process and its ready box.
struct Up {
    name: String,
    /// The parsed `ready` line.
    ready: serde_json::Value,
    starting: Starting,
    /// The `box down` that Drop runs, built while the test env is at hand.
    down: Command,
}

impl Up {
    fn pid(&self) -> u32 {
        self.starting.child.id()
    }

    fn wait(&mut self) -> ExitStatus {
        self.starting.child.wait().expect("wait for box up")
    }

    fn kill(&mut self) {
        self.starting.child.kill().expect("kill box up");
    }

    /// Run `box down` on this box and assert it succeeded.
    fn down(&self, binary: &Path, env: &TestEnv) {
        let status = box_down(binary, env, &self.name);
        assert!(status.success(), "box down failed: {status}");
    }

    /// Close `up`'s stdin and read the rest of its stdout. `up` ends its
    /// stream once teardown is done, so reading to EOF waits for the `down`
    /// line without a sleep.
    fn close_stdin(&mut self) -> Vec<serde_json::Value> {
        drop(self.starting.stdin.take());
        self.starting.rest()
    }
}

impl Drop for Up {
    fn drop(&mut self) {
        // Best effort, so a panicking test does not leak a box.
        let _ = self.down.status();
        let _ = self.starting.child.kill();
        let _ = self.starting.child.wait();
    }
}

fn box_up(binary: &Path, env: &TestEnv, spec: &serde_json::Value, name: &str) -> Up {
    box_up_with_env(binary, env, spec, name, &[])
}

/// [`box_up`] with extra variables in `up`'s own environment.
fn box_up_with_env(
    binary: &Path,
    env: &TestEnv,
    spec: &serde_json::Value,
    name: &str,
    vars: &[(&str, &str)],
) -> Up {
    let mut starting = box_up_start(binary, env, spec, vars);
    let ready = starting.first_line();
    assert_eq!(ready["event"], "ready", "first line was {ready}");
    assert_eq!(ready["box"], name, "ready named another box: {ready}");
    starting.into_up(binary, env, name, ready)
}

/// A spawned `box up` with its spec written and stdin still open, before
/// any of its output is read.
struct Starting {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Starting {
    /// Read and parse `up`'s first line.
    fn first_line(&mut self) -> serde_json::Value {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("read box up's first line");
        serde_json::from_str(line.trim())
            .unwrap_or_else(|error| panic!("box up's first line {line:?} is not JSON: {error}"))
    }

    /// Read the rest of `up`'s stdout to EOF.
    fn rest(&mut self) -> Vec<serde_json::Value> {
        let mut text = String::new();
        self.stdout
            .read_to_string(&mut text)
            .expect("read box up output");
        json_lines(&text)
    }

    /// The [`Up`] of a start whose first line was `ready`.
    fn into_up(self, binary: &Path, env: &TestEnv, name: &str, ready: serde_json::Value) -> Up {
        let mut down = env.command(binary);
        down.args(["box", "down", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Up {
            name: name.to_string(),
            ready,
            starting: self,
            down,
        }
    }
}

/// Spawn `box up` and write its spec, reading nothing, so a test can start
/// several at once.
fn box_up_start(
    binary: &Path,
    env: &TestEnv,
    spec: &serde_json::Value,
    vars: &[(&str, &str)],
) -> Starting {
    let mut child = env
        .command(binary)
        .envs(vars.iter().copied())
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
    let stdout = BufReader::new(child.stdout.take().expect("box up stdout"));
    Starting {
        child,
        stdin: Some(stdin),
        stdout,
    }
}

/// Run `box up` with a spec and return its exit code and first stdout line.
/// `vars` go in `up`'s own environment, as [`box_up_with_env`] gives them.
/// For an `up` that refuses before it holds; close the spec stdin so the
/// child cannot park.
fn box_up_refused(
    binary: &Path,
    env: &TestEnv,
    spec: &serde_json::Value,
    vars: &[(&str, &str)],
) -> (i32, serde_json::Value) {
    let mut starting = box_up_start(binary, env, spec, vars);
    drop(starting.stdin.take());
    let line = starting.first_line();
    let status = starting.child.wait().expect("wait for box up");
    (exit_code(status), line)
}

/// The test's box name, unique to this run.
fn box_name(test: &str) -> String {
    format!("pinfold-e2e-{}-{test}", std::process::id())
}

/// The state dir whose `pid` file names `owner`, if it is there yet. State
/// dirs are keyed by a hash of the box name, so a test finds one by the
/// owner it recorded.
fn find_box_state_dir(env: &TestEnv, owner: u32) -> Option<PathBuf> {
    let boxes = env.state.join("pinfold").join("boxes");
    for entry in fs::read_dir(boxes).ok()?.flatten() {
        let dir = entry.path();
        if fs::read_to_string(dir.join("pid")).is_ok_and(|text| text.trim() == owner.to_string()) {
            return Some(dir);
        }
    }
    None
}

/// Assert that the `what` up (`refused` or `failed`) left no state dir and no
/// box labeled `label`. State dirs are keyed by a hash of the name, so the
/// test cannot name one; this test has no live box at any call, so `boxes/`
/// must be empty.
fn assert_left_nothing(binary: &Path, env: &TestEnv, name: &str, label: &str, what: &str) {
    let boxes = env.state.join("pinfold").join("boxes");
    let leftovers: Vec<PathBuf> = fs::read_dir(&boxes)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "the {what} up for {name} left a state dir: {leftovers:?}"
    );
    assert!(
        box_list(binary, env, label).is_empty(),
        "the {what} up left a box"
    );
}

fn box_prune(binary: &Path, env: &TestEnv) -> Output {
    env.command(binary)
        .args(["box", "prune"])
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold box prune")
}

fn box_down(binary: &Path, env: &TestEnv, name: &str) -> ExitStatus {
    env.command(binary)
        .args(["box", "down", name])
        .stdin(Stdio::null())
        .status()
        .expect("run pinfold box down")
}
