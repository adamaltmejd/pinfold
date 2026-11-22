//! Guarantees 11 and 45: a disk outlives its box, attaches to one writer
//! at a time and is not read while unclean; export writes only a new
//! destination.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::thread;

use e2e::{BoxUp, COMMAND, TestEnv, box_name, default_image, hold_maintenance, json_lines, run};
use serde_json::{Value, json};

/// A box with one disk at `/data`.
fn spec(name: &str, disk: &str, readonly: bool) -> Value {
    json!({
        "name": name,
        "image": default_image(),
        "disks": [{ "name": disk, "guest": "/data", "size": "1G", "readonly": readonly }],
    })
}

/// The reason `up` gave instead of `ready`.
fn refusal(started: Result<BoxUp, Value>) -> String {
    let line = match started {
        Ok(up) => panic!("came ready instead of refused: {}", up.ready()),
        Err(line) => line,
    };
    assert_eq!(line["event"], "refused", "{line}");
    line["reason"].as_str().unwrap_or_default().to_string()
}

#[test]
fn disks_persist_and_attach_exclusively() {
    // Sabotage (run): delete the `disk-unclean` refusal in core/disk.rs's
    // `claim`. The read-only attachment after the prune then comes ready.
    // The writable recovery run and the read-only run after it are the
    // positive controls.
    default_image();
    let env = TestEnv::new("disk-attach");
    let disk = format!("e2e-{}-attach", std::process::id());
    let (first, second) = (box_name("attach-a"), box_name("attach-b"));

    // Two writable `up`s race for one new disk.
    let (a, b) = thread::scope(|scope| {
        let a = scope.spawn(|| env.start(&spec(&first, &disk, false)));
        let b = scope.spawn(|| env.start(&spec(&second, &disk, false)));
        (a.join().unwrap(), b.join().unwrap())
    });
    let (writer, writer_name, refused) = match (a, b) {
        (Ok(up), Err(line)) => (up, &first, line),
        (Err(line), Ok(up)) => (up, &second, line),
        (a, b) => panic!(
            "exactly one writer must run: {:?}, {:?}",
            a.map(|_| "ready"),
            b.map(|_| "ready")
        ),
    };
    assert_eq!(refusal(Err(refused)), "disk-in-use");

    let marker = format!("pf-disk-{}", std::process::id());
    let write = env.exec(
        writer_name,
        &[
            "sh",
            "-c",
            "echo \"$1\" > /data/marker && sync",
            "sh",
            &marker,
        ],
    );
    assert_eq!(write.code, 0, "{write:?}");

    // The dead writer's claim pins the disk until a prune retires it; then
    // the disk is unclean until a writable run ends.
    let reader = box_name("attach-reader");
    let hold = hold_maintenance();
    let dead = writer.kill_owner();
    assert_eq!(
        refusal(env.start(&spec(&reader, &disk, true))),
        "disk-in-use"
    );
    let pruned = run(env.command().args(["box", "prune"]), COMMAND);
    assert_eq!(pruned.code, 0, "{pruned:?}");
    assert_eq!(
        refusal(env.start(&spec(&reader, &disk, true))),
        "disk-unclean"
    );
    // Only now: the guard's cleanup could itself retire the claim.
    drop(hold);
    drop(dead);

    // Control: the next box to attach the disk reads the writer's file.
    let read = |name: &str| {
        let out = env.exec(name, &["cat", "/data/marker"]);
        assert_eq!(out.code, 0, "{out:?}");
        out.stdout.trim().to_string()
    };
    let recovery = env.up(&spec(&first, &disk, false));
    assert_eq!(read(&first), marker);

    // An attached disk is not removed.
    let rm = |env: &TestEnv| {
        let out = run(env.command().args(["disk", "rm", &disk, "--json"]), COMMAND);
        (out.code, json_lines(&out.stdout))
    };
    let (code, lines) = rm(&env);
    assert_eq!(code, 1, "{lines:?}");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(
        (&lines[0]["event"], &lines[0]["disk"], &lines[0]["reason"]),
        (&json!("refused"), &json!(disk), &json!("disk-in-use")),
        "{}",
        lines[0]
    );
    drop(recovery);

    // After the writable run ends with `down`, a read-only one comes up.
    drop(env.up(&spec(&reader, &disk, true)));
    let (code, lines) = rm(&env);
    assert_eq!(code, 0, "{lines:?}");
    assert_eq!(
        lines,
        [json!({ "event": "removed", "disk": disk, "existed": true })]
    );
}

