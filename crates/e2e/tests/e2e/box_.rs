//! End-to-end tests for the guarantees in docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with rootless podman. The harness builds the `pinfold` binary, builds the
//! default profile image once, and drives pinfold as a user would: the CLI,
//! environment variables and the box spec are its only seams.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use e2e::{
    HttpFixture, ImageCleanup, TestDir, TestEnv, assert_denied, assert_ok, box_exec, box_list,
    box_stat, build_profile, curl, default_image, egress_log, exit_code, git, image_cli, image_id,
    json_lines, labeled_images, pinfold, profile_containerfile, project_id, project_state_dir,
    run_ok, runtime_images, tagged_images, untagged_images,
};

#[test]
fn box_lifecycle_works_for_a_caller() {
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
    // Sabotage: drop init's SIGCHLD SIG_IGN; the orphaned `true` stays a
    // zombie of PID 1 and the no-zombie assertion fails.
    // Sabotage: skip exec's existence check; the post-down exec returns the
    // runtime's 125 instead of 3.
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
    // Sabotage: give a box without `egress` a derived log path in `ready`;
    // the null assertion fails.
    // Sabotage: drop the owner-label check from cli.rs `present` (or from
    // core::box::down's list check); exec or stat on the foreign container
    // runs, or down removes it. Drop list's owner-label filter and the
    // runtime-created container appears beside the live pinfold box.
    // Sabotage: drop the valid-name gate in core::box::down; on podman the
    // `--filter` name reaches `rm` as a flag and removes the live box, so the
    // exec assertion fails; Apple's `rm` rejects the flag, so the exit
    // assertion fails.
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

    // `ready` carries the owner and the image it runs.
    assert_eq!(
        up.ready["owner"],
        up.pid(),
        "ready owner is not up's pid: {}",
        up.ready
    );
    assert_eq!(
        up.ready["image"]["id"],
        image_id.as_str(),
        "ready names the wrong image id: {}",
        up.ready
    );
    // The spec has no `egress`, so there is no proxy and no log: `ready`
    // carries the field as null.
    assert!(
        up.ready.get("egress_log").is_some_and(|log| log.is_null()),
        "ready gave a box without egress a log: {}",
        up.ready
    );

    // Sabotage: discard parse_exec's workdir; pwd reports the image's
    // default directory instead of the caller's /tmp.
    let directory = run_ok(env.command(pinfold()).args([
        "box",
        "exec",
        &name,
        "--workdir",
        "/tmp",
        "--",
        "pwd",
    ]));
    assert_eq!(String::from_utf8(directory.stdout).unwrap(), "/tmp\n");

    // Sabotage: discard the explicit tty flag; stdout is a host pipe, so
    // automatic TTY detection stays false and the guest's test -t fails.
    // The host's stdin PTY lets either runtime set terminal attributes.
    let terminal = run_ok(
        env.command(Path::new("python3"))
            .args([
                "-c",
                r#"
import os, pty, subprocess, sys
master, slave = pty.openpty()
try:
    result = subprocess.run(sys.argv[1:], stdin=slave, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=20)
finally:
    os.close(slave)
    os.close(master)
sys.stdout.buffer.write(result.stdout)
sys.stderr.buffer.write(result.stderr)
sys.exit(result.returncode)
"#,
            ])
            .arg(pinfold())
            .args([
                "box",
                "exec",
                &name,
                "--tty",
                "--",
                "sh",
                "-c",
                "test -t 0 && test -t 1 && printf tty-ready",
            ]),
    );
    assert_eq!(
        String::from_utf8(terminal.stdout).unwrap().trim(),
        "tty-ready"
    );

    // A child orphaned by an exec session is reparented to init and reaped:
    // after it, no process in the box is a zombie. The image has no `ps`, so
    // the kernel's own state line is read.
    let orphan = box_exec(&env, &name, &["sh", "-c", "true & exit 0"]);
    assert_eq!(orphan.code, 0, "orphaning exec failed: {}", orphan.stderr);
    let states = box_exec(
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
    let listed = box_list(&env, label);
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
            down.stdout.is_empty(),
            "down {absent:?} printed on stdout: {}",
            String::from_utf8_lossy(&down.stdout)
        );
        assert!(
            down.stderr.is_empty(),
            "down {absent:?} printed on stderr: {}",
            String::from_utf8_lossy(&down.stderr)
        );
        let still = box_exec(&env, &name, &["sh", "-c", "exit 0"]);
        assert_eq!(
            still.code, 0,
            "the live box stopped answering exec after down {absent:?}: {}",
            still.stderr
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

    // A container pinfold did not create is no box, even under a name `up`
    // accepts: `exec` and `stat` exit 3 and `down` leaves it running.
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

    // Keep the attached runtime output active through teardown. The FIFO
    // confirms a write to init's stdout, rather than the exec stream.
    // Sabotage: block the owner's async thread in runtime removal again;
    // its output drain stops and teardown can exceed the caller deadline.
    // Apple controls the race in which forced rm's two output waiters
    // split completion events; restoring rm -f reintroduces that race.
    let writing = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            "mkfifo /tmp/output-ready; \
             (printf 'first\\n'; echo ready >/tmp/output-ready; exec yes output-pressure) \
             >/proc/1/fd/1 2>/dev/null </dev/null & \
             read -r ready </tmp/output-ready; test \"$ready\" = ready",
        ],
    );
    assert_ok(&writing, "writing attached output before teardown");

    // `down` removes the box and the owner exits with output still active.
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
        let stopped_state =
            find_box_state_dir(&env, stopped.pid()).expect("find stopped owner's state");
        struct Resume(u32);
        impl Drop for Resume {
            fn drop(&mut self) {
                let _ = Command::new("kill")
                    .args(["-CONT", &self.0.to_string()])
                    .status();
            }
        }
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
    let mut again = box_up(&env, &untagged_spec, &name);
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

    // podman also resolves the image by its id. Apple's inspect resolves
    // neither id nor digest; macOS asserts nothing about this form.
    if cfg!(target_os = "linux") {
        let spec = serde_json::json!({
            "name": name,
            "image": image_id,
            "labels": { "dev.example.test": "lifecycle" },
        });
        let mut up = box_up(&env, &spec, &name);
        assert_eq!(
            up.ready["image"]["id"],
            image_id.as_str(),
            "the image id resolved to another id: {}",
            up.ready
        );
        up.close_stdin();
        assert!(up.wait().success(), "up did not exit 0 for stdin-closed");
    }
    // Sabotage: propagate a bookkeeping error before the terminal signal
    // event. A cold real artifact request holds startup after the claim;
    // host permissions make cleanup fail, independently of pinfold's logic.
    // Sabotage: drop the post-start actual-image comparison. Retagging while
    // that request is held then reports ready for the wrong image instead
    // of failing and removing it. A stable-tag launch is the control.
    {
        let env = TestEnv::with_private_cache("lifecycle-startup");
        let name = box_name("startup");
        let label = "dev.example.test=lifecycle-startup";
        let mut spec = serde_json::json!({
            "name": name,
            "image": image,
            "harness": "pi",
            "labels": { "dev.example.test": "lifecycle-startup" },
        });
        let mut proxy = HeldDownload::new();
        let mut child = env
            .command(pinfold())
            .envs(proxy.vars())
            .args(["box", "up"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(spec.to_string().as_bytes())
            .unwrap();
        let mut errors = child.stderr.take().unwrap();
        let mut child = ChildOwner::new(child, &env, None);
        let mut output = BufReader::new(child.stdout.take().unwrap());
        proxy.event("held");
        let state = find_box_state_dir(&env, child.id()).expect("claimed startup state");
        let parent = state.parent().unwrap().to_path_buf();
        struct RestorePermissions(PathBuf, fs::Permissions);
        impl Drop for RestorePermissions {
            fn drop(&mut self) {
                fs::set_permissions(&self.0, self.1.clone()).unwrap();
            }
        }
        let restore =
            RestorePermissions(parent.clone(), fs::metadata(&parent).unwrap().permissions());
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
        run_ok(Command::new("kill").args(["-TERM", &child.id().to_string()]));
        proxy.event("closed");
        let text = read_bounded(
            &mut child,
            &mut output,
            std::time::Duration::from_secs(10),
            true,
        )
        .expect("cancelled startup terminates stdout despite bookkeeping failure");
        let terminal = json_lines(&text);
        assert_eq!(terminal.len(), 1, "startup terminal events: {terminal:?}");
        assert_eq!(terminal[0]["event"], "down");
        assert_eq!(terminal[0]["reason"], "signal");
        assert!(child.wait_bounded().success(), "runtime removal succeeded");
        let mut diagnostic = String::new();
        errors.read_to_string(&mut diagnostic).unwrap();
        assert!(!diagnostic.is_empty(), "bookkeeping failure was silent");
        assert!(
            state.exists(),
            "permission failure did not leave recoverable state"
        );
        assert!(
            box_list(&env, label).is_empty(),
            "cancelled start left a runtime box"
        );
        drop(restore);
        drop(proxy);
        let mut healed_spec = spec.clone();
        healed_spec.as_object_mut().unwrap().remove("harness");
        let mut healed = box_up(&env, &healed_spec, &name);
        healed.close_stdin();
        assert!(healed.wait().success(), "same-name startup did not recover");
        assert_left_nothing(&env, &name, label, "recovered");

        let image_name = box_name("startup-image");
        let _images = ImageCleanup {
            repository: format!("pinfold/image-{image_name}"),
        };
        let context = TestDir::new(&env, "startup-image");
        let containerfile = context.path().join("Containerfile");
        fs::write(&containerfile, format!("FROM {image}\nCOPY stamp /stamp\n")).unwrap();
        fs::write(context.path().join("stamp"), "first\n").unwrap();
        let (code, built) = image_build(&env, &image_name, &containerfile, context.path());
        assert_eq!(code, 0, "first race image build: {built}");
        let first_ref = built["ref"].as_str().unwrap().to_string();
        // Apple's tag command normalizes an unqualified target to docker.io.
        let prefix = if cfg!(target_os = "macos") {
            "docker.io/"
        } else {
            ""
        };
        let latest = format!("{prefix}pinfold/image-{image_name}:latest");
        let _tagged_images = cfg!(target_os = "macos").then(|| ImageCleanup {
            repository: format!("{prefix}pinfold/image-{image_name}"),
        });
        fs::write(context.path().join("stamp"), "second\n").unwrap();
        let (code, built) = image_build(&env, &image_name, &containerfile, context.path());
        assert_eq!(code, 0, "second race image build: {built}");
        let second_ref = built["ref"].as_str().unwrap();
        let tag = |source: &str| {
            let mut command = Command::new(image_cli());
            if cfg!(target_os = "macos") {
                command.arg("image");
            }
            run_ok(command.args(["tag", source, &latest]));
        };
        // Build before the held CONNECT: curl's production connection
        // deadline must cover only the native tag move, not a queued build.
        tag(&first_ref);
        let first = e2e::image_id(&latest).expect("runtime resolves first tagged image");
        spec["image"] = latest.clone().into();
        let mut proxy = HeldDownload::new();
        let mut starting = box_up_start(&env, &spec, &proxy.vars());
        proxy.event("held");
        tag(second_ref);
        proxy.release();
        let second = e2e::image_id(&latest).expect("runtime resolves second tagged image");
        assert_ne!(first, second, "fixture images must differ");
        let failed = starting.first_line();
        assert_eq!(failed["event"], "failed", "retagged startup: {failed}");
        assert!(
            failed["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("image-changed")),
            "startup failed for another reason: {failed}"
        );
        assert_eq!(starting.child.wait_bounded().code(), Some(1));
        assert!(
            starting.rest().is_empty(),
            "failed startup emitted another event"
        );
        assert_left_nothing(&env, &name, label, "retagged");
        drop(proxy);
        let mut stable = box_up(&env, &spec, &name);
        assert_eq!(stable.ready["image"]["id"], second);
        let stamp = box_exec(&env, &name, &["cat", "/stamp"]);
        assert_ok(&stamp, "stable tag's file");
        assert_eq!(stamp.stdout, "second\n");
        stable.close_stdin();
        assert!(stable.wait().success(), "stable image did not tear down");
    }
}

#[test]
fn up_refuses_before_it_creates() {
    let _runtime = crate::shared_runtime();
    // Guarantee 17: up refuses before it creates.
    // Sabotage: drop `deny_unknown_fields` from `Mount`; the misspelled
    // mount spec then comes up `ready` and the refusal assertion fails.
    // Sabotage: parse the first JSON value straight into `Plan` again; the
    // serde refusal is raised before any name is read, so the misspelled
    // spec's `box` assertion fails.
    // Sabotage: drop the character-class check from the env-name validation,
    // keeping only the old empty-or-`=` test; the wildcard spec then comes up
    // `ready`, so the refusal assertion fails, and on podman the box's
    // environment also holds `HOSTSECRET_TOKEN=leaked`.
    // Sabotage: drop the comma and control-character check from the
    // mount-path validation; the comma path then reaches the runtime, whose
    // option parser reads the rest as mount options, so the refusal assertion
    // fails.
    // Sabotage: drop the not-a-directory check from the mount validation;
    // the file-mount spec then comes up `ready` on podman and ends `failed`
    // on Apple after the claim, so its `refused` assertion fails on both.
    // Sabotage: check duplicate guest paths before pinfold's own mounts are
    // added, as today; the profile-mount spec then comes up `ready` and its
    // `refused` assertion fails.
    // Sabotage: make the claim treat an existing state dir as success, as
    // `create_dir_all` does, and make the name checks always pass; the
    // concurrent loser is never refused `name-in-use`, and its failure path
    // removes the winner's box, so the winner's `exec` fails too.
    // Sabotage: keep the name check after the state dir is created; the
    // racing loser overwrites the winner's `pid` and its cleanup takes the
    // winner's box down, so the winner's `exec` fails.
    // Sabotage: leave metadata errors to the runtime; the missing mount
    // ends `failed` after the claim instead of `refused` as `spec`.
    // Sabotage: drop the reserved-label check; the `dev.pinfold.owner` spec
    // comes up `ready`, so its `refused` assertion fails.
    // Sabotage: drop the memory check; the `1000` spec reaches the runtime
    // unchecked and is not refused as spec, so its `refused` assertion
    // fails. Sabotage: drop the 256M floor, keeping the unit check; the
    // `255M` spec likewise reaches the runtime and its assertion fails.
    // Sabotage: drop the route check from `Plan::validate`; the URL route
    // spec comes up `ready`, so its `refused` assertion fails.
    // Sabotage: drop the "has one below it" clause from the public-suffix
    // check, keeping only rule equality; `.amazonaws.com` is not itself on
    // the list, so its spec comes up `ready` and its `refused` assertion
    // fails.
    let env = TestEnv::with_private_cache("refuses");
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

    // Sabotage: materialize a bundled share during profile resolution before
    // the missing-image refusal. The host cache must stay exactly as it was
    // after init was cached by the preceding refusal.
    let cache = env.root.join("cache");
    let before = super::cli::host_tree(&cache);
    let mut missing_profile = missing.clone();
    missing_profile["profile"] = serde_json::json!("documents");
    missing_profile["env"] = serde_json::json!({ "HOME": "/home/refused" });
    missing_profile["mounts"] = serde_json::json!([
        { "host": env.root, "guest": "/home/refused" }
    ]);
    let (code, refused) = box_up_refused(&env, &missing_profile, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["reason"], "image-missing");
    assert_left_nothing(&env, &name, label, "refused");
    assert_eq!(
        super::cli::host_tree(&cache),
        before,
        "a refusal wrote profile cache"
    );

    // Sabotage: accept unknown fields inside an env `from` object. A valid
    // reference succeeds in the race's winner below; this nested misspelling
    // must be refused as spec before runtime lookup or cache extraction.
    let nested = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "env": { "NESTED_VALUE": { "from": "PINFOLD_E2E_NESTED", "misspelled": true } },
    });
    let (code, refused) = box_up_refused(&env, &nested, &[("PINFOLD_E2E_NESTED", "fixture-value")]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("misspelled"),
        "the refusal did not name the nested key: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // A spec whose mount misspells `readonly` as `read_only` is refused as
    // data, naming the key, and leaves no state dir and no box. The positive
    // control, a spelled `readonly` that rejects writes, is the hook write in
    // guarantee 22's `a_caller_owned_box_cannot_write_git`.
    let misspelled = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": env.root, "guest": "/workspace", "read_only": true }],
    });
    let (code, refused) = box_up_refused(&env, &misspelled, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert_eq!(refused["box"], name);
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("read_only"),
        "the refusal did not name the key: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // A spec whose env name is not a POSIX name is refused as data, naming
    // the key, and leaves no state dir and no box. `HOSTSECRET_*` would make
    // podman import every `HOSTSECRET_` variable from `up`'s own
    // environment, which is the caller's. The positive control, a valid name
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
    // a caller label reaching `ready`, is guarantee 9's
    // `box_lifecycle_works_for_a_caller`.
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

    // A spec memory without a unit, or below the 256M floor, is refused as
    // data, naming `memory` and the value, and leaves no state dir and no
    // box: the value would otherwise reach the runtime unchecked. The
    // positive control, the `256M` box coming up on both runtimes, is
    // guarantee 20's `a_caller_can_tell_an_oom_kill_from_a_failure`.
    for memory in ["1000", "255M"] {
        let spec = serde_json::json!({
            "name": name,
            "image": default_image(&env),
            "labels": { "dev.example.test": "refuses" },
            "memory": memory,
        });
        let (code, refused) = box_up_refused(&env, &spec, &[]);
        assert_eq!(code, 1, "a refused up exits 1: {refused}");
        assert_eq!(refused["event"], "refused");
        assert_eq!(refused["reason"], "spec");
        let detail = refused["detail"].as_str().unwrap_or_default();
        assert!(
            detail.contains("memory") && detail.contains(memory),
            "the refusal did not name memory and its value: {refused}"
        );
        assert_left_nothing(&env, &name, label, "refused");
    }

    // A spec whose route value is a URL rather than host:port is refused as
    // data, naming the route, and leaves no state dir and no box: the proxy
    // would otherwise dial the string as an address and answer every request
    // 502. The positive control, a host:port route answering, is guarantee
    // 3's `the_proxy_refuses_the_tricks`.
    let url_route = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "egress": { "routes": { "ex": "https://example.com" } },
    });
    let (code, refused) = box_up_refused(&env, &url_route, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("\"ex\""),
        "the refusal did not name the route: {refused}"
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

    // An otherwise valid claude login cannot serve a codex harness.
    // Guarantee 26 supplies the matching-login control. Sabotage: drop the
    // harness-match check; this spec then comes up instead of refusing.
    let login_fixture = HttpFixture::start(None);
    let login_mismatch = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "harness": "codex",
        "egress": {
            "routes": {
                "login.internal": {
                    "login": "claude",
                    "from": "PINFOLD_E2E_LOGIN",
                    "to": format!("http://{}", login_fixture.route()),
                },
            },
        },
    });
    let (code, refused) = box_up_refused(
        &env,
        &login_mismatch,
        &[("PINFOLD_E2E_LOGIN", "fixture-token")],
    );
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("login.internal"),
        "the refusal did not name the route: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // A spec whose mount path holds a comma is refused as data, naming the
    // path, and leaves no state dir and no box: the bind value is built by
    // concatenation, so the runtime reads the rest as mount options. The
    // positive control, a plain path mounting, is guarantee 10's
    // `box_shares_files_with_the_host`.
    let comma = env.root.join("a,b");
    fs::create_dir_all(&comma).unwrap();
    let comma_mount = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": comma, "guest": "/workspace", "readonly": false }],
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

    // A spec whose mount host is a regular file is refused as data, naming
    // the path, and leaves no state dir and no box: the runtimes treat a
    // file mount differently, so one spec must be refused the same way on
    // both. The race's winner below is the control that the same command
    // path comes up `ready`.
    let file = env.root.join("f.txt");
    fs::write(&file, b"not a directory\n").unwrap();
    let file_mount = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": file, "guest": "/yard/f.txt", "readonly": true }],
    });
    let (code, refused) = box_up_refused(&env, &file_mount, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("f.txt"),
        "the refusal did not name the path: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // A spec whose mount names a guest path pinfold's own mount uses is
    // refused as data, naming the path, and leaves no state dir and no box:
    // pinfold adds the profile's `share/` at /opt/pinfold/profile after the
    // spec's own mounts, so the spec's directory would otherwise be silently
    // shadowed. The positive control is the race's winner below, which comes
    // up `ready` from the same image. Sabotage: check duplicates before
    // pinfold's mounts are added, as today; this spec then comes up `ready`,
    // so its `refused` assertion fails.
    let work = env.root.join("work");
    fs::create_dir_all(&work).unwrap();
    let spec_mount = env.root.join("spec-share");
    fs::create_dir_all(&spec_mount).unwrap();
    let profile_mount = serde_json::json!({
        "name": name,
        "profile": "default",
        "labels": { "dev.example.test": "refuses" },
        "env": { "HOME": "/work" },
        "mounts": [
            { "host": work, "guest": "/work", "readonly": false },
            { "host": spec_mount, "guest": "/opt/pinfold/profile", "readonly": true },
        ],
    });
    let (code, refused) = box_up_refused(&env, &profile_mount, &[]);
    assert_eq!(code, 1, "a refused up exits 1: {refused}");
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "spec");
    assert!(
        refused["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("/opt/pinfold/profile"),
        "the refusal did not name the path: {refused}"
    );
    assert_left_nothing(&env, &name, label, "refused");

    // Refuse both an absent directory and a dangling symlink to it. The
    // race below uses the same spec after the target is created, proving
    // that a symlink to a directory remains a valid mount.
    let absent_host = env.root.join("absent");
    let linked_host = env.root.join("linked");
    std::os::unix::fs::symlink(&absent_host, &linked_host).unwrap();
    let mut live = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "refuses" },
        "mounts": [{ "host": absent_host, "guest": "/workspace", "readonly": true }],
        "egress": { "allow": [".github.com"] },
        "env": { "NESTED_VALUE": { "from": "PINFOLD_E2E_NESTED" } },
    });
    for host in [&absent_host, &linked_host] {
        live["mounts"][0]["host"] = serde_json::json!(host);
        let (code, refused) =
            box_up_refused(&env, &live, &[("PINFOLD_E2E_NESTED", "fixture-value")]);
        assert_eq!(code, 1, "a refused up exits 1: {refused}");
        assert_eq!(refused["event"], "refused");
        assert_eq!(refused["reason"], "spec");
        assert!(
            refused["detail"]
                .as_str()
                .unwrap_or_default()
                .contains(host.to_str().unwrap()),
            "the refusal did not name the host path: {refused}"
        );
        assert_left_nothing(&env, &name, label, "refused");
    }
    fs::create_dir(&absent_host).unwrap();

    // Two `up`s on one free name at once: the claim lets exactly one hold
    // it, and the loser touches nothing of the winner's. Both are spawned
    // before either's first line is read. The winner also proves that
    // `.github.com` remains an allowed suffix entry.
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
fn box_shares_files_with_the_host() {
    let _runtime = crate::shared_runtime();
    // Sabotage: bind every spec mount read-only in the adapter (pass `true`
    // for `mount.readonly` in runtime/mod.rs); creating files in /workspace
    // then fails and the create assertion fails. Sabotage: drop
    // `--userns=keep-id` from podman's `run` argv; the box user is then a
    // subordinate uid on the host, so the host 0600 file is not writable and
    // the write assertion fails (Linux only).
    // Not a sabotage on Apple: running `box exec` as root. virtiofs reports
    // every host file as the host user's whatever the guest uid, so the owner
    // assertion still passes; the uid itself is guarantee 5's to check.
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

    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "mounts": [{ "host": dir.path(), "guest": "/workspace" }],
    });
    let up = box_up(&env, &spec, &name);

    // Box-created files: a 644 file, a 755 directory and an executable.
    let created = box_exec(
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

    up.down(&env);
}

