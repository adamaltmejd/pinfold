//! End-to-end tests for the guarantees in docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with Firecracker and rootless podman for images. The harness builds the `pinfold` binary, builds the
//! default profile image once, and drives pinfold as a user would: the CLI,
//! environment variables and the box spec are its only seams.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
use e2e::image_cli;
use e2e::{
    HttpFixture, ImageCleanup, TestDir, TestEnv, assert_denied, assert_ok, box_exec, box_list,
    box_stat, build_profile, curl, default_image, egress_log, exit_code, git, image_id, json_lines,
    pinfold, pinfold_in, profile_containerfile, run_ok, runtime_images, tagged_images,
};

#[test]
fn box_lifecycle_works_for_a_caller() {
    struct Resume(u32);
    impl Drop for Resume {
        fn drop(&mut self) {
            let _ = Command::new("kill")
                .args(["-CONT", &self.0.to_string()])
                .status();
        }
    }
    let _runtime = crate::shared_runtime();
    // Guarantee 9: the lifecycle works for a caller.
    // Sabotage: make `box down` a no-op; the post-down list assertion fails.
    // Sabotage: restore the empty-name sentinel in `box_state_dir`; the
    // `down ""` before the box's teardown removes the whole boxes directory,
    // the live box's lock with it, and the `owner_alive` assertion fails.
    // Sabotage: drop the final `down` line; the last-line assertion fails.
    // Sabotage: make `box exec` drop the runtime's exit status and return 0;
    // the exit-3 assertion fails, and the zero-exit command below is the
    // positive control that the same path can succeed.
    // Sabotage: skip exec's existence check; the post-down exec fails
    // without exit 3 and the exit-3 assertion fails.
    // Sabotage: let Apple's `down` pass the runtime's stderr through again;
    // on macOS the second `down` prints the runtime's not-found error and the
    // empty-stderr assertion fails.
    // Sabotage: drop the owner-label check from cli.rs `present` (or from
    // core::box::down's list check); exec or stat on the foreign container
    // runs, or down removes it. Drop list's owner-label filter and the
    // runtime-created container appears beside the live pinfold box.
    // Sabotage: drop the valid-name gate in core::box::down; Apple's `rm`
    // rejects the `--filter` name as a flag, so the exit assertion fails.
    // Sabotage: key the state dir by the name again; the 60-character box
    // fails at `up` with the socket-length error, so the ready read panics.
    // Sabotage: give a box without `egress` a derived log path in `ready`;
    // the null assertion fails.
    let env = TestEnv::new("lifecycle");
    // A caller's names can be long. The state dir is keyed by the name's
    // hash, so 60 characters, past the old socket-path budget, still come
    // up.
    let name = format!("{:-<60}", box_name("lifecycle"));
    let label = "dev.example.test=lifecycle";
    let image = default_image(&env);
    let image_id = image_id(image).expect("the runtime lists the default image");
    let spec = serde_json::json!({
        "name": name,
        "image": image,
        "labels": { "dev.example.test": "lifecycle", "dev.example.box": name },
    });
    let mut up = box_up(&env, &spec, &name);

    // Exec immediately after ready, without another runtime query first.
    // Sabotage: wait for Apple to report running only with a proxy socket;
    // if the first exec reaches Apple before it records running, this
    // no-egress box's command fails. The runtime controls that race window.
    // `exec` streams both streams and returns the process exit code.
    let failed = box_exec(&env, &name, &["sh", "-c", "echo out; echo err >&2; exit 3"]);
    assert_eq!(
        failed.code, 3,
        "first exec after ready: stdout={:?}, stderr={:?}",
        failed.stdout, failed.stderr
    );
    assert_eq!(failed.stdout, "out\n");
    assert_eq!(failed.stderr, "err\n");

    // Positive control: the same command path passes a zero exit through.
    let ok = box_exec(&env, &name, &["sh", "-c", "exit 0"]);
    assert_eq!(ok.code, 0);

    // `ready` names the image the runtime resolved. The spec has no
    // `egress`, so there is no proxy and no log: `egress_log` is null.
    assert_eq!(
        up.ready["image"]["id"],
        image_id.as_str(),
        "ready names the wrong image id: {}",
        up.ready
    );
    assert!(
        up.ready.get("egress_log").is_some_and(|log| log.is_null()),
        "ready gave a box without egress a log: {}",
        up.ready
    );

    // `list` finds the box by the caller's label, with the same image id.
    let listed = box_list(&env, label);
    let line = listed
        .iter()
        .find(|box_| box_["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| panic!("list did not find {name}: {listed:?}"));
    assert_eq!(
        line["image"]["id"],
        image_id.as_str(),
        "list names the wrong image id: {line}"
    );

    // An empty name, or one a runtime would read as a flag, is not a box:
    // `down` behaves as for any absent box and leaves the live box alone.
    // The filter matches only this box, so a regression removes nothing
    // else on the host.
    let flag = format!("--filter=label=dev.example.box={name}");
    for absent in ["", flag.as_str()] {
        let down = env
            .command(pinfold())
            .args(["box", "down", absent])
            .stdin(Stdio::null())
            .output()
            .expect("run pinfold box down");
        assert!(
            down.status.success(),
            "down {absent:?} failed: {}: {}",
            down.status,
            String::from_utf8_lossy(&down.stderr)
        );
        assert!(
            down.stdout.is_empty() && down.stderr.is_empty(),
            "down {absent:?} printed: {}{}",
            String::from_utf8_lossy(&down.stdout),
            String::from_utf8_lossy(&down.stderr)
        );
        let survivors = box_list(&env, label);
        let live = survivors
            .iter()
            .find(|box_| box_["name"].as_str() == Some(name.as_str()))
            .unwrap_or_else(|| {
                panic!("down {absent:?} took the live box out of list: {survivors:?}")
            });
        assert_eq!(
            live["owner_alive"], true,
            "list reports the live box's owner gone after down {absent:?}: {live}"
        );
    }

    // On Apple, a container pinfold did not create is no box, even under a
    // name `up` accepts: `exec` and `stat` exit 3 and `down` leaves it
    // running. Linux has no runtime that holds boxes.
    #[cfg(target_os = "macos")]
    {
        let foreign = RuntimeContainer::run(&box_name("foreign"), image, Some(label));
        let listed = run_ok(
            env.command(pinfold())
                .args(["box", "list", "--label", label]),
        );
        let listed = json_lines(&String::from_utf8_lossy(&listed.stdout));
        assert!(listed.iter().any(|box_| box_["name"] == name));
        assert!(
            !listed.iter().any(|box_| box_["name"] == foreign.name),
            "list included a runtime-created container: {listed:?}"
        );
        for args in [
            &["box", "exec", foreign.name.as_str(), "--", "true"][..],
            &["box", "stat", foreign.name.as_str()],
        ] {
            let refused = env
                .command(pinfold())
                .args(args)
                .stdin(Stdio::null())
                .output()
                .expect("run pinfold box");
            assert_eq!(
                refused.status.code(),
                Some(3),
                "{args:?} treated a foreign container as a box: {}",
                String::from_utf8_lossy(&refused.stderr)
            );
        }
        let down = env
            .command(pinfold())
            .args(["box", "down", foreign.name.as_str()])
            .stdin(Stdio::null())
            .output()
            .expect("run pinfold box down");
        assert!(
            down.status.success() && down.stdout.is_empty() && down.stderr.is_empty(),
            "down on a foreign container did not behave as for an absent box: {}: {}",
            down.status,
            String::from_utf8_lossy(&down.stderr)
        );
        assert!(
            foreign.listed(),
            "down removed a container pinfold did not create"
        );
        drop(foreign);
    }

    // `down` removes the box and the owner exits.
    up.down(&env);
    let listed = box_list(&env, label);
    assert!(
        !listed
            .iter()
            .any(|box_| box_["name"].as_str() == Some(name.as_str())),
        "box survived down: {listed:?}"
    );
    assert!(up.wait().success(), "box up did not exit cleanly");

    // A stopped owner still owns its name. Sabotage: restore down's
    // timed fallback removal; down succeeds, its state disappears, and a
    // replacement can start before the previous owner finishes cleanup.
    // Sabotage: refuse the first claim lock conflict; the second-root up
    // ends before the spec's ten-second contention wait.
    // The runtime, host state and second XDG root are outside observers.
    {
        let other = TestEnv::new("lifecycle-other");
        let stopped_name = box_name("lifecycle-stopped");
        let mut stopped_spec = spec.clone();
        stopped_spec["name"] = stopped_name.clone().into();
        let mut stopped = box_up(&env, &stopped_spec, &stopped_name);
        let stopped_state = find_box_state_dir(&env).expect("find stopped owner's state");
        let resume = Resume(stopped.pid());
        run_ok(Command::new("kill").args(["-STOP", &stopped.pid().to_string()]));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let state = Command::new("ps")
                .args(["-o", "state=", "-p", &stopped.pid().to_string()])
                .output()
                .expect("observe stopped owner");
            if String::from_utf8_lossy(&state.stdout)
                .trim_start()
                .starts_with('T')
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "owner did not stop");
            std::thread::yield_now();
        }
        let down_started = std::time::Instant::now();
        let timed_out = other
            .command(pinfold())
            .args(["box", "down", &stopped_name])
            .output()
            .expect("down stopped owner from another state root");
        assert!(
            !timed_out.status.success(),
            "down removed a still-owned generation"
        );
        assert!(
            down_started.elapsed() < std::time::Duration::from_secs(15),
            "down did not bound its owner wait"
        );
        assert!(
            stopped_state.is_dir(),
            "down removed the stopped owner's state"
        );
        let replacement_started = std::time::Instant::now();
        let (code, refused) = box_up_refused(&other, &stopped_spec, &[]);
        let contention_wait = replacement_started.elapsed();
        assert!(
            contention_wait >= std::time::Duration::from_secs(10)
                && contention_wait < std::time::Duration::from_secs(15),
            "claim did not wait its bounded contention budget: {contention_wait:?}"
        );
        assert_ne!(code, 0, "replacement ran while old owner was stopped");
        assert_eq!(
            refused["reason"], "name-in-use",
            "replacement was refused for another reason: {refused}"
        );
        drop(resume);
        let lines = stopped.starting.rest();
        assert_eq!(
            lines.last().expect("stopped owner's down line")["reason"],
            "signal"
        );
        assert!(stopped.wait().success(), "queued intentional down failed");
        drop(stopped);
        // Positive control: the same second-root spec works after teardown.
        let mut replacement = box_up(&other, &stopped_spec, &stopped_name);
        assert_ok(
            &box_exec(&other, &stopped_name, &["true"]),
            "replacement after intentional teardown",
        );
        replacement.down(&other);
        assert!(
            replacement.wait().success(),
            "replacement did not exit cleanly"
        );
    }

    // `down` is idempotent: on the box already gone it exits 0 and prints
    // nothing on either stream.
    let second = env
        .command(pinfold())
        .args(["box", "down", &name])
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
    let absent = box_exec(&env, &name, &["sh", "-c", "exit 0"]);
    assert_eq!(
        absent.code, 3,
        "exec on an absent box did not exit 3: {}",
        absent.stderr
    );

    // Closing stdin is `down`: the owner prints the final `down` line and
    // exits 0.
    let mut again = box_up(&env, &spec, &name);
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
    let _runtime = crate::shared_runtime();
    // Guarantee 17: up refuses before it creates.
    // Sabotage: parse the first JSON value straight into `Plan` again; the
    // serde refusal is raised before any name is read, so the nested
    // misspelling's `box` assertion fails.
    // Sabotage: drop the character-class check from the env-name validation,
    // keeping only the old empty-or-`=` test; the wildcard spec then comes up
    // `ready`, so the refusal assertion fails.
    // Sabotage: drop the comma and control-character check from the
    // mount-path validation; the comma spec then comes up `ready` (on Apple
    // the runtime's option parser reads the rest as mount options), so the
    // refusal assertion fails.
    // Sabotage: make the claim treat an existing state dir as success, as
    // `create_dir_all` does, and make the name checks always pass; the
    // concurrent loser is never refused `name-in-use`, and its failure path
    // removes the winner's box, so the winner's `exec` fails too.
    // Sabotage: keep the name check after the state dir is created; the
    // racing loser overwrites the winner's `pid` and its cleanup takes the
    // winner's box down, so the winner's `exec` fails.
    // Sabotage: drop the reserved-label check; the `dev.pinfold.owner` spec
    // comes up `ready`, so its `refused` assertion fails.
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
    let (code, refused) = box_up_refused(&env, &missing, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "image-missing");
    assert_left_nothing(&env, &name, label, "refused");

    // Sabotage: accept unknown fields inside an env `from` object. A valid
    // reference succeeds in the race's winner below; this nested misspelling
    // must be refused as spec, naming the key and the box, before runtime
    // lookup or cache extraction.
    let nested = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "env": { "NESTED_VALUE": { "from": "PINFOLD_E2E_NESTED", "misspelled": true } },
    });
    let (code, refused) = box_up_refused(&env, &nested, &[("PINFOLD_E2E_NESTED", "fixture-value")]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["reason"], "spec");
    assert_eq!(refused["box"], name);
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("misspelled"),
        "the refusal did not name the nested key: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // A spec whose env name is not a POSIX name is refused as data, naming
    // the key, and leaves no state dir and no box. A runtime's CLI may read
    // `HOSTSECRET_*` as a pattern over `up`'s own environment, which is
    // the caller's. The positive control, a valid name
    // arriving in the box, is guarantee 6's `the_environment_is_exactly_the_spec`.
    let wildcard = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "env": { "HOSTSECRET_*": "x" },
    });
    let (code, refused) = box_up_refused(&env, &wildcard, &[("HOSTSECRET_TOKEN", "leaked")]);
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
    assert_left_nothing(&env, &name, label, "refused");

    // A spec label in pinfold's `dev.pinfold.` namespace is refused as data,
    // naming the label, and leaves no state dir and no box:
    // `dev.pinfold.owner` drives `list`'s `owner_alive` and what `prune`
    // removes, and the identity labels name the image. The positive control,
    // a caller label, is the race's winner below.
    let reserved = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": {
            "dev.example.test": "refuses",
            "dev.pinfold.owner": "1",
        },
    });
    let (code, refused) = box_up_refused(&env, &reserved, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("dev.pinfold.owner"),
        "the refusal did not name the label: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // A suffix entry on or above a public suffix is refused as data, naming
    // the entry, and leaves no state dir and no box: the proxy would admit
    // every name below it, including one an attacker registers.
    // `.amazonaws.com` is the hard case, below the list rather than on it.
    // The positive control is the race's winner below, whose allow list
    // carries `.github.com`. Sabotage: drop the "has one below it" clause
    // from the check; this spec then comes up `ready`, so its `refused`
    // assertion fails.
    let public_suffix = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "egress": { "allow": [".amazonaws.com"] },
    });
    let (code, refused) = box_up_refused(&env, &public_suffix, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains(".amazonaws.com"),
        "the refusal did not name the entry: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // A spec whose mount path holds a comma is refused as data, naming the
    // path, and leaves no state dir and no box: on Apple the bind value is
    // built by concatenation, so the runtime reads the rest as mount
    // options. The
    // positive control, a plain path mounting, is the race's winner below.
    let comma = env.root.join("a,b");
    fs::create_dir_all(&comma).unwrap();
    let comma_mount = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": comma, "guest": "/workspace" }],
    });
    let (code, refused) = box_up_refused(&env, &comma_mount, &[]);
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
    assert_left_nothing(&env, &name, label, "refused");

    // Disk specs are refused as data, naming the cause, before anything is
    // created: a path with a comma (on Apple the volume value is
    // concatenated like a bind) and a name that is a flag or a path. The
    // positive control, disks attaching, is guarantee 11's
    // `disks_persist_and_attach_exclusively`.
    let work = env.root.join("work");
    fs::create_dir_all(&work).unwrap();
    let disk_spec = |disk: serde_json::Value| {
        serde_json::json!({
            "name": name,
            "image": default_image(&env),
            "labels": { "dev.example.test": "refuses" },
            "mounts": [{ "host": work, "guest": "/work" }],
            "disks": [disk],
        })
    };
    let disk = box_name("refused-disk");
    let comma_from = serde_json::json!({ "name": disk, "guest": "/disk", "from": comma });
    let comma_guest = serde_json::json!({ "name": disk, "guest": "/a,b" });
    let mut cases = vec![(comma_from, "a,b"), (comma_guest, "a,b")];
    for bad in ["-disk", "a/b"] {
        cases.push((serde_json::json!({ "name": bad, "guest": "/disk" }), bad));
    }
    // A disk at or below the guest kernel's /proc, /sys or /dev would
    // replace the userns sysctl or the work cgroup; the hard case reaches
    // one only after normalization. The positive control is the race's
    // winner below, whose mount at /devices shares the text but not the
    // component. Sabotage: drop the KERNEL_PATHS check from
    // `validate_guests`; these specs come up `ready` on Apple.
    for guest in ["/proc/sys/user", "/work/../sys/fs/cgroup", "/dev"] {
        cases.push((serde_json::json!({ "name": disk, "guest": guest }), guest));
    }
    // A disk at /, /tmp, /etc or /etc/resolv.conf would hide the tmpfs /tmp
    // or the empty resolv.conf (Always applied); three reach theirs only
    // after normalization. The positive control is again the winner's mount at
    // /devices. Sabotage: drop the HIDING_PATHS check from
    // `validate_guests`; the `/tmp` disk then reaches the runtime, which
    // creates the disk, and the `refused` and no-disk assertions fail.
    for guest in ["/work/..", "/tmp", "/work/../etc", "/etc/./resolv.conf"] {
        cases.push((serde_json::json!({ "name": disk, "guest": guest }), guest));
    }
    for (case, named) in cases {
        let (code, refused) = box_up_refused(&env, &disk_spec(case), &[]);
        assert_eq!(code, 1, "a refused up exits 1: {refused}");
        assert_eq!(refused["reason"], "spec", "{refused}");
        assert!(
            refused["detail"]
                .as_str()
                .unwrap_or_default()
                .contains(named),
            "the refusal did not name {named}: {refused}"
        );
        assert_left_nothing(&env, &name, label, "refused");
        let disks = run_ok(env.command(pinfold()).args(["disk", "list"]));
        assert!(
            !String::from_utf8_lossy(&disks.stdout).contains(&disk),
            "a refused disk spec left a disk"
        );
    }
    let kernel_mount = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": work, "guest": "/sys/fs/cgroup" }],
    });
    let (code, refused) = box_up_refused(&env, &kernel_mount, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["reason"], "spec", "{refused}");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("/sys/fs/cgroup"),
        "the refusal did not name the path: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // Two `up`s on one free name at once: the claim lets exactly one hold
    // it, and the loser touches nothing of the winner's. Both are spawned
    // before either's first line is read. The winner also proves that
    // `.github.com` remains an allowed suffix entry.
    let live = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": work, "guest": "/devices" }],
        "egress": { "allow": [".github.com"] },
        "env": { "NESTED_VALUE": { "from": "PINFOLD_E2E_NESTED" } },
    });
    let mut starts = [
        box_up_start(&env, &live, &[("PINFOLD_E2E_NESTED", "fixture-value")]),
        box_up_start(&env, &live, &[("PINFOLD_E2E_NESTED", "fixture-value")]),
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
    let mut winner = winner.into_up(&name, ready);
    assert_eq!(
        refused["event"], "refused",
        "the loser was not refused: {refused}"
    );
    assert_eq!(refused["reason"], "name-in-use", "wrong reason: {refused}");
    drop(loser.stdin);
    let status = loser.child.wait_bounded();
    assert_eq!(exit_code(status), 1, "the losing up did not exit 1");
    let ok = box_exec(&env, &name, &["true"]);
    assert_ok(&ok, "exec in the winner after the losing up");
    let value = box_exec(&env, &name, &["sh", "-c", r#"printf %s "$NESTED_VALUE""#]);
    assert_ok(&value, "reading the allowed nested env reference");
    assert_eq!(value.stdout, "fixture-value");
    let _ = box_down(&env, &name);
    assert!(
        winner.wait().success(),
        "the winning up did not exit cleanly"
    );
}

#[test]
fn disks_persist_and_attach_exclusively() {
    // Exclusive: name locks are host-wide, so another test's maintenance
    // pruning the dead writer holds its name and the disk counts as attached.
    let _runtime = crate::exclusive_runtime();
    // Guarantee 11: disks persist and attach exclusively.
    // Sabotage: skip the attachment check in `disk::claim`; the read-only
    // second `up` is refused `disk-unclean` instead, so the `disk-in-use`
    // assertion fails. Sabotage: skip the `UNCLEAN` mark in `disk::claim`;
    // after the prune the read-only `up` comes ready and the `disk-unclean`
    // assertion fails.
    // Sabotage: skip the size check; the sparse `from` reaches the copy,
    // which fails in the box, and the `limit` assertion fails. Sabotage:
    // write every copied file 644 in `init receive`; the exec-bit assertion
    // fails. Sabotage: drop the disk locks in `disk::claim`; both racing
    // `up`s create the disk and the loser ends `failed`, not `disk-in-use`.
    // Sabotage: call `disk::detached` on every runtime stop in `Box::down`;
    // after `container kill` the read-only `up` comes ready.
    // Sabotage: drop `value.push(",readonly")` from `apple::bind`; on Apple
    // the readers' mount is then writable and the "a write to a mount"
    // assertion fails. Sabotage: on Linux, mount every attachment without
    // `MS_RDONLY` in init's Firecracker setup and pass each drive writable
    // in `vm::config`; the readers' writes succeed and both read-only
    // assertions fail. Sabotage: skip the unfilled check in `disk::claim`;
    // the `up` after an owner killed mid-copy keeps the half-filled disk and
    // the refilled tree assertion fails. Sabotage: check the project disk
    // with a bare `list_disks` in `pi::launch::crossing`; `fetch` recreates
    // the unfilled disk empty in its transient box and marks it filled, so
    // the next `up` skips the copy and the refilled tree assertion fails.
    let env = TestEnv::new("disks");
    let image = default_image(&env);
    let names = ["work", "sparse", "race", "nested"].map(|disk| box_name(&format!("disk-{disk}")));
    let [work, sparse, race, nested] = &names;
    // The refilled disk is a project's disk, so `pinfold fetch` there sees it.
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);
    let config = run_ok(&mut pinfold_in(&env, project.path(), &["config"]));
    let config: serde_json::Value = serde_json::from_slice(&config.stdout).unwrap();
    let refill = &config["project"]["disks"]["project"]
        .as_str()
        .expect("project.disks.project is a string")
        .to_string();
    let boxes =
        ["writer", "second", "reader-a", "reader-b"].map(|b| box_name(&format!("disks-{b}")));
    let [writer, second, reader_a, reader_b] = &boxes;
    let label = "dev.example.test=disks";
    let spec = |name: &str, disk: serde_json::Value| {
        serde_json::json!({
            "name": name,
            "image": image,
            "labels": { "dev.example.test": "disks" },
            "disks": [disk],
        })
    };

    // The host tree: a plain file, an executable, a symlink and a FIFO.
    let from = TestDir::new(&env, "from");
    fs::write(from.path().join("plain.txt"), b"plain\n").unwrap();
    fs::write(from.path().join("run.sh"), b"#!/bin/sh\necho ran\n").unwrap();
    fs::set_permissions(
        from.path().join("run.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::create_dir(from.path().join("sub")).unwrap();
    fs::write(from.path().join("sub/deep.txt"), b"deep\n").unwrap();
    std::os::unix::fs::symlink("plain.txt", from.path().join("link")).unwrap();
    nix::unistd::mkfifo(&from.path().join("fifo"), nix::sys::stat::Mode::S_IRWXU).unwrap();
    let work_disk =
        serde_json::json!({ "name": work, "guest": "/work", "size": "1G", "from": from.path() });
    // `from` is read only when the disk is created; a reader names none.
    let reader = serde_json::json!({ "name": work, "guest": "/work", "readonly": true });
    let mut up = box_up(&env, &spec(writer, work_disk.clone()), writer);
    fs::write(from.path().join("later.txt"), b"later\n").unwrap();
    let tree = box_exec(
        &env,
        writer,
        &[
            "sh",
            "-c",
            "test -x /work/run.sh && /work/run.sh && test ! -x /work/plain.txt \
             && cat /work/sub/deep.txt && test -L /work/link && readlink /work/link \
             && test ! -e /work/fifo && test ! -e /work/later.txt",
        ],
    );
    assert_ok(&tree, "the copied tree");
    assert_eq!(tree.stdout, "ran\ndeep\nplain.txt\n");
    let full = box_exec(
        &env,
        writer,
        &["dd", "if=/dev/zero", "of=/work/big", "bs=1M", "count=1100"],
    );
    assert_denied(&full, "No space left on device", "a write past the 1G disk");
    assert_ok(
        &box_exec(
            &env,
            writer,
            &["sh", "-c", "rm /work/big && echo kept > /work/written"],
        ),
        "writing the disk",
    );
    // On Linux the box plants a symlink where a later nested disk's mount
    // point goes; see below.
    #[cfg(target_os = "linux")]
    assert_ok(
        &box_exec(
            &env,
            writer,
            &["sh", "-c", "mkdir /work/other && ln -s other /work/x"],
        ),
        "planting a symlink on the disk",
    );
    #[cfg(not(target_os = "linux"))]
    let _ = nested;

    // While the writer attaches it, another `up` naming the disk, read-only
    // or writable, is refused and creates nothing, and `disk rm` refuses.
    for disk in [&reader, &work_disk] {
        let (code, refused) = box_up_refused(&env, &spec(second, disk.clone()), &[]);
        assert_eq!(code, 1, "a refused up exits 1: {refused}");
        assert_eq!(refused["reason"], "disk-in-use", "{refused}");
        let states = fs::read_dir(env.state.join("pinfold/boxes"))
            .unwrap()
            .count();
        assert_eq!(states, 1, "the refused up left a state dir");
        assert_eq!(box_list(&env, label).len(), 1, "the refused up left a box");
    }
    let (code, refused) = disk_rm(&env, work);
    assert_eq!(
        (code, &refused["reason"]),
        (1, &"disk-in-use".into()),
        "{refused}"
    );

    // Another state root naming the disk gets its own: its writable `up`
    // runs beside the writer, sees none of the writer's files, and its
    // `disk list` names only its own box. Sabotage: drop the state-root
    // filter in `disk::attachments`; root B's `up` is refused `disk-in-use`.
    // Sabotage (macOS): name volumes without the root's key in
    // `apple::volume`; root B's `up` attaches root A's volume beside the
    // writer, which the runtime refuses, and it ends `failed`, not `ready`.
    let other = TestEnv::new("disks-b");
    let theirs = serde_json::json!({ "name": work, "guest": "/work" });
    let mut theirs = box_up(&other, &spec(second, theirs), second);
    assert_ok(
        &box_exec(&other, second, &["test", "!", "-e", "/work/written"]),
        "another root's disk of the same name",
    );
    let listed: Vec<_> = disk_list(&other)
        .into_iter()
        .filter(|disk| disk["name"] == work.as_str())
        .map(|disk| disk["attached"].clone())
        .collect();
    assert_eq!(
        listed,
        [serde_json::json!([{ "box": second, "readonly": false }])],
        "root B's disk list"
    );
    theirs.down(&other);
    assert!(theirs.wait().success(), "root B's box did not exit cleanly");

    up.down(&env);
    assert!(up.wait().success(), "the writer did not exit cleanly");

    // A `from` whose file sizes pass the disk's size is refused as `limit`
    // and leaves no disk.
    let big = TestDir::new(&env, "big");
    File::create(big.path().join("sparse"))
        .unwrap()
        .set_len(2 << 30)
        .unwrap();
    let sparse_disk =
        serde_json::json!({ "name": sparse, "guest": "/big", "size": "1G", "from": big.path() });
    let (code, refused) = box_up_refused(&env, &spec(second, sparse_disk), &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["reason"], "limit", "{refused}");
    assert_left_nothing(&env, second, label, "limit");
    assert!(
        disk_names(&env).iter().all(|name| name != sparse),
        "the limit refusal left a disk"
    );

    // Two writable `up`s racing for one new disk: exactly one runs. Both
    // are spawned before either's first line is read.
    let racer = serde_json::json!({ "name": race, "guest": "/race" });
    let mut starts = [
        box_up_start(&env, &spec(reader_a, racer.clone()), &[]),
        box_up_start(&env, &spec(reader_b, racer), &[]),
    ];
    let lines = starts.each_mut().map(Starting::first_line);
    let [first, second_start] = starts;
    let [first_line, second_line] = lines;
    let (winner, ready, mut loser, refused) = if first_line["event"] == "ready" {
        (first, first_line, second_start, second_line)
    } else {
        (second_start, second_line, first, first_line)
    };
    assert_eq!(ready["event"], "ready", "neither racer came ready: {ready}");
    assert_eq!(
        refused["reason"], "disk-in-use",
        "the losing racer: {refused}"
    );
    let winner_name = ready["box"].as_str().unwrap().to_string();
    let mut winner = winner.into_up(&winner_name, ready);
    drop(loser.stdin);
    assert_eq!(exit_code(loser.child.wait_bounded()), 1, "the loser's exit");
    winner.down(&env);
    assert!(
        winner.wait().success(),
        "the racing winner did not exit cleanly"
    );

    // After `down`, two boxes attach it read-only at once, read the writer's
    // file and can write neither it nor their mount.
    let mount = TestDir::new(&env, "mount");
    let with_mount = |name: &str| {
        let mut spec = spec(name, reader.clone());
        spec["mounts"] = serde_json::json!([{ "host": mount.path(), "guest": "/mount" }]);
        spec
    };
    let mut starts = [
        box_up_start(&env, &with_mount(reader_a), &[]),
        box_up_start(&env, &with_mount(reader_b), &[]),
    ];
    let lines = starts.each_mut().map(Starting::first_line);
    let [a, b] = starts;
    let [a_line, b_line] = lines;
    let mut readers = [a.into_up(reader_a, a_line), b.into_up(reader_b, b_line)];
    for name in [reader_a, reader_b] {
        let read = box_exec(
            &env,
            name,
            &["sh", "-c", "cat /work/written; test ! -e /work/later.txt"],
        );
        assert_ok(&read, "reading the disk read-only");
        assert_eq!(read.stdout, "kept\n");
        let write = box_exec(&env, name, &["touch", "/work/new"]);
        assert_denied(
            &write,
            "Read-only file system",
            "a write to a read-only disk",
        );
        let write = box_exec(&env, name, &["touch", "/mount/new"]);
        assert_denied(&write, "Read-only file system", "a write to a mount");
    }
    for reader in &mut readers {
        reader.down(&env);
        assert!(reader.wait().success(), "a reader did not exit cleanly");
    }

    // On Linux PID 1 makes a nested disk's mount point. One reached through
    // the symlink the writer planted ends the start `failed`, naming it, and
    // PID 1 creates nothing through the link; the same nesting at a plain
    // path mounts and takes a write. The failed start leaves the disk
    // unclean, which the writable `up` below clears. Sabotage: make the
    // mount point with `fs::create_dir_all` in `mount_at`, keeping a
    // canonicalize check; the start still fails, but PID 1 first makes
    // /work/other/sub through the link and the absence assertion fails.
    #[cfg(target_os = "linux")]
    {
        let with_nested = |guest: &str| {
            let mut spec = spec(writer, work_disk.clone());
            spec["disks"] = serde_json::json!([work_disk, { "name": nested, "guest": guest }]);
            spec
        };
        let (code, failed) = box_up_refused(&env, &with_nested("/work/x/sub"), &[]);
        assert_eq!(code, 1, "a failed up exits 1: {failed}");
        assert_eq!(failed["event"], "failed", "{failed}");
        assert!(
            failed["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("/work/x/sub")),
            "the failure did not name the mount point: {failed}"
        );
        let mut plain = box_up(&env, &with_nested("/work/plain/sub"), writer);
        assert_ok(
            &box_exec(
                &env,
                writer,
                &[
                    "sh",
                    "-c",
                    "test -L /work/x && touch /work/plain/sub/written",
                ],
            ),
            "the link kept and the plain nesting writable",
        );
        assert_ok(
            &box_exec(&env, writer, &["test", "!", "-e", "/work/other/sub"]),
            "nothing made at the planted link's target",
        );
        plain.down(&env);
        assert!(
            plain.wait().success(),
            "the nested writer did not exit cleanly"
        );
    }

    // A writer whose box is killed under it leaves the disk unclean: its
    // owner tears the box down, but nobody asked it to stop.
    let mut ended = box_up(&env, &spec(writer, work_disk.clone()), writer);
    #[cfg(target_os = "macos")]
    run_ok(Command::new(image_cli()).args(["kill", writer.as_str()]));
    #[cfg(target_os = "linux")]
    run_ok(Command::new("kill").args(["-KILL", &vmm_pid(ended.pid()).to_string()]));
    let _ = ended.wait();
    let (code, refused) = box_up_refused(&env, &spec(reader_a, reader.clone()), &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["reason"], "disk-unclean", "{refused}");
    // A teardown the runtime fails leaves the stopped box for `prune`, and
    // its name is not free until then (#167).
    run_ok(env.command(pinfold()).args(["box", "prune"]));
    let mut recovery = box_up(&env, &spec(writer, work_disk.clone()), writer);
    recovery.down(&env);
    assert!(recovery.wait().success());

    // A writer whose owner dies leaves the disk unclean: read-only is
    // refused, writable recovers it. On macOS the box outlives its owner
    // until the runtime ends it; on Linux the VMM dies soon after its owner.
    // Once the box has stopped, the dead writer releases the disk before any
    // `prune`.
    // Sabotage: count a claim in `disk::attachments` while the runtime lists
    // its box, running or not; the read-only `up` is refused `disk-in-use`
    // and the `disk-unclean` assertion fails.
    let mut killed = box_up(&env, &spec(writer, work_disk.clone()), writer);
    killed.kill();
    let _ = killed.wait();
    // `stop` returns once the box has stopped. On Linux the VMM can outlive
    // its owner briefly, so the wait reads `box list`.
    #[cfg(target_os = "macos")]
    run_ok(Command::new(image_cli()).args(["stop", "--signal", "KILL", writer.as_str()]));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while box_list(&env, label)
        .iter()
        .any(|listed| listed["name"] == writer.as_str() && listed["state"] == "running")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the dead writer's box never stopped"
        );
    }
    let (code, refused) = box_up_refused(&env, &spec(reader_a, reader.clone()), &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["reason"], "disk-unclean", "{refused}");
    run_ok(env.command(pinfold()).args(["box", "prune"]));
    assert_left_nothing(&env, reader_a, label, "unclean");
    let mut recovery = box_up(&env, &spec(writer, work_disk.clone()), writer);
    recovery.down(&env);
    assert!(
        recovery.wait().success(),
        "the recovering writer did not exit cleanly"
    );
    let mut after = box_up(&env, &spec(reader_a, reader), reader_a);
    assert_ok(
        &box_exec(&env, reader_a, &["cat", "/work/written"]),
        "reading after recovery",
    );
    #[cfg(target_os = "linux")]
    {
        after.down(&env);
        assert!(after.wait().success());
    }
    // A dead owner's box the runtime has created but not started may still
    // boot, so it holds its disk. Apple lists such a box `stopped`, as an
    // orphaned `container run` leaves it; `container create` makes one.
    // Sabotage: drop the `started_date` condition from `stopped` in
    // `Apple::list`; the writable `up` runs and the `disk-in-use` assertion
    // fails.
    #[cfg(target_os = "macos")]
    {
        after.kill();
        let _ = after.wait();
        let _ = Command::new(image_cli())
            .args(["rm", "--force", reader_a.as_str()])
            .output();
        run_ok(Command::new(image_cli()).args([
            "create",
            "--name",
            reader_a.as_str(),
            image,
            "true",
        ]));
        let (code, refused) = box_up_refused(&env, &spec(writer, work_disk.clone()), &[]);
        run_ok(Command::new(image_cli()).args(["rm", reader_a.as_str()]));
        assert_eq!(code, 1, "a refused up exits 1: {refused}");
        assert_eq!(refused["reason"], "disk-in-use", "{refused}");
    }

    // An owner killed while it copies `from` leaves no filled disk: the next
    // `up` naming it copies the whole tree. Once the disk exists, the first
    // file the copy opens is renamed over by a FIFO, so the copy can neither
    // finish nor fail. The kill waits until the box runs: an owner killed
    // before that leaves a `container run` that may create its container
    // after the prune. The disk surviving and no `ready` show the kill
    // landed mid-copy, not after it or in a cleaned-up failure.
    let first = from.path().join("0first.txt");
    fs::write(&first, b"first\n").unwrap();
    let refill_disk =
        serde_json::json!({ "name": refill, "guest": "/work", "size": "1G", "from": from.path() });
    let mut interrupted = box_up_start(&env, &spec(writer, refill_disk.clone()), &[]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !disk_names(&env).contains(refill) {
        assert!(
            std::time::Instant::now() < deadline,
            "the interrupted up created no disk"
        );
    }
    let fifo = from.path().join("fifo-swap");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
    fs::rename(&fifo, &first).unwrap();
    while !box_list(&env, label)
        .iter()
        .any(|listed| listed["name"] == writer.as_str() && listed["state"] == "running")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the interrupted up's box never ran"
        );
    }
    interrupted.child.kill().expect("kill box up");
    let _ = interrupted.child.wait_bounded();
    run_ok(env.command(pinfold()).args(["box", "prune"]));
    let printed = interrupted.rest();
    assert!(
        printed.iter().all(|line| line["event"] != "ready"),
        "the interrupted up came ready: {printed:?}"
    );
    assert!(
        disk_names(&env).contains(refill),
        "the interrupted up's disk is gone"
    );
    // The unfilled disk is no project disk to `fetch` either: it exits
    // nonzero and starts no box, so prints no crossing line and records no
    // run, and leaves the disk for the next `up` to refill.
    let runs = || -> Vec<_> {
        let mut names: Vec<_> = fs::read_dir(env.state.join("pinfold/runs"))
            .expect("read the runs dir")
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let runs_before = runs();
    let fetched = pinfold_in(&env, project.path(), &["fetch"])
        .output()
        .expect("run pinfold fetch");
    assert!(
        !fetched.status.success() && fetched.stdout.is_empty(),
        "fetch on an unfilled disk: {:?} {}",
        fetched.status,
        String::from_utf8_lossy(&fetched.stderr)
    );
    assert_eq!(
        runs(),
        runs_before,
        "fetch on an unfilled disk recorded a run"
    );
    fs::remove_file(&first).unwrap();
    fs::write(&first, b"first\n").unwrap();
    let mut refilled = box_up(&env, &spec(writer, refill_disk), writer);
    let tree = box_exec(
        &env,
        writer,
        &[
            "sh",
            "-c",
            "cat /work/0first.txt && /work/run.sh && cat /work/sub/deep.txt \
             && readlink /work/link && cat /work/later.txt",
        ],
    );
    assert_ok(&tree, "the refilled tree");
    assert_eq!(tree.stdout, "first\nran\ndeep\nplain.txt\nlater\n");
    refilled.down(&env);
    assert!(refilled.wait().success());

    // After `down`, `disk rm` removes it, and an absent disk exits 0. A
    // name that is no disk name is refused as `spec`.
    let (code, refused) = disk_rm(&env, "../escape");
    assert_eq!((code, &refused["reason"]), (1, &"spec".into()), "{refused}");
    let (code, removed) = disk_rm(&env, work);
    assert_eq!((code, &removed["existed"]), (0, &true.into()), "{removed}");
    let (code, removed) = disk_rm(&env, work);
    assert_eq!((code, &removed["existed"]), (0, &false.into()), "{removed}");
}

fn disk_list(env: &TestEnv) -> Vec<serde_json::Value> {
    e2e::disk_list(env).unwrap_or_else(|error| panic!("{error}"))
}

pub(super) fn disk_names(env: &TestEnv) -> Vec<String> {
    disk_list(env)
        .iter()
        .filter_map(|disk| disk["name"].as_str().map(String::from))
        .collect()
}

fn disk_rm(env: &TestEnv, name: &str) -> (i32, serde_json::Value) {
    one_json_line(env.command(pinfold()).args(["disk", "rm", name]))
}

/// Every client-side hook name in githooks(5).
const CLIENT_HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
    "post-index-change",
    "reference-transaction",
];

#[test]
fn work_leaves_the_box_only_by_fetch() {
    let _runtime = crate::shared_runtime();
    // Guarantee 10: disk writes stay in the box, fetch moves branches only,
    // and nothing the box plants runs on the host.
    // Sabotage: write fetched branches under `refs/heads/` in crossing.rs;
    // the fetched refs assertion fails. Sabotage: bundle `--tags` too and
    // fetch with `--tags` for `--no-tags`; the box's tag lands. Sabotage: set REPO's
    // `core.hooksPath` to `.githooks` after the fetch, as if fetch adopted
    // the box's config; a host checkout touches a marker. Sabotage: drop
    // `check_branch` from crossing.rs; the hostile bundle's
    // `refs/heads/injected` lands on the host.
    let env = TestEnv::new("crossing");
    let name = box_name("crossing");
    let project = TestDir::new(&env, "project");
    let repo = project.path();
    let markers = TestDir::new(&env, "markers");
    let marker = |hook: &str| markers.path().join(hook);
    let host_git = |args: &[&str]| git(repo, args).trim().to_string();
    host_git(&["init", "-q", "-b", "main"]);
    host_git(&["config", "user.name", "host"]);
    host_git(&["config", "user.email", "host@example.test"]);
    fs::write(repo.join("a.txt"), b"a\n").unwrap();
    fs::write(repo.join("run.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(repo.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::create_dir(repo.join("sub")).unwrap();
    fs::write(repo.join("sub/b.txt"), b"b\n").unwrap();
    host_git(&["add", "-A"]);
    host_git(&["commit", "-q", "-m", "host one"]);
    host_git(&["branch", "feature"]);

    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "disks": [{ "name": name, "guest": "/work", "size": "1G", "from": repo }],
        // The hostile case's `git` goes here, absent until then.
        "env": { "PATH": "/work/.hostile:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" },
    });
    let mut up = box_up(&env, &spec, &name);
    let in_box = |script: &str| {
        let output = box_exec(&env, &name, &["sh", "-c", script]);
        assert_ok(&output, script);
        output.stdout
    };
    in_box("git -C /work config user.name box && git -C /work config user.email box@example.test");

    // The box writes, chmods and deletes on its disk; the host project,
    // .git included, keeps its bytes, modes and mtimes.
    let before = tree_snapshot(repo);
    in_box(
        "printf more >> /work/a.txt && chmod 600 /work/run.sh && rm /work/sub/b.txt \
         && echo box > /work/box.txt && git -C /work add box.txt \
         && git -C /work commit -q -m 'box one'",
    );
    assert_eq!(
        tree_snapshot(repo),
        before,
        "the box changed the host project"
    );

    // The box tracks hooks in .githooks/ and .husky/, tags, and only then
    // plants hooks under every client-side name, core.hooksPath and
    // core.fsmonitor, each touching a host marker.
    let hooks: String = CLIENT_HOOKS.iter().map(|hook| format!("{hook} ")).collect();
    in_box(&format!(
        "cd /work && for dir in .githooks .husky; do mkdir -p $dir; for hook in {hooks}; do \
              printf '#!/bin/sh\\ntouch {markers}/%s-%s\\n' $dir $hook > $dir/$hook; \
              chmod 755 $dir/$hook; done; done \
         && git add .githooks .husky && git commit -q -m hooks \
         && git tag box-tag \
         && for hook in {hooks}; do \
              printf '#!/bin/sh\\ntouch {markers}/git-%s\\n' $hook > .git/hooks/$hook; \
              chmod 755 .git/hooks/$hook; done \
         && git config core.hooksPath .githooks \
         && git config core.fsmonitor 'touch {markers}/fsmonitor'",
        markers = markers.path().display(),
    ));
    let host_heads = host_git(&["for-each-ref", "refs/heads/"]);
    let (code, fetched) = crossing_fetch(&env, &name, repo);
    assert_eq!(code, 0, "{fetched}");
    let box_branches = branch_map(
        &in_box("git -C /work for-each-ref refs/heads/"),
        &format!("refs/remotes/{name}/"),
    );
    assert_eq!(fetched["event"], "fetched", "{fetched}");
    assert_eq!(fetched["remote"], name.as_str(), "{fetched}");
    assert_eq!(
        fetched["refs"],
        serde_json::json!(box_branches),
        "{fetched}"
    );
    // Only the box's branches arrive, under its namespace: no tag, and the
    // host's own branches are as they were.
    let mut expected = branch_map(&host_heads, "refs/heads/");
    expected.extend(box_branches.clone());
    assert_eq!(
        branch_map(&host_git(&["for-each-ref"]), "refs/heads/"),
        expected,
        "the host's refs after fetch"
    );

    // A fast-forward merge and two checkouts on the host run nothing the box
    // planted.
    host_git(&[
        "merge",
        "-q",
        "--ff-only",
        &format!("refs/remotes/{name}/main"),
    ]);
    assert!(
        repo.join(".husky/post-checkout").exists(),
        "the merge brought the hooks"
    );
    host_git(&["checkout", "-q", "feature"]);
    host_git(&["checkout", "-q", "main"]);
    let touched: Vec<_> = fs::read_dir(markers.path()).unwrap().collect();
    assert!(
        touched.is_empty(),
        "a box hook ran on the host: {touched:?}"
    );
    // Control: the same hook in the host's own .git/hooks runs.
    let control = repo.join(".git/hooks/post-checkout");
    fs::write(
        &control,
        format!("#!/bin/sh\ntouch {}\n", marker("control").display()),
    )
    .unwrap();
    fs::set_permissions(&control, fs::Permissions::from_mode(0o755)).unwrap();
    host_git(&["checkout", "-q", "feature"]);
    assert!(
        marker("control").exists(),
        "the host's own hook did not run"
    );
    fs::remove_file(&control).unwrap();

    // A box that writes its own bundles: a `git` ahead of the real one on
    // PATH stands in for a box that controls its processes' output. Each
    // bundle fails on its own, so refusing one can't mask another: a real
    // commit under a `refs/heads/x` head plus a
    // `refs/heads/x:refs/heads/injected` head; the same plus a
    // `refs/heads/x:refs/replace/<host main>` head; and a commit whose tree
    // holds a `.git` entry under a plain `refs/heads/x`; and a sound pack
    // under a `refs/heads/x` naming a commit it lacks, which fetch unbundles
    // before its connectivity check fails; and a sound bundle with both
    // `refs/heads/x` and `refs/heads/x/y`, which only the ref transaction
    // refuses; and a sound bundle under `refs/heads/z` while the box's
    // listing names a `refs/heads/x` the bundle leaves out, at a host commit
    // the box's namespace does not reach. Each fetch fails, no host ref
    // changes, and the host's object store keeps its paths and sizes.
    // Sabotage: drop `--fsck-objects` from crossing::fetch's index-pack; the
    // `.git` bundle then lands on any host git, and its assertions fail.
    // Sabotage: drop the quarantine env from that index-pack; the `.git`
    // bundle leaves a `tmp_pack_*`. Sabotage: drop it from the fetch; the
    // missing commit's bundle leaves its pack. Sabotage: migrate in
    // crossing::update_refs before `prepare`; the `x/y` bundle leaves its
    // pack. Sabotage: drop the `rev-list --not --glob` reach check from
    // crossing::fetch's listed loop; the omitted `x` lands at the host-only
    // commit.
    let host_main = host_git(&["rev-parse", "main"]);
    // A host commit no box branch reaches: `feature` is checked out.
    host_git(&["commit", "-q", "--allow-empty", "-m", "host only"]);
    let host_only = host_git(&["rev-parse", "feature"]);
    let host_refs = host_git(&["for-each-ref"]);
    let host_objects = object_store(repo);
    let absent = in_box(&format!(
        r#"set -e
cd /work
mkdir .hostile
g() {{ git -c core.hooksPath=/nonexistent -c core.fsmonitor=false "$@"; }}
# mk NAME COMMIT TIP HEAD...: a bundle of COMMIT with TIP under the given
# head lines.
mk() {{
  name=$1 commit=$2 tip=$3; shift 3
  g update-ref refs/evil/x "$commit"
  g bundle create -q .hostile/real refs/evil/x
  header=$(sed -n '1,/^$/p' .hostile/real | wc -c)
  {{ printf '# v2 git bundle\n'; for head; do printf '%s %s\n' "$tip" "$head"; done; printf '\n'
    tail -c +$((header + 1)) .hostile/real; }} > .hostile/$name
}}
evil=$(g commit-tree -m evil 'HEAD^{{tree}}')
absent=$(g commit-tree -m absent 'HEAD^{{tree}}')
blob=$(echo evil | g hash-object -w --stdin)
inner=$(printf '100644 blob %s\tconfig\n' "$blob" | g mktree)
dotgit=$(g commit-tree -m dotgit "$(printf '040000 tree %s\t.git\n' "$inner" | g mktree)")
mk injected "$evil" "$evil" refs/heads/x refs/heads/x:refs/heads/injected
mk replace "$evil" "$evil" refs/heads/x refs/heads/x:refs/replace/{host_main}
mk dotgit "$dotgit" "$dotgit" refs/heads/x
mk missing "$evil" "$absent" refs/heads/x
mk conflict "$evil" "$evil" refs/heads/x refs/heads/x/y
mk omitted "$evil" "$evil" refs/heads/z
printf '%s refs/heads/x\n' {host_only} > .hostile/omitted.heads
cat > .hostile/git <<'END'
#!/bin/sh
case " $* " in
*" for-each-ref "*)
  /usr/bin/git "$@" || exit
  if [ -e /work/.hostile/heads ]; then cat /work/.hostile/heads; fi
  exit 0 ;;
*" rev-list "*) echo more; exit 0 ;;
*" bundle create "*) exec cat /work/.hostile/bundle ;;
esac
exec /usr/bin/git "$@"
END
chmod 755 .hostile/git
echo "$absent""#
    ));
    let conflict = format!("refs/remotes/{name}/x/y");
    for case in [
        "injected", "replace", "dotgit", "missing", "conflict", "omitted",
    ] {
        in_box(&format!(
            "cd /work/.hostile && cp {case} bundle && rm -f heads \
             && if [ -e {case}.heads ]; then cp {case}.heads heads; fi"
        ));
        let (code, failed) = crossing_fetch(&env, &name, repo);
        assert_eq!(
            (code, &failed["event"]),
            (1, &"failed".into()),
            "{case}: {failed}"
        );
        assert_eq!(
            host_git(&["for-each-ref"]),
            host_refs,
            "the {case} bundle changed a host ref"
        );
        assert_eq!(
            object_store(repo),
            host_objects,
            "the {case} bundle left bytes in the host's objects"
        );
        // git names the fsck entry, the commit the pack lacks, and the ref
        // it can't update beside `x`, and the omitted branch's failure names
        // its commit: each fetch failed for it, not for a malformed bundle.
        let reason = match case {
            "dotgit" => "hasDotgit",
            "missing" => absent.trim(),
            "conflict" => conflict.as_str(),
            "omitted" => host_only.as_str(),
            _ => continue,
        };
        assert!(
            failed["detail"].as_str().unwrap().contains(reason),
            "{failed}"
        );
    }

    up.down(&env);
    assert!(up.wait().success(), "the box did not exit cleanly");
}