/// The names in a directory, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn disk_export_writes_only_a_new_destination() {
    // Sabotage (run): in core/disk_export.rs's `Staging::new`, drop
    // AT_SYMLINK_NOFOLLOW from the `fstatat`, and publish with plain
    // `renameat`. The dangling link at the destination then passes the
    // check, and the rename fails without naming `destination-exists`.
    // Sabotage (run): in `receive`, after `symlinkat`, `fchmodat` the new
    // link to 0644 without AT_SYMLINK_NOFOLLOW. The 0600 host target then
    // becomes 0644.
    // The export into a new destination is the positive control.
    default_image();
    let env = TestEnv::new("export");
    let disk = format!("e2e-{}-export", std::process::id());
    let root = env.root().to_path_buf();

    // A host file the box links to, mode 0600, so a mode normalization
    // that follows the link changes it.
    let target = root.join("host-target");
    fs::write(&target, "host\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    let before = fs::metadata(&target).unwrap();

    let name = box_name("export");
    let up = env.up(&spec(&name, &disk, false));
    let made = env.exec(
        &name,
        &[
            "sh",
            "-c",
            "ln -s \"$1\" /data/link && printf '#!/bin/sh\\n' > /data/tool && chmod 755 /data/tool \
             && sync",
            "sh",
            target.to_str().unwrap(),
        ],
    );
    assert_eq!(made.code, 0, "{made:?}");
    drop(up);

    let export = |dest: &Path| {
        let out = run(
            env.command()
                .args(["disk", "export", &disk])
                .arg(dest)
                .arg("--json"),
            COMMAND,
        );
        (out.code, json_lines(&out.stdout), out.stderr)
    };

    // An existing destination that is a dangling symlink: a check that
    // follows it finds nothing there.
    let existing = root.join("existing");
    let absent = root.join("absent");
    std::os::unix::fs::symlink(&absent, &existing).unwrap();
    let inode = fs::symlink_metadata(&existing).unwrap().ino();
    let (code, lines, stderr) = export(&existing);
    assert_eq!(code, 1, "{lines:?} {stderr}");
    assert!(
        stderr
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .any(|word| word == "destination-exists"),
        "{stderr}"
    );
    assert_eq!(fs::symlink_metadata(&existing).unwrap().ino(), inode);
    assert_eq!(fs::read_link(&existing).unwrap(), absent);

    // Control: a new destination.
    let dest = root.join("exported");
    let (code, lines, _) = export(&dest);
    assert_eq!(code, 0, "{lines:?}");
    assert_eq!(
        lines,
        [json!({ "event": "exported", "disk": disk, "destination": dest.to_str().unwrap() })]
    );
    assert_eq!(names(&dest), ["link", "tool"]);
    let link = dest.join("link");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&link).unwrap(), target);
    assert_eq!(
        (
            mode(&dest.join("tool")),
            fs::read(dest.join("tool")).unwrap()
        ),
        (0o755, b"#!/bin/sh\n".to_vec())
    );
    let after = fs::metadata(&target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"host\n");
    assert_eq!(
        (
            after.ino(),
            after.mtime(),
            after.mtime_nsec(),
            mode(&target)
        ),
        (
            before.ino(),
            before.mtime(),
            before.mtime_nsec(),
            before.permissions().mode() & 0o7777
        )
    );
}
