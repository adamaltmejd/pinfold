//! Tests that require exclusive access to the shared host runtime.
//! They live together so they start after every shared test: libtest starts
//! tests in name order, and a queued writer blocks later readers.

use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use e2e::{
    HttpFixture, ImageCleanup, TestEnv, assert_denied, assert_ok, box_exec, box_list,
    build_profile, curl, default_image, exit_code, image_cli, image_id, image_ids, inspect,
    json_lines, pinfold, profile_containerfile, project_id, project_state_dir, run_dir, run_ok,
    tagged_images, test_dir,
};

#[cfg(target_os = "macos")]
use super::box_::RuntimeContainer;
#[cfg(target_os = "linux")]
use super::box_::vmm_pid;
use super::box_::{
    Starting, assert_left_nothing, box_name, box_up, box_up_refused, box_up_start,
    built_unique_ref, find_box_state_dir, image_build, image_rm, one_json_line,
};
use super::runs::run_id;

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    let _runtime = crate::exclusive_runtime();
    // Guarantee 15: cleanup removes only pinfold's garbage.
    //
    // Sabotage: make `keep_two_images` return before it removes anything;
    // three images remain and the two-image assertion fails. Sabotage: drop
    // the images step from the shared automatic pass; b2 survives `clean`
    // and the post-clean two-image assertion fails. Sabotage: drop
    // the in-use skip from `keep_two_images` and restore the `?` on
    // `remove_image`; build 4 deletes b2 under box A and the "b2 is still
    // listed" assertion fails. Sabotage: drop
    // the `dev.pinfold.profile` label from the build; no image matches and
    // the count is zero. Sabotage: make `clean` remove every image instead
    // of only pinfold's own; the unlabeled image assertion fails. Sabotage:
    // drop the live-box check from `clean --unused`; the live project's
    // state is deleted and its survives assertion fails. Sabotage: make
    // `clean` remove every project state instead of only the stale ones;
    // the other project's state assertion fails. Sabotage: make `clean`
    // remove every box; the live box assertion fails. Sabotage: make
    // `clean` skip boxes whose owner is gone; the dead box assertion fails.
    // Sabotage: make `--unused` skip the
    // live-box check (drop `live_projects` from `CleanPlan::measure`'s
    // stale test); the live project's state goes and its assertion fails.
    // Sabotage: in `clean::boxes`, insert into `live_projects` above `if
    // alive`; the dead box then protects the other project, whose state
    // survives `--unused`, and the removal assertion fails. Sabotage: drop the
    // in-use check from `remove_images`; the live box's last tag is offered
    // to the runtime, which deletes the image under the box, and the
    // live-box image assertion fails. Sabotage: count only the source's
    // own tags in `remove_images`; the twin box's shared image is reported
    // in use, the retired name's tag on it survives, and the assertion that
    // the retired name's free tags are gone fails. Sabotage: make
    // `remove_images` remove every reference of an image's id instead of
    // only the name's tags; the twin's tag goes with the shared image and
    // the twin assertion fails. Sabotage: apply the newest-two rule or
    // the caller's one-hour grace in `image rm`; a fresh build survives and
    // the removed-tags assertion fails. Sabotage: match the name as a prefix in
    // `remove_images`; the image of the name extending the retired one goes
    // and its survival assertion fails. Sabotage: make `--dry-run` run the
    // real clean; the other project's state goes and the dry-run assertion
    // fails. Sabotage: on Linux, keep podman's repeated `Names` in
    // `list_images`; the second removal of the newest image's tag is
    // `image not known` and the one-line assertion fails.
    // Sabotage: drop the `pinfold_box` check from `clean::boxes`; on Apple
    // the user's container, which has no owner label, is removed as a dead
    // box and its survival assertion fails. Linux lists only pinfold's own
    // claims, so no runtime container can reach `clean` there.
    let env = TestEnv::new("cleanup");
    default_image(&env);
    // A profile of this test's own, named for this run, so the operator's
    // default profile images, the other tests and a failed run's leftovers
    // cannot share the source.
    let profile = format!("e2e-maintenance-{}", std::process::id());
    let _images = ImageCleanup {
        repository: format!("localhost/pinfold/profile-{profile}"),
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

    // Box A pins b2, the older of the two, so the next build's retention
    // meets a pinned image.
    let box_a = box_name("cleanup-a");
    let pin_a = serde_json::json!({ "name": box_a, "image": b2.as_str() });
    let mut up_a = box_up(&env, &pin_a, &box_a);

    // Build 4: b2 cannot go while box A holds it. The build still exits 0.
    build_changed();
    assert!(
        image_id(&b2).is_some(),
        "the image box A pins is gone after build 4"
    );

    // Box A goes down and frees b2; the rest of the test starts its own
    // boxes.
    up_a.down(&env);
    assert!(up_a.wait().success(), "box A's up did not exit cleanly");

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
    let unlabeled = format!("localhost/pinfold/profile-{profile}:unlabeled");
    runtime_build(&[&unlabeled], &[]);
    assert!(
        image_id(&unlabeled).is_some(),
        "the unlabeled image is missing before clean"
    );

    // A project's state: a refused `pinfold pi` creates it before it names
    // the missing image. The profile is never built, so this part downloads
    // no pi artifact. Two projects get a state marker: one runs a live box,
    // the other does not.
    let missing = format!("{profile}-missing");
    profile_containerfile(&env, &missing, "FROM scratch\n");
    let seed_project = |project: &Path, text: &[u8]| {
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
    let live_disk = format!("pi-{live_id}-home");
    let other_disk = format!("pi-{other_id}-home");
    // The live box attaches its project's home disk.
    let live_spec = serde_json::json!({
        "name": live,
        "image": live_ref,
        "disks": [{ "name": live_disk, "guest": "/home-disk" }],
    });
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

    // A container the user started with Apple's CLI from a pinfold-built
    // image is not a box: pinfold never owned it.
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
    // `--unused` clean below removes, survives it.
    run_ok(
        env.command(pinfold())
            .args(["clean", "--dry-run", "--unused", "0s"]),
    );
    assert!(
        other_marker.is_file(),
        "--dry-run removed the other project's state"
    );

    // The real clean removes the dead box and only the dead box.
    run_ok(env.command(pinfold()).args(["clean"]));
    assert!(
        image_id(&unlabeled).is_some(),
        "clean removed an unlabeled image"
    );
    // No build follows box A's going down. The profile is left holding b2,
    // b3 and b4: b2 was pinned at build 4 and the two newer images stayed.
    // `clean` applies the after-build rule to every source, so the profile
    // falls to its newest two and b2 goes.
    let profile_images = labeled_images("dev.pinfold.profile", &profile);
    assert_eq!(
        profile_images.len(),
        2,
        "clean did not leave the profile at its newest two images: {profile_images:?}"
    );
    assert!(
        image_id(&b2).is_none(),
        "clean kept b2, an image past the newest two whose box went down"
    );
    assert!(
        !box_list(&env, &live_label).is_empty(),
        "clean removed a live box"
    );
    assert!(
        live_marker.is_file(),
        "clean removed a live box's project state"
    );
    assert!(
        box_list(&env, dead_label).is_empty(),
        "clean left a box whose owner is gone"
    );
    dead_up.wait();
    #[cfg(target_os = "macos")]
    assert!(
        user_container.listed(),
        "clean removed a container pinfold did not start"
    );

    // Plain `clean` keeps every project's disks. Sabotage: make
    // `CleanPlan::idle` select every project under plain `clean`; the idle
    // project's disk goes and its assertion fails. Sabotage: drop the
    // `--unused` disk removal from `CleanPlan::remove`; the idle project's
    // disk survives `--unused` and its assertion fails.
    let creator = box_name("cleanup-disk");
    let spec = serde_json::json!({
        "name": creator,
        "image": default_image(&env),
        "disks": [{ "name": other_disk, "guest": "/disk" }],
    });
    let mut up = box_up(&env, &spec, &creator);
    up.down(&env);
    assert!(up.wait().success(), "the disk's box did not exit cleanly");
    run_ok(env.command(pinfold()).arg("clean"));
    let listed = disk_names(&env);
    for disk in [&live_disk, &other_disk] {
        assert!(listed.contains(disk), "clean removed disk {disk}");
    }

    // `--unused AGE` removes the state and disks of projects not run for
    // that long, except one with a live box. Every project last ran seconds
    // ago, so `0s` ages them out and only the live one survives.
    run_ok(env.command(pinfold()).args(["clean", "--unused", "0s"]));
    let listed = disk_names(&env);
    assert!(
        listed.contains(&live_disk),
        "clean --unused removed a live project's disk"
    );
    assert!(
        !listed.contains(&other_disk),
        "clean --unused kept the idle project's disk"
    );
    assert!(
        !other_state.exists(),
        "clean --unused kept the state of a project that has not run"
    );
    assert!(
        live_state.is_dir(),
        "clean --unused removed the state of a project with a live box"
    );
    #[cfg(target_os = "macos")]
    {
        assert!(
            user_container.listed(),
            "clean --unused removed a container pinfold did not start"
        );
        drop(user_container);
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
        repository: format!("localhost/pinfold/image-{retire_name}"),
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
    // A second name extending the first, with one build: `image rm` of
    // `retire` must not touch it.
    let similar_name = format!("{retire_name}-x");
    let _similar_images = ImageCleanup {
        repository: format!("localhost/pinfold/image-{similar_name}"),
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
        repository: format!("localhost/pinfold/image-{twin_name}"),
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
    assert!(
        retire_refs[1..]
            .iter()
            .all(|reference| image_id(reference).is_none()),
        "image rm left a retired name's free tag: {retire_refs:?}"
    );
    assert!(
        image_id(&format!("localhost/pinfold/image-{retire_name}:latest")).is_none(),
        "image rm left the retire name's latest tag"
    );
    // The retire box pins an image only `retire` tags, so its last tag
    // stays and is reported.
    assert_eq!(
        removed["in_use"],
        serde_json::json!([retire_ids[0]]),
        "image rm did not report the last tag it kept: {removed}"
    );
    assert!(
        image_id(&retire_refs[0]).is_some(),
        "image rm removed the live box's last tag"
    );
    // A Linux box boots its image's root disk. The retire box's image just
    // lost every tag but the one its use holds; its root disk stays.
    // Sabotage: drop the listed-image check from `prune_root_disks`; the
    // disk goes with the first `image rm`.
    #[cfg(target_os = "linux")]
    let root_disk = |id: &str| env.cache.join(format!("pinfold/root/{id}.ext4"));
    #[cfg(target_os = "linux")]
    assert!(
        root_disk(&retire_ids[0]).is_file(),
        "image rm removed the root disk of the live box's image"
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
    assert_ok(&alive, "the twin box's exec after image rm");
    assert_eq!(
        alive.stdout.trim(),
        "retire-2",
        "the twin box answered from another image"
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
        tagged_images(&format!("localhost/pinfold/image-{retire_name}")).is_empty(),
        "the second image rm left a tag of the retired name"
    );
    // With its box down, the image goes and its root disk with it.
    // Sabotage: drop `prune_root_disks` from `image rm`; the disk remains.
    #[cfg(target_os = "linux")]
    assert!(
        !root_disk(&retire_ids[0]).exists(),
        "the removed image's root disk survived image rm"
    );
    up_twin.down(&env);
    assert!(
        up_twin.wait().success(),
        "the twin box's up did not exit cleanly"
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

/// The ids of every image carrying `label = value`, from the runtime
/// itself: its image list is the ground truth for what remains.
fn labeled_images(label: &str, value: &str) -> Vec<String> {
    image_ids(|image| image.labels.get(label).map(String::as_str) == Some(value))
}