fn crossing_fetch(env: &TestEnv, name: &str, repo: &Path) -> (i32, serde_json::Value) {
    one_json_line(env.command(pinfold()).args([
        "box",
        "fetch",
        name,
        "--repo",
        "/work",
        "--into",
        repo.to_str().unwrap(),
    ]))
}

/// `for-each-ref` output as `{name: oid}`, each branch's `refs/heads/`
/// replaced by `prefix`.
fn branch_map(listing: &str, prefix: &str) -> std::collections::BTreeMap<String, String> {
    listing
        .lines()
        .map(|line| {
            let (oid, name) = line.split_once(" commit\t").expect("a commit ref");
            let name = match name.strip_prefix("refs/heads/") {
                Some(branch) => format!("{prefix}{branch}"),
                None => name.to_string(),
            };
            (name, oid.to_string())
        })
        .collect()
}

/// Every path under the repository's `.git/objects` with its size, a
/// directory's as 0 since filesystems size those differently.
fn object_store(repo: &Path) -> std::collections::BTreeMap<PathBuf, u64> {
    let mut store = std::collections::BTreeMap::new();
    let mut pending = vec![repo.join(".git/objects")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let size = if metadata.is_dir() {
                pending.push(path.clone());
                0
            } else {
                metadata.len()
            };
            store.insert(path, size);
        }
    }
    store
}