#[test]
fn nothing_can_gain_privileges() {
    let _runtime = crate::shared_runtime();
    // Sabotage: drop `find / -xdev -perm /6000 -type f -exec chmod a-s {} +`
    // from the bundled image fragments; the setuid/setgid scan then lists
    // files and fails. Sabotage: drop `--read-only` from the adapter's `run` argv;
    // the rootfs write then succeeds and its assertion fails.
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

    // PID 1 also runs as the host uid:gid.
    let init = box_exec(&env, &name, &["cat", "/proc/1/status"]);
    assert_ok(&init, "reading /proc/1/status");
    assert_process_ids(&init.stdout, "PID 1");

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

    // On Linux, the podman seccomp profile must also block nested user
    // namespaces. Sabotage: prepend the ERRNO rules for clone and unshare
    // instead of removing `clone`, `clone3` and `unshare` from the default
    // profile's unconditional SCMP_ACT_ALLOW entry; the allow wins,
    // `unshare -U true` succeeds, and this assertion fails. Nested user
    // namespaces on Apple `container` are an open question in the spec, so
    // assert nothing there.
    if cfg!(target_os = "linux") {
        let unshare = box_exec(&env, &name, &["unshare", "-U", "true"]);
        assert_denied(&unshare, "Operation not permitted", "unshare -U");
        // Positive control: the same box still runs a plain child process.
        let child = box_exec(&env, &name, &["true"]);
        assert_ok(&child, "a plain child process");
    }

    up.down(&env);
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
    let lines = json_lines(&egress_log(&env, &name));
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
fn losing_the_owner_fails_closed() {
    let _runtime = crate::exclusive_runtime();
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
    let env = TestEnv::new("owner-gone");
    let name = box_name("owner-gone");
    let label = "dev.example.test=owner-gone";
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "owner-gone" },
        "egress": { "allow": ["api.github.com"] },
    });
    let mut up = box_up(&env, &spec, &name);

    // Positive control: the box has egress while its owner lives.
    let allowed = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "positive control");

    // Keep the killed owner unreaped. Sabotage: use kill(pid, 0) across roots;
    // the zombie appears live and the second-root liveness assertion fails.
    let other = TestEnv::new("owner-gone-other");
    let live = box_list(&other, label);
    assert!(
        live.iter()
            .any(|box_| box_["name"] == name && box_["owner_alive"] == true),
        "second state root did not see the live owner: {live:?}"
    );
    up.kill();

    // The positive control left its decision in the log. A live proxy
    // anywhere would log before it dials, so no new line means no proxy saw
    // the request; the request itself may hang, because Apple's forwarder
    // does not reliably close after the owner dies.
    // The log-line count is the assertion; the curl exit is only a
    // precondition, since a hang and a refusal both exit non-zero.
    let before = json_lines(&egress_log(&env, &name));
    assert!(
        !before.is_empty(),
        "the positive control left no egress log line"
    );
    let denied = curl(
        &env,
        &name,
        "5",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ne!(
        denied.code, 0,
        "the box still had egress after the owner died"
    );
    let after = json_lines(&egress_log(&env, &name));
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
    let old_state = find_box_state_dir(&env, owner).expect("find the dead owner's state dir");
    fs::write(old_state.join("pid"), "1").expect("overwrite the dead owner's pid");

    // Pinfold's own liveness test reports the owner gone before prune acts:
    // the box is still listed, with `owner_alive` false.
    let listed = box_list(&other, label);
    let leftover = listed
        .iter()
        .find(|box_| box_["name"] == name)
        .unwrap_or_else(|| panic!("the dead owner's box is not listed: {listed:?}"));
    assert_eq!(
        leftover["owner_alive"], false,
        "the dead owner's box reports owner_alive true: {leftover:?}"
    );

    // `box prune` removes the leftover by label.
    run_ok(env.command(pinfold()).args(["box", "prune"]));
    up.wait();
    let listed = box_list(&env, label);
    assert!(
        !listed.iter().any(|box_| box_["name"] == name),
        "prune left the box: {listed:?}"
    );

    // A crash before writing pid leaves only a state dir. Sabotage: treat
    // a missing pid as permanently live; this fresh up refuses name-in-use.
    // The host-created abandoned dir is the outside fixture.
    // Reuse the actual directory observed before prune, without deriving
    // pinfold's name hash in the test.
    fs::create_dir_all(&old_state).expect("leave abandoned partial claim");
    let mut fresh = box_up(&env, &spec, &name);
    fresh.down(&env);
    assert!(fresh.wait().success(), "the fresh up did not exit cleanly");
}

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    let _runtime = crate::exclusive_runtime();
    // Guarantee 15: cleanup removes only pinfold's garbage.
    //
    // Sabotage: make `keep_two_images` return before it removes anything;
    // three images remain and the two-image assertion fails. Sabotage: drop
    // the images step from the shared automatic pass; b3 survives `clean`
    // and the post-clean two-image assertion fails. Sabotage: drop
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
    // is still listed" assertion fails. Sabotage: make `--unused` skip the
    // live-box check (drop `live_projects` from `CleanPlan::measure`'s
    // stale test); the live project's state goes and its assertion fails.
    // Sabotage: in `clean::boxes`, insert into `live_projects` above `if
    // alive`; the dead box then protects the other project, whose marker
    // survives, and the removal assertion fails. Sabotage: past the newest
    // two build tags, remove every tag of the tag's image, as retention by
    // image did; `clean` takes the image the newest two builds share and
    // the shared-image assertion fails. Sabotage: skip a tag whose image a
    // newer tag names; the oldest-tag assertion fails. Sabotage: drop the
    // in-use check from `remove_images`; the live box's last tag is offered
    // to the runtime: podman refuses the removal and `image rm` exits
    // non-zero, Apple deletes the image under the box and the live-box
    // image assertion fails. Sabotage: restore the per-id skip in
    // `remove_images`; the twin box's shared image is reported in use, the
    // retired name's tag on it survives, and the assertion that
    // `retire_refs[1]` is gone fails. Sabotage: apply the newest-two rule or
    // the caller's one-hour grace in `image rm`; a fresh build survives and
    // the removed-tags assertion fails. Sabotage: match the name as a prefix in
    // `remove_images`; the image of the name extending the retired one goes
    // and its survival assertion fails. Sabotage: make `--dry-run` run the
    // real clean; the other project's state goes and the dry-run assertion
    // fails. Sabotage: keep
    // podman's repeated `Names` in `list_images`; the second removal of the
    // newest image's tag is `image not known` and the one-line assertion
    // fails. Sabotage: write the caller's name into `dev.pinfold.image`;
    // the twin build makes a second image under the name and the shared-id
    // assertion fails. Sabotage: make
    // `remove_images` remove every reference of an image's id instead of
    // only the name's tags; the twin's tag goes with the shared image and
    // the twin assertion fails.
    // Sabotage: force-delete Apple's shared builder during clean; the
    // other process's build, held in RUN by the host fixture, fails.
    // Sabotage: omit Apple's allocated build-cache storage from the
    // report; its byte count falls below the backing filesystem's du.
    // Sabotage: omit Apple's online trim after pruning; the real RUN's
    // deleted 64 MiB file remains allocated on the host across clean.
    // Sabotage: in `clean::boxes`, take any box with a `dev.pinfold.` label
    // instead of the owner label; on podman the user's container, which
    // carries the image's labels and no owner, is removed as a dead box and
    // its survival assertion fails.
    let env = TestEnv::new("cleanup");
    let build_env = cfg!(target_os = "macos").then(|| {
        let other = TestEnv::new("clean-build");
        // This test's second caller must finish its daily pass before the
        // dead fixture exists. Artifact inspection does not run maintenance.
        box_list(&other, "dev.example.test=e2e-cleanup-dead");
        other
    });
    default_image(&env);
    // A profile of this test's own, named for this run, so the operator's
    // default profile images, the other tests and a failed run's leftovers
    // cannot share the source.
    let profile = format!("e2e-maintenance-{}", std::process::id());
    let _images = ImageCleanup {
        repository: format!("pinfold/profile-{profile}"),
    };
    // `FROM scratch` keeps the test off the network and fast.
    let containerfile = profile_containerfile(&env, &profile, "FROM scratch\n");

    // Each build changes the Containerfile, so every build is a distinct
    // image; unchanged inputs could share one.
    let mut step = 0;
    let mut build_changed = || {
        step += 1;
        fs::write(&containerfile, format!("FROM scratch\nENV STEP={step}\n")).unwrap();
        build_profile(&env, &profile);
    };

    // Record each build's unique tag. The `:latest` tag moves along, so a
    // box pins the image it started from by this tag. After three builds b1
    // is gone; b2 and b3 remain and b2 is the next removal candidate.
    let [_, b2, b3] = [(); 3].map(|()| {
        build_changed();
        built_unique_ref(&profile)
    });

    let images = labeled_images("dev.pinfold.profile", &profile);
    assert_eq!(
        images.len(),
        2,
        "after three builds of one source, two images should remain: {images:?}"
    );
    assert!(
        image_id(&b2).is_some() && image_id(&b3).is_some(),
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
        let (code, built) = image_build(&env, &x_name, &x_containerfile, &x_context);
        assert_eq!(built["event"], "built", "x build {}: {built}", i + 1);
        assert_eq!(code, 0, "x build {} exited {code}", i + 1);
        x_refs.push(
            built["ref"]
                .as_str()
                .unwrap_or_else(|| panic!("x build {} carries no ref: {built}", i + 1))
                .to_string(),
        );
    }

    // Box A pins b2, the older of the two, so the next build's retention
    // meets a pinned image first.
    let box_a = box_name("cleanup-a");
    let pin_a = serde_json::json!({ "name": box_a, "image": b2.as_str() });
    let mut up_a = box_up(&env, &pin_a, &box_a);

    // Build 4: b2 cannot go while box A holds it. The build still exits 0.
    build_changed();
    assert!(
        image_id(&b2).is_some(),
        "the image box A pins is gone after build 4"
    );
    // Build 4 counts only the profile's own images, so the caller's two
    // survive and b3 remains the profile's older rollback image.
    assert!(
        x_refs.iter().all(|reference| image_id(reference).is_some()),
        "build 4 removed a caller image built on the profile: {x_refs:?}"
    );
    assert!(
        image_id(&b3).is_some(),
        "build 4 removed the profile's b3 for its caller images"
    );

    // Box B pins b3, then box A goes down and frees b2. Build 5's retention
    // meets pinned b3 first and must still remove the free b2 behind it.
    let box_b = box_name("cleanup-b");
    let pin_b = serde_json::json!({ "name": box_b, "image": b3.as_str() });
    let mut up_b = box_up(&env, &pin_b, &box_b);
    up_a.down(&env);
    assert!(up_a.wait().success(), "box A's up did not exit cleanly");

    build_changed();
    assert!(
        image_id(&b2).is_none(),
        "build 5 kept the freed b2 because the pinned b3 failed first"
    );
    assert!(image_id(&b3).is_some(), "build 5 removed the pinned b3");

    // Box B goes down before the rest of the test, which starts its own
    // boxes from the profile's `:latest`.
    up_b.down(&env);
    assert!(up_b.wait().success(), "box B's up did not exit cleanly");

    // The runtime's own build of the `FROM scratch` Containerfile, so the
    // image carries only the labels given here. No profile build's `ENV`
    // step is in it for the runtime's cache to return.
    fs::write(&containerfile, "FROM scratch\n").unwrap();
    let runtime_build = |tags: &[&str], labels: &[&str]| {
        let mut command = Command::new(image_cli());
        // Match pinfold's builder configuration; fixture builds must not
        // replace the shared Apple builder under another test's build.
        if cfg!(target_os = "macos") {
            command.env_remove("NO_COLOR").env_remove("BUILDKIT_COLORS");
        }
        let status = command
            .args(["build", "--file"])
            .arg(&containerfile)
            .args(tags.iter().flat_map(|tag| ["--tag", tag]))
            .args(labels.iter().flat_map(|label| ["--label", label]))
            .arg(containerfile.parent().unwrap())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .expect("run the runtime's build");
        assert!(status.success(), "building {tags:?} failed");
    };

    // An unlabeled image: no `dev.pinfold` label, so `clean` must leave it.
    // Its tag shares the profile prefix, so the guard above deletes it.
    let unlabeled = format!("pinfold/profile-{profile}:unlabeled");
    runtime_build(&[&unlabeled], &[]);
    assert!(
        image_id(&unlabeled).is_some(),
        "the unlabeled image is missing before clean"
    );

    // A caller source whose three builds share one image, all past the
    // hour's grace: the runtime builds one image under three build tags
    // whose `<build>` times fall in 1970, in one build as pinfold tags its
    // own. `clean` finds the name by its tags and removes only the oldest
    // tag; the image stays under the newest two.
    let shared = format!("{profile}-shared");
    let shared_repository = format!("pinfold/image-{shared}");
    let _shared_images = ImageCleanup {
        repository: shared_repository.clone(),
    };
    let shared_refs = ["1-1", "2-1", "3-1"].map(|build| format!("{shared_repository}:{build}"));
    runtime_build(&shared_refs.each_ref().map(String::as_str), &[]);
    let shared_id = image_id(&shared_refs[0]);
    assert!(
        shared_id.is_some()
            && shared_refs
                .iter()
                .all(|reference| image_id(reference) == shared_id),
        "the three build tags do not name one image before clean"
    );

    // A project's state: a refused `pinfold pi` creates it before it names
    // the missing image. The profile is never built, so this part downloads
    // no pi artifact. Two projects get a cache marker: one runs a live box,
    // the other does not.
    let missing = format!("{profile}-missing");
    profile_containerfile(&env, &missing, "FROM scratch\n");
    let seed_project = |project: &TestDir, text: &[u8]| {
        let refused = env
            .command(pinfold())
            .args(["pi", "--version"])
            .env("PINFOLD_PROFILE", &missing)
            .current_dir(project.path())
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
    let other_id = project_id(&env, other_project.path());

    // A live pinfold box for the live project, and a dead box for the other
    // project: a dead box protects nothing. A caller spec may not name
    // pinfold's label namespace, so each box's `dev.pinfold.project` label
    // comes from its image, as a real project image carries it.
    let live_label = format!("dev.pinfold.project={live_id}");
    let live = box_name("cleanup-live");
    let live_repository = format!("e2e-cleanup-live-{}", std::process::id());
    let _live_images = ImageCleanup {
        repository: live_repository.clone(),
    };
    let live_ref = format!("{live_repository}:latest");
    runtime_build(&[&live_ref], &[&live_label]);
    let live_spec = serde_json::json!({ "name": live, "image": live_ref });
    let _live = box_up(&env, &live_spec, &live);

    let dead_label = "dev.example.test=e2e-cleanup-dead";
    let dead = box_name("cleanup-dead");
    let dead_repository = format!("e2e-cleanup-dead-{}", std::process::id());
    let _dead_images = ImageCleanup {
        repository: dead_repository.clone(),
    };
    let dead_ref = format!("{dead_repository}:latest");
    runtime_build(&[&dead_ref], &[&format!("dev.pinfold.project={other_id}")]);
    let dead_spec = serde_json::json!({
        "name": dead,
        "image": dead_ref,
        "labels": { "dev.example.test": "e2e-cleanup-dead" },
    });
    let mut dead_up = box_up(&env, &dead_spec, &dead);
    dead_up.kill();
    dead_up.close_stdin();

    // A container the user started with the runtime's own CLI from a
    // pinfold-built image is not a box: podman copies the image's
    // `dev.pinfold.` labels onto it, but pinfold never owned it.
    let user_container =
        RuntimeContainer::run(&box_name("cleanup-user"), default_image(&env), None);

    // Positive controls: everything `clean` sorts out exists before it runs.
    // Sabotage: report `owner_alive` as false whenever the owner label
    // parses; the live-owner assertion fails.
    let listed = box_list(&env, &live_label);
    let live_box = listed
        .iter()
        .find(|box_| box_["name"] == live)
        .unwrap_or_else(|| panic!("the live box is missing before clean: {listed:?}"));
    assert_eq!(
        live_box["owner_alive"], true,
        "the live box reports its owner dead before clean: {live_box:?}"
    );
    assert!(
        !box_list(&env, dead_label).is_empty(),
        "the dead box is missing before clean"
    );
    assert!(
        user_container.listed(),
        "the user's container is missing before clean"
    );
    assert!(
        live_marker.is_file(),
        "the live project's marker is missing before clean"
    );
    assert!(
        other_marker.is_file(),
        "the other project's marker is missing before clean"
    );

    // `--dry-run` only lists: the other project's state, which the real
    // clean below removes, survives it.
    run_ok(env.command(pinfold()).args(["clean", "--dry-run"]));
    assert!(
        other_marker.is_file(),
        "--dry-run removed the other project's state"
    );

    // The real clean removes the dead box and only the dead box. Apple's
    // builder is shared across state directories, so keep another caller's
    // RUN active while clean prunes unused cache.
    if cfg!(target_os = "macos") {
        let other = build_env.as_ref().unwrap();
        let name = format!("{profile}-active");
        let _active_images = ImageCleanup {
            repository: format!("pinfold/image-{name}"),
        };
        let build = HeldBuild::start(other, &name, default_image(other), &[]);
        let inspected = run_ok(Command::new("container").args(["inspect", "buildkit"]));
        let builder: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
        let exports = builder[0]["configuration"]["mounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|mount| mount["destination"] == "/var/lib/container-builder-shim/exports")
            .unwrap()["source"]
            .as_str()
            .unwrap();
        let backing = Path::new(exports)
            .parent()
            .unwrap()
            .join("containers/buildkit/rootfs.ext4");
        let allocated_kib = || {
            let output = run_ok(Command::new("du").arg("-k").arg(&backing));
            String::from_utf8(output.stdout)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse::<u64>()
                .unwrap()
        };
        // Trim existing free blocks before the fixture writes, so old
        // storage cannot supply the allocation this scenario must reclaim.
        run_ok(Command::new("container").args(["clean", "buildkit"]));
        let before_fixture = allocated_kib();
        let cache_name = format!("{profile}-cache");
        let _cache_images = ImageCleanup {
            repository: format!("pinfold/image-{cache_name}"),
        };
        let context = other.root.join("cache-context");
        fs::create_dir_all(&context).unwrap();
        let containerfile = context.join("Containerfile");
        fs::write(
            &containerfile,
            format!(
                "FROM {}\nRUN dd if=/dev/urandom of=/{cache_name} bs=1M count=64 && sync && rm /{cache_name} && sync\n",
                default_image(other)
            ),
        )
        .unwrap();
        let (code, cached) = image_build(other, &cache_name, &containerfile, &context);
        assert_eq!(code, 0, "the cache fixture build failed: {cached}");
        let allocated = allocated_kib();
        assert!(
            allocated > before_fixture,
            "the cache fixture allocated no host blocks: {before_fixture} -> {allocated} KiB"
        );
        let report = run_ok(env.command(pinfold()).args(["clean", "--dry-run"]));
        let report = String::from_utf8(report.stdout).unwrap();
        let measured: u64 = report
            .lines()
            .find_map(|line| line.trim().strip_prefix("build cache: "))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        // Other builds can allocate more between du and the report. This
        // test is the only one pruning, and it has not pruned yet.
        assert!(
            measured >= allocated * 1024,
            "build cache undercounts allocated backing storage: {report}"
        );
        assert!(
            allocated_kib() >= allocated,
            "--dry-run reclaimed builder blocks"
        );
        let cleaned = env.command(pinfold()).arg("clean").output().unwrap();
        let after_clean = allocated_kib();
        let built = build.finish();
        assert!(
            cleaned.status.success(),
            "clean during another caller's build: {cleaned:?}"
        );
        assert!(
            after_clean < allocated,
            "clean did not reclaim freed builder blocks: {allocated} -> {after_clean} KiB"
        );
        assert!(
            built.status.success(),
            "build held active across clean: {built:?}"
        );
        let built: serde_json::Value = serde_json::from_slice(&built.stdout).unwrap();
        assert_eq!(built["event"], "built");
        assert!(image_id(built["ref"].as_str().unwrap()).is_some());
    } else {
        run_ok(env.command(pinfold()).args(["clean"]));
    }
    assert!(
        image_id(&unlabeled).is_some(),
        "clean removed an unlabeled image"
    );
    // No build follows box B's going down. The profile is left holding b3,
    // b4 and b5: b3 was pinned at build 5 and the two newer images stayed.
    // `clean` applies the after-build rule to every source, so the profile
    // falls to its newest two and b3 goes.
    let profile_images = labeled_images("dev.pinfold.profile", &profile);
    assert_eq!(
        profile_images.len(),
        2,
        "clean did not leave the profile at its newest two images: {profile_images:?}"
    );
    assert!(
        image_id(&b3).is_none(),
        "clean kept b3, an image past the newest two whose box went down"
    );
    assert!(
        image_id(&shared_refs[0]).is_none(),
        "clean kept the shared image's oldest build tag"
    );
    assert!(
        shared_refs[1..]
            .iter()
            .all(|reference| image_id(reference) == shared_id),
        "clean removed the image the newest two builds share"
    );
    assert!(
        !box_list(&env, &live_label).is_empty(),
        "clean removed a live box"
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
        box_list(&env, dead_label).is_empty(),
        "clean left a box whose owner is gone"
    );
    dead_up.wait();
    assert!(
        user_container.listed(),
        "clean removed a container pinfold did not start"
    );
    drop(user_container);

    // `--unused AGE` removes the state of projects not run for that long,
    // except one with a live box. Both projects last ran seconds ago, so
    // `0s` ages them out and only the live one survives.
    run_ok(env.command(pinfold()).args(["clean", "--unused", "0s"]));
    assert!(
        !other_state.exists(),
        "clean --unused kept the state of a project that has not run"
    );
    assert!(
        live_state.is_dir(),
        "clean --unused removed the state of a project with a live box"
    );

    // Sabotage: remove the shared project locks from pi startup and box up;
    // clean deletes the existing home while the cold download holds startup
    // after its claim but before a runtime box can protect the project.
    // Moving the checkout makes its state stale without a clock wait.
    {
        let env = TestEnv::with_private_cache("clean-startup");
        let project = TestDir::new(&env, "starting-project");
        let missing = format!("{profile}-startup-missing");
        profile_containerfile(&env, &missing, "FROM scratch\n");
        let refused = env
            .command(pinfold())
            .args(["pi", "--version"])
            .env("PINFOLD_PROFILE", &missing)
            .current_dir(project.path())
            .output()
            .unwrap();
        assert!(!refused.status.success(), "the missing image was accepted");
        let state = project_state_dir(&env, project.path());
        let id = project_id(&env, project.path());
        let marker = state.join("home/.cache/marker");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, b"starting project\n").unwrap();

        let mut proxy = HeldDownload::new();
        let child = env
            .command(pinfold())
            .envs(proxy.vars())
            .args(["pi", "--version"])
            .current_dir(project.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut child = ChildOwner::new(child, &env, None);
        let mut output = BufReader::new(child.stdout.take().unwrap());
        proxy.event("held");
        let label = format!("dev.pinfold.project={id}");
        assert!(
            box_list(&env, &label).is_empty(),
            "startup already had a runtime box before the artifact arrived"
        );
        struct RestoreProject {
            root: PathBuf,
            moved: PathBuf,
        }
        impl Drop for RestoreProject {
            fn drop(&mut self) {
                if self.moved.exists() {
                    fs::rename(&self.moved, &self.root).unwrap();
                }
            }
        }
        let restore = RestoreProject {
            root: project.path().to_path_buf(),
            moved: env.root.join("moved-project"),
        };
        fs::rename(&restore.root, &restore.moved).unwrap();
        run_ok(env.command(pinfold()).args(["clean", "--unused", "0s"]));
        assert_eq!(
            fs::read(&marker).unwrap(),
            b"starting project\n",
            "clean removed the claimed startup's existing home"
        );
        fs::rename(&restore.moved, &restore.root).unwrap();
        proxy.release();
        read_bounded(
            &mut child,
            &mut output,
            std::time::Duration::from_secs(90),
            true,
        )
        .expect("pi finishes after the artifact download resumes");
        assert!(
            child.wait_bounded().success(),
            "pi startup failed after clean"
        );

        // Positive control: the same stale state is removable after the
        // real pi launch finishes and releases its project locks.
        fs::rename(&restore.root, &restore.moved).unwrap();
        run_ok(env.command(pinfold()).args(["clean", "--unused", "0s"]));
        assert!(!state.exists(), "clean kept the idle project's stale state");
    }

    // `image rm NAME` retires one caller image name: every tag of the name,
    // whatever its age, and no other name's, except an image's last tag
    // while a listed box uses it. Three builds of `retire` are fresh, so
    // the caller grace would keep them all. A retire box pins the oldest
    // build, whose only tags are `retire`'s, so its last tag stays; a twin
    // box pins the second build's image, which the twin's tags also hold,
    // so `retire`'s tag on it goes. A second name extending the first is
    // untouched.
    let base = default_image(&env);
    let rm_context = env.root.join("image-rm-context");
    fs::create_dir_all(&rm_context).unwrap();
    let rm_containerfile = rm_context.join("Containerfile");
    fs::write(
        &rm_containerfile,
        format!("FROM {base}\nCOPY marker.txt /marker.txt\n"),
    )
    .unwrap();
    let retire_name = format!("{profile}-retire");
    let _retire_images = ImageCleanup {
        repository: format!("pinfold/image-{retire_name}"),
    };
    let mut retire_refs: Vec<String> = Vec::new();
    for marker in ["retire-1", "retire-2", "retire-3"] {
        fs::write(rm_context.join("marker.txt"), format!("{marker}\n")).unwrap();
        let (code, built) = image_build(&env, &retire_name, &rm_containerfile, &rm_context);
        assert_eq!(built["event"], "built", "the {marker} build: {built}");
        assert_eq!(code, 0, "the {marker} build exited {code}");
        retire_refs.push(
            built["ref"]
                .as_str()
                .unwrap_or_else(|| panic!("built carries no ref: {built}"))
                .to_string(),
        );
    }
    let retire_ids: Vec<String> = retire_refs
        .iter()
        .map(|reference| image_id(reference).unwrap_or_else(|| panic!("no image for {reference}")))
        .collect();
    let mut distinct_ids = retire_ids.clone();
    distinct_ids.sort_unstable();
    distinct_ids.dedup();
    assert_eq!(
        distinct_ids.len(),
        3,
        "the changed retire builds do not name three images: {retire_refs:?}"
    );
    assert_eq!(
        tagged_images(&format!("pinfold/image-{retire_name}")),
        distinct_ids,
        "the runtime does not list the three retire images before image rm"
    );
    // A second name extending the first, with one build: `image rm` of
    // `retire` must not touch it.
    let similar_name = format!("{retire_name}-x");
    let _similar_images = ImageCleanup {
        repository: format!("pinfold/image-{similar_name}"),
    };
    fs::write(rm_context.join("marker.txt"), "similar\n").unwrap();
    let (code, similar) = image_build(&env, &similar_name, &rm_containerfile, &rm_context);
    assert_eq!(similar["event"], "built", "the similar build: {similar}");
    assert_eq!(code, 0, "the similar build exited {code}");
    let similar_latest = similar["latest"]
        .as_str()
        .unwrap_or_else(|| panic!("built carries no latest: {similar}"))
        .to_string();

    // A third name built from the second retire build's identical inputs:
    // its build returns that image and tags it under the third name.
    let twin_name = format!("{retire_name}-twin");
    let _twin_images = ImageCleanup {
        repository: format!("pinfold/image-{twin_name}"),
    };
    fs::write(rm_context.join("marker.txt"), "retire-2\n").unwrap();
    let (code, twin) = image_build(&env, &twin_name, &rm_containerfile, &rm_context);
    assert_eq!(twin["event"], "built", "the twin build: {twin}");
    assert_eq!(code, 0, "the twin build exited {code}");
    let twin_latest = twin["latest"]
        .as_str()
        .unwrap_or_else(|| panic!("built carries no latest: {twin}"))
        .to_string();
    assert_eq!(
        image_id(&twin_latest),
        Some(retire_ids[1].clone()),
        "identical inputs under the twin name did not share the retire image"
    );

    // Box the oldest build, so the newest two, `latest` among them, are the
    // free ones.
    let retire_box = box_name("cleanup-retire");
    let retire_spec = serde_json::json!({ "name": retire_box, "image": retire_refs[0] });
    let mut up_retire = box_up(&env, &retire_spec, &retire_box);

    // A twin box runs the shared image, so `retire`'s tag on it must be
    // freed while the twin's tags hold the image up.
    let twin_box = box_name("cleanup-twin");
    let twin_spec = serde_json::json!({ "name": twin_box, "image": twin_latest });
    let mut up_twin = box_up(&env, &twin_spec, &twin_box);

    let (code, removed) = image_rm(&env, &retire_name);
    assert_eq!(code, 0, "image rm {retire_name} exited {code}: {removed}");
    assert_eq!(removed["event"], "removed", "image rm: {removed}");
    // The twin's tags hold the shared image up, so the retire tag on it
    // goes.
    assert!(
        image_id(&retire_refs[1]).is_none(),
        "image rm left the retired name's tag on the twin box's image: {retire_refs:?}"
    );
    assert!(
        image_id(&retire_refs[2]).is_none(),
        "image rm left the retired name's tag on the removed image: {retire_refs:?}"
    );
    assert!(
        image_id(&format!("pinfold/image-{retire_name}:latest")).is_none(),
        "image rm left the retire name's latest tag"
    );
    // The retire box pins an image only `retire` tags, so its last tag
    // stays and is reported.
    assert_eq!(
        removed["in_use"],
        serde_json::json!([retire_ids[0]]),
        "image rm did not report the last tag it kept: {removed}"
    );
    let untagged: Vec<&str> = removed["ids"]
        .as_array()
        .expect("ids is an array")
        .iter()
        .map(|id| id.as_str().expect("an id is a string"))
        .collect();
    assert!(
        untagged.contains(&retire_ids[1].as_str()) && untagged.contains(&retire_ids[2].as_str()),
        "image rm did not report both untagged images: {removed}"
    );
    assert!(
        image_id(&retire_refs[0]).is_some(),
        "image rm removed the live box's last tag"
    );
    assert!(
        image_id(&similar_latest).is_some(),
        "image rm touched the image of a name it prefixes"
    );
    assert_eq!(
        image_id(&twin_latest),
        Some(retire_ids[1].clone()),
        "image rm removed the twin's tag with the shared image"
    );
    // Only references went, not the image: the twin box still answers from
    // the shared image.
    let alive = box_exec(&env, &twin_box, &["cat", "/marker.txt"]);
    assert_eq!(
        alive.code, 0,
        "the twin box failed exec after image rm: {}",
        alive.stderr
    );
    assert_eq!(
        alive.stdout.trim(),
        "retire-2",
        "the twin box answered from another image: {}",
        alive.stdout
    );

    // With the retire box down, its last tag goes; the twin box stays up
    // through the removal and goes down below.
    up_retire.down(&env);
    assert!(
        up_retire.wait().success(),
        "the retire box's up did not exit cleanly"
    );
    let (code, removed) = image_rm(&env, &retire_name);
    assert_eq!(code, 0, "the second image rm exited {code}: {removed}");
    assert!(
        tagged_images(&format!("pinfold/image-{retire_name}")).is_empty(),
        "the second image rm left a tag of the retired name"
    );
    up_twin.down(&env);
    assert!(
        up_twin.wait().success(),
        "the twin box's up did not exit cleanly"
    );
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
        repository: format!("pinfold/profile-{profile}"),
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
    // `up` is refused `image-missing`. Sabotage: make `build_id` in
    // core/image.rs return a fixed string; the three builds then share one
    // ref, which names the third build, and the box reads `third`.
    // Sabotage: report `local_image_id(runtime, &latest)` resolved before
    // the build; the first build's id is absent and each later build's is
    // stale, so the `built["id"]` assertion fails. Sabotage: write a unique
    // build label into every image again, as `dev.pinfold.build` was; the
    // repeated build makes a new image and the one-id assertion fails.
    let env = TestEnv::new("image-build");
    let other = cfg!(target_os = "macos").then(|| TestEnv::new("image-colors"));
    let base = default_image(&env);

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
        assert_eq!(
            image_id(&latest),
            image_id(&reference),
            "{latest} does not name the {marker} build"
        );
        assert_eq!(
            built["id"].as_str(),
            image_id(&reference).as_deref(),
            "the {marker} built line names another image id"
        );
        refs.push(reference);
    }

    let names_ids = || tagged_images(&repository);
    let kept = names_ids();
    // A caller's `built` ref is a handle for later, so later builds of the
    // name neither reclaim nor move it: a box started from the first
    // build's ref reads the file that build COPYed.
    let read = file_from_image(&env, &refs[0], "image", "/marker.txt");
    assert_eq!(read, "first\n", "the first build's ref names another build");

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

    // Sabotage: ignore the caller's no_cache in Image::build; the runtime
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
            repository: format!("pinfold/image-{held_name}"),
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

/// Hold a real cold artifact CONNECT after startup has claimed its name.
struct HeldDownload {
    child: Child,
    events: BufReader<ChildStdout>,
    url: String,
}

impl HeldDownload {
    fn new() -> Self {
        let mut child = Command::new("python3")
            .args(["-u", "-c", include_str!("connect_proxy.py")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start artifact CONNECT fixture");
        let events = BufReader::new(child.stdout.take().unwrap());
        let mut fixture = Self {
            child,
            events,
            url: String::new(),
        };
        let port: u16 = fixture.line().trim().parse().expect("fixture port");
        fixture.url = format!("http://127.0.0.1:{port}");
        fixture
    }

    fn vars(&self) -> [(&str, &str); 4] {
        [
            ("HTTPS_PROXY", self.url.as_str()),
            ("https_proxy", self.url.as_str()),
            ("NO_PROXY", ""),
            ("no_proxy", ""),
        ]
    }

    fn line(&mut self) -> String {
        if self.events.buffer().is_empty() {
            let mut ready = [nix::poll::PollFd::new(
                self.events.get_ref().as_fd(),
                nix::poll::PollFlags::POLLIN,
            )];
            assert!(
                nix::poll::poll(&mut ready, 30_000_u16).unwrap() > 0,
                "artifact fixture missed its readiness deadline"
            );
        }
        let mut line = String::new();
        assert_ne!(
            self.events.read_line(&mut line).unwrap(),
            0,
            "artifact fixture exited"
        );
        line
    }

    fn event(&mut self, expected: &str) {
        assert_eq!(self.line().trim(), expected, "artifact fixture event");
    }

    fn release(&mut self) {
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"release\n")
            .unwrap();
    }
}

impl Drop for HeldDownload {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A real Apple build held inside RUN until its host HTTP fixture answers.
struct HeldBuild {
    child: Option<Child>,
    release: Option<std::sync::mpsc::Sender<()>>,
    server: Option<std::thread::JoinHandle<()>>,
}

impl HeldBuild {
    fn start(env: &TestEnv, name: &str, base: &str, vars: &[(&str, &str)]) -> HeldBuild {
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

    fn finish(mut self) -> std::process::Output {
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
fn image_build(
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
fn image_rm(env: &TestEnv, name: &str) -> (i32, serde_json::Value) {
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
    let _runtime = crate::shared_runtime();
    // Sabotage: drop `--network none` from the adapter's `run` argv;
    // the box gains an interface and reaches `1.1.1.1`, so the
    // interface and unreachable assertions fail. The route to the fixture is
    // the positive control that the same box still has its one way out.
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

    up.down(&env);
}

#[test]
fn a_route_reaches_exactly_one_host_service() {
    let _runtime = crate::shared_runtime();
    // Sabotage: forward the client's Host header unchanged; the evil-Host
    // request then reaches the fixture as evil.example and its assertion
    // fails.
    // Sabotage: report the route before dialing, so `status` stays null;
    // the status assertion fails. Sabotage: log the whole request target;
    // the query sentinel then appears in the log and that assertion fails.
    let env = TestEnv::new("route");
    let fixture = HttpFixture::start(None);
    let name = box_name("route");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "egress": {
            "routes": { "fixture.internal": fixture.route() },
        },
    });
    let mut up = box_up(&env, &spec, &name);

    // The route reaches the fixture, and the Host header is rewritten from
    // the absolute-form target, not passed through from the client.
    let rewritten = curl(
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

    // A route's log line names the method, the request path without its
    // query, and the status the fixture sent. The method is not curl's
    // default, so a constant GET fails (sabotage: log "GET" for every
    // route). The expected status comes from the fixture. The query's
    // sentinel never reaches the log.
    let sentinel = format!("pf-sentinel-{}", std::process::id());
    let missing = curl(
        &env,
        &name,
        "5",
        &[
            "-X",
            "DELETE",
            &format!(
                "http://fixture.internal{}?{sentinel}",
                HttpFixture::NOT_FOUND_PATH
            ),
        ],
    );
    assert_eq!(
        missing.code, 0,
        "the missing route failed: {}",
        missing.stderr
    );
    let log = egress_log(&env, &name);
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
                && line["path"] == HttpFixture::NOT_FOUND_PATH
        })
        .unwrap_or_else(|| panic!("no route line for the missing path: {lines:?}"));
    assert_eq!(line["method"], "DELETE", "the route line's method: {line}");
    assert_eq!(
        line["status"],
        HttpFixture::NOT_FOUND_STATUS,
        "the route line's status: {line}"
    );
    // Sabotage: ignore an audit append error or omit its owner notification.
    // Earlier requests prove this route works with a writable log. Replacing
    // ready's regular log with a directory causes a real host write failure.
    let log_path = PathBuf::from(up.ready["egress_log"].as_str().expect("ready log path"));
    fs::remove_file(&log_path).expect("remove the audit log");
    fs::create_dir(&log_path).expect("make audit append fail");
    let failed = curl(&env, &name, "5", &["http://fixture.internal/"]);
    assert_ne!(failed.code, 0, "request succeeded without its audit record");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = up
            .starting
            .child
            .try_wait()
            .expect("observe audit shutdown")
        {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "audit failure did not stop the owner"
        );
        std::thread::yield_now();
    };
    let lines = up.starting.rest();
    assert_eq!(
        lines.last().expect("audit down line")["reason"],
        "audit-log"
    );
    assert_eq!(status.code(), Some(1), "audit failure's exit status");
    // This guard queries the runtime directly and also cleans up on failure.
    let runtime_box = RuntimeContainer { name: name.clone() };
    assert!(!runtime_box.listed(), "audit failure left a runtime box");
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
    let log = egress_log(&env, &name);
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
    let env = TestEnv::new("login");
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
    let log = egress_log(&env, &name);
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
    let box_home = TestDir::new(&env, "codex-box-home");
    let codex_spec = serde_json::json!({
        "name": codex_name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "login" },
        "harness": "codex",
        "mounts": [{ "host": box_home.path(), "guest": "/home/codex" }],
        "env": { "HOME": "/home/codex" },
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
    request
        .recv_timeout(std::time::Duration::from_secs(90))
        .expect("the first helper reached the held refresh fixture");

    // Separate state/config roots, but the cache is shared and the first
    // helper reaching the fixture proves its host artifact is installed.
    let contender = TestEnv::new("login-contender");
    let contender_name = box_name("login-contender");
    let contender_home = TestDir::new(&contender, "home");
    let mut contender_spec = codex_spec.clone();
    contender_spec["name"] = serde_json::json!(contender_name);
    contender_spec["mounts"][0]["host"] = serde_json::json!(contender_home.path());
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
    assert_eq!(
        refresh.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "a second helper refreshed while the first held the login lock"
    );
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
    // the spec's home mount, and the writable paths. A canary written to
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
             /etc /opt/pinfold /tmp /workspace /home/codex; true",
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
    let log = egress_log(&env, &codex_name);
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
        let home = TestDir::new(&env, "codex-live-home");
        let live_name = box_name("login-live");
        let live_spec = serde_json::json!({
            "name": live_name,
            "image": default_image(&env),
            "labels": { "dev.example.test": "login" },
            "harness": "codex",
            "mounts": [{ "host": home.path(), "guest": "/home/codex" }],
            "env": { "HOME": "/home/codex" },
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
                "/home/codex",
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
    // route in the spec is the positive control.
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

    up.down(&env);
}

#[test]
fn a_caller_can_tell_an_oom_kill_from_a_failure() {
    let _runtime = crate::shared_runtime();
    // Guarantee 20: a caller can tell an OOM kill from a failure.
    // Sabotage: read `memory.events` but report `high` instead of `oom_kill`;
    // with no memory.high set the count stays 0 and the post-exec assertion
    // fails.
    // Sabotage: skip the oom_score_adj write in init's exec wrapper; `cat`
    // then prints 0 and the oom_score_adj assertion fails. (The original
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
    let home = TestDir::new(&env, "home");
    default_image(&env);

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

        // A caller-owned box: the spec names the harness and the allow list;
        // the profile supplies the image and home seeds.
        let name = box_name(&format!("harness-{harness}"));
        let spec = serde_json::json!({
            "name": name,
            "profile": "default",
            "harness": harness,
            "mounts": [{ "host": home.path(), "guest": "/home/harness" }],
            "env": { "HOME": "/home/harness" },
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

#[test]
fn a_caller_owned_box_cannot_write_git() {
    use std::os::unix::fs::MetadataExt;
    let _runtime = crate::shared_runtime();
    // Guarantee 22: a caller-owned box cannot write `.git`.
    // Sabotage: drop `readonly` from the adapter's bind mounts (pass `false`
    // for `mount.readonly` in runtime/mod.rs); `.git` is then writable and
    // the hook write assertion fails. The worktree write is the positive
    // control that the same write works on a writable mount.
    let env = TestEnv::new("caller-git");
    let name = box_name("caller-git");
    let image = default_image(&env);
    let repo = TestDir::new(&env, "repo");
    let root = repo.path().to_string_lossy().into_owned();
    let dot_git = repo.path().join(".git");
    // On Apple the top directory of every mount is root:root inside the
    // box while the files under it keep the host ids: `stat` printed 0:0
    // for REPO and REPO/.git and 501:20 for REPO/f.txt, so git refuses any
    // mounted repository until `safe.directory` names it.
    let safe = format!("safe.directory={root}");

    // One host commit, so the box has history to read.
    git(repo.path(), &["init", "-q"]);
    fs::write(repo.path().join("committed.txt"), b"one\n").expect("write committed.txt");
    git(repo.path(), &["add", "committed.txt"]);
    git(
        repo.path(),
        &[
            "-c",
            "user.name=a",
            "-c",
            "user.email=a@b",
            "commit",
            "-q",
            "-m",
            "one",
        ],
    );

    // The `.git` mount first and read-only, the repository second and
    // writable: a runtime that applied mounts in spec order would let the
    // parent shadow `.git`, so this checks that both runtimes apply a
    // nested mount by path.
    let mut spec = serde_json::json!({
        "name": name,
        "image": image,
        "labels": { "dev.example.test": "caller-git" },
        "mounts": [
            { "host": dot_git, "guest": dot_git, "readonly": true },
            { "host": repo.path(), "guest": repo.path(), "readonly": false },
        ],
    });
    let config = dot_git.join("config");
    let expected = fs::read(&config).expect("read host config");
    let extra = TestDir::new(&env, "extra");
    let root_alias = repo.path().join("config-alias");
    let extra_alias = extra.path().join("config-alias");
    fs::hard_link(&config, &root_alias).expect("link config in project");
    let original = fs::metadata(&config).unwrap();
    let linked = fs::metadata(&root_alias).unwrap();
    assert_eq!(
        (original.dev(), original.ino()),
        (linked.dev(), linked.ino())
    );
    // Guarantee 22 admission. Sabotage: omit the check, or omit additional
    // writable exports. Neither layout may reach ready. Expected bytes are
    // the host config before launch; no guest write runs in refused cases.
    for in_extra in [false, true] {
        if in_extra {
            fs::remove_file(&root_alias).unwrap();
            fs::hard_link(&config, &extra_alias).unwrap();
            spec["mounts"].as_array_mut().unwrap().reverse();
            spec["mounts"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "host": extra.path(), "guest": "/extra", "readonly": false,
                }));
        }
        let (code, refused) = box_up_refused(&env, &spec, &[]);
        assert_eq!(code, 1, "wrong refusal exit: {refused}");
        assert_eq!(refused["event"], "refused", "reached ready: {refused}");
        assert_eq!(refused["reason"], "mount-alias", "wrong reason: {refused}");
        let detail = refused["detail"].as_str().unwrap();
        assert!(detail.contains(config.to_str().unwrap()), "{refused}");
        let writable = if in_extra {
            "/extra/config-alias"
        } else {
            root_alias.to_str().unwrap()
        };
        assert!(detail.contains(writable), "{refused}");
        assert_left_nothing(&env, &name, "dev.example.test=caller-git", "refused");
        assert_eq!(fs::read(&config).unwrap(), expected);
    }
    fs::remove_file(&extra_alias).unwrap();

    // Sabotage: use nlink as permission. A single-link file still conflicts
    // when its source directory has a second, writable guest export.
    let single = dot_git.join("single");
    fs::create_dir(&single).unwrap();
    fs::write(single.join("file"), b"protected\n").unwrap();
    assert_eq!(fs::metadata(single.join("file")).unwrap().nlink(), 1);
    spec["mounts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "host": single, "guest": "/duplicate", "readonly": false,
        }));
    let (code, refused) = box_up_refused(&env, &spec, &[]);
    assert_eq!(code, 1);
    assert_eq!(refused["event"], "refused");
    assert_eq!(refused["reason"], "mount-alias");
    assert_left_nothing(&env, &name, "dev.example.test=caller-git", "refused");
    assert_eq!(fs::read(single.join("file")).unwrap(), b"protected\n");
    spec["mounts"].as_array_mut().unwrap().pop();

    // Restore .git before its writable parent for runtime coverage.
    // Sabotage: apply mounts in input order; the hook write then succeeds.
    spec["mounts"].as_array_mut().unwrap().swap(0, 1);

    // Sabotage: blanket nlink rejection (including Git objects). An object
    // shared with a local clone outside every writable export is admitted.
    let outside = TestDir::new(&env, "outside-clone");
    let commit = git(repo.path(), &["rev-parse", "HEAD"]);
    let commit = commit.trim();
    let object = dot_git
        .join("objects")
        .join(&commit[..2])
        .join(&commit[2..]);
    let object_bytes = fs::read(&object).unwrap();
    fs::hard_link(&object, outside.path().join("object")).unwrap();
    let mut up = box_up(&env, &spec, &name);

    // The box reads history and status, and writes the worktree.
    let log = box_exec(
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
        &env,
        &name,
        &["git", "-C", &root, "-c", &safe, "status", "--porcelain"],
    );
    assert_eq!(status.code, 0, "box git status failed: {}", status.stderr);
    let wrote = box_exec(
        &env,
        &name,
        &["sh", "-c", &format!("echo x > '{root}/new.txt'")],
    );
    assert_ok(&wrote, "writing the worktree");
    assert_eq!(fs::read(repo.path().join("new.txt")).unwrap(), b"x\n");
    assert_eq!(fs::read(&object).unwrap(), object_bytes);

    // A write into `.git` fails.
    let hook = dot_git.join("hooks/pre-commit");
    let denied = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("printf '#!/bin/sh\\n' > '{}'", hook.display()),
        ],
    );
    assert_denied(&denied, "Read-only file system", "the hook write");

    up.down(&env);
    assert!(up.wait().success(), "box up did not exit cleanly");
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
pub(crate) struct Up {
    name: String,
    /// The parsed `ready` line.
    ready: serde_json::Value,
    starting: Starting,
}

impl Up {
    fn pid(&self) -> u32 {
        self.starting.child.id()
    }

    fn wait(&mut self) -> ExitStatus {
        self.starting.child.wait_bounded()
    }

    fn kill(&mut self) {
        self.starting.child.kill().expect("kill box up");
    }

    /// Run `box down` on this box and assert it succeeded.
    fn down(&self, env: &TestEnv) {
        let status = box_down(env, &self.name);
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
struct Starting {
    child: ChildOwner,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Starting {
    /// Read and parse `up`'s first line.
    fn first_line(&mut self) -> serde_json::Value {
        let line = read_bounded(
            &mut self.child,
            &mut self.stdout,
            std::time::Duration::from_secs(90),
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
    fn into_up(self, name: &str, ready: serde_json::Value) -> Up {
        Up {
            name: name.to_string(),
            ready,
            starting: self,
        }
    }
}

/// Spawn `box up` and write its spec, reading nothing, so a test can start
/// several at once.
fn box_up_start(env: &TestEnv, spec: &serde_json::Value, vars: &[(&str, &str)]) -> Starting {
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
pub(crate) fn assert_left_nothing(env: &TestEnv, name: &str, label: &str, what: &str) {
    let boxes = env.state.join("pinfold").join("boxes");
    let leftovers: Vec<PathBuf> = fs::read_dir(&boxes)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "the {what} up for {name} left a state dir: {leftovers:?}"
    );
    assert!(box_list(env, label).is_empty(), "the {what} up left a box");
}

/// A container started with the runtime's own CLI, not pinfold, and
/// removed with it on drop.
struct RuntimeContainer {
    name: String,
}

impl RuntimeContainer {
    fn run(name: &str, image: &str, label: Option<&str>) -> RuntimeContainer {
        let mut command = Command::new(image_cli());
        command.args(["run", "-d", "--name", name]);
        if let Some(label) = label {
            command.args(["--label", label]);
        }
        run_ok(command.args([image, "sleep", "infinity"]));
        RuntimeContainer {
            name: name.to_string(),
        }
    }

    /// Whether the runtime's own container list names it: podman's `Names`,
    /// Apple's `id`.
    fn listed(&self) -> bool {
        let list = if cfg!(target_os = "linux") {
            "ps"
        } else {
            "list"
        };
        let output = run_ok(Command::new(image_cli()).args([list, "--all", "--format", "json"]));
        let containers: Vec<serde_json::Value> =
            serde_json::from_slice(&output.stdout).expect("the runtime's container list is JSON");
        containers.iter().any(|container| {
            container["id"] == self.name.as_str()
                || container["Names"]
                    .as_array()
                    .is_some_and(|names| names.iter().any(|name| name == self.name.as_str()))
        })
    }
}

impl Drop for RuntimeContainer {
    fn drop(&mut self) {
        // Best effort: a Drop during unwinding must not panic. `sleep`
        // ignores podman's SIGTERM, so skip its grace period.
        let mut command = Command::new(image_cli());
        command.args(["rm", "--force"]);
        if cfg!(target_os = "linux") {
            command.args(["--time", "0"]);
        }
        let _ = command.arg(&self.name).output();
    }
}

fn box_down(env: &TestEnv, name: &str) -> ExitStatus {
    env.command(pinfold())
        .args(["box", "down", name])
        .stdin(Stdio::null())
        .status()
        .expect("run pinfold box down")
}
