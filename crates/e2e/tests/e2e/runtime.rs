//! Tests that require exclusive access to the shared host runtime.
//! They live together so they start after every shared test: libtest starts
//! tests in name order, and a queued writer blocks later readers.

use std::fs::{self, File};
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
#[cfg(target_os = "macos")]
use std::process::Stdio;
use std::time::{Duration, SystemTime};

#[cfg(target_os = "macos")]
use e2e::image_cli;
use e2e::{
    HttpFixture, ImageCleanup, TestEnv, assert_denied, assert_ok, box_exec, box_list, curl,
    default_image, exit_code, git, image_id, inspect, json_lines, pinfold, profile_containerfile,
    project_id, project_state_dir, run_dir, run_ok, tagged_images, test_dir,
};

#[cfg(target_os = "macos")]
use super::box_::RuntimeContainer;
#[cfg(target_os = "linux")]
use super::box_::vmm_pid;
use super::box_::{
    Starting, assert_left_nothing, box_name, box_up, box_up_refused, box_up_start,
    find_box_state_dir, image_build, image_rm, one_json_line,
};
use super::runs::run_id;

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    let _runtime = crate::exclusive_runtime();
    // Guarantee 15: cleanup removes only pinfold's garbage.
    // Sabotage: make --dry-run run the real clean; the idle state or disk
    // disappears. Drop the live-project check from clean --unused; the
    // live state or disk goes. Skip unused disk removal; the idle disk
    // survives. Make plain clean remove project disks; the idle disk goes
    // before --unused. Expected state bytes and disk contents are fixtures.
    // Sabotage: drop the in-use check from image rm; the caller image goes
    // while its box still uses it. Apply retention's grace to image rm;
    // the fresh image survives after its box stops. Runtime inventory and
    // the running box observe image lifetime independently of the rm line.
    // On Apple, drop the pinfold_box check from clean::boxes; the foreign
    // runtime container, which has no owner label, is removed. Remove all
    // runtime images instead of only managed ones; the unlabeled image goes.
    let env = TestEnv::new("cleanup");
    let base = default_image(&env);
    let missing = format!("e2e-maintenance-{}-missing", std::process::id());
    profile_containerfile(&env, &missing, "FROM scratch\n");
    let seed_project = |project: &Path, text: &[u8]| {
        git(project, &["init", "-q"]);
        let refused = env
            .command(pinfold())
            .args(["pi", "--version"])
            .env("PINFOLD_PROFILE", &missing)
            .current_dir(project)
            .output()
            .expect("run pinfold pi");
        assert!(!refused.status.success(), "pi ran a profile with no image");
        let marker = project_state_dir(&env, project).join("marker");
        fs::write(&marker, text).unwrap();
        marker
    };
    let live_project = test_dir(&env, "live-project");
    let live_marker = seed_project(&live_project, b"live\n");
    let live_state = project_state_dir(&env, live_project.as_path());
    let live_id = project_id(&env, live_project.as_path());
    let other_project = test_dir(&env, "other-project");
    let other_marker = seed_project(&other_project, b"other\n");
    let other_state = project_state_dir(&env, other_project.as_path());
    let other_id = project_id(&env, other_project.as_path());
    let live_disk = format!("pi-{live_id}-home");
    let other_disk = format!("pi-{other_id}-home");

    // One project image supplies both the live-project label and tools to
    // write its attached disk. The idle project needs state and a disk only.
    let containerfile = live_project.as_path().join("Containerfile");
    fs::write(&containerfile, format!("FROM {base}\n")).unwrap();
    let live_repository = if cfg!(target_os = "linux") {
        format!("localhost/pinfold/project-{live_id}")
    } else {
        format!("e2e-cleanup-live-{}", std::process::id())
    };
    let _live_images = ImageCleanup {
        repository: live_repository.clone(),
    };
    let live_ref = format!("{live_repository}:latest");
    let live_label = format!("dev.pinfold.project={live_id}");
    #[cfg(target_os = "linux")]
    {
        fs::write(
            live_project.as_path().join(".pinfold.toml"),
            format!("profile = \"{missing}\"\ncontainerfile = \"Containerfile\"\n"),
        )
        .unwrap();
        e2e::allow(&env, live_project.as_path());
        run_ok(
            env.command(pinfold())
                .arg("build")
                .current_dir(live_project.as_path()),
        );
    }
    // The project fixture must not inherit the base's profile identity.
    #[cfg(target_os = "macos")]
    run_ok(
        Command::new(image_cli())
            .env_remove("NO_COLOR")
            .env_remove("BUILDKIT_COLORS")
            .args(["build", "--file"])
            .arg(&containerfile)
            .args(["--tag", &live_ref, "--label", &live_label])
            .args(["--label", "dev.pinfold.profile="])
            .arg(live_project.as_path())
            .stdin(Stdio::null()),
    );
    let live = box_name("cleanup-live");
    let live_spec = serde_json::json!({
        "name": live,
        "image": live_ref,
        "disks": [{ "name": live_disk, "guest": "/home-disk" }],
    });
    let _live = box_up(&env, &live_spec, &live);
    assert_ok(
        &box_exec(
            &env,
            &live,
            &[
                "sh",
                "-c",
                "printf 'live disk write\\n' > /home-disk/marker",
            ],
        ),
        "writing the live project's disk",
    );

    let creator = box_name("cleanup-disk");
    let mut up = box_up(
        &env,
        &serde_json::json!({
            "name": creator,
            "image": base,
            "disks": [{ "name": other_disk, "guest": "/disk" }],
        }),
        &creator,
    );
    up.down(&env);
    assert!(up.wait().success(), "the disk's box did not exit cleanly");
    #[cfg(target_os = "macos")]
    let unlabeled_repository =
        format!("localhost/pinfold/e2e-cleanup-unlabeled-{}", std::process::id());
    #[cfg(target_os = "macos")]
    let _unlabeled_images = ImageCleanup {
        repository: unlabeled_repository.clone(),
    };
    #[cfg(target_os = "macos")]
    let unlabeled = {
        // This external runtime fixture has no pinfold labels. It is not
        // one of the two case builds through pinfold's image lifecycle.
        let context = test_dir(&env, "unlabeled-context");
        let recipe = context.as_path().join("Containerfile");
        fs::write(&recipe, "FROM scratch\n").unwrap();
        let reference = format!("{unlabeled_repository}:latest");
        run_ok(
            Command::new(image_cli())
                .env_remove("NO_COLOR")
                .env_remove("BUILDKIT_COLORS")
                .args(["build", "--file"])
                .arg(&recipe)
                .args(["--tag", &reference])
                .arg(context.as_path())
                .stdin(Stdio::null()),
        );
        assert!(
            image_id(&reference).is_some(),
            "the unlabeled image is missing"
        );
        reference
    };
    #[cfg(target_os = "macos")]
    let user_container = RuntimeContainer::run(&box_name("cleanup-user"), base, None);

    let listed = box_list(&env, &live_label);
    let live_box = listed
        .iter()
        .find(|box_| box_["name"] == live)
        .unwrap_or_else(|| panic!("the live box is missing before clean: {listed:?}"));
    assert_eq!(live_box["owner_alive"], true, "{live_box:?}");
    assert_eq!(fs::read(&live_marker).unwrap(), b"live\n");
    assert_eq!(fs::read(&other_marker).unwrap(), b"other\n");
    let listed = disk_names(&env);
    assert!(listed.contains(&live_disk) && listed.contains(&other_disk));
    #[cfg(target_os = "macos")]
    assert!(user_container.listed(), "the foreign container is missing");

    run_ok(
        env.command(pinfold())
            .args(["clean", "--dry-run", "--unused", "0s"]),
    );
    assert_eq!(fs::read(&other_marker).unwrap(), b"other\n");
    assert_eq!(fs::read(&live_marker).unwrap(), b"live\n");
    let listed = disk_names(&env);
    assert!(listed.contains(&live_disk) && listed.contains(&other_disk));
    run_ok(env.command(pinfold()).arg("clean"));
    let listed = disk_names(&env);
    assert!(listed.contains(&live_disk) && listed.contains(&other_disk));

    run_ok(env.command(pinfold()).args(["clean", "--unused", "0s"]));
    let listed = disk_names(&env);
    assert!(listed.contains(&live_disk), "clean removed the live disk");
    assert!(!listed.contains(&other_disk), "clean kept the idle disk");
    assert!(!other_state.exists(), "clean kept the idle project state");
    assert!(live_state.is_dir(), "clean removed the live project state");
    assert_eq!(fs::read(&live_marker).unwrap(), b"live\n");
    let written = box_exec(&env, &live, &["cat", "/home-disk/marker"]);
    assert_ok(&written, "reading the live disk after clean");
    assert_eq!(written.stdout, "live disk write\n");
    #[cfg(target_os = "macos")]
    assert!(
        user_container.listed(),
        "clean removed the foreign container"
    );
    #[cfg(target_os = "macos")]
    assert!(
        image_id(&unlabeled).is_some(),
        "clean removed the unlabeled image"
    );

    // One fresh caller image: its last reference must remain while a box
    // holds it, then disappear on the same rm after that box stops.
    let context = test_dir(&env, "image-rm-context");
    let recipe = context.as_path().join("Containerfile");
    fs::write(
        &recipe,
        format!("FROM {base}\nCOPY marker.txt /marker.txt\n"),
    )
    .unwrap();
    fs::write(context.as_path().join("marker.txt"), "retire\n").unwrap();
    let name = format!("e2e-maintenance-{}-retire", std::process::id());
    let repository = format!("localhost/pinfold/image-{name}");
    let _images = ImageCleanup {
        repository: repository.clone(),
    };
    let (code, built) = image_build(&env, &name, &recipe, context.as_path());
    assert_eq!(code, 0, "{built}");
    assert_eq!(built["event"], "built", "{built}");
    let reference = built["ref"].as_str().expect("caller build has a ref");
    let id = image_id(reference).expect("the caller image exists before rm");
    let pinned = box_name("cleanup-retire");
    let mut up = box_up(
        &env,
        &serde_json::json!({ "name": pinned, "image": reference }),
        &pinned,
    );
    #[cfg(target_os = "linux")]
    let (root_disk, root_identity) = {
        // Observe the open root, not an older completed representation.
        let vmm = vmm_pid(up.pid());
        let opened: std::collections::BTreeSet<_> = fs::read_dir(format!("/proc/{vmm}/fd"))
            .expect("read the VMM's descriptors")
            .filter_map(|entry| {
                let path = entry.expect("read a VMM descriptor").path();
                match fs::metadata(&path) {
                    Ok(metadata) => Some((metadata.dev(), metadata.ino())),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => panic!("stat VMM descriptor {}: {error}", path.display()),
                }
            })
            .collect();
        let mut roots: Vec<_> = fs::read_dir(
            env.cache
                .join("pinfold/images")
                .join(id.strip_prefix("sha256:").unwrap()),
        )
        .expect("read the caller image's files")
        .filter_map(|entry| {
            let path = entry.expect("read an image file").path();
            let metadata = fs::symlink_metadata(&path).expect("stat an image file");
            metadata
                .is_file()
                .then_some((path, (metadata.dev(), metadata.ino())))
        })
        .filter(|(_, identity)| opened.contains(identity))
        .collect();
        assert_eq!(
            roots.len(),
            1,
            "the VMM must open exactly one caller-image backing file: {roots:?}"
        );
        roots.pop().unwrap()
    };
    let (code, removed) = image_rm(&env, &name);
    assert_eq!(code, 0, "{removed}");
    assert_eq!(removed["event"], "removed", "{removed}");
    assert_eq!(removed["in_use"], serde_json::json!([&id]), "{removed}");
    assert!(
        tagged_images(&repository).contains(&id),
        "rm removed the live image"
    );
    // Sabotage: remove a completed root despite its live image lease.
    #[cfg(target_os = "linux")]
    {
        let metadata = fs::metadata(&root_disk).expect("rm removed the live root disk");
        assert_eq!(
            (metadata.dev(), metadata.ino()),
            root_identity,
            "rm replaced the VMM's live root disk"
        );
    }
    let observed = box_exec(&env, &pinned, &["cat", "/marker.txt"]);
    assert_ok(&observed, "the caller box after image rm");
    assert_eq!(observed.stdout, "retire\n");
    up.down(&env);
    assert!(up.wait().success(), "the caller box did not exit cleanly");
    let (code, removed) = image_rm(&env, &name);
    assert_eq!(code, 0, "{removed}");
    assert!(
        tagged_images(&repository).is_empty(),
        "rm kept the stopped image"
    );
    // Sabotage: leave a completed root after its last reference goes.
    #[cfg(target_os = "linux")]
    assert!(!root_disk.exists(), "rm kept the stopped root disk");
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
    // the refilled tree assertion fails.
    let env = TestEnv::new("disks");
    let image = default_image(&env);
    let names = ["work", "sparse", "race", "nested", "refill"]
        .map(|disk| box_name(&format!("disk-{disk}")));
    let [work, sparse, race, nested, refill] = &names;
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

    // The host tree: a plain file, an executable and a symlink.
    let from = test_dir(&env, "from");
    fs::write(from.as_path().join("plain.txt"), b"plain\n").unwrap();
    fs::write(from.as_path().join("run.sh"), b"#!/bin/sh\necho ran\n").unwrap();
    fs::set_permissions(
        from.as_path().join("run.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::create_dir(from.as_path().join("sub")).unwrap();
    fs::write(from.as_path().join("sub/deep.txt"), b"deep\n").unwrap();
    std::os::unix::fs::symlink("plain.txt", from.as_path().join("link")).unwrap();
    let work_disk =
        serde_json::json!({ "name": work, "guest": "/work", "size": "1G", "from": from.as_path() });
    // `from` is read only when the disk is created; a reader names none.
    let reader = serde_json::json!({ "name": work, "guest": "/work", "readonly": true });
    let mut up = box_up(&env, &spec(writer, work_disk.clone()), writer);
    fs::write(from.as_path().join("later.txt"), b"later\n").unwrap();
    let tree = box_exec(
        &env,
        writer,
        &[
            "sh",
            "-c",
            "test -x /work/run.sh && /work/run.sh && test ! -x /work/plain.txt \
             && cat /work/sub/deep.txt && test -L /work/link && readlink /work/link \
             && test ! -e /work/later.txt",
        ],
    );
    assert_ok(&tree, "the copied tree");
    assert_eq!(tree.stdout, "ran\ndeep\nplain.txt\n");
    assert_ok(
        &box_exec(&env, writer, &["sh", "-c", "echo kept > /work/written"]),
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
    let big = test_dir(&env, "big");
    File::create(big.as_path().join("sparse"))
        .unwrap()
        .set_len(2 << 30)
        .unwrap();
    let sparse_disk =
        serde_json::json!({ "name": sparse, "guest": "/big", "size": "1G", "from": big.as_path() });
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
    let mount = test_dir(&env, "mount");
    let with_mount = |name: &str| {
        let mut spec = spec(name, reader.clone());
        spec["mounts"] = serde_json::json!([{ "host": mount.as_path(), "guest": "/mount" }]);
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
    // orphaned `container run` leaves it; `container create` makes one
    // under the name a dead reader's claim holds.
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
    let first = from.as_path().join("0first.txt");
    fs::write(&first, b"first\n").unwrap();
    let refill_disk = serde_json::json!({ "name": refill, "guest": "/work", "size": "1G", "from": from.as_path() });
    let mut interrupted = box_up_start(&env, &spec(writer, refill_disk.clone()), &[]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !disk_names(&env).contains(refill) {
        assert!(
            std::time::Instant::now() < deadline,
            "the interrupted up created no disk"
        );
    }
    let fifo = from.as_path().join("fifo-swap");
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

    // A name that is no disk name is refused as `spec`; control: with no
    // box attached, `disk rm` removes the work disk.
    let (code, refused) = disk_rm(&env, "../escape");
    assert_eq!((code, &refused["reason"]), (1, &"spec".into()), "{refused}");
    let (code, removed) = disk_rm(&env, work);
    assert_eq!((code, &removed["existed"]), (0, &true.into()), "{removed}");
    assert!(!disk_names(&env).iter().any(|name| name == work));
}

fn disk_list(env: &TestEnv) -> Vec<serde_json::Value> {
    let output = run_ok(env.command(pinfold()).args(["disk", "list"]));
    json_lines(&String::from_utf8_lossy(&output.stdout))
}

fn disk_names(env: &TestEnv) -> Vec<String> {
    disk_list(env)
        .iter()
        .filter_map(|disk| disk["name"].as_str().map(String::from))
        .collect()
}

fn disk_rm(env: &TestEnv, name: &str) -> (i32, serde_json::Value) {
    one_json_line(env.command(pinfold()).args(["disk", "rm", name]))
}

#[test]
fn losing_the_owner_fails_closed() {
    let _runtime = crate::exclusive_runtime();
    // Sabotage: make `box prune` skip boxes whose owner is gone; the box
    // survives prune and the post-prune list assertion fails. Sabotage:
    // report `owner_alive` as true whenever the label parses; the dead-owner
    // `owner_alive` assertion fails. Sabotage, on Apple: start the proxy
    // outside the `box up` process; it survives the SIGKILL, logs the post-kill request
    // and lets it through, so the unchanged-log assertion fails. Curl's own
    // error is not asserted: Apple's forwarder sometimes hangs after the
    // owner dies rather than closing, so the request may end in a timeout
    // instead of a refusal. The positive control is the live box in
    // `cleanup_removes_only_pinfolds_garbage`, whose `owner_alive` is true
    // through the same test. Sabotage: make `inspect` report terminal
    // `exited` for a run whose box's owner is dead; the dead owner's run
    // reports a terminal record and the `not-observed` assertion fails.
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
    let run = run_id(&up);

    // Positive control: the box has egress while its owner lives. A Linux VM
    // ends with its owner (guarantee 43), so only Apple's box outlives it to
    // try the proxy.
    #[cfg(target_os = "macos")]
    let allowed = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
    {
        let before = e2e::json_lines(&e2e::egress_log(up.ready()));
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
        let after = e2e::json_lines(&e2e::egress_log(up.ready()));
        assert_eq!(
            after.len(),
            before.len(),
            "the egress log gained a line after the owner died: {after:?}"
        );
    }

    let old_state = find_box_state_dir(&env).expect("find the dead owner's state dir");
    // A dead owner leaves no end, and nothing infers one later.
    let no_end = |when: &str| {
        let (code, record) = inspect(&env, &run);
        assert_eq!(code, 0, "inspect {when} failed");
        assert_eq!(record["launch"]["state"], "available", "{when}: {record}");
        assert_eq!(
            record["terminal"]["state"], "not-observed",
            "{when}: {record}"
        );
        assert_eq!(record["terminal"]["record"], serde_json::Value::Null);
    };
    no_end("after the owner's SIGKILL");

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

    // `box prune` from the other root removes the leftover.
    run_ok(other.command(pinfold()).args(["box", "prune"]));
    up.wait();
    let listed = box_list(&env, label);
    assert!(
        !listed.iter().any(|box_| box_["name"] == name),
        "prune left the box: {listed:?}"
    );
    no_end("after prune");

    // A crash before publishing its record leaves only a state dir.
    // Sabotage: treat a state dir without a record as permanently live; this
    // fresh up refuses name-in-use.
    // The host-created abandoned dir is the outside fixture.
    // Reuse the actual directory observed before prune, without deriving
    // pinfold's name hash in the test.
    fs::create_dir_all(&old_state).expect("leave abandoned partial claim");
    let mut fresh = box_up(&env, &spec, &name);
    fresh.down(&env);
    assert!(fresh.wait().success(), "the fresh up did not exit cleanly");
}

/// Set the modification time of the run's directory and every file in it.
fn age(dir: &Path, age: Duration) {
    let time = SystemTime::now() - age;
    for entry in fs::read_dir(dir).expect("read the run dir") {
        let path = entry.expect("run dir entry").path();
        File::open(&path).unwrap().set_modified(time).unwrap();
    }
    File::open(dir).unwrap().set_modified(time).unwrap();
}

/// Append `bytes` of valid decision lines to the run's egress log.
fn pad(dir: &Path, bytes: usize) {
    let line = b"{\"time\":\"2026-10-01T00:00:00Z\",\"host\":\"pad.invalid\",\"decision\":\"refused\",\"reason\":\"not allowlisted\"}\n";
    let padding = line.repeat(bytes.div_ceil(line.len()));
    fs::OpenOptions::new()
        .append(true)
        .open(dir.join("egress.jsonl"))
        .expect("open the run's egress log")
        .write_all(&padding)
        .expect("pad the run's egress log");
}

/// The length of the run's egress log, read from the host file.
fn log_bytes(env: &TestEnv, run_id: &str) -> u64 {
    fs::metadata(run_dir(env, run_id).join("egress.jsonl"))
        .expect("stat the run's egress log")
        .len()
}

#[test]
fn run_records_are_retained_within_bounds() {
    // `clean` prunes dead boxes host-wide.
    let _runtime = crate::exclusive_runtime();
    // Guarantee 42. Sabotage: age out runs without checking the lifetime
    // lock; the aged live run is removed. Sabotage: drop the 14-day rule;
    // the aged newer run survives the second clean, which is under the
    // quota. Sabotage: evict the newest inactive run first; the newer
    // padded run goes and the older stays. Sabotage: truncate the live
    // run's log to meet the quota; its bytes shrink. Sabotage: count live
    // runs toward the quota; the newer padded run goes too. The live log's
    // length is the host file's, not `inspect`'s. The 128 MiB pad and the
    // backdated mtimes act on host paths the spec names, as a user's disk
    // and clock would.
    let env = TestEnv::new("run-retention");
    let fixture = HttpFixture::start(None);
    let image = default_image(&env);
    let spec = |name: &str| {
        serde_json::json!({
            "name": name,
            "image": image,
            "egress": { "routes": { "fixture.internal": fixture.route() } },
        })
    };
    let finished = |test: &str| {
        let name = box_name(test);
        let mut up = box_up(&env, &spec(&name), &name);
        let run = run_id(&up);
        up.close_stdin();
        run
    };
    let older = finished("run-retention-older");
    let newer = finished("run-retention-newer");
    let name = box_name("run-retention-live");
    let mut live_up = box_up(&env, &spec(&name), &name);
    let live = run_id(&live_up);

    // Inactive runs together hold just over 128 MiB; the live one would
    // tip the newer over the quota if it counted. Neither inactive run is
    // old enough to retire by age.
    let day = Duration::from_secs(24 * 60 * 60);
    pad(&run_dir(&env, &older), (64 << 20) + (256 << 10));
    pad(&run_dir(&env, &newer), (64 << 20) + (256 << 10));
    pad(&run_dir(&env, &live), 64 << 20);
    age(&run_dir(&env, &live), 15 * day);
    age(&run_dir(&env, &older), 3 * day);
    age(&run_dir(&env, &newer), 2 * day);
    let live_bytes = log_bytes(&env, &live);

    run_ok(env.command(pinfold()).arg("clean"));

    assert_eq!(inspect(&env, &older).0, 3, "the older padded run was kept");
    let (code, record) = inspect(&env, &newer);
    assert_eq!(code, 0, "the newer padded run was removed");
    assert_eq!(record["launch"]["state"], "available", "{record}");
    assert_eq!(
        log_bytes(&env, &live),
        live_bytes,
        "the live run's log changed"
    );

    // Under the quota, only age retires: the newer run goes once aged, the
    // live one stays however old.
    age(&run_dir(&env, &newer), 15 * day);
    run_ok(env.command(pinfold()).arg("clean"));
    assert_eq!(inspect(&env, &newer).0, 3, "the aged inactive run was kept");
    let (code, record) = inspect(&env, &live);
    assert_eq!(code, 0, "the aged live run was removed");
    assert_eq!(record["launch"]["state"], "available", "{record}");
    let request = curl(&env, &name, "5", &["http://fixture.internal/after-clean"]);
    assert_ok(&request, "the live run's request after clean");
    assert!(
        log_bytes(&env, &live) > live_bytes,
        "the live run's log did not keep appending"
    );
    live_up.close_stdin();
}