/// Every path under `dir` with its bytes (files only), mode and mtime.
fn tree_snapshot(dir: &Path) -> std::collections::BTreeMap<PathBuf, (Vec<u8>, u32, i64, i64)> {
    let mut snapshot = std::collections::BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).unwrap();
        let bytes = if metadata.is_file() {
            fs::read(&path).unwrap()
        } else {
            Vec::new()
        };
        if metadata.is_dir() {
            pending.extend(
                fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        }
        snapshot.insert(
            path,
            (
                bytes,
                metadata.mode(),
                metadata.mtime(),
                metadata.mtime_nsec(),
            ),
        );
    }
    snapshot
}

#[test]
fn nothing_can_gain_privileges() {
    let _runtime = crate::shared_runtime();
    // Sabotage: drop `find / -xdev -perm /6000 -type f -exec chmod a-s {} +`
    // from the bundled image fragments; the setuid/setgid scan then lists
    // files and fails. Sabotage: drop `--read-only` from Apple's `run` argv,
    // or init's read-only remount of the root on Linux; the rootfs write
    // then succeeds and its assertion fails. Sabotage: keep every
    // capability in PID 1 on Linux, or start the exec service before it
    // drops them; the CapBnd assertions fail.
    let env = TestEnv::new("privileges");
    let image = default_image(&env);
    let name = box_name("privileges");
    let spec = serde_json::json!({
        "name": name,
        "image": image,
    });
    let up = box_up(&env, &spec, &name);

    // Exec'd work runs as the host uid:gid with an empty capability bounding
    // set. Read /proc: Apple's virtiofs reports host files as the host user's
    // whatever the guest uid, so ownership cannot show a root exec.
    let work = box_exec(&env, &name, &["cat", "/proc/self/status"]);
    assert_ok(&work, "reading /proc/self/status");
    assert_eq!(
        status_field(&work.stdout, "CapBnd:"),
        "0000000000000000",
        "exec capability bound"
    );
    assert_process_ids(&work.stdout, "exec");

    // On Apple PID 1 also runs as the host uid:gid. On Linux it is root
    // with only CAP_SYS_ADMIN (bit 21), and the exec service, every exec's
    // parent, is the host uid with every capability set empty.
    let init = box_exec(&env, &name, &["cat", "/proc/1/status"]);
    assert_ok(&init, "reading /proc/1/status");
    if cfg!(target_os = "linux") {
        assert_eq!(
            status_field(&init.stdout, "Uid:").split_whitespace().next(),
            Some("0")
        );
        assert_eq!(
            status_field(&init.stdout, "CapBnd:"),
            "0000000000200000",
            "PID 1 capability bound"
        );
        let service = box_exec(&env, &name, &["sh", "-c", "cat /proc/$PPID/status"]);
        assert_ok(&service, "reading the exec service's status");
        assert_process_ids(&service.stdout, "the exec service");
        for set in ["CapInh:", "CapPrm:", "CapEff:", "CapBnd:", "CapAmb:"] {
            assert_eq!(
                status_field(&service.stdout, set),
                "0000000000000000",
                "the exec service's {set}"
            );
        }
        // The service is not dumpable, so box work cannot reach its
        // descriptors, the exec listener among them. Sabotage:
        // `prctl(PR_SET_DUMPABLE, 1)` after `setresuid` in
        // `become_box_user`; the listing then succeeds.
        let fds = box_exec(&env, &name, &["sh", "-c", "ls /proc/$PPID/fd"]);
        assert_denied(&fds, "Permission denied", "listing the exec service's fds");
        // Positive control: box work lists its own.
        let own = box_exec(&env, &name, &["ls", "/proc/self/fd"]);
        assert_ok(&own, "listing its own fds");
    } else {
        assert_process_ids(&init.stdout, "PID 1");
    }

    // No setuid or setgid files on the root filesystem. The marker proves the
    // scan ran even though unreadable directories make find exit nonzero.
    let setuid = box_exec(
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
        &env,
        &name,
        &["sh", "-c", "printf x > /pinfold-root-write-test"],
    );
    assert_denied(&rootfs, "Read-only file system", "a rootfs write");

    // Positive control: the same write works on /tmp.
    let tmp = box_exec(
        &env,
        &name,
        &["sh", "-c", "printf x > /tmp/pinfold-write-test"],
    );
    assert_eq!(tmp.code, 0, "writing /tmp failed: {}", tmp.stderr);

    // On Linux the guest allows no user namespace. Sabotage: skip init's
    // `user.max_user_namespaces` 0; `unshare -U true` succeeds as the box
    // user and this assertion fails. Nested user namespaces on Apple
    // `container` are an open question in the spec, so assert nothing there.
    if cfg!(target_os = "linux") {
        let unshare = box_exec(&env, &name, &["unshare", "-U", "true"]);
        assert_denied(&unshare, "No space left on device", "unshare -U");
        // Positive control: the same box still runs a plain child process.
        let child = box_exec(&env, &name, &["true"]);
        assert_ok(&child, "a plain child process");
    }

    up.down(&env);
}

