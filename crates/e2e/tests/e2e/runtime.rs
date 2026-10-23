//! Tests that require exclusive access to the shared host runtime.

use std::fs;
use std::io::BufReader;
#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use e2e::{
    ImageCleanup, TestDir, TestEnv, assert_ok, box_exec, box_list, build_profile, curl,
    default_image, egress_log, image_cli, image_id, json_lines, labeled_images, pinfold,
    profile_containerfile, project_id, project_state_dir, run_ok, tagged_images,
};

use super::box_::{
    ChildOwner, HeldBuild, HeldDownload, RuntimeContainer, box_name, box_up, built_unique_ref,
    find_box_state_dir, image_build, image_rm, read_bounded,
};

#[cfg(target_os = "linux")]
use super::box_::{Starting, box_up_start, status_field};

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

    // A caller image built on the profile's `:latest`. The runtime copies
    // the base image's labels onto it, so without the explicit empty family
    // labels it would count as the profile's image in build 4's retention.
    let x_name = format!("{profile}-x");
    let _x_images = ImageCleanup {
        repository: format!("localhost/pinfold/image-{x_name}"),
    };
    let profile_ref = format!("localhost/pinfold/profile-{profile}:latest");
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
            repository: format!("localhost/pinfold/image-{name}"),
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
            repository: format!("localhost/pinfold/image-{cache_name}"),
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
    assert_eq!(
        tagged_images(&format!("localhost/pinfold/image-{retire_name}")),
        distinct_ids,
        "the runtime does not list the three retire images before image rm"
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
    up_twin.down(&env);
    assert!(
        up_twin.wait().success(),
        "the twin box's up did not exit cleanly"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn linux_boxes_apply_their_own_seccomp_profile() {
    use std::os::unix::fs::OpenOptionsExt;

    let _runtime = crate::exclusive_runtime();
    // Sabotage: write both derived policies to one shared pathname again.
    // Both Podman consumers are held until the second policy replaces it;
    // the first box then wrongly refuses getppid.
    let env = TestEnv::with_private_cache("seccomp-selection");
    let image = default_image(&env);
    let info = run_ok(
        env.command(Path::new("podman"))
            .args(["info", "--format", "json"]),
    );
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    let source = info["host"]["security"]["seccompProfilePath"]
        .as_str()
        .expect("Podman's host seccomp profile");
    let normal: serde_json::Value = serde_json::from_slice(&fs::read(source).unwrap()).unwrap();
    let mut strict = normal.clone();
    let rules = strict["syscalls"].as_array_mut().unwrap();
    for rule in rules.iter_mut() {
        if rule["action"] == "SCMP_ACT_ALLOW" {
            rule["names"]
                .as_array_mut()
                .unwrap()
                .retain(|name| name != "getppid");
        }
    }
    rules.retain(|rule| !rule["names"].as_array().unwrap().is_empty());
    rules.push(serde_json::json!({
        "names": ["getppid"],
        "action": "SCMP_ACT_ERRNO",
        "errnoRet": nix::libc::EPERM,
    }));

    // Observe raw syscalls, including errno, rather than libc's cached IDs.
    let fixture = env.root.join("fixture");
    fs::create_dir_all(&fixture).unwrap();
    let observer = env.root.join("seccomp-observer.c");
    fs::write(
        &observer,
        r#"#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <sys/syscall.h>
#include <unistd.h>
int main(void) {
    errno = 0;
    long parent = syscall(SYS_getppid);
    int parent_errno = errno;
    errno = 0;
    long self = syscall(SYS_getpid);
    int self_errno = errno;
    printf("{\"getppid\":%ld,\"getppid_errno\":%d,\"getpid\":%ld,\"getpid_errno\":%d}\n",
           parent, parent_errno, self, self_errno);
    return 0;
}
"#,
    )
    .unwrap();
    run_ok(
        Command::new("musl-gcc")
            .args(["-O2", "-static", "-o"])
            .arg(fixture.join("seccomp-observer"))
            .arg(observer),
    );

    let names = [box_name("seccomp-normal"), box_name("seccomp-strict")];
    let sources = [env.root.join("normal.fifo"), env.root.join("strict.fifo")];
    let configs = [env.root.join("normal.conf"), env.root.join("strict.conf")];
    for (source, config) in sources.iter().zip(&configs) {
        nix::unistd::mkfifo(
            source,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )
        .unwrap();
        fs::write(
            config,
            format!(
                "[containers]\nseccomp_profile = {}\n",
                serde_json::to_string(source.to_str().unwrap()).unwrap()
            ),
        )
        .unwrap();
    }
    let mut starts = std::array::from_fn::<_, 2, _>(|index| {
        let spec = serde_json::json!({
            "name": names[index],
            "image": image,
            "mounts": [{"host": fixture, "guest": "/fixture", "readonly": true}],
        });
        box_up_start(
            &env,
            &spec,
            &[("CONTAINERS_CONF", configs[index].to_str().unwrap())],
        )
    });

    // FIFO writers rendezvous only once both owners have completed their
    // preflight and preparation. Keep their profile reads open until then.
    let writers = sources.each_ref().map(|source| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .custom_flags(nix::libc::O_NONBLOCK)
                .open(source)
            {
                Ok(writer) => break writer,
                Err(error) if error.raw_os_error() == Some(nix::libc::ENXIO) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "box up did not read its host seccomp profile"
                    );
                    std::thread::yield_now();
                }
                Err(error) => panic!("open host seccomp fixture: {error}"),
            }
        }
    });

    let store = &info["store"];
    let lock_path = Path::new(store["graphRoot"].as_str().unwrap()).join(format!(
        "{}-images/images.lock",
        store["graphDriverName"].as_str().unwrap()
    ));
    // Declared after the owners and FIFO writers: panic releases the lock
    // before owner cleanup needs Podman. No image metadata is changed.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .expect("open Podman's image-store lock");
    let metadata = lock.metadata().unwrap();
    let lock_identity = format!(
        "{:02x}:{:02x}:{}",
        nix::sys::stat::major(metadata.dev()),
        nix::sys::stat::minor(metadata.dev()),
        metadata.ino()
    );
    let held = nix::libc::flock {
        l_type: nix::libc::F_WRLCK as i16,
        l_whence: nix::libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    crate::hold_image_lock(&lock, &held);
    for ((mut writer, policy), starting) in writers.into_iter().zip([normal, strict]).zip(&starts) {
        let policy = serde_json::to_vec(&policy).unwrap();
        let mut remaining = policy.as_slice();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !remaining.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "box up stopped reading its host seccomp profile"
            );
            match writer.write(remaining) {
                Ok(0) => panic!("host seccomp fixture wrote no bytes"),
                Ok(written) => remaining = &remaining[written..],
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    std::thread::yield_now();
                }
                Err(error) => panic!("write host seccomp fixture: {error}"),
            }
        }
        drop(writer);
        // Podman's image lookup precedes seccomp consumption. Its kernel
        // lock waiter proves derivation finished while consumption is held.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let locks = fs::read_to_string("/proc/locks").unwrap();
            let waiting = locks.lines().any(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                if !fields.contains(&"->") || !fields.contains(&lock_identity.as_str()) {
                    return false;
                }
                let Some(pid) = fields.get(5) else {
                    return false;
                };
                fs::read_to_string(format!("/proc/{pid}/status")).is_ok_and(|status| {
                    status_field(&status, "PPid:") == starting.child.id().to_string()
                })
            });
            if waiting {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "box up's Podman consumer did not wait on the image lock"
            );
            std::thread::yield_now();
        }
    }
    drop(lock);
    let ready = starts.each_mut().map(Starting::first_line);
    let boxes = starts
        .into_iter()
        .zip(ready)
        .zip(&names)
        .map(|((starting, ready), name)| {
            assert_eq!(ready["event"], "ready", "box up failed: {ready}");
            starting.into_up(name, ready)
        })
        .collect::<Vec<_>>();
    let observed = names.each_ref().map(|name| {
        let output = box_exec(&env, name, &["/fixture/seccomp-observer"]);
        assert_ok(&output, "the syscall observer");
        serde_json::from_str::<serde_json::Value>(&output.stdout).unwrap()
    });
    assert!(observed[0]["getppid"].as_i64().unwrap() >= 0);
    assert_eq!(observed[0]["getppid_errno"], 0);
    assert_eq!(observed[1]["getppid"], -1);
    assert_eq!(observed[1]["getppid_errno"], nix::libc::EPERM);
    // Positive control in the stricter box: another raw syscall succeeds.
    assert!(observed[1]["getpid"].as_i64().unwrap() > 0);
    assert_eq!(observed[1]["getpid_errno"], 0);
    for up in boxes {
        up.down(&env);
    }
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
    let before = json_lines(&egress_log(up.ready()));
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
    let after = json_lines(&egress_log(up.ready()));
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
