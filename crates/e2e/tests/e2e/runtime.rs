//! Tests that require exclusive access to the shared host runtime.
//! They live together so they start after every shared test: libtest starts
//! tests in name order, and a queued writer blocks later readers.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, SystemTime};

use e2e::{
    HttpFixture, ImageCleanup, TestEnv, assert_ok, box_exec, box_list, build_profile, curl,
    default_image, image_cli, image_id, image_ids, inspect, pinfold, profile_containerfile,
    project_id, project_state_dir, run_dir, run_ok, tagged_images, test_dir,
};

#[cfg(target_os = "macos")]
use super::box_::RuntimeContainer;
use super::box_::{
    ChildOwner, HeldBuild, box_name, box_up, built_unique_ref, disk_names, find_box_state_dir,
    image_build, image_rm, read_bounded,
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
    // survives `--unused`, and the removal assertion fails. Sabotage: past the newest
    // two build tags, remove every tag of the tag's image, as retention by
    // image did; `clean` takes the image the newest two builds share and
    // the shared-image assertion fails. Sabotage: skip a tag whose image a
    // newer tag names; the oldest-tag assertion fails. Sabotage: drop the
    // in-use check from `remove_images`; the live box's last tag is offered
    // to the runtime, which deletes the image under the box, and the
    // live-box image assertion fails. Sabotage: restore the per-id skip in
    // `remove_images`; the twin box's shared image is reported in use, the
    // retired name's tag on it survives, and the assertion that
    // `retire_refs[1]` is gone fails. Sabotage: apply the newest-two rule or
    // the caller's one-hour grace in `image rm`; a fresh build survives and
    // the removed-tags assertion fails. Sabotage: match the name as a prefix in
    // `remove_images`; the image of the name extending the retired one goes
    // and its survival assertion fails. Sabotage: make `--dry-run` run the
    // real clean; the other project's state goes and the dry-run assertion
    // fails. Sabotage: on Linux, keep podman's repeated `Names` in
    // `list_images`; the second removal of the newest image's tag is
    // `image not known` and the one-line assertion fails. Sabotage: make
    // `remove_images` remove every reference of an image's id instead of
    // only the name's tags; the twin's tag goes with the shared image and
    // the twin assertion fails.
    // Sabotage: force-delete Apple's shared builder during clean; the
    // other process's build, held in RUN by the host fixture, fails.
    // Sabotage: drop the `pinfold_box` check from `clean::boxes`; on Apple
    // the user's container, which has no owner label, is removed as a dead
    // box and its survival assertion fails. Linux lists only pinfold's own
    // claims, so no runtime container can reach `clean` there.
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

    // A caller source whose three builds share one image, all past the
    // hour's grace: the runtime builds one image under three build tags
    // whose `<build>` times fall in 1970, in one build as pinfold tags its
    // own. `clean` finds the name by its tags and removes only the oldest
    // tag; the image stays under the newest two.
    let shared = format!("{profile}-shared");
    let shared_repository = format!("localhost/pinfold/image-{shared}");
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
    let moved_project = test_dir(&env, "moved-project");
    seed_project(&moved_project, b"moved\n");
    let moved_state = project_state_dir(&env, moved_project.as_path());
    let moved_disk = format!("pi-{}-project", project_id(&env, moved_project.as_path()));
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

    // The real clean removes the dead box and only the dead box. Apple's
    // builder is shared across state directories, so keep another caller's
    // RUN active while clean prunes unused cache.
    if cfg!(target_os = "macos") {
        let other = build_env.as_ref().unwrap();
        let name = format!("{profile}-active");
        let _active_images = ImageCleanup {
            repository: format!("localhost/pinfold/image-{name}"),
        };
        let build = HeldBuild::start(other, &name, default_image(other), &[]);
        let cleaned = env.command(pinfold()).arg("clean").output().unwrap();
        let built = build.finish();
        assert!(
            cleaned.status.success(),
            "clean during another caller's build: {cleaned:?}"
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

    // Plain `clean` keeps every project's disks, and a moved checkout's
    // state too. Sabotage: make `CleanPlan::idle` select every project
    // under plain `clean`; the moved checkout's state goes and its
    // assertion fails. Sabotage: drop the `--unused` disk removal from
    // `CleanPlan::remove`; the idle project's disk survives and its
    // assertion fails.
    for disk in [&other_disk, &moved_disk] {
        let creator = box_name("cleanup-disk");
        let spec = serde_json::json!({
            "name": creator,
            "image": default_image(&env),
            "disks": [{ "name": disk, "guest": "/disk" }],
        });
        let mut up = box_up(&env, &spec, &creator);
        up.down(&env);
        assert!(up.wait().success(), "the disk's box did not exit cleanly");
    }
    fs::rename(moved_project.as_path(), env.root.join("moved-away")).unwrap();
    run_ok(env.command(pinfold()).arg("clean"));
    assert!(
        moved_state.is_dir(),
        "clean removed a moved checkout's state"
    );
    let listed = disk_names(&env);
    for disk in [&live_disk, &other_disk, &moved_disk] {
        assert!(listed.contains(disk), "clean removed disk {disk}");
    }

    // `--unused AGE` removes the state and disks of projects not run for
    // that long, except one with a live box. Every project last ran seconds
    // ago, so `0s` ages them out and only the live one survives.
    run_ok(env.command(pinfold()).args(["clean", "--unused", "0s"]));
    assert!(
        !moved_state.exists(),
        "clean --unused kept the moved checkout's state"
    );
    let listed = disk_names(&env);
    assert!(
        listed.contains(&live_disk),
        "clean --unused removed a live project's disk"
    );
    for disk in [&other_disk, &moved_disk] {
        assert!(
            !listed.contains(disk),
            "clean --unused kept idle disk {disk}"
        );
    }
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

    // Sabotage: remove the shared project locks from pi startup and box up;
    // clean deletes the existing state while the cold download holds startup
    // after its claim but before a runtime box can protect the project.
    // The checkout is moved away meanwhile; that alone protects nothing.
    let env = TestEnv::with_private_cache("clean-startup");
    let project = test_dir(&env, "starting-project");
    let missing = format!("{profile}-startup-missing");
    profile_containerfile(&env, &missing, "FROM scratch\n");
    let refused = env
        .command(pinfold())
        .args(["pi", "--version"])
        .env("PINFOLD_PROFILE", &missing)
        .current_dir(project.as_path())
        .output()
        .unwrap();
    assert!(!refused.status.success(), "the missing image was accepted");
    let state = project_state_dir(&env, project.as_path());
    let id = project_id(&env, project.as_path());
    let marker = state.join("marker");
    fs::write(&marker, b"starting project\n").unwrap();

    let mut proxy = HeldDownload::new();
    let child = env
        .command(pinfold())
        .envs(proxy.vars())
        .args(["pi", "--version"])
        .current_dir(project.as_path())
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
        root: project.as_path().to_path_buf(),
        moved: env.root.join("moved-project"),
    };
    fs::rename(&restore.root, &restore.moved).unwrap();
    run_ok(env.command(pinfold()).args(["clean", "--unused", "0s"]));
    assert_eq!(
        fs::read(&marker).unwrap(),
        b"starting project\n",
        "clean removed the claimed startup's existing state"
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
    // through the same test.
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

#[test]
fn a_runs_end_is_recorded_apart_from_its_start() {
    // `box prune` removes every dead box on the host.
    let _runtime = crate::exclusive_runtime();
    // Guarantee 40. Sabotage: drop `detail` from the terminal record; the
    // 137 assertion fails. Sabotage: write a `signal` record from `prune`
    // (or from orphan removal); the killed owner's terminal is then
    // available after prune. Sabotage: skip the terminal record in
    // `Box::down` for a requested stop; the first inspect reports terminal
    // `not-observed`. Writing it after `finish`'s ack is caught only when
    // `inspect` wins the race: the owner waits on nothing after the ack, so
    // no outside observer can hold it there.
    let env = TestEnv::new("run-end");
    let image = default_image(&env);
    let spec = |name: &str| serde_json::json!({ "name": name, "image": image });

    // `box down` returns only after the owner acknowledged teardown, so the
    // terminal record is already there.
    let name = box_name("run-end-down");
    let mut up = box_up(&env, &spec(&name), &name);
    let run = run_id(&up);
    up.down(&env);
    let (code, record) = inspect(&env, &run);
    assert_eq!(code, 0, "inspect after down failed");
    assert_eq!(record["terminal"]["state"], "available", "{record}");
    assert_eq!(record["terminal"]["record"]["reason"], "signal", "{record}");
    up.wait();

    // SIGKILL of the box from the host: of its init through Apple's runtime,
    // of its VMM on Linux. The box exits 137.
    let name = box_name("run-end-killed");
    let mut up = box_up(&env, &spec(&name), &name);
    let run = run_id(&up);
    #[cfg(target_os = "linux")]
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(super::box_::vmm_pid(up.pid()) as i32),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL the box's VMM");
    #[cfg(not(target_os = "linux"))]
    run_ok(std::process::Command::new(e2e::image_cli()).args(["kill", "--signal", "KILL", &name]));
    assert_eq!(up.wait().code(), Some(1), "an exited box's up exit code");
    let down = up.close_stdin();
    let down = down.last().expect("down line");
    assert_eq!(down["reason"], "exited", "{down}");
    let (code, record) = inspect(&env, &run);
    assert_eq!(code, 0, "inspect after an init kill failed");
    let terminal = &record["terminal"]["record"];
    assert_eq!(terminal["reason"], "exited", "{record}");
    assert!(
        terminal["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("137")),
        "the terminal detail names no exit status 137: {record}"
    );

    // An owner that dies without teardown leaves no end, and nothing infers
    // one later.
    let name = box_name("run-end-owner");
    let mut up = box_up(&env, &spec(&name), &name);
    let run = run_id(&up);
    up.kill();
    up.wait();
    for when in ["after the owner's SIGKILL", "after prune"] {
        let (code, record) = inspect(&env, &run);
        assert_eq!(code, 0, "inspect {when} failed");
        assert_eq!(record["launch"]["state"], "available", "{when}: {record}");
        assert_eq!(
            record["terminal"]["state"], "not-observed",
            "{when}: {record}"
        );
        assert_eq!(record["terminal"]["record"], serde_json::Value::Null);
        if when == "after the owner's SIGKILL" {
            let pruned = run_ok(env.command(pinfold()).args(["box", "prune"]));
            assert!(
                String::from_utf8_lossy(&pruned.stdout).contains(&name),
                "prune did not remove the dead owner's box"
            );
        }
    }
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
    // lock; the live run is removed. Sabotage: drop the 14-day rule; the
    // aged run survives the first clean, which is under the quota.
    // Sabotage: evict the newest inactive run first; the newer padded run
    // goes and the older stays. Sabotage:
    // truncate the live run's log to meet the quota; its bytes shrink.
    // Sabotage: count live runs toward the quota; the newer padded run goes
    // too. The live log's length is the host file's, not `inspect`'s. The 128 MiB pad and the backdated mtimes act on host paths the
    // spec names, as a user's disk and clock would.
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
    let aged = finished("run-retention-aged");
    let older = finished("run-retention-older");
    let newer = finished("run-retention-newer");
    let name = box_name("run-retention-live");
    let mut live_up = box_up(&env, &spec(&name), &name);
    let live = run_id(&live_up);

    // Under the quota, only age retires: the aged run goes, the live one
    // stays however old.
    let day = Duration::from_secs(24 * 60 * 60);
    age(&run_dir(&env, &aged), 15 * day);
    age(&run_dir(&env, &live), 15 * day);
    run_ok(env.command(pinfold()).arg("clean"));
    assert_eq!(inspect(&env, &aged).0, 3, "the aged inactive run was kept");
    let (code, record) = inspect(&env, &live);
    assert_eq!(code, 0, "the aged live run was removed");
    assert_eq!(record["launch"]["state"], "available", "{record}");

    // Inactive runs together hold just over 128 MiB; the live one would
    // tip the newer over the quota if it counted.
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
    let (code, record) = inspect(&env, &live);
    assert_eq!(code, 0, "the live run was removed");
    assert_eq!(record["launch"]["state"], "available", "{record}");
    assert_eq!(
        log_bytes(&env, &live),
        live_bytes,
        "the live run's log changed"
    );
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