#[test]
fn each_box_has_its_own_kernel() {
    let _runtime = crate::shared_runtime();
    // Guarantee 37. Sabotage: pin another guest kernel under vmm.toml's
    // version (on Linux), or run the box on the host's kernel; `uname -r`
    // then names another release and its assertion fails.
    let env = TestEnv::new("own-kernel");
    let name = box_name("own-kernel");
    let spec = serde_json::json!({ "name": name, "image": default_image(&env) });
    let up = box_up(&env, &spec, &name);
    let release = box_exec(&env, &name, &["uname", "-r"]);
    assert_ok(&release, "uname -r in the box");
    let version = box_exec(&env, &name, &["cat", "/proc/version"]);
    assert_ok(&version, "reading /proc/version in the box");
    let host = run_ok(Command::new("uname").arg("-r"));
    let host = String::from_utf8_lossy(&host.stdout);
    assert_ne!(
        release.stdout.trim(),
        host.trim(),
        "the box runs the host's kernel"
    );
    assert!(
        !version.stdout.contains(host.trim()),
        "the box's /proc/version names the host's release: {}",
        version.stdout
    );
    // The release is the kernel half of the VMM pin, read from the pin file.
    #[cfg(target_os = "linux")]
    {
        let pins =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../pinfold/vmm.toml"))
                .expect("read vmm.toml");
        let pinned = pins
            .lines()
            .find_map(|line| line.strip_prefix("version = "))
            .and_then(|version| version.trim_matches('"').split_once('+'))
            .map(|(_, kernel)| kernel.to_string())
            .expect("vmm.toml pins firecracker+kernel");
        assert_eq!(
            release.stdout.trim(),
            pinned,
            "the box's kernel is not the pinned one"
        );
    }
    up.down(&env);
}

#[test]
#[cfg(target_os = "linux")]
fn the_vmm_reaches_only_its_own_box() {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let _runtime = crate::shared_runtime();
    // Guarantee 43. Sabotage: create the VMM from a `spawn_blocking` thread
    // instead of its own; PDEATHSIG fires when that pool thread exits and
    // the VMM's PPid or liveness assertion fails. Sabotage: drop CLONE_NEWPID
    // from the jail's clone flags; the pid namespace assertion fails.
    // Sabotage: skip `pivot_root`; the VMM's root lists the host's tree and
    // its mount points lie under the home. Sabotage: bind the box's state
    // dir into the jail; the root then holds that directory. Sabotage: drop
    // `close_range` before exec; the VMM inherits a descriptor under the
    // home and the fd assertion fails. Sabotage: bind the cache's pinned
    // artifacts directory instead of each file; the root-list assertion
    // fails.
    let env = TestEnv::new("vmm-jail");
    let home = std::env::var("HOME").expect("HOME is set");
    let names = [box_name("vmm-a"), box_name("vmm-b")];
    let disks = [format!("{}-disk", names[0]), format!("{}-disk", names[1])];
    let mut ups: Vec<Up> = names
        .iter()
        .zip(&disks)
        .map(|(name, disk)| {
            let spec = serde_json::json!({
                "name": name,
                "image": default_image(&env),
                "disks": [{ "name": disk, "guest": "/disk" }],
            });
            box_up(&env, &spec, name)
        })
        .collect();
    // Each disk's host file, by device and inode, from `disk list`'s names
    // in the state root's disk store.
    let disk_files: Vec<(u64, u64)> = disks
        .iter()
        .map(|disk| {
            let metadata = fs::metadata(env.state.join(format!("pinfold/disks/{disk}.ext4")))
                .unwrap_or_else(|error| panic!("stat disk {disk}: {error}"));
            (metadata.dev(), metadata.ino())
        })
        .collect();
    let vmms: Vec<u32> = ups.iter().map(|up| vmm_pid(up.pid())).collect();
    let namespace = |pid: u32, kind: &str| {
        fs::read_link(format!("/proc/{pid}/ns/{kind}"))
            .unwrap_or_else(|error| panic!("read {pid}'s {kind} namespace: {error}"))
    };
    for (index, (up, &vmm)) in ups.iter().zip(&vmms).enumerate() {
        let status =
            fs::read_to_string(format!("/proc/{vmm}/status")).expect("read the VMM's status");
        assert_eq!(
            status_field(&status, "PPid:"),
            up.pid().to_string(),
            "the VMM's parent"
        );
        assert_eq!(
            status_field(&status, "CapBnd:"),
            "0000000000000000",
            "the VMM's capability bound"
        );
        assert_eq!(
            status_field(&status, "NoNewPrivs:"),
            "1",
            "the VMM's no_new_privs"
        );
        assert_process_ids(&status, "the VMM");
        for kind in ["user", "mnt", "pid", "net", "ipc", "uts"] {
            assert_ne!(
                namespace(vmm, kind),
                namespace(up.pid(), kind),
                "the VMM shares box up's {kind} namespace"
            );
        }
        // Its root holds exactly the spec's jail list (VMM jail): the
        // Firecracker binary, the guest kernel, the root and init disks, the
        // box's one disk, `/dev/kvm`, the vsock socket directory and the VM
        // configuration, in the spec's fixed jail layout for one disk and
        // no mounts; the expected names are not read from `config.json`.
        // The disk is the box's own host file.
        let root: Vec<(String, (u64, u64))> = fs::read_dir(format!("/proc/{vmm}/root"))
            .expect("read the VMM's root")
            .map(|entry| {
                let entry = entry.unwrap();
                let metadata = fs::metadata(entry.path()).unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    (metadata.dev(), metadata.ino()),
                )
            })
            .collect();
        assert!(
            root.iter().any(|(_, file)| *file == disk_files[index]),
            "the VMM's root lacks its own disk: {root:?}"
        );
        assert!(
            !root.iter().any(|(_, file)| *file == disk_files[1 - index]),
            "the VMM's root holds the other box's disk: {root:?}"
        );
        let mut names: Vec<&str> = root.iter().map(|(name, _)| name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "config.json",
                "dev",
                "disk0.ext4",
                "firecracker",
                "init.ext4",
                "root.ext4",
                "vmlinux",
                "vsock",
            ],
            "the VMM's root is not its jail's list"
        );
        let dev: Vec<_> = fs::read_dir(format!("/proc/{vmm}/root/dev"))
            .expect("read the VMM's /dev")
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(dev, ["kvm"], "the VMM's /dev holds more than kvm");
        let mounts =
            fs::read_to_string(format!("/proc/{vmm}/mountinfo")).expect("read the VMM's mounts");
        for line in mounts.lines() {
            let point = line.split_whitespace().nth(4).unwrap_or_default();
            assert!(
                !point.starts_with(&home),
                "a VMM mount point is under the home: {line}"
            );
        }
        for fd in fs::read_dir(format!("/proc/{vmm}/fd")).expect("read the VMM's descriptors") {
            let fd = fd.unwrap();
            let target = fs::read_link(fd.path()).unwrap_or_default();
            let number: u32 = fd.file_name().to_string_lossy().parse().unwrap();
            if number <= 2 {
                assert_eq!(target, Path::new("/dev/null"), "the VMM's fd {number}");
            }
            assert!(
                !target.starts_with(&home),
                "the VMM holds fd {number} on {}",
                target.display()
            );
        }
    }

    // SIGKILL of `box up` ends its VM; the other box keeps running.
    // SAFETY: pidfd_open returns a new descriptor or -1.
    let pidfd = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_open, vmms[0] as i32, 0u32) };
    assert!(
        pidfd >= 0,
        "pidfd_open the first VMM: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: a new descriptor, owned here alone.
    let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd as i32) };
    ups[0].kill();
    let mut ended = [nix::libc::pollfd {
        fd: pidfd.as_raw_fd(),
        events: nix::libc::POLLIN,
        revents: 0,
    }];
    // SAFETY: poll on one owned descriptor.
    let ready = unsafe { nix::libc::poll(ended.as_mut_ptr(), 1, 10_000) };
    assert_eq!(ready, 1, "the VM outlived its box up's SIGKILL");
    ups[0].wait();
    let other = box_exec(&env, &names[1], &["true"]);
    assert_ok(&other, "the other box after the first's owner died");
    ups[1].down(&env);
    run_ok(env.command(pinfold()).args(["box", "prune"]));
}

#[test]
fn only_allowlisted_hosts_get_through() {
    let _runtime = crate::shared_runtime();
    // Sabotage: make the proxy's allowlist check accept every host; example.com
    // then answers and the 403 and "not allowlisted" log assertions fail. The
    // api.github.com request is the positive control that the same path lets
    // an allowlisted host through.
    // Sabotage: match suffix entries as exact names; api.github.com fails.
    // Sabotage: match a suffix without its leading dot; evilgithub.com
    // passes policy, so its 403 and "not allowlisted" assertions fail.
    // Sabotage: drop `egress_log` from `ready`, or report a path other than
    // the log handed to the proxy; the refusal is not found there.
    let env = TestEnv::new("egress");
    let name = box_name("egress");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "egress": { "allow": [".github.com"] },
    });
    let up = box_up(&env, &spec, &name);

    let allowed = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "host admitted by .github.com");

    // evilgithub.com must stop at policy, before DNS or a public dial.
    for host in ["example.com", "evilgithub.com"] {
        let denied = curl(
            &env,
            &name,
            "30",
            &["-o", "/dev/null", &format!("https://{host}/")],
        );
        assert_denied(&denied, "403", &format!("a request to {host}"));
    }

    // The log at the path `ready` names holds the refusal and its reason.
    let log = up.ready["egress_log"]
        .as_str()
        .unwrap_or_else(|| panic!("ready names no egress log: {}", up.ready));
    let lines = json_lines(&fs::read_to_string(log).expect("read the egress log ready names"));
    for host in ["example.com", "evilgithub.com"] {
        assert!(
            lines.iter().any(|line| line["host"] == host
                && line["decision"] == "refused"
                && line["reason"] == "not allowlisted"),
            "no not-allowlisted refusal for {host}: {lines:?}"
        );
    }

    up.down(&env);
}

#[test]
fn the_proxy_refuses_the_tricks() {
    let _runtime = crate::shared_runtime();
    // Guarantee 3. Each trick has its control in the same box.
    //
    // Sabotage: delete either IP literal check; that request then
    // logs "not allowlisted" (or reaches the box's own loopback), so the
    // "ip literal" assertions fail.
    // Sabotage: stop stripping IPv6 brackets in network::literal; [::1]
    // logs "not allowlisted" instead of "ip literal".
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
    // raw request reaches the fixture, so the 400 and "ambiguous
    // framing" assertions fail. Sabotage: parse Content-Length with
    // `str::parse::<u64>` alone; `+5` is read as 5, the request is
    // forwarded, and the framing-refusal count falls short.
    let env = TestEnv::new("tricks");
    let fixture = HttpFixture::start(None);
    let name = box_name("tricks");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "egress": {
            "allow": ["api.github.com", "localhost"],
            "routes": { "fixture.internal": fixture.route() },
        },
    });
    let up = box_up(&env, &spec, &name);

    // Controls: the same paths work when the trick is not played.
    let connect = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&connect, "allowlisted CONNECT");
    let plain = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "http://api.github.com/"],
    );
    assert_ok(&plain, "allowlisted plain HTTP");
    let route = curl(&env, &name, "5", &["http://fixture.internal/"]);
    assert_eq!(route.code, 0, "route control failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "route control answered: {}",
        route.stdout
    );
    // The framing parser also accepts a body whose length is all digits.
    // The host fixture observes the bytes; a parser that rejects every
    // Content-Length cannot pass the malformed-request assertions alone.
    let body = box_exec(
        &env,
        &name,
        &[
            "bash",
            "-c",
            "exec 3<>/dev/tcp/127.0.0.1/3128; \
             printf 'POST http://fixture.internal/ HTTP/1.1\\r\\nHost: fixture.internal\\r\\nContent-Length: 5\\r\\n\\r\\nhello' >&3; \
             cat <&3",
        ],
    );
    assert_ok(&body, "valid Content-Length body");
    assert!(body.stdout.starts_with("HTTP/1.1 200"), "{}", body.stdout);
    assert_eq!(
        fixture.requests().last().expect("fixture saw POST").1,
        "hello"
    );

    // Distinct literals let each request form prove its own log reason.
    let literal_connect = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://127.0.0.1/"],
    );
    assert_denied(&literal_connect, "403", "CONNECT to an IP literal");
    let literal_ipv6 = curl(&env, &name, "30", &["-o", "/dev/null", "https://[::1]/"]);
    assert_denied(&literal_ipv6, "403", "CONNECT to a bracketed IPv6 literal");
    let literal_plain = curl(
        &env,
        &name,
        "30",
        &["-f", "-o", "/dev/null", "http://127.0.0.2/"],
    );
    assert_denied(&literal_plain, "403", "plain HTTP to an IP literal");

    // A name that resolves to loopback.
    let loopback = curl(
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

    // CONNECT to a route name.
    let route_connect = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://fixture.internal/"],
    );
    assert_denied(&route_connect, "403", "CONNECT to a route");

    // Ambiguous framing, sent raw because curl will not: two Content-Length
    // headers, and one whose value is not RFC 9110's 1*DIGIT. The `+5`
    // request carries five body bytes, so a proxy that reads it as 5
    // forwards it and the exchange still ends.
    let framings = [
        "Content-Length: 0\\r\\nContent-Length: 0\\r\\n\\r\\n",
        "Content-Length: +5\\r\\n\\r\\nhello",
    ];
    for framing in framings {
        let answer = box_exec(
            &env,
            &name,
            &[
                "bash",
                "-c",
                &format!(
                    "exec 3<>/dev/tcp/127.0.0.1/3128; \
                     printf 'POST http://fixture.internal/ HTTP/1.1\\r\\nHost: fixture.internal\\r\\n{framing}' >&3; \
                     cat <&3"
                ),
            ],
        );
        assert!(
            answer.stdout.contains("400"),
            "ambiguous framing {framing:?} got no 400: {}",
            answer.stdout
        );
    }

    // Every refusal names its own reason, distinct from "not allowlisted".
    let lines = json_lines(&egress_log(&up.ready));
    for (host, reason) in [
        ("127.0.0.1", "ip literal"),
        ("[::1]", "ip literal"),
        ("127.0.0.2", "ip literal"),
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
    let framing_refusals = lines
        .iter()
        .filter(|line| line["decision"] == "refused" && line["reason"] == "ambiguous framing")
        .count();
    assert_eq!(
        framing_refusals,
        framings.len(),
        "not one ambiguous framing refusal per request: {lines:?}"
    );

    up.down(&env);
}

#[test]
fn every_build_reruns_its_steps() {
    let _runtime = crate::shared_runtime();
    // Every build reruns every step, so a rebuild picks up base updates
    // instead of replaying a cached `RUN` layer.
    //
    // Sabotage: drop `--no-cache` from Apple's build argv, or
    // `--layers=false` from podman's. The second build then serves `/stamp`
    // from the first build's layer, the two values match, and podman's
    // untagged-image assertion fails.
    let env = TestEnv::new("rerun");
    default_image(&env);

    let profile = format!("e2e-rerun-{}", std::process::id());
    let layer_label = format!("dev.example.rerun={profile}");
    let _images = ImageCleanup {
        repository: format!("localhost/pinfold/profile-{profile}"),
    };
    // Random bytes at build time: a cached `RUN` layer replays the first
    // build's file, so the two builds agree only when the cache is reused.
    profile_containerfile(
        &env,
        &profile,
        &format!(
            "FROM debian:trixie-slim\nLABEL dev.example.rerun={profile}\nRUN head -c8 /dev/urandom | od -An -tx1 > /stamp\n"
        ),
    );

    // Podman's cached intermediates inherit the Containerfile's first
    // label. Scope to this source so another test's cached caller build
    // cannot look like our leak. Compare IDs so removing old layers cannot
    // hide a newly leaked layer.
    let before = if cfg!(target_os = "linux") {
        Some(untagged_images(&layer_label))
    } else {
        None
    };

    build_profile(&env, &profile);
    let first = built_unique_ref(&profile);
    let first_stamp = file_from_image(&env, &first, "rerun-1", "/stamp");
    // Positive control: the first build ran the `RUN` step and wrote /stamp.
    assert!(
        !first_stamp.trim().is_empty(),
        "the first build left no /stamp"
    );

    build_profile(&env, &profile);
    let second = built_unique_ref(&profile);
    let second_stamp = file_from_image(&env, &second, "rerun-2", "/stamp");
    assert_ne!(
        first_stamp, second_stamp,
        "the second build reused the first build's RUN layer"
    );

    if let Some(before) = before {
        let after = untagged_images(&layer_label);
        assert!(
            after.is_subset(&before),
            "the builds left new untagged images: {:?}",
            after.difference(&before).collect::<Vec<_>>()
        );
    }
}

#[test]
fn a_caller_builds_an_image_from_its_own_tree() {
    let _runtime = crate::shared_runtime();
    // Guarantee 23: a caller builds an image from its own tree.
    //
    // Sabotage: tag the build but skip the `--context` argument, so the
    // runtime gets no context holding marker.txt; the COPY fails and the
    // first `built` assertion fails. Sabotage: set the caller image window
    // to zero; the third build removes the first build's ref, so the ref's
    // `up` is refused `image-missing`. Sabotage: make the new build's `id`
    // in `image::build` a fixed string; the three builds then share one
    // ref, which names the third build, and the box reads `third`.
    // Sabotage: in `image::build`, resolve the reported id from `latest`
    // before the build; the first build's id is absent and each later
    // build's is stale, so the `built["id"]` assertion fails. Sabotage: write a unique
    // build label into every image again, as `dev.pinfold.build` was; the
    // repeated build makes a new image and the one-id assertion fails.
    let env = TestEnv::new("image-build");
    let other = cfg!(target_os = "macos").then(|| TestEnv::new("image-colors"));
    let base = default_image(&env);

    let name = format!("pinfold-e2e-{}", std::process::id());
    let repository = format!("localhost/pinfold/image-{name}");
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
        format!(
            "FROM {base}\nRUN head -c8 /dev/urandom | od -An -tx1 > /stamp\nCOPY marker.txt /marker.txt\n"
        ),
    )
    .unwrap();

    let mut refs: Vec<String> = Vec::new();
    for marker in ["first", "second", "third"] {
        fs::write(context.join("marker.txt"), format!("{marker}\n")).unwrap();
        let (code, built) = image_build(&env, &name, &containerfile, &context);
        assert_eq!(built["event"], "built", "the {marker} build: {built}");
        assert_eq!(code, 0, "the {marker} build exited {code}");
        let reference = built["ref"]
            .as_str()
            .unwrap_or_else(|| panic!("built carries no ref: {built}"))
            .to_string();
        assert_eq!(built["labels"]["dev.example.test"], "image");
        // The runtime must hold the ref, or a missing ref, latest and a null
        // id compare equal. Sabotage: drop the build's tag arguments; neither
        // ref resolves and this assertion fails.
        let id =
            image_id(&reference).unwrap_or_else(|| panic!("the runtime has no image {reference}"));
        assert_eq!(
            image_id(&latest).as_deref(),
            Some(id.as_str()),
            "{latest} does not name the {marker} build"
        );
        assert_eq!(
            built["id"].as_str(),
            Some(id.as_str()),
            "the {marker} built line names another image id"
        );
        refs.push(reference);
    }

    let names_ids = || tagged_images(&repository);
    let kept = names_ids();
    // A caller's `built` ref is a handle for later, so later builds of the
    // name neither reclaim nor move it: a box started from the first
    // build's ref reads the file that build COPYed.
    // On Linux the box's labels hold the image's too, the build's
    // `dev.example.test` among them, which the spec does not set.
    // Sabotage: record only the plan's labels in the Linux inventory; the
    // label is missing from `ready`.
    let first = box_name("image");
    let up = box_up(
        &env,
        &serde_json::json!({ "name": first, "image": refs[0] }),
        &first,
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        up.ready()["labels"]["dev.example.test"],
        "image",
        "the box lacks its image's label: {}",
        up.ready()
    );
    let read = box_exec(&env, &first, &["cat", "/marker.txt"]);
    assert_ok(&read, "reading the first build's marker");
    assert_eq!(
        read.stdout, "first\n",
        "the first build's ref names another build"
    );
    up.down(&env);

    // A failed build carries a bounded tail and makes no image. Sabotage:
    // collect the full build output or cap only its line count; the giant
    // unterminated line exceeds 64 KiB. Keeping only the beginning loses
    // the final marker, whose value comes from the caller's context file.
    let failing = env.root.join("Containerfile.fail");
    let final_marker = "pinfold-final-build-diagnostic";
    fs::write(context.join("diagnostic-marker.txt"), final_marker).unwrap();
    fs::write(
        &failing,
        format!(
            "FROM {base}\nCOPY diagnostic-marker.txt /diagnostic-marker\n\
             RUN head -c 200000 /dev/zero | tr '\\0' '\\377'; printf '\\n'; cat /diagnostic-marker; false\n"
        ),
    )
    .unwrap();
    let (code, failed) = image_build(&env, &name, &failing, &context);
    assert_eq!(failed["event"], "failed", "a failing build: {failed}");
    assert_eq!(code, 1, "a failed build exited {code}");
    let log = failed["log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().expect("build log string"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(log.len() <= 64 * 1024, "failed build log exceeds 64 KiB");
    assert!(
        log.contains(final_marker),
        "failed build lost its tail: {failed}"
    );
    assert_eq!(
        names_ids(),
        kept,
        "the failed build changed the name's images"
    );

    // Two builds of unchanged inputs share one image. A build identical to
    // the second, after the changed third, gets its own ref but the
    // second's id, and latest moves back to that image.
    fs::write(context.join("marker.txt"), "second\n").unwrap();
    let (code, again) = image_build(&env, &name, &containerfile, &context);
    assert_eq!(again["event"], "built", "the repeated build: {again}");
    assert_eq!(code, 0, "the repeated build exited {code}");
    let again_ref = again["ref"]
        .as_str()
        .unwrap_or_else(|| panic!("built carries no ref: {again}"))
        .to_string();
    assert_ne!(again_ref, refs[1], "the repeated build reused a ref");
    let second_id = image_id(&refs[1]);
    assert!(second_id.is_some(), "the second build's ref is gone");
    assert_eq!(
        image_id(&again_ref),
        second_id,
        "unchanged inputs made a new image instead of the second build's"
    );
    assert_eq!(
        image_id(&latest),
        second_id,
        "{latest} did not move back to the second build's image"
    );

    // Sabotage: ignore `Build::cache` in `image::build`; the runtime
    // reuses the cached RUN layer and /stamp stays equal. The cached build
    // is the positive control; the bytes come from the guest's urandom.
    let first_stamp = file_from_image(&env, &refs[0], "caller-stamp-1", "/stamp");
    let cached_stamp = file_from_image(&env, &again_ref, "caller-stamp-cached", "/stamp");
    assert!(
        !first_stamp.trim().is_empty(),
        "the caller's RUN wrote no stamp"
    );
    assert_eq!(
        first_stamp, cached_stamp,
        "the caller build skipped its cache"
    );
    let (code, fresh) = one_json_line(
        env.command(pinfold())
            .args(["image", "build", &name, "--no-cache", "--containerfile"])
            .arg(&containerfile)
            .arg("--context")
            .arg(&context),
    );
    assert_eq!(code, 0, "the caller's uncached build failed: {fresh}");
    let fresh_stamp = file_from_image(
        &env,
        fresh["ref"].as_str().expect("the uncached build has a ref"),
        "caller-stamp-fresh",
        "/stamp",
    );
    assert_ne!(cached_stamp, fresh_stamp, "--no-cache reused the RUN layer");

    // Two callers with opposite colour settings share Apple's builder.
    // Hold one inside RUN before the other starts. Sabotage: inherit
    // NO_COLOR or BUILDKIT_COLORS in Apple's build command again; the
    // second caller replaces the builder and the held build fails.
    if let Some(other) = other.as_ref() {
        let held_name = format!("{name}-held");
        let _held_images = ImageCleanup {
            repository: format!("localhost/pinfold/image-{held_name}"),
        };
        let held = HeldBuild::start(
            other,
            &held_name,
            base,
            &[("NO_COLOR", "1"), ("BUILDKIT_COLORS", "run=1,2,3")],
        );
        let (code, built) = one_json_line(
            env.command(pinfold())
                .env_remove("NO_COLOR")
                .env_remove("BUILDKIT_COLORS")
                .args(["image", "build", &name, "--containerfile"])
                .arg(&containerfile)
                .arg("--context")
                .arg(&context),
        );
        let held = held.finish();
        assert_eq!(code, 0, "the concurrent build: {built}");
        assert_eq!(built["event"], "built");
        assert!(image_id(built["ref"].as_str().unwrap()).is_some());
        assert!(held.status.success(), "the held caller's build: {held:?}");
        let held: serde_json::Value = serde_json::from_slice(&held.stdout).unwrap();
        assert_eq!(held["event"], "built");
        assert!(image_id(held["ref"].as_str().unwrap()).is_some());
    }
}

/// A real Apple build held inside RUN until its host HTTP fixture answers.
pub(super) struct HeldBuild {
    child: Option<Child>,
    release: Option<std::sync::mpsc::Sender<()>>,
    server: Option<std::thread::JoinHandle<()>>,
}

impl HeldBuild {
    pub(super) fn start(env: &TestEnv, name: &str, base: &str, vars: &[(&str, &str)]) -> HeldBuild {
        let network = run_ok(Command::new("container").args(["network", "ls", "--format", "json"]));
        let networks: serde_json::Value = serde_json::from_slice(&network.stdout).unwrap();
        let gateway = networks
            .as_array()
            .unwrap()
            .iter()
            .find(|network| network["id"] == "default")
            .unwrap()["status"]["ipv4Gateway"]
            .as_str()
            .unwrap();
        let listener = std::net::TcpListener::bind((gateway, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (active_tx, active_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(60)))
                .unwrap();
            let mut request = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                assert_ne!(request.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            active_tx.send(()).unwrap();
            let _ = release_rx.recv();
            request
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
        });
        let context = env.root.join("held-build-context");
        fs::create_dir_all(&context).unwrap();
        let containerfile = context.join("Containerfile");
        fs::write(
            &containerfile,
            format!("FROM {base}\nRUN curl --fail --max-time 60 http://{address}/\n"),
        )
        .unwrap();
        let mut child = env
            .command(pinfold())
            .envs(vars.iter().copied())
            .args(["image", "build", name, "--containerfile"])
            .arg(&containerfile)
            .arg("--context")
            .arg(&context)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Err(error) = active_rx.recv_timeout(std::time::Duration::from_secs(60)) {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("build did not reach the host fixture: {error}; {output:?}");
        }
        HeldBuild {
            child: Some(child),
            release: Some(release_tx),
            server: Some(server),
        }
    }

    pub(super) fn finish(mut self) -> std::process::Output {
        self.release.take().unwrap().send(()).unwrap();
        self.server.take().unwrap().join().unwrap();
        self.child.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for HeldBuild {
    fn drop(&mut self) {
        // A failed assertion releases the fixture and removes its caller.
        drop(self.release.take());
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

/// Run `pinfold image build NAME` with the caller label `dev.example.test`,
/// and return its exit code and its one stdout line, parsed.
pub(super) fn image_build(
    env: &TestEnv,
    name: &str,
    containerfile: &Path,
    context: &Path,
) -> (i32, serde_json::Value) {
    one_json_line(
        env.command(pinfold())
            .args(["image", "build", name, "--containerfile"])
            .arg(containerfile)
            .arg("--context")
            .arg(context)
            .args(["--label", "dev.example.test=image"]),
    )
}

/// Run `pinfold image rm NAME` and return its exit code and its one stdout
/// line, parsed.
pub(super) fn image_rm(env: &TestEnv, name: &str) -> (i32, serde_json::Value) {
    one_json_line(env.command(pinfold()).args(["image", "rm", name]))
}

/// Run `command` and return its exit code and its one stdout line, parsed.
fn one_json_line(command: &mut Command) -> (i32, serde_json::Value) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("run {command:?}: {error}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = json_lines(&stdout);
    assert_eq!(
        lines.len(),
        1,
        "{command:?} printed {} stdout lines: {stdout}\nstderr: {}",
        lines.len(),
        String::from_utf8_lossy(&output.stderr)
    );
    (exit_code(output.status), lines.remove(0))
}

/// The file at `path` in the image `reference`, read through a box named
/// `name`.
fn file_from_image(env: &TestEnv, reference: &str, name: &str, path: &str) -> String {
    let name = box_name(name);
    let spec = serde_json::json!({ "name": name, "image": reference });
    let up = box_up(env, &spec, &name);
    let output = box_exec(env, &name, &["cat", path]);
    assert_eq!(output.code, 0, "reading {path} failed: {}", output.stderr);
    up.down(env);
    output.stdout
}

/// The unique tag of the image that `localhost/pinfold/profile-<profile>:latest` names
/// now. Each build tags its image `:latest` and one unique tag; the stable
/// tag moves to every new build, so a box pins an image by the unique tag. The unique tag is collected from
/// every runtime image entry sharing the `:latest` image's id, because a
/// runtime may list one entry per tag.
pub(super) fn built_unique_ref(profile: &str) -> String {
    let latest = format!("localhost/pinfold/profile-{profile}:latest");
    let id = image_id(&latest).unwrap_or_else(|| panic!("no image names {latest}"));
    runtime_images()
        .expect("list the runtime's images")
        .iter()
        .filter(|image| image.id == id)
        .flat_map(|image| image.names.iter())
        .find(|name| **name != latest)
        .cloned()
        .unwrap_or_else(|| panic!("the image {latest} names has no unique tag"))
}
#[test]
fn box_has_no_network_but_loopback() {
    let _runtime = crate::shared_runtime();
    // Sabotage: on Apple, drop `--network none` from the adapter's `run`
    // argv; on Linux, give the VM a network interface. The box gains an
    // interface and reaches `1.1.1.1`, so the interface and unreachable
    // assertions fail. The route to the fixture is the positive control
    // that the same box still has its one way out. Sabotage (Linux): keep
    // the boot listener on `v.sock_1000` open after boot; the guest's
    // connect to port 1000 is accepted and its reset assertion fails.
    let env = TestEnv::new("network");
    let fixture = HttpFixture::start(None);
    let name = box_name("network");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "egress": {
            "routes": { "fixture.internal": fixture.route() },
        },
    });
    let up = box_up(&env, &spec, &name);

    // Only loopback exists.
    let dev = box_exec(&env, &name, &["cat", "/proc/net/dev"]);
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

    // A public address fails at once with the kernel's reason, not a timeout.
    // The short --max-time turns a hang into a failure; -v carries the
    // kernel's reason, which curl's summary line omits.
    let public = curl(
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

    // Positive control: the route answers through the proxy while the box
    // has no network.
    let route = curl(&env, &name, "5", &["http://fixture.internal/"]);
    assert_eq!(route.code, 0, "the route failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "the route answered: {}",
        route.stdout
    );

    // The host's vsock side: only the proxy listens. The exec port is the
    // guest's own, and the boot port's listener is gone once the box is
    // ready. A reset is the host refusing; a timeout would be a dropped
    // packet.
    #[cfg(target_os = "linux")]
    {
        let connect = |port: u32| vsock_to_host(&env, &name, port);
        for port in [1024, 1000] {
            let reset = connect(port);
            assert_ok(&reset, "a vsock connect from the box");
            assert_eq!(
                reset.stdout.trim(),
                "Connection reset by peer",
                "vsock CID 2 port {port} from the box"
            );
        }
        let proxy = connect(3128);
        assert_ok(&proxy, "a vsock connect to the proxy");
        assert_eq!(proxy.stdout.trim(), "connected", "vsock CID 2 port 3128");
        let config = box_exec(&env, &name, &["sh", "-c", "zcat /proc/config.gz"]);
        assert_ok(&config, "reading the guest kernel's config");
        assert!(
            config
                .stdout
                .lines()
                .any(|line| line == "CONFIG_VIRTIO_VSOCKETS=y"),
            "the guest config lacks virtio vsock"
        );
        assert!(
            !config
                .stdout
                .lines()
                .any(|line| line.starts_with("CONFIG_VSOCKETS_LOOPBACK=")),
            "the guest kernel sets vsock loopback"
        );
    }

    up.down(&env);
}

#[test]
fn a_route_reaches_exactly_one_host_service() {
    let _runtime = crate::shared_runtime();
    // Sabotage: report the route before dialing, so `status` stays null;
    // the status assertion fails. Sabotage: log the whole request target;
    // the query sentinel then appears in the log and that assertion fails.
    let env = TestEnv::new("route");
    // The host service answers every request 404, so the logged status can
    // only be the one it sent.
    const STATUS: u16 = 404;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let route = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut request = BufReader::new(&stream);
            let mut line = String::new();
            loop {
                line.clear();
                if request.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let _ = (&stream).write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    // This one answers with the Host header it received.
    let echo = HttpFixture::start(None);
    let name = box_name("route");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "egress": {
            "routes": { "fixture.internal": route, "echo.internal": echo.route() },
        },
    });
    let mut up = box_up(&env, &spec, &name);

    // The Host header is rewritten from the absolute-form target, not
    // passed through from the client. Sabotage: forward the client's Host
    // header unchanged; the request then reaches the service as
    // evil.example and this assertion fails.
    let rewritten = curl(
        &env,
        &name,
        "5",
        &["-H", "Host: evil.example", "http://echo.internal/"],
    );
    assert_eq!(rewritten.code, 0, "the route failed: {}", rewritten.stderr);
    assert!(
        rewritten.stdout.contains("fixture host=echo.internal"),
        "the Host header was not rewritten: {}",
        rewritten.stdout
    );

    // A route reaches the service, and its log line names the method, the
    // request path without its query, and the status the service sent. The
    // method is not curl's default, so a constant GET fails (sabotage: log
    // "GET" for every route). The query's sentinel never reaches the log.
    let path = "/missing";
    let sentinel = format!("pf-sentinel-{}", std::process::id());
    let missing = curl(
        &env,
        &name,
        "5",
        &[
            "-X",
            "DELETE",
            &format!("http://fixture.internal{path}?{sentinel}"),
        ],
    );
    assert_eq!(
        missing.code, 0,
        "the missing route failed: {}",
        missing.stderr
    );
    let log = egress_log(&up.ready);
    assert!(
        !log.contains(&sentinel),
        "the egress log holds the query sentinel"
    );
    let lines = json_lines(&log);
    let line = lines
        .iter()
        .find(|line| {
            line["host"] == "fixture.internal"
                && line["decision"] == "allowed"
                && line["reason"] == "route"
                && line["path"] == path
        })
        .unwrap_or_else(|| panic!("no route line for the missing path: {lines:?}"));
    assert_eq!(line["method"], "DELETE", "the route line's method: {line}");
    assert_eq!(line["status"], STATUS, "the route line's status: {line}");
    // Sabotage: ignore an audit append error or omit its owner notification.
    // Earlier requests prove this route works with a writable log. Replacing
    // ready's regular log with a directory causes a real host write failure.
    let log_path = PathBuf::from(up.ready["egress_log"].as_str().expect("ready log path"));
    fs::remove_file(&log_path).expect("remove the audit log");
    fs::create_dir(&log_path).expect("make audit append fail");
    let failed = curl(&env, &name, "5", &["http://fixture.internal/"]);
    assert_ne!(failed.code, 0, "request succeeded without its audit record");
    assert_ne!(failed.code, 28, "audit failure only timed out the request");
    // Native exit bookkeeping may wait behind another box's startup. This
    // observes completed teardown with the same bound as other owner exits.
    let status = up.wait();
    let lines = up.starting.rest();
    assert_eq!(
        lines.last().expect("audit down line")["reason"],
        "audit-log"
    );
    assert_eq!(status.code(), Some(1), "audit failure's exit status");
    // This guard queries Apple directly and also cleans up on failure. A
    // Linux VM ends with its owner, which guarantee 43 proves.
    #[cfg(target_os = "macos")]
    {
        let runtime_box = RuntimeContainer { name: name.clone() };
        assert!(!runtime_box.listed(), "audit failure left a runtime box");
    }
}

#[test]
#[ignore = "real five-minute response deadline; run the slow gate"]
fn blocked_route_responses_release_the_upstream() {
    let _runtime = crate::shared_runtime();
    // Guarantee 34. Sabotage: remove write deadlines from the host proxy
    // and guest relay; their full buffers keep the fixture blocked until
    // its own 330-second safety deadline. This exercises a route response,
    // not CONNECT's shared activity clock. The first small response is the
    // positive control; the second client deliberately never reads fd 3.
    let env = TestEnv::new("blocked-response");
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let route = listener.local_addr().unwrap().to_string();
    let (started, start) = std::sync::mpsc::channel();
    let (ended, end) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for large in [false, true] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(330)))
                .unwrap();
            let mut request = BufReader::new(&stream);
            let mut line = String::new();
            loop {
                line.clear();
                if request.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                if line == "\r\n" {
                    break;
                }
            }
            drop(request);
            if !large {
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\ncontrol",
                    )
                    .unwrap();
                continue;
            }
            let began = std::time::Instant::now();
            started.send(began).unwrap();
            let sent = (|| -> std::io::Result<()> {
                stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1073741824\r\nConnection: close\r\n\r\n",
                )?;
                let chunk = [b'x'; 64 * 1024];
                for _ in 0..16384 {
                    stream.write_all(&chunk)?;
                }
                Ok(())
            })();
            let _ = ended.send((began.elapsed(), sent.map_err(|error| error.kind())));
        }
    });
    let name = box_name("blocked-response");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "egress": { "routes": { "fixture.internal": route } },
    });
    let mut up = box_up(&env, &spec, &name);
    let control = curl(&env, &name, "5", &["http://fixture.internal/"]);
    assert_ok(&control, "small route response");
    assert_eq!(control.stdout, "control");

    let holder = env
        .command(pinfold())
        .args([
            "box", "exec", &name, "--", "bash", "-c",
            "exec 3<>/dev/tcp/127.0.0.1/3128; \
             printf 'GET http://fixture.internal/ HTTP/1.1\\r\\nHost: fixture.internal\\r\\n\\r\\n' >&3; \
             read -r release",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start a client that does not read the response");
    let mut holder = ChildOwner::new(holder, &env, Some(up.pid()));
    holder.observe(&up.ready);
    let ready = start.recv_timeout(std::time::Duration::from_secs(10));
    let observed = ready
        .as_ref()
        .map(|_| end.recv_timeout(std::time::Duration::from_secs(330)));
    let still_held = holder
        .try_wait()
        .expect("observe the held client")
        .is_none();
    // Release and reap before asserting, so a failed deadline leaves no
    // guest exec or host runtime client behind.
    if let Some(mut input) = holder.stdin.take() {
        let _ = input.write_all(b"release\n");
    }
    let status = holder.wait_bounded();
    up.down(&env);
    assert!(up.wait().success(), "slow test's owner did not exit");
    assert!(ready.is_ok(), "fixture did not observe the held request");
    let (elapsed, sent) = observed.unwrap().expect("blocked response did not close");
    assert!(still_held, "the guest closed its own response socket early");
    assert!(status.success(), "held client failed: {status}");
    assert!(
        matches!(
            sent,
            Err(std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset)
        ),
        "fixture ended without peer closure: {sent:?}"
    );
    assert!(
        elapsed >= std::time::Duration::from_secs(295),
        "response closed early: {elapsed:?}"
    );
    assert!(
        elapsed <= std::time::Duration::from_secs(330),
        "response stayed blocked: {elapsed:?}"
    );
}

#[test]
fn an_injecting_route_keeps_the_credential_on_the_host() {
    let _runtime = crate::shared_runtime();
    // Guarantee 21. Sabotage: pass the header value through the box's
    // environment as well (spec env `ROUTE_KEY: {from:
    // PINFOLD_E2E_ROUTE_KEY}`); the environment assertion fails. The value
    // lives only in `up`'s environment; the box sends its own Authorization,
    // which the proxy replaces.
    let env = TestEnv::new("inject");
    let fixture = HttpFixture::start(None);
    let secret = format!("pf-secret-{}", std::process::id());
    let name = box_name("inject");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
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
    let up = box_up_with_env(&env, &spec, &name, &[("PINFOLD_E2E_ROUTE_KEY", &secret)]);

    // The fixture receives the header from the host, not the box's own.
    let route = curl(
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
    let requests = fixture.requests();
    let authorization: Vec<&str> = requests[0]
        .0
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
    let environment = box_exec(&env, &name, &["env"]);
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

    // The log never holds the value.
    let log = egress_log(&up.ready);
    assert!(!log.contains(&secret), "the egress log holds the value");

    up.down(&env);
}

#[test]
fn a_login_route_keeps_the_login_on_the_host() {
    let _runtime = crate::shared_runtime();
    // Guarantee 26, claude half. Sabotage: in `resolve_harness`, drop the
    // `CLAUDE_CODE_OAUTH_TOKEN` placeholder and pass `$VAR` into the box
    // instead (`Env::From { from: login.from }`); the box environment
    // assertion fails. The value lives only in `up`'s environment; the box
    // sends its own Authorization, which the proxy replaces with the login's
    // Bearer header.
    // codex half. Sabotage: forward the token into the box env (in
    // `resolve_harness`, add `CODEX_ACCESS_TOKEN` as an `Env::Exact` of the
    // token `resolve_login` returned); the codex box environment assertion
    // fails. The login lapses within 5 minutes, so the pinned host helper
    // refreshes it through the refresh fixture at `up`, and the model
    // fixture must see the refreshed token, never the lapsing one.
    // pi half, first. Sabotage: in `forward`, keep a box-sent header that
    // shares an injected header's name; pi's stored auth.json key then
    // reaches the openrouter fixture beside the host key. Sabotage: drop
    // anthropic's `strip`; the box's forged Authorization reaches the
    // anthropic fixture. Sabotage: in `run_box`, omit `-e`; pi dials
    // openrouter.ai, which this run's allowlist refuses, and `pi -p` fails
    // on the proxy's 403. Sabotage: in `Login::env`, give pi's key variable
    // `$VAR`'s value instead of the placeholder; the box environment
    // assertion fails. Sabotage: drop the per-provider check in `validate`;
    // the second openrouter route comes up ready instead of `spec`.
    let env = TestEnv::new("login");
    let openrouter = HttpFixture::start(Some((
        "text/event-stream",
        concat!(
            "data: {\"id\":\"gen-e2e\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"openai/gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"gen-e2e\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"openai/gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        ),
    )));
    let anthropic = HttpFixture::start(None);
    let openrouter_key = format!("pf-openrouter-key-{}", std::process::id());
    let anthropic_key = format!("pf-anthropic-key-{}", std::process::id());
    let keys = [
        ("PINFOLD_E2E_OPENROUTER_KEY", openrouter_key.as_str()),
        ("PINFOLD_E2E_ANTHROPIC_KEY", anthropic_key.as_str()),
    ];
    // Both providers' login routes in one profile's pinfold.toml, which
    // needs no trust, and a different openrouter key stored in pi's
    // auth.json, which pi prefers over the box's placeholder.
    let profile = "e2e-login";
    run_ok(
        env.command(pinfold())
            .args(["profile", "new", profile, "--from", "default"]),
    );
    let profile_dir = env.config.join("pinfold/profiles").join(profile);
    fs::write(
        profile_dir.join("pinfold.toml"),
        format!(
            "allow = [\"login-allow.invalid\"]\n\
             [routes]\n\
             \"openrouter.login\" = {{ login = \"pi\", provider = \"openrouter\", from = \"PINFOLD_E2E_OPENROUTER_KEY\", to = \"http://{}\" }}\n\
             \"anthropic.login\" = {{ login = \"pi\", provider = \"anthropic\", from = \"PINFOLD_E2E_ANTHROPIC_KEY\", to = \"http://{}\" }}\n",
            openrouter.route(),
            anthropic.route()
        ),
    )
    .expect("write the profile's pinfold.toml");
    let agent_dir = profile_dir.join("home/.pi/agent");
    fs::create_dir_all(&agent_dir).expect("create the profile's agent dir");
    let stored = "pf-openrouter-stored-in-box";
    fs::write(
        agent_dir.join("auth.json"),
        serde_json::json!({ "openrouter": { "type": "api_key", "key": stored } }).to_string(),
    )
    .expect("write the stored key");
    fs::write(
        profile_dir.join("Containerfile"),
        format!("FROM {}\n", default_image(&env)),
    )
    .expect("write the profile's Containerfile");
    build_profile(&env, profile);
    let _image = ImageCleanup {
        repository: format!("localhost/pinfold/profile-{profile}"),
    };
    let project = TestDir::new(&env, "pi-login");
    git(project.path(), &["init", "-q"]);

    // pi sends its stored key to the route at OpenRouter's API path; the
    // fixture gets only the host key. The model is in pi's built-in
    // openrouter catalog, so only the extension can redirect it.
    let output = env
        .command(pinfold())
        .args([
            "pi",
            "-p",
            "--provider",
            "openrouter",
            "--model",
            "openai/gpt-4o-mini",
            "reply with ok",
        ])
        .current_dir(project.path())
        .env("PINFOLD_PROFILE", profile)
        .envs(keys)
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold pi -p");
    assert!(
        output.status.success(),
        "pi -p through the openrouter login route failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = openrouter.targets();
    assert!(
        requests
            .iter()
            .any(|(target, _)| target == "/api/v1/chat/completions"),
        "pi's chat request never reached the fixture at OpenRouter's path: {:?}",
        requests
            .iter()
            .map(|(target, _)| target)
            .collect::<Vec<_>>()
    );
    for (target, headers) in &requests {
        let authorization: Vec<&str> = headers
            .iter()
            .filter(|(header, _)| header.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.as_str())
            .collect();
        assert!(
            authorization == [format!("Bearer {openrouter_key}")],
            "the openrouter fixture's Authorization at {target} is not exactly the host key"
        );
    }

    // In a running pi box: only placeholders in the environment, no key in
    // any file the box could read, and the anthropic route sets x-api-key
    // and removes a forged Authorization.
    let (run, _, pi_box) = crate::pi::PiRpc::start_with_env(
        &env,
        project.path(),
        &[("PINFOLD_PROFILE", profile), keys[0], keys[1]],
    );
    let environment = box_exec(&env, &pi_box, &["env"]);
    assert_ok(&environment, "env in the pi box");
    for variable in ["OPENROUTER_API_KEY", "ANTHROPIC_API_KEY"] {
        let placeholder = environment
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{variable}=")))
            .unwrap_or_else(|| panic!("the pi box has no {variable}"));
        assert!(!placeholder.is_empty(), "the pi box's {variable} is empty");
    }
    for key in [&openrouter_key, &anthropic_key] {
        assert!(
            !environment.stdout.contains(key.as_str()),
            "the pi box's environment holds a host key"
        );
    }
    let canary = format!("pf-canary-{}", std::process::id());
    let scan = box_exec(
        &env,
        &pi_box,
        &[
            "sh",
            "-c",
            "printf %s \"$3\" > /tmp/pf-canary; \
             grep -rlsF -e \"$1\" -e \"$2\" -e \"$3\" \
             /etc /opt/pinfold /tmp \"$HOME\"; true",
            "sh",
            &openrouter_key,
            &anthropic_key,
            &canary,
        ],
    );
    assert_eq!(
        scan.stdout.lines().collect::<Vec<_>>(),
        ["/tmp/pf-canary"],
        "the pi box's files or home disk hold a key, or the scan missed the canary: {}",
        scan.stderr
    );
    let forged = curl(
        &env,
        &pi_box,
        "5",
        &[
            "-H",
            "x-api-key: from-box",
            "-H",
            "Authorization: Bearer from-box",
            "http://anthropic.login/v1/messages",
        ],
    );
    assert_ok(&forged, "the anthropic login route");
    let requests = anthropic.requests();
    let values = |name: &str| -> Vec<String> {
        requests[0]
            .0
            .iter()
            .filter(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .collect()
    };
    assert!(
        values("x-api-key") == [anthropic_key.clone()],
        "the anthropic fixture's x-api-key is not exactly the host key"
    );
    assert_eq!(
        values("authorization"),
        Vec::<String>::new(),
        "the box's Authorization reached the anthropic fixture"
    );
    // On the host, nothing under the state root holds a key: the box's
    // state dir with pinfold's extension, the run records and egress logs.
    let mut scanned = Vec::new();
    for key in [&openrouter_key, &anthropic_key] {
        crate::pi::holding(&env.state, key.as_bytes(), &mut scanned);
    }
    for written in ["pi.ts", "egress.jsonl"] {
        assert!(
            scanned.iter().any(|path| path.ends_with(written)),
            "the state root scan read no {written}"
        );
    }
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");

    // A second pi login route for one provider is refused as `spec`; an
    // unset `from` as `login`. The runs above are the positive control.
    let refused_name = box_name("login-pi-refused");
    let route =
        |from: &str| serde_json::json!({ "login": "pi", "provider": "openrouter", "from": from });
    let twice = serde_json::json!({
        "name": refused_name,
        "image": default_image(&env),
        "harness": "pi",
        "egress": { "routes": {
            "first.login": route("PINFOLD_E2E_OPENROUTER_KEY"),
            "second.login": route("PINFOLD_E2E_OPENROUTER_KEY"),
        } },
    });
    let (code, refused) = box_up_refused(&env, &twice, &keys);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(
        refused["reason"], "spec",
        "a second openrouter route: {refused}"
    );
    let unset = serde_json::json!({
        "name": refused_name,
        "image": default_image(&env),
        "harness": "pi",
        "egress": { "routes": { "openrouter.login": route("PINFOLD_E2E_UNSET_KEY") } },
    });
    let (code, refused) = box_up_refused(&env, &unset, &keys);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(
        refused["reason"], "login",
        "an unset from variable: {refused}"
    );

    let fixture = HttpFixture::start(None);
    let secret = format!("pf-login-{}", std::process::id());
    let name = box_name("login");
    let label = "dev.example.test=login";
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "login" },
        "harness": "claude",
        "egress": {
            "routes": {
                "claude.internal": {
                    "login": "claude",
                    "from": "PINFOLD_E2E_LOGIN",
                    "to": format!("http://{}", fixture.route()),
                },
            },
        },
    });
    let mut up = box_up_with_env(&env, &spec, &name, &[("PINFOLD_E2E_LOGIN", &secret)]);

    // The fixture receives the login's Bearer token, not the box's own.
    let login = curl(
        &env,
        &name,
        "5",
        &[
            "-H",
            "Authorization: Bearer from-box",
            "http://claude.internal/",
        ],
    );
    assert_eq!(login.code, 0, "the login route failed: {}", login.stderr);
    assert!(
        login
            .stdout
            .contains(&format!("fixture host={}", fixture.route())),
        "the login route answered: {}",
        login.stdout
    );
    let requests = fixture.requests();
    let authorization: Vec<&str> = requests[0]
        .0
        .iter()
        .filter(|(header, _)| header.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_str())
        .collect();
    assert_eq!(
        authorization,
        [format!("Bearer {secret}")],
        "the fixture's Authorization headers"
    );

    // The box gets the base URL and a placeholder, never the value.
    let environment = box_exec(&env, &name, &["env"]);
    assert_eq!(environment.code, 0, "env failed: {}", environment.stderr);
    assert!(
        environment
            .stdout
            .lines()
            .any(|line| line == "ANTHROPIC_BASE_URL=http://claude.internal"),
        "the box's ANTHROPIC_BASE_URL is not the route: {}",
        environment.stdout
    );
    let placeholder = environment
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("CLAUDE_CODE_OAUTH_TOKEN="))
        .unwrap_or_else(|| {
            panic!(
                "the box has no CLAUDE_CODE_OAUTH_TOKEN: {}",
                environment.stdout
            )
        });
    assert!(
        !placeholder.is_empty() && placeholder != secret,
        "the box's CLAUDE_CODE_OAUTH_TOKEN is the login"
    );
    assert!(
        !environment.stdout.contains(&secret),
        "the box's environment holds the value"
    );

    // The egress log never holds the value.
    let log = egress_log(&up.ready);
    assert!(!log.contains(&secret), "the egress log holds the value");

    up.down(&env);
    assert!(up.wait().success(), "box up did not exit cleanly");

    // codex: a file-store login in its own CODEX_HOME whose access token
    // lapses in 2 minutes, and a refresh endpoint that answers with a new
    // token for the same account, hours out.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970")
        .as_secs();
    let account = format!("pf-account-{}", std::process::id());
    let token = |exp: u64| {
        unsigned_jwt(&serde_json::json!({
            "exp": exp,
            "https://api.openai.com/auth": { "chatgpt_account_id": account },
        }))
    };
    let lapsing = token(now + 120);
    let refreshed = token(now + 4 * 3600);
    let refresh_token = format!("pf-refresh-{}", std::process::id());
    let codex_home = TestDir::new(&env, "codex-home");
    let auth = serde_json::json!({
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": token(now + 4 * 3600),
            "access_token": lapsing,
            "refresh_token": refresh_token,
            "account_id": account,
        },
    });
    fs::write(codex_home.path().join("auth.json"), auth.to_string()).unwrap();
    // Sabotage: remove the login lock or put it under XDG_STATE_HOME; the
    // second caller cannot report login-busy while the first holds refresh.
    // Answer only the first real helper request. A second refresh cannot
    // complete, so both ready boxes also prove that no overlapping refresh
    // was hidden by the barrier observation.
    let answer = serde_json::json!({ "access_token": refreshed }).to_string();
    let refresh = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let refresh_url = format!("http://{}/oauth/token", refresh.local_addr().unwrap());
    let listener = refresh.try_clone().unwrap();
    let (received, request) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let responder = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(40)))
            .unwrap();
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        let mut length = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 {
                return;
            }
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        received.send(()).unwrap();
        if released
            .recv_timeout(std::time::Duration::from_secs(20))
            .is_ok()
        {
            write!(stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()).unwrap();
        }
    });
    let model = HttpFixture::start(None);
    let codex_name = box_name("login-codex");
    let codex_spec = serde_json::json!({
        "name": codex_name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "login" },
        "harness": "codex",
        "env": { "HOME": "/tmp" },
        "egress": {
            "routes": {
                "codex.internal": {
                    "login": "codex",
                    "to": format!("http://{}", model.route()),
                },
            },
        },
    });
    let codex_home_path = codex_home.path().to_str().expect("a UTF-8 temp path");
    let vars = [
        ("CODEX_HOME", codex_home_path),
        ("CODEX_REFRESH_TOKEN_URL_OVERRIDE", refresh_url.as_str()),
    ];
    let mut first = box_up_start(&env, &codex_spec, &vars);
    // A cold helper download may use the artifact's five-minute deadline,
    // followed by the helper's thirty-second request deadline.
    request
        .recv_timeout(std::time::Duration::from_secs(330))
        .expect("the first helper reached the held refresh fixture");

    // Separate state/config roots, but the cache is shared and the first
    // helper reaching the fixture proves its host artifact is installed.
    let contender = TestEnv::new("login-contender");
    let contender_name = box_name("login-contender");
    let mut contender_spec = codex_spec.clone();
    contender_spec["name"] = serde_json::json!(contender_name);
    let child = contender
        .command(pinfold())
        .args(["box", "up"])
        .envs(vars)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the contending login owner");
    let mut child = ChildOwner::new(child, &contender, None);
    let stderr = child.stderr.take().unwrap();
    let (reported, busy) = std::sync::mpsc::channel();
    let diagnostics = std::thread::spawn(move || {
        let mut count = 0;
        for line in BufReader::new(stderr).lines() {
            if line.unwrap().contains("login-busy") {
                count += 1;
                let _ = reported.send(());
            }
        }
        count
    });
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(&serde_json::to_vec(&contender_spec).unwrap())
        .unwrap();
    stdin.flush().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut second = Starting {
        child,
        stdin: Some(stdin),
        stdout,
    };
    busy.recv_timeout(std::time::Duration::from_secs(10))
        .expect("the second XDG root observed the shared login lock");
    refresh.set_nonblocking(true).unwrap();
    release.send(()).unwrap();
    responder.join().unwrap();
    let ready = first.first_line();
    assert_eq!(ready["event"], "ready", "first login failed: {ready}");
    let mut up = first.into_up(&codex_name, ready);
    let ready = second.first_line();
    assert_eq!(ready["event"], "ready", "contending login failed: {ready}");
    let mut other_up = second.into_up(&contender_name, ready);

    // The model fixture receives the refreshed token and its account, not
    // the box's own Authorization.
    let login = curl(
        &env,
        &codex_name,
        "5",
        &[
            "-H",
            "Authorization: Bearer from-box",
            "http://codex.internal/backend-api/codex/responses",
        ],
    );
    assert_eq!(
        login.code, 0,
        "the codex login route failed: {}",
        login.stderr
    );
    assert!(
        login
            .stdout
            .contains(&format!("fixture host={}", model.route())),
        "the codex login route answered: {}",
        login.stdout
    );
    let requests = model.requests();
    let values = |name: &str| -> Vec<String> {
        requests[0]
            .0
            .iter()
            .filter(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .collect()
    };
    assert!(
        values("authorization") == [format!("Bearer {refreshed}")],
        "the model fixture's Authorization is not the refreshed token"
    );
    assert_eq!(
        values("chatgpt-account-id"),
        [account.as_str()],
        "the model fixture's account header"
    );

    let login = curl(
        &contender,
        &contender_name,
        "5",
        &["http://codex.internal/backend-api/codex/responses"],
    );
    assert_ok(&login, "the contending caller's login route");
    let requests = model.requests();
    let headers = &requests.last().unwrap().0;
    for (name, expected) in [
        ("authorization", format!("Bearer {refreshed}")),
        ("chatgpt-account-id", account.clone()),
    ] {
        assert_eq!(
            headers
                .iter()
                .filter(|(header, _)| header.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>(),
            [expected.as_str()],
            "the contending caller used the wrong {name}"
        );
    }
    other_up.down(&contender);
    assert!(other_up.wait().success(), "the contending owner failed");
    assert_eq!(
        diagnostics.join().unwrap(),
        1,
        "login-busy was not once per ask"
    );
    assert_eq!(
        refresh.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "the contending helper reused the refresh token"
    );

    // Neither token is in the box's environment, its files or the egress
    // log.
    let environment = box_exec(&env, &codex_name, &["env"]);
    assert_eq!(environment.code, 0, "env failed: {}", environment.stderr);
    for (what, value) in [("lapsing", &lapsing), ("refreshed", &refreshed)] {
        assert!(
            !environment.stdout.contains(value.as_str()),
            "the codex box's environment holds the {what} token"
        );
    }
    // Every file the box could hold them in: pinfold's config and harness,
    // and the writable paths, `$HOME` among them. A canary written to
    // /tmp first is the control that the scan runs and reads files; it is
    // the only file the scan may list. /proc, /sys and /usr are left out for
    // speed; none is a place pinfold writes.
    let canary = format!("pf-canary-{}", std::process::id());
    let scan = box_exec(
        &env,
        &codex_name,
        &[
            "sh",
            "-c",
            "printf %s \"$3\" > /tmp/pf-canary; \
             grep -rlsF -e \"$1\" -e \"$2\" -e \"$3\" \
             /etc /opt/pinfold /tmp; true",
            "sh",
            &lapsing,
            &refreshed,
            &canary,
        ],
    );
    assert_eq!(
        scan.stdout.lines().collect::<Vec<_>>(),
        ["/tmp/pf-canary"],
        "the codex box's files hold a token, or the scan missed the canary: {}",
        scan.stderr
    );
    let log = egress_log(&up.ready);
    for (what, value) in [("lapsing", &lapsing), ("refreshed", &refreshed)] {
        assert!(
            !log.contains(value.as_str()),
            "the egress log holds the {what} token"
        );
    }
    up.down(&env);
    assert!(up.wait().success(), "box up did not exit cleanly");

    // Sabotage: remove the helper deadline or cancellation checks; a real
    // helper waiting on this refresh service does not close its socket or
    // finish up within the spec's bound. Omit kill/wait and the observed
    // helper PID remains alive after up finishes. The successful real
    // refresh above is the positive control. All timing and process
    // observations come from the host, not helper output.
    for cancelled in [false, true] {
        fs::write(codex_home.path().join("auth.json"), auth.to_string()).unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let refresh_url = format!("http://{}/oauth/token", listener.local_addr().unwrap());
        let (accepted, request) = std::sync::mpsc::channel();
        let (closed, eof) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(40)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let mut length = 0usize;
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            accepted.send(()).unwrap();
            // Never answer. EOF is the outside observer that the real
            // helper stopped its unfinished HTTP operation.
            let mut byte = [0u8; 1];
            let ended = matches!(reader.read(&mut byte), Ok(0));
            let _ = closed.send(ended);
        });
        let began = std::time::Instant::now();
        let mut starting = box_up_start(
            &env,
            &codex_spec,
            &[
                ("CODEX_HOME", codex_home_path),
                ("CODEX_REFRESH_TOKEN_URL_OVERRIDE", &refresh_url),
            ],
        );
        let owner = starting.child.id();
        let mut output = starting.stdout;
        let (finished, events) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            let result = output.read_to_string(&mut text).map(|_| text);
            let _ = finished.send(result);
        });
        if request
            .recv_timeout(std::time::Duration::from_secs(15))
            .is_err()
        {
            let _ = starting.child.kill();
            let _ = reap_killed(&mut starting.child);
            panic!("the real helper never reached the stalled refresh fixture");
        }
        let processes = run_ok(Command::new("ps").args(["-axo", "pid=,ppid=,comm="]));
        let helper = String::from_utf8(processes.stdout)
            .unwrap()
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?;
                let parent = fields.next()?.parse::<u32>().ok()?;
                let executable = fields.next()?;
                if parent != owner {
                    return None;
                }
                // Linux comm truncates to 15 bytes; the helper name has 16.
                let is_helper = if cfg!(target_os = "linux") {
                    fs::read_link(format!("/proc/{pid}/exe"))
                        .ok()?
                        .file_name()
                        .is_some_and(|name| name == "codex-app-server")
                } else {
                    executable.ends_with("codex-app-server")
                };
                is_helper.then(|| pid.to_string())
            });
        let helper = match helper {
            Some(helper) => helper,
            None => {
                let _ = starting.child.kill();
                let _ = reap_killed(&mut starting.child);
                panic!("up has no observable real Codex helper child");
            }
        };
        if cancelled {
            run_ok(Command::new("kill").args(["-TERM", &owner.to_string()]));
        }
        let bound = std::time::Duration::from_secs(if cancelled { 5 } else { 35 });
        let result = events.recv_timeout(bound);
        let text = match result {
            Ok(Ok(text)) => text,
            _ => {
                let _ = starting.child.kill();
                let _ = reap_killed(&mut starting.child);
                panic!("stalled helper kept up alive past its completion bound");
            }
        };
        let status = starting.child.wait_bounded();
        drop(starting.stdin);
        let lines = json_lines(&text);
        if cancelled {
            assert_eq!(lines.last().unwrap()["reason"], "signal");
            assert!(status.success(), "cancelled login up failed: {status}");
        } else {
            assert_eq!(lines.first().unwrap()["event"], "refused");
            assert_eq!(lines.first().unwrap()["reason"], "login");
            assert!(!status.success(), "stalled login was accepted");
            assert!(began.elapsed() < std::time::Duration::from_secs(35));
        }
        assert!(
            eof.recv_timeout(std::time::Duration::from_secs(1)).unwrap(),
            "the helper left its refresh connection open"
        );
        let remains = Command::new("kill")
            .args(["-0", &helper])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!remains.success(), "up left the Codex helper running");
        assert_left_nothing(&env, &codex_name, label, "cancelled or timed-out login");
    }

    // An empty CODEX_HOME has no login: up is refused as `login` and leaves
    // nothing. The box above is its positive control.
    let empty = TestDir::new(&env, "codex-empty");
    let empty_path = empty.path().to_str().expect("a UTF-8 temp path");
    let (code, refused) = box_up_refused(&env, &codex_spec, &[("CODEX_HOME", empty_path)]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "login");
    assert_left_nothing(&env, &codex_name, label, "refused");

    // Live, on the Apple runtime only: the operator's own Codex login, and
    // pinned codex completes one shell tool round trip through a login route
    // to chatgpt.com, stdin closed. The tool's output is computed by the
    // shell, so it is in no prompt; the model's prose is never asserted.
    // The gate runs with its own HOME, so the caller hands `up` the
    // operator's Codex home: $CODEX_HOME, else .codex in the passwd home,
    // which the gate resolves the same way.
    if cfg!(target_os = "macos") {
        let operator_codex_home = std::env::var("CODEX_HOME").unwrap_or_else(|_| {
            let home = run_ok(Command::new("sh").args(["-c", "eval echo \"~$(id -un)\""]));
            format!("{}/.codex", String::from_utf8_lossy(&home.stdout).trim())
        });
        let live_name = box_name("login-live");
        let live_spec = serde_json::json!({
            "name": live_name,
            "image": default_image(&env),
            "labels": { "dev.example.test": "login" },
            "harness": "codex",
            "env": { "HOME": "/tmp" },
            "egress": { "routes": { "codex.internal": { "login": "codex" } } },
        });
        let mut up = box_up_with_env(
            &env,
            &live_spec,
            &live_name,
            &[("CODEX_HOME", &operator_codex_home)],
        );
        let run = box_exec(
            &env,
            &live_name,
            &[
                "/opt/pinfold/codex/codex",
                "exec",
                "--json",
                "--skip-git-repo-check",
                "--sandbox",
                "danger-full-access",
                "-C",
                "/tmp",
                "Run this exact shell command once and reply with its output: echo pf-$((40+2))-live",
            ],
        );
        assert_ok(&run, "codex exec through the login route");
        let ran = json_lines(&run.stdout).iter().any(|event| {
            event["item"]["type"] == "command_execution"
                && event["item"]["aggregated_output"]
                    .as_str()
                    .is_some_and(|output| output.contains("pf-42-live"))
        });
        assert!(
            ran,
            "codex ran no shell command that printed pf-42-live: {}",
            run.stdout
        );
        up.down(&env);
        assert!(up.wait().success(), "box up did not exit cleanly");
    }
}

/// An unsigned JWT (`alg: none`) carrying `claims`, as a login's token.
fn unsigned_jwt(claims: &serde_json::Value) -> String {
    let encode = |bytes: &[u8]| -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut text = String::new();
        for chunk in bytes.chunks(3) {
            let word = chunk.iter().enumerate().fold(0u32, |word, (index, byte)| {
                word | (u32::from(*byte) << (16 - 8 * index))
            });
            for index in 0..=chunk.len() {
                text.push(char::from(
                    ALPHABET[((word >> (18 - 6 * index)) & 63) as usize],
                ));
            }
        }
        text
    };
    let header = encode(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = encode(claims.to_string().as_bytes());
    format!("{header}.{payload}.{}", encode(b"unsigned"))
}

#[test]
fn no_egress_means_no_way_out() {
    let _runtime = crate::shared_runtime();
    // Sabotage: start the proxy and relay even without `egress` in the spec;
    // the explicit-proxy request then reaches the proxy instead of a refused
    // connection, and the refusal assertions fail. The same request with the
    // route in the spec is the positive control. Sabotage (Linux): listen on
    // the VM's host proxy port, `v.sock_3128`, even without `egress`; the
    // guest's vsock connect to CID 2 port 3128 is accepted and its reset
    // assertion fails. The control box's connect is the positive control.
    let env = TestEnv::new("no-egress");
    let fixture = HttpFixture::start(None);
    let name = box_name("no-egress");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
    });
    let up = box_up(&env, &spec, &name);

    // No relay: a request sent to the proxy's port finds no listener.
    let proxied = curl(
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
    // Past the guest's own network: the host's proxy port on vsock.
    #[cfg(target_os = "linux")]
    {
        let reset = vsock_to_host(&env, &name, 3128);
        assert_ok(&reset, "a vsock connect from the box");
        assert_eq!(
            reset.stdout.trim(),
            "Connection reset by peer",
            "vsock CID 2 port 3128 without egress"
        );
    }

    up.down(&env);
    drop(up);

    // Positive control: the same request with the route in the spec answers.
    let name = box_name("no-egress-control");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "egress": {
            "routes": { "fixture.internal": fixture.route() },
        },
    });
    let up = box_up(&env, &spec, &name);
    let route = curl(&env, &name, "5", &["http://fixture.internal/"]);
    assert_eq!(route.code, 0, "the control route failed: {}", route.stderr);
    assert!(
        route.stdout.contains("fixture host=fixture.internal"),
        "the control route answered: {}",
        route.stdout
    );
    #[cfg(target_os = "linux")]
    {
        let proxy = vsock_to_host(&env, &name, 3128);
        assert_ok(&proxy, "a vsock connect to the proxy");
        assert_eq!(proxy.stdout.trim(), "connected", "vsock CID 2 port 3128");
    }

    up.down(&env);
}

#[test]
fn a_caller_can_tell_an_oom_kill_from_a_failure() {
    let _runtime = crate::shared_runtime();
    // Guarantee 20: a caller can tell an OOM kill from a failure.
    // Sabotage: read `memory.events` but report `high` instead of `oom_kill`;
    // with no memory.high set the count stays 0 and the post-exec assertion
    // fails.
    // Sabotage: skip the oom_score_adj write in `init exec`, the exec
    // wrapper on both runtimes; `cat` then prints 0 on Apple or the exec
    // service's -1000 on Linux and the oom_score_adj assertion fails. (The original
    // flake is not usable: it needs a kernel that picks init, which the
    // macOS host and Debian do not.)
    let env = TestEnv::new("oom");
    let name = box_name("oom");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "memory": "256M",
    });
    let up = box_up(&env, &spec, &name);

    // Every key is present, and the limit is the one in force: 256 MiB. A
    // field the runtime cannot answer is null, not absent.
    let before = box_stat(&env, &name);
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
    let score = box_exec(&env, &name, &["cat", "/proc/self/oom_score_adj"]);
    assert_ok(&score, "cat /proc/self/oom_score_adj");
    assert_eq!(
        score.stdout.trim(),
        "1000",
        "exec'd process oom_score_adj: {}",
        score.stdout
    );

    // On Linux the limit in force is the VM's size, which the guest kernel
    // reports. Sabotage: size every VM at the default in `vm::config`; stat
    // still reports 256 MiB and this assertion fails.
    #[cfg(target_os = "linux")]
    assert_guest_memory(&env, &name, 256 << 20);

    if cfg!(target_os = "linux") {
        // A fresh box's cgroup has killed nothing; the caller's baseline.
        assert_eq!(before["oom_kills"].as_u64(), Some(0), "stat: {before}");
        // A command that allocates past the limit is killed; the box stays
        // up and the count rises. The command's non-zero exit must be read
        // together with stat, not as a failure.
        let killed = box_exec(&env, &name, &["sh", "-c", "head -c 1G /dev/zero | tail"]);
        assert_ne!(killed.code, 0, "the memory hog exited 0");
        let after = box_stat(&env, &name);
        let kills = after["oom_kills"]
            .as_u64()
            .unwrap_or_else(|| panic!("stat lost oom_kills: {after}"));
        assert!(kills >= 1, "stat reports no OOM kill after one: {after}");
    }

    up.down(&env);
}

#[test]
fn a_caller_owned_box_launches_the_pinned_harness() {
    let _runtime = crate::shared_runtime();
    // Guarantee 19: a caller-owned box launches the pinned harness, for
    // each of pi, claude and codex.
    // Sabotage: drop the harness mount (or mount the wrong directory); the
    // `/opt/pinfold/<name>/<name> --version` assertion fails. Sabotage:
    // mount the harness but leave `PINFOLD_ALLOW` unset; the allow list
    // assertion fails. Sabotage: install codex under its target triple's
    // name instead of `codex`; its `--version` exec fails. Sabotage:
    // restore the directory-exists check in `dirs::install_dir`; after pi's
    // executable is deleted from the cache the second box mounts the broken
    // install and its `--version` exec exits 127.
    // codex's `codex-code-mode-host` companion is not asserted: only a
    // model-driven MCP tool call observes it.
    let env = TestEnv::new("harness");

    // The versions `pinfold artifacts` pins. The caller records which
    // harness it ran from the same report.
    let output = run_ok(env.command(pinfold()).arg("artifacts"));
    let pins: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("pinfold artifacts output is JSON");

    for harness in ["pi", "claude", "codex"] {
        let pin = pins
            .iter()
            .find(|pin| pin["name"] == harness)
            .unwrap_or_else(|| panic!("pinfold artifacts does not name {harness}: {pins:?}"));
        let version = pin["version"].as_str().expect("pin version is a string");

        // A caller-owned box: the spec names the image, the harness and the
        // allow list. `$HOME` is the box's tmpfs.
        let name = box_name(&format!("harness-{harness}"));
        let spec = serde_json::json!({
            "name": name,
            "image": default_image(&env),
            "harness": harness,
            "env": { "HOME": "/tmp" },
            "egress": { "allow": ["api.github.com"] },
        });
        let mut up = box_up(&env, &spec, &name);

        // The pinned harness is in the box and runs at the pinned version.
        let binary = format!("/opt/pinfold/{harness}/{harness}");
        let ran = box_exec(&env, &name, &[&binary, "--version"]);
        assert_ok(&ran, &format!("{binary} --version"));
        assert!(
            ran.stdout.contains(version),
            "the box's {harness} is not the pinned {version}: {}",
            ran.stdout
        );

        // The harness environment is the spec's allow list, exactly.
        let allow = box_exec(&env, &name, &["sh", "-c", "printf %s \"$PINFOLD_ALLOW\""]);
        assert_eq!(
            allow.stdout, "api.github.com",
            "the box's PINFOLD_ALLOW is not the spec's allow list"
        );

        up.down(&env);
        assert!(up.wait().success(), "box up did not exit cleanly");

        // The host deleted the harness's executable from the cache after
        // the install, the way the OS's temp cleaner or a partial delete
        // does. The next use must reinstall rather than mount a directory
        // whose binary is gone. Only pi: the repair downloads on the host
        // again, and one harness proves the path without spending the
        // suite's time three times. This scenario runs against a cache of
        // its own: the pi tests run in this binary beside this one and
        // mount the shared cache, which the deletion would break.
        if harness == "pi" {
            let private = TestEnv::with_private_cache("harness-damaged");

            // Install pi into the private cache and read its path there.
            let mut up = box_up(&private, &spec, &name);
            up.down(&private);
            assert!(up.wait().success(), "box up did not exit cleanly");
            let output = run_ok(private.command(pinfold()).arg("artifacts"));
            let report: Vec<serde_json::Value> =
                serde_json::from_slice(&output.stdout).expect("pinfold artifacts output is JSON");
            let pin = report
                .iter()
                .find(|pin| pin["name"] == harness)
                .expect("pinfold artifacts names pi");
            let install = pin["path"].as_str().expect("pin path is a string");
            let host_binary = Path::new(install).join(harness);
            fs::remove_file(&host_binary)
                .unwrap_or_else(|error| panic!("remove {}: {error}", host_binary.display()));

            // The second box repairs the install and runs the pinned pi.
            let mut up = box_up(&private, &spec, &name);
            let guest_binary = format!("/opt/pinfold/{harness}/{harness}");
            let ran = box_exec(&private, &name, &[&guest_binary, "--version"]);
            assert_ok(
                &ran,
                &format!("{guest_binary} --version after the cache lost it"),
            );
            assert!(
                ran.stdout.contains(version),
                "the re-installed {harness} is not the pinned {version}: {}",
                ran.stdout
            );
            up.down(&private);
            assert!(up.wait().success(), "box up did not exit cleanly");
        }
    }
}

/// The VMM of a ready box on Linux: the one child process of its `box up`,
/// across all of `up`'s threads.
#[cfg(target_os = "linux")]
pub(super) fn vmm_pid(up: u32) -> u32 {
    let mut children = Vec::new();
    for task in fs::read_dir(format!("/proc/{up}/task")).expect("read box up's threads") {
        let task = task.expect("read a box up thread").path();
        // A thread that ended since the listing has no children.
        let listed = fs::read_to_string(task.join("children")).unwrap_or_default();
        children.extend(
            listed
                .split_whitespace()
                .map(|pid| pid.parse::<u32>().unwrap()),
        );
    }
    match children.as_slice() {
        [vmm] => *vmm,
        _ => panic!("box up {up} has children {children:?}, not one VMM"),
    }
}

/// From inside the box, connect a vsock stream to the host, CID 2, on
/// `port`, and print `connected` or the error. perl-base opens the socket
/// by number: AF_VSOCK is 40 and a sockaddr_vm is family, reserved, port
/// and CID.
#[cfg(target_os = "linux")]
fn vsock_to_host(env: &TestEnv, name: &str, port: u32) -> e2e::ExecOutput {
    box_exec(
        env,
        name,
        &[
            "perl",
            "-e",
            "socket(my $s, 40, 1, 0) or die \"socket: $!\\n\"; \
             print connect($s, pack('SSLLx4', 40, 0, $ARGV[0], 2)) ? \"connected\\n\" : \"$!\\n\"",
            &port.to_string(),
        ],
    )
}

/// Assert that the guest kernel of a Firecracker box has `limit` bytes of
/// memory: its `MemTotal` is the VM's size less what the kernel keeps, and
/// any other size the VM could get (the 1 GiB default, half or twice the
/// spec's) falls outside the 64 MiB below `limit`.
#[cfg(target_os = "linux")]
pub(super) fn assert_guest_memory(env: &TestEnv, name: &str, limit: u64) {
    let meminfo = box_exec(env, name, &["cat", "/proc/meminfo"]);
    assert_ok(&meminfo, "cat /proc/meminfo");
    let total = meminfo
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|kib| kib.trim().strip_suffix("kB"))
        .and_then(|kib| kib.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no MemTotal in the guest: {}", meminfo.stdout))
        * 1024;
    assert!(
        total <= limit && total > limit - (64 << 20),
        "the guest's MemTotal {total} is not a {limit}-byte VM's"
    );
}

/// One `/proc/<pid>/status` field's value.
pub(super) fn status_field(status: &str, key: &str) -> String {
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
pub(crate) struct Up {
    name: String,
    /// The parsed `ready` line.
    ready: serde_json::Value,
    starting: Starting,
}

impl Up {
    pub(super) fn ready(&self) -> &serde_json::Value {
        &self.ready
    }

    pub(super) fn pid(&self) -> u32 {
        self.starting.child.id()
    }

    pub(super) fn wait(&mut self) -> ExitStatus {
        self.starting.child.wait_bounded()
    }

    pub(super) fn kill(&mut self) {
        self.starting.child.kill().expect("kill box up");
    }

    /// Run `box down` on this box and assert it succeeded.
    pub(super) fn down(&self, env: &TestEnv) {
        let status = box_down(env, &self.name);
        assert!(status.success(), "box down failed: {status}");
    }

    /// Close `up`'s stdin and read the rest of its stdout. `up` ends its
    /// stream once teardown is done, so reading to EOF waits for the `down`
    /// line without a sleep.
    pub(super) fn close_stdin(&mut self) -> Vec<serde_json::Value> {
        drop(self.starting.stdin.take());
        self.starting.rest()
    }
}

pub(crate) fn box_up(env: &TestEnv, spec: &serde_json::Value, name: &str) -> Up {
    box_up_with_env(env, spec, name, &[])
}

/// [`box_up`] with extra variables in `up`'s own environment.
fn box_up_with_env(
    env: &TestEnv,
    spec: &serde_json::Value,
    name: &str,
    vars: &[(&str, &str)],
) -> Up {
    let mut starting = box_up_start(env, spec, vars);
    let ready = starting.first_line();
    assert_eq!(ready["event"], "ready", "first line was {ready}");
    assert_eq!(ready["box"], name, "ready named another box: {ready}");
    starting.into_up(name, ready)
}

/// A spawned `box up` with its spec written and stdin still open, before
/// any of its output is read.
pub(super) struct Starting {
    pub(super) child: ChildOwner,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Starting {
    /// Read and parse `up`'s first line.
    pub(super) fn first_line(&mut self) -> serde_json::Value {
        // Codex has two cold artifacts, each with a five-minute transfer
        // deadline. Allow another minute for extraction and runtime startup;
        // successful readiness still comes from the owner line.
        let line = read_bounded(
            &mut self.child,
            &mut self.stdout,
            std::time::Duration::from_secs(660),
            false,
        )
        .expect("read box up's first line");
        let parsed: serde_json::Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|error| panic!("box up's first line {line:?} is not JSON: {error}"));
        self.child.observe(&parsed);
        parsed
    }

    /// Read the rest of `up`'s stdout to EOF.
    fn rest(&mut self) -> Vec<serde_json::Value> {
        let text = read_bounded(
            &mut self.child,
            &mut self.stdout,
            std::time::Duration::from_secs(60),
            true,
        )
        .expect("read box up output");
        json_lines(&text)
    }

    /// The [`Up`] of a start whose first line was `ready`.
    pub(super) fn into_up(self, name: &str, ready: serde_json::Value) -> Up {
        Up {
            name: name.to_string(),
            ready,
            starting: self,
        }
    }
}

/// Spawn `box up` and write its spec, reading nothing, so a test can start
/// several at once.
pub(super) fn box_up_start(
    env: &TestEnv,
    spec: &serde_json::Value,
    vars: &[(&str, &str)],
) -> Starting {
    let child = env
        .command(pinfold())
        .envs(vars.iter().copied())
        .args(["box", "up"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn pinfold box up");
    let mut child = ChildOwner::new(child, env, None);
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
    env: &TestEnv,
    spec: &serde_json::Value,
    vars: &[(&str, &str)],
) -> (i32, serde_json::Value) {
    let mut starting = box_up_start(env, spec, vars);
    drop(starting.stdin.take());
    let line = starting.first_line();
    let status = starting.child.wait_bounded();
    (exit_code(status), line)
}

/// Installed immediately after spawn, before any readiness assertion.
pub(crate) struct ChildOwner {
    child: Child,
    listed: Command,
    down_environment: Command,
    owner: u32,
    generation: Option<String>,
}

impl ChildOwner {
    pub(crate) fn new(child: Child, env: &TestEnv, owner: Option<u32>) -> Self {
        let owner = owner.unwrap_or(child.id());
        let mut listed = env.command(pinfold());
        listed.args([
            "box",
            "list",
            "--label",
            &format!("dev.pinfold.owner={owner}"),
        ]);
        Self {
            child,
            listed,
            down_environment: env.command(pinfold()),
            owner,
            generation: None,
        }
    }

    pub(crate) fn observe(&mut self, ready: &serde_json::Value) {
        self.generation = ready["labels"]["dev.pinfold.generation"]
            .as_str()
            .map(String::from);
    }

    fn stop(&mut self) {
        // Once reaped, only the observed generation can authorize cleanup.
        if self.child.try_wait().is_ok_and(|status| status.is_some()) && self.generation.is_none() {
            return;
        }
        // Keep this Child unreaped until its owner label is checked. Killing
        // first makes orphan down independent of a stuck owner's listener.
        let _ = self.child.kill();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        if let Ok(output) = command_bounded(&mut self.listed, deadline) {
            for box_ in String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            {
                if box_["owner"].as_u64() != Some(u64::from(self.owner)) {
                    continue;
                }
                if self.generation.as_ref().is_some_and(|generation| {
                    box_["labels"]["dev.pinfold.generation"].as_str() != Some(generation.as_str())
                }) {
                    continue;
                }
                let Some(name) = box_["name"].as_str() else {
                    continue;
                };
                let mut down = Command::new(self.down_environment.get_program());
                for (key, value) in self.down_environment.get_envs() {
                    match value {
                        Some(value) => {
                            down.env(key, value);
                        }
                        None => {
                            down.env_remove(key);
                        }
                    }
                }
                down.args(["box", "down", name]);
                let _ = command_bounded(&mut down, deadline);
            }
        }
        if let Err(error) = reap_killed(&mut self.child) {
            eprintln!("failed to reap test owner: {error}");
        }
    }

    pub(crate) fn wait_bounded(&mut self) -> ExitStatus {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if let Some(status) = self.child.try_wait().expect("observe owner completion") {
                return status;
            }
            if std::time::Instant::now() >= deadline {
                self.stop();
                panic!("owner did not complete within 60 seconds");
            }
            std::thread::yield_now();
        }
    }
}

impl std::ops::Deref for ChildOwner {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl std::ops::DerefMut for ChildOwner {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl Drop for ChildOwner {
    fn drop(&mut self) {
        // Completed failures remain observable to assert_left_nothing. A
        // refused child's name may belong to another live owner.
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            self.stop();
        }
    }
}

fn nonblocking(fd: impl AsFd) -> std::io::Result<()> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    let flags = OFlag::from_bits_truncate(fcntl(&fd, FcntlArg::F_GETFL)?);
    fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    Ok(())
}

fn reap_killed(child: &mut Child) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "killed child did not exit",
            ));
        }
        std::thread::yield_now();
    }
}

const HARNESS_OUTPUT_LIMIT: usize = 1024 * 1024;

/// Poll the existing pipe; no reader thread can retain it past the deadline.
/// A timeout has a separate bounded cleanup/reaping allowance.
pub(crate) fn read_bounded(
    child: &mut ChildOwner,
    reader: &mut BufReader<ChildStdout>,
    timeout: std::time::Duration,
    to_eof: bool,
) -> std::io::Result<String> {
    use nix::poll::{PollFd, PollFlags, poll};
    nonblocking(reader.get_ref())?;
    let deadline = std::time::Instant::now() + timeout;
    let mut bytes = Vec::new();
    loop {
        if std::time::Instant::now() >= deadline {
            child.stop();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "owner output deadline",
            ));
        }
        match reader.fill_buf() {
            Ok([]) => {
                return String::from_utf8(bytes)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error));
            }
            Ok(buffer) => {
                let newline = (!to_eof)
                    .then(|| buffer.iter().position(|byte| *byte == b'\n'))
                    .flatten();
                let count = newline.map_or(buffer.len(), |position| position + 1);
                bytes.extend_from_slice(&buffer[..count]);
                reader.consume(count);
                if bytes.len() > HARNESS_OUTPUT_LIMIT {
                    child.stop();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "owner output exceeds 1 MiB",
                    ));
                }
                if newline.is_some() {
                    return String::from_utf8(bytes).map_err(|error| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, error)
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    child.stop();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "owner output deadline",
                    ));
                }
                let millis = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
                let mut fds = [PollFd::new(reader.get_ref().as_fd(), PollFlags::POLLIN)];
                match poll(&mut fds, nix::poll::PollTimeout::try_from(millis).unwrap()) {
                    Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn command_bounded(
    command: &mut Command,
    deadline: std::time::Instant,
) -> std::io::Result<std::process::Output> {
    use nix::poll::{PollFd, PollFlags, poll};
    if std::time::Instant::now() >= deadline {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "cleanup command deadline",
        ));
    }
    let mut child = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let result = (|| {
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut out_done = false;
        let mut err_done = false;
        let mut status = None;
        loop {
            if !out_done {
                out_done = drain(&mut stdout, &mut out, deadline)?;
            }
            if !err_done {
                err_done = drain(&mut stderr, &mut err, deadline)?;
            }
            if out_done && err_done && status.is_none() {
                // Do not free the process-group leader PID while a descendant
                // can still hold either pipe. Timeout cleanup owns that group.
                status = child.try_wait()?;
            }
            if out_done
                && err_done
                && let Some(status) = status
            {
                return Ok(std::process::Output {
                    status,
                    stdout: out,
                    stderr: err,
                });
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "cleanup command deadline",
                ));
            }
            let mut fds = Vec::new();
            if !out_done {
                fds.push(PollFd::new(stdout.as_fd(), PollFlags::POLLIN));
            }
            if !err_done {
                fds.push(PollFd::new(stderr.as_fd(), PollFlags::POLLIN));
            }
            if fds.is_empty() {
                std::thread::yield_now();
                continue;
            }
            let millis = remaining.as_millis().clamp(1, 1000) as u16;
            match poll(&mut fds, millis) {
                Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
    })();
    if result.is_err() {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(child.id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = child.kill();
        reap_killed(&mut child)?;
    }
    result
}

fn drain(
    reader: &mut impl Read,
    bytes: &mut Vec<u8>,
    deadline: std::time::Instant,
) -> std::io::Result<bool> {
    let mut buffer = [0; 4096];
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "cleanup output deadline",
            ));
        }
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() > HARNESS_OUTPUT_LIMIT {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "cleanup output exceeds 1 MiB",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error),
        }
    }
}

/// The test's box name, unique to this run.
pub(crate) fn box_name(test: &str) -> String {
    format!("pinfold-e2e-{}-{test}", std::process::id())
}

/// The state root's one box state dir. State dirs are keyed by a hash of
/// the box name, so a test with one box finds its dir as the only entry.
pub(super) fn find_box_state_dir(env: &TestEnv) -> Option<PathBuf> {
    let boxes = env.state.join("pinfold").join("boxes");
    let mut entries = fs::read_dir(boxes).ok()?.flatten();
    let only = entries.next()?.path();
    entries.next().is_none().then_some(only)
}

/// Assert that the `what` up (`refused` or `failed`) left no state dir and no
/// box labeled `label`. State dirs are keyed by a hash of the name, so the
/// test cannot name one; this test has no live box at any call, so `boxes/`
/// must be empty.
pub(crate) fn assert_left_nothing(env: &TestEnv, name: &str, label: &str, what: &str) {
    let boxes = env.state.join("pinfold").join("boxes");
    let leftovers: Vec<PathBuf> = match fs::read_dir(&boxes) {
        Ok(entries) => entries
            .map(|entry| entry.expect("read box state entry").path())
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("read {}: {error}", boxes.display()),
    };
    assert!(
        leftovers.is_empty(),
        "the {what} up for {name} left a state dir: {leftovers:?}"
    );
    assert!(box_list(env, label).is_empty(), "the {what} up left a box");
}

/// An Apple container started with the `container` CLI, not pinfold, and
/// removed with it on drop. Linux has no runtime that holds boxes.
#[cfg(target_os = "macos")]
pub(super) struct RuntimeContainer {
    name: String,
}

#[cfg(target_os = "macos")]
impl RuntimeContainer {
    pub(super) fn run(name: &str, image: &str, label: Option<&str>) -> RuntimeContainer {
        let mut command = Command::new("container");
        command.args(["run", "-d", "--name", name]);
        if let Some(label) = label {
            command.args(["--label", label]);
        }
        run_ok(command.args([image, "sleep", "infinity"]));
        RuntimeContainer {
            name: name.to_string(),
        }
    }

    /// Whether Apple's own container list names it.
    pub(super) fn listed(&self) -> bool {
        let output = run_ok(Command::new("container").args(["list", "--all", "--format", "json"]));
        let containers: Vec<serde_json::Value> =
            serde_json::from_slice(&output.stdout).expect("the runtime's container list is JSON");
        containers
            .iter()
            .any(|container| container["id"] == self.name.as_str())
    }
}

#[cfg(target_os = "macos")]
impl Drop for RuntimeContainer {
    fn drop(&mut self) {
        // Best effort: a Drop during unwinding must not panic.
        let _ = Command::new("container")
            .args(["rm", "--force", &self.name])
            .output();
    }
}

fn box_down(env: &TestEnv, name: &str) -> ExitStatus {
    env.command(pinfold())
        .args(["box", "down", name])
        .stdin(Stdio::null())
        .status()
        .expect("run pinfold box down")
}

/// The IDs of untagged images with the caller's label, including the
/// intermediate layers a cached build leaves behind. Linux only.
fn untagged_images(label: &str) -> std::collections::BTreeSet<String> {
    let output = run_ok(Command::new("podman").args([
        "images",
        "-a",
        "-q",
        "--filter",
        "dangling=true",
        "--filter",
        &format!("label={label}"),
    ]));
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}
