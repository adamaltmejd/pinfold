//! End-to-end tests for guarantees 6, 12, 13, 14, 16, 18, 33 and 39 in
//! docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with Firecracker and rootless podman for images. The harness isolates the XDG dirs, builds the
//! default profile image once, and drives `pinfold pi` as a user would. No
//! model is needed: `pi --mode rpc` answers `get_state` while the box runs,
//! and `pi --version` exits on its own.
//! Guarantee 14's test runs `pi -p` through the shim against a fake model on
//! the host, reached through a route.

use std::fs;
use std::io::{BufReader, Write};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, ExitStatus, Stdio};

use crate::box_::{ChildOwner, box_name, box_up, read_bounded};
use e2e::{
    HttpFixture, ImageCleanup, TestDir, TestEnv, allow, assert_denied, assert_ok, box_exec,
    box_list, build_profile, curl, default_image, git, inspect, json_lines, pinfold, pinfold_in,
    project_id, run_dir, run_ok,
};

/// The PINFOLD_ENV_ value `PiRpc` launches with: long enough that no disk
/// image holds it by chance.
const SECRET: &str = "pinfold-e2e-secret-4f9c1d7a2be6";

/// Walk `dir`, panic on a regular file whose data holds `needle`, and
/// list every file read. Only data extents are read, so a sparse disk
/// costs what it holds.
fn holding(dir: &Path, needle: &[u8], scanned: &mut Vec<std::path::PathBuf>) {
    use nix::errno::Errno;
    use nix::unistd::{Whence, lseek};
    use std::io::{Read, Seek, SeekFrom};
    use std::os::fd::AsFd;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("read {}: {error}", dir.display()),
    };
    // A running box's entries, such as a used socket, may vanish during
    // the walk.
    for entry in entries {
        let path = entry.unwrap().path();
        let kind = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata.file_type(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("stat {}: {error}", path.display()),
        };
        if kind.is_dir() {
            holding(&path, needle, scanned);
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("open {}: {error}", path.display()),
        };
        let mut offset = 0;
        loop {
            let data = match lseek(file.as_fd(), offset, Whence::SeekData) {
                Ok(data) => data,
                Err(Errno::ENXIO) => break,
                Err(error) => panic!("seek {}: {error}", path.display()),
            };
            let hole = lseek(file.as_fd(), data, Whence::SeekHole).unwrap();
            file.seek(SeekFrom::Start(data as u64)).unwrap();
            // 8 MiB at a time, carrying the needle's length less one byte
            // across chunks.
            let mut window = Vec::new();
            let mut left = (hole - data) as usize;
            while left > 0 {
                let start = window.len();
                window.resize(start + left.min(8 << 20), 0);
                file.read_exact(&mut window[start..]).unwrap();
                left -= window.len() - start;
                assert!(
                    !window.windows(needle.len()).any(|bytes| bytes == needle),
                    "{} holds the secret",
                    path.display()
                );
                window.drain(..window.len().saturating_sub(needle.len() - 1));
            }
            offset = hole;
        }
        scanned.push(path);
    }
}

#[test]
fn the_environment_is_exactly_the_spec() {
    let _runtime = crate::shared_runtime();
    // Sabotage: pass the PINFOLD_ENV_* value on the command line (for
    // example `container exec --env SECRET=...`) instead of through the
    // child's environment; the value then appears in host `ps` while the box
    // runs and the ps assertion fails. Sabotage: let the box inherit the
    // host environment; the unprefixed proxy variables are then present and
    // their assertions fail. On Linux, seed the VM's environment
    // from the owner's; the host's proxy variables arrive and the proxy
    // assertions fail.
    let env = TestEnv::new("pi-env");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);

    // Warm the shared harness cache while no proxy variable is set: pinfold
    // fetches the harness on the host with curl, which honors https_proxy,
    // so the recognisable values below would break the fetch.
    pi_version(&env, project.path());

    let (run, _, name) = PiRpc::start_with_env(
        &env,
        project.path(),
        &[
            ("HTTP_PROXY", "http://upper-proxy.invalid:3128"),
            ("https_proxy", "http://lower-proxy.invalid:3128"),
            ("NO_PROXY", "proxy-marker.invalid"),
        ],
    );

    // PINFOLD_ENV_SECRET arrives as SECRET.
    let secret = box_exec(&env, &name, &["sh", "-c", "printf %s \"$SECRET\""]);
    assert_eq!(secret.code, 0, "reading SECRET failed: {}", secret.stderr);
    assert_eq!(secret.stdout, SECRET, "SECRET did not arrive");

    // The host's proxy variables are absent: copied in, they would send
    // every HTTPS request to the host's unreachable proxy, since curl
    // prefers lowercase https_proxy.
    let boxed_env = box_exec(&env, &name, &["env"]);
    assert_eq!(
        boxed_env.code, 0,
        "reading the box environment failed: {}",
        boxed_env.stderr
    );
    for value in [
        "http://upper-proxy.invalid:3128",
        "http://lower-proxy.invalid:3128",
        "proxy-marker.invalid",
    ] {
        assert!(
            !boxed_env.stdout.contains(value),
            "{value} leaked into the box environment:\n{}",
            boxed_env.stdout
        );
    }

    // The secret never shows in host `ps` while the box runs.
    let ps = Command::new("ps")
        .args(["-ww", "-A", "-o", "command"])
        .output()
        .expect("run ps");
    // A failed ps prints nothing and would pass the absence check whatever
    // argv held, hiding the argv sabotage above.
    assert!(
        ps.status.success(),
        "host ps failed: {}",
        String::from_utf8_lossy(&ps.stderr)
    );
    let ps = String::from_utf8_lossy(&ps.stdout);
    assert!(
        !ps.contains(SECRET),
        "the secret is in host ps output:\n{ps}"
    );

    // On Linux the secret reaches the guest only over vsock: not on its
    // kernel command line, nor in any file pinfold writes for the box (the
    // VM configuration, the init disk, mount images, disks). Sabotage: put
    // the environment in BOOT_ARGS (config.json and /proc/cmdline fail) or
    // write it into the init disk (init.ext4 fails).
    #[cfg(target_os = "linux")]
    {
        let cmdline = box_exec(&env, &name, &["cat", "/proc/cmdline"]);
        assert_ok(&cmdline, "reading the guest's /proc/cmdline");
        assert!(
            !cmdline.stdout.contains(SECRET),
            "the secret is on the guest's kernel command line"
        );
        let mut scanned = Vec::new();
        holding(&env.state, SECRET.as_bytes(), &mut scanned);
        holding(
            &env.cache.join("pinfold/root"),
            SECRET.as_bytes(),
            &mut scanned,
        );
        let names: Vec<String> = scanned
            .iter()
            .filter_map(|path| Some(path.file_name()?.to_string_lossy().into_owned()))
            .collect();
        for written in ["config.json", "init.ext4", "mount0.ext4"] {
            assert!(
                names.iter().any(|name| name == written),
                "no {written} under the state dir: {names:?}"
            );
        }
    }

    assert!(run.finish().success(), "pinfold pi did not exit cleanly");

    // A caller-owned box whose spec PATH is one directory that does not
    // exist, so the runtime's directory is omitted on both macOS and Linux
    // (on Ubuntu /bin is /usr/bin, so a list of system dirs would not show
    // the bug). The spec's PATH reaches the runtime client only under its
    // prefixed name, so the runtime is found on pinfold's own PATH and the
    // box still comes up. Sabotage: export guest env unprefixed; Rust then
    // resolves the runtime against the spec PATH, `up` fails before
    // `ready`, and `box_up` panics on its first line.
    let path = env.root.join("no-such-path");
    let name = box_name("env-path");
    // Sabotage: on Apple, export guest values under their original names to
    // `container` again. Guest HOME and XDG_RUNTIME_DIR then move its state
    // and storage, so `list` and `down` from the caller's own environment
    // miss the box, and LD_PRELOAD reaches the host loader. Encoding is also
    // needed for a multiline value and a guest name that uses the transport
    // prefix. Expectations are the caller's literal spec values, observed
    // through the real guest's env output. Sabotage: stop removing transport
    // variables in init exec; the extra-prefixed-entry assertion fails.
    let guest = [
        ("HOME", "/pinfold-guest-home-missing"),
        ("XDG_RUNTIME_DIR", "/pinfold-guest-runtime-missing"),
        ("LD_PRELOAD", "/pinfold-guest-preload-missing.so"),
        ("MULTILINE", "first line\nsecond line"),
        ("PINFOLD_BOX_ENV_HOME", "literal guest prefix variable"),
        ("NODE_USE_ENV_PROXY", "0"),
    ];
    let mut variables = serde_json::Map::new();
    variables.insert("PATH".into(), serde_json::json!(path));
    for (key, value) in guest {
        variables.insert(key.into(), serde_json::json!(value));
    }
    let label = "dev.example.test=env-path";
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "labels": { "dev.example.test": "env-path" },
        "env": variables,
    });
    let mut up = box_up(&env, &spec, &name);
    let boxed = box_exec(&env, &name, &["/bin/sh", "-c", "printf %s \"$PATH\""]);
    assert_eq!(boxed.code, 0, "reading PATH failed: {}", boxed.stderr);
    assert_eq!(
        boxed.stdout,
        path.to_str().expect("the PATH is UTF-8"),
        "the box's PATH is not the spec's"
    );
    let environment = env
        .command(pinfold())
        .args(["box", "exec", &name, "--", "/usr/bin/env", "-0"])
        .output()
        .expect("read the real guest environment");
    assert!(
        environment.status.success(),
        "guest env failed: {environment:?}"
    );
    let entries: Vec<&[u8]> = environment.stdout.split(|byte| *byte == 0).collect();
    for (key, value) in guest {
        let value = if key == "NODE_USE_ENV_PROXY" {
            "1"
        } else {
            value
        };
        let expected = format!("{key}={value}");
        assert!(entries.contains(&expected.as_bytes()), "guest lost {key}");
    }
    let prefixed: Vec<_> = entries
        .iter()
        .copied()
        .filter(|entry| entry.starts_with(b"PINFOLD_BOX_ENV_"))
        .collect();
    assert_eq!(
        prefixed,
        [b"PINFOLD_BOX_ENV_HOME=literal guest prefix variable".as_slice()],
        "internal transport variables reached the guest command"
    );
    // Local list and down still manage the box.
    let listed = box_list(&env, label);
    assert!(
        listed.iter().any(|box_| box_["name"] == name.as_str()),
        "list did not find the box: {listed:?}"
    );
    up.down(&env);
    assert!(up.wait().success(), "the box's up did not exit cleanly");
    assert!(box_list(&env, label).is_empty(), "down left the box");
}

#[test]
fn project_state_persists_and_stays_separate() {
    let _runtime = crate::shared_runtime();
    // Sabotage: derive the project id from the directory name alone (drop
    // the root hash in state.rs::project_id); the two checkouts named
    // `checkout` then share a home disk, and the second project sees the
    // first's marker. Sabotage: seed $HOME on every run (make `init
    // receive` ignore `missing`); the edited settings.json is overwritten and
    // the survives-a-run assertion fails. Sabotage: skip the attachment
    // check in `disk::claim`; the second `pinfold pi` starts and its
    // refusal assertion fails.
    let env = TestEnv::new("pi-state");
    default_image(&env);
    let a = TestDir::new(&env, "a/checkout");
    let b = TestDir::new(&env, "b/checkout");
    git(a.path(), &["init", "-q"]);
    git(b.path(), &["init", "-q"]);
    let settings = "$HOME/.pi/agent/settings.json";
    let in_box = |name: &str, script: &str| {
        let output = box_exec(&env, name, &["sh", "-c", script]);
        assert_ok(&output, script);
        output.stdout
    };

    // The first run seeds the default profile's settings.json, and an edit
    // survives the next run: a seed is copied only when missing.
    let (run_a, _, box_a) = PiRpc::start(&env, a.path());
    let seeded = in_box(&box_a, &format!("cat {settings}"));
    assert!(seeded.contains("defaultProjectTrust"), "{seeded}");
    in_box(
        &box_a,
        &format!(
            r#"printf '{{"marker":"project-a"}}' > {settings} && printf project-a > "$HOME/project-a-marker""#
        ),
    );

    // One pi box per project: a second start is refused while the first
    // holds the disks.
    let refused = pinfold_in(&env, a.path(), &["pi", "--version"])
        .output()
        .expect("run a second pinfold pi");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "a second pi box started");
    assert!(stderr.contains("disk-in-use"), "wrong refusal: {stderr}");
    assert!(run_a.finish().success(), "the first project failed");

    // A second checkout with the same directory name gets its own home and
    // does not see the first project's marker.
    let (run_b, _, box_b) = PiRpc::start(&env, b.path());
    let separate_b = in_box(
        &box_b,
        r#"test ! -e "$HOME/project-a-marker" && printf project-b > "$HOME/project-b-marker" && cat "$HOME/project-b-marker""#,
    );
    assert_eq!(separate_b, "project-b");
    assert!(run_b.finish().success(), "the second project failed");

    // The first project's home kept its edit and its marker. A deleted seed
    // comes back on the next run.
    let (run_a, _, box_a) = PiRpc::start(&env, a.path());
    let kept = in_box(
        &box_a,
        &format!(
            r#"test ! -e "$HOME/project-b-marker" && cat "$HOME/project-a-marker" {settings} && rm {settings}"#
        ),
    );
    assert_eq!(
        kept, r#"project-a{"marker":"project-a"}"#,
        "a run reseeded an edited file"
    );
    assert!(run_a.finish().success(), "the returning project failed");
    let (run_a, _, box_a) = PiRpc::start(&env, a.path());
    let reseeded = in_box(&box_a, &format!("cat {settings}"));
    assert!(
        reseeded.contains("defaultProjectTrust"),
        "reseed content: {reseeded}"
    );
    assert!(run_a.finish().success(), "the reseeded project failed");

    // Sabotage: stop bounding the cosmetic project-id prefix; the trust
    // filename exceeds the host's 255-byte component limit before pi runs.
    let long = TestDir::new(&env, &"x".repeat(245));
    git(long.path(), &["init", "-q"]);
    allow(&env, long.path());
    pi_version(&env, long.path());
}

#[test]
fn a_changed_project_file_stops_the_run() {
    let _runtime = crate::shared_runtime();
    // Sabotage: drop the trust::check call from pi::launch::run; the fetched
    // and merged `.pinfold.toml` then runs and both refusal assertions fail.
    // Sabotage: drop it from pi::launch::crossing; the untrusted fetch runs.
    // Sabotage: drop `untrusted:` from trust::check's refusal; every reason
    // assertion fails.
    // Sabotage: hash an absent `.pinfold.toml` as the empty file instead of
    // recording its absence; the first run, with no `.pinfold.toml` and no
    // record, refuses as "not trusted" and the bare-run control fails.
    // Sabotage: record no Containerfile hash in trust::current
    // (`containerfile: None`); the changed Containerfile then runs and
    // builds, and the two refusal assertions after the change fail.
    // Sabotage: drop the system identity from base.Containerfile; the
    // box's first commit fails as "Author identity unknown".
    // Sabotage: push under the `pinfold` remote in pi::launch::crossing;
    // the box's merge of host/main fails. Sabotage: check trust in `build`
    // only when the project names a Containerfile; the build after the
    // domain change runs and its refusal assertion fails.
    let env = TestEnv::new("pi-trust");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    let root = project.path();
    let host_git = |args: &[&str]| git(root, args);
    host_git(&["init", "-q", "-b", "main"]);
    // The host identity stays out of the repository's config, which the
    // project disk copies: the box commits under its image's identity.
    let id = ["-c", "user.name=e2e", "-c", "user.email=e2e@example.test"];
    host_git(&[&id[..], &["commit", "-q", "--allow-empty", "-m", "start"]].concat());
    let config = root.join(".pinfold.toml");
    let in_box = |name: &str, script: &str| {
        let output = box_exec(&env, name, &["sh", "-c", script]);
        assert_ok(&output, script);
    };
    let refused = |args: &[&str]| {
        let refused = pinfold_in(&env, root, args).output().expect("run pinfold");
        assert!(!refused.status.success(), "{args:?} ran untrusted");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(stderr.contains("untrusted"), "wrong refusal: {stderr}");
    };

    // A project with no `.pinfold.toml` has nothing to trust: it runs
    // without `pinfold allow`.
    pi_version(&env, root);

    // `pinfold allow` records the absence. The agent creates the file and
    // commits it; the host fetches and merges that commit, after the box
    // has ended, so the fetch runs in a transient box.
    allow(&env, root);
    let (run, _, name) = PiRpc::start(&env, root);
    in_box(
        &name,
        &format!(
            "cd {} && printf 'allow = [\"example.com\"]\n' > .pinfold.toml && git add .pinfold.toml && git commit -qm config",
            root.display()
        ),
    );
    assert!(run.finish().success(), "the bare run did not exit cleanly");
    run_ok(&mut pinfold_in(&env, root, &["fetch"]));
    host_git(&["merge", "-q", "--ff-only", "pinfold/main"]);

    // The file that appeared stops the run and the next fetch until
    // `pinfold allow` records it.
    refused(&["pi", "--version"]);
    refused(&["fetch"]);
    allow(&env, root);
    pi_version(&env, root);

    // The host pushes a commit into the live box, which merges it and adds
    // a domain to the now-trusted file.
    let (run, _, name) = PiRpc::start(&env, root);
    fs::write(root.join("host.txt"), "host\n").unwrap();
    host_git(&["add", "host.txt"]);
    host_git(&[&id[..], &["commit", "-q", "-m", "host"]].concat());
    let pushed = run_ok(&mut pinfold_in(&env, root, &["push"]));
    let pushed = json_lines(&String::from_utf8_lossy(&pushed.stdout));
    assert_eq!(pushed[0]["event"], "pushed", "{pushed:?}");
    in_box(
        &name,
        &format!(
            "cd {} && git merge -q --ff-only host/main && printf 'allow = [\"example.com\", \"api.github.com\"]\n' > .pinfold.toml && git commit -qam domain",
            root.display()
        ),
    );
    run_ok(&mut pinfold_in(&env, root, &["fetch"]));
    assert!(
        run.finish().success(),
        "the trusted run did not exit cleanly"
    );
    host_git(&["merge", "-q", "--ff-only", "pinfold/main"]);
    let author = host_git(&["log", "-1", "--format=%an <%ae>", "pinfold/main"]);
    assert_eq!(author.trim(), "pi <pi@pinfold.invalid>");

    // The change stops the run, the fetch and the build again, and
    // `pinfold allow` clears it.
    for args in [&["pi", "--version"][..], &["fetch"][..], &["build"][..]] {
        refused(args);
    }
    allow(&env, root);
    pi_version(&env, root);

    // The project's own Containerfile is trusted the same way: the
    // unchanged file builds and runs, and a change to it stops both the run
    // and the build until `pinfold allow` records the new bytes.
    let _images = ImageCleanup {
        repository: format!("localhost/pinfold/project-{}", project_id(&env, root)),
    };
    let containerfile = root.join("Containerfile.pinfold");
    fs::write(
        &config,
        "containerfile = \"Containerfile.pinfold\"\nallow = [\"example.com\"]\n",
    )
    .expect("point .pinfold.toml at the project Containerfile");
    fs::write(
        &containerfile,
        "FROM localhost/pinfold/profile-default:latest\n",
    )
    .expect("write the project Containerfile");
    allow(&env, root);
    run_ok(&mut pinfold_in(&env, root, &["build"]));
    // Control: with the Containerfile unchanged, the project image runs.
    pi_version(&env, root);

    fs::write(
        &containerfile,
        "FROM localhost/pinfold/profile-default:latest\n# changed\n",
    )
    .expect("change the project Containerfile");
    refused(&["pi", "--version"]);
    refused(&["build"]);
    allow(&env, root);
    run_ok(&mut pinfold_in(&env, root, &["build"]));
    pi_version(&env, root);
}

#[test]
fn both_pi_config_levels_load_behind_a_route() {
    let _runtime = crate::shared_runtime();
    // Sabotage: drop the route (leave PINFOLD_ROUTES empty, or point it at
    // another name); the proxy refuses fake.model with a 403 and the
    // "pi -p failed" assertion fails before any request reaches the model.
    // Sabotage: skip the profile skill under the profile's share/pi/skills;
    // the profile marker is absent from the request. Sabotage: stop the
    // project from being trusted (remove defaultProjectTrust from the seeded
    // settings); the project marker is absent.
    // Sabotage: omit the operating-context extension; its host-chosen
    // allowlist never reaches the model.
    let env = TestEnv::new("pi-levels");
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);

    // The profile carries the user level: a skill in its share/pi package,
    // and the agent dir's models.json, where pi reads provider settings
    // (pi's models.md and custom-provider.md). The seeded settings.json from
    // `default` keeps defaultProjectTrust, so the project level loads too.
    let profile = "e2e-fake";
    run_ok(
        env.command(pinfold())
            .args(["profile", "new", profile, "--from", "default"]),
    );
    let profile_dir = env.config.join("pinfold/profiles").join(profile);
    let profile_marker = "pinfold-e2e-profile-skill-marker";
    write_skill(
        &profile_dir.join("share/pi/skills/e2e-profile-skill"),
        "e2e-profile-skill",
        profile_marker,
    );
    let agent_dir = profile_dir.join("home/.pi/agent");
    fs::create_dir_all(&agent_dir).expect("create the profile's agent dir");
    fs::write(
        agent_dir.join("models.json"),
        r#"{
  "providers": {
    "openai": {
      "baseUrl": "http://fake.model/v1",
      "api": "openai-completions",
      "models": [{ "id": "fake-model", "name": "Fake Model" }]
    }
  }
}
"#,
    )
    .expect("write the profile's models.json");
    // This fixture customizes mounted configuration, not image contents.
    let image = default_image(&env);
    fs::write(profile_dir.join("Containerfile"), format!("FROM {image}\n"))
        .expect("write the fixture's Containerfile");
    build_profile(&env, profile);
    let _image = ImageCleanup {
        repository: format!("localhost/pinfold/profile-{profile}"),
    };
    // The project carries the project level: a skill under .pi/.
    let project_marker = "pinfold-e2e-project-skill-marker";
    write_skill(
        &project.path().join(".pi/skills/e2e-project-skill"),
        "e2e-project-skill",
        project_marker,
    );

    // The fake model listens on the host; only the route can reach it. It
    // answers pi's one streaming request with a chat completion and keeps
    // the request body, so the test can assert what pi loaded.
    let model = HttpFixture::start(Some((
        "text/event-stream",
        concat!(
            "data: {\"id\":\"chatcmpl-e2e\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"fake-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-e2e\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"fake-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        ),
    )));

    // `pi -p` through the shim, without a TTY. The provider and model pin the
    // request to the fake model in the profile's models.json.
    let shim = env.root.join("pi");
    std::os::unix::fs::symlink(pinfold(), &shim).expect("symlink pi to pinfold");
    let allow = "custom-profile-allow.invalid";
    let output = env
        .command(&shim)
        .args([
            "-p",
            "--provider",
            "openai",
            "--model",
            "fake-model",
            "reply with ok",
        ])
        .current_dir(project.path())
        .env("PINFOLD_PROFILE", profile)
        .env("PINFOLD_ALLOW", allow)
        .env("PINFOLD_ROUTES", format!("fake.model={}", model.route()))
        .env("PINFOLD_ENV_OPENAI_API_KEY", "sk-fake")
        .output()
        .expect("run pi -p");
    assert!(
        output.status.success(),
        "pi -p failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = model
        .requests()
        .last()
        .expect("the fake model got no request")
        .1
        .clone();
    assert!(
        request.contains(profile_marker),
        "the profile skill never reached the model; the profile config level did not load"
    );
    assert!(
        request.contains(project_marker),
        "the project skill never reached the model; the project config level did not load"
    );
    let body: serde_json::Value = serde_json::from_str(&request).unwrap();
    let system = body["messages"]
        .as_array()
        .expect("the model request has messages")
        .iter()
        .find(|message| message["role"] == "system")
        .and_then(|message| message["content"].as_str())
        .expect("the model request has a system prompt");
    assert!(
        system.contains(allow),
        "the system prompt did not state the host's allowlist: {system}"
    );
}

#[test]
fn the_highest_layer_sets_the_allowlist() {
    let _runtime = crate::shared_runtime();
    // Sabotage: union DEFAULT_ALLOW under the merged allow in
    // `Config::load`; the default hosts include npm, so the npm 403
    // assertion fails. Sabotage: merge the environment layer below the
    // project's in `Config::load` (or drop `Layer::from_env`'s allow); the
    // allowlist is then the project's registry.npmjs.org, so npm is let
    // through and the npm 403 assertion fails. Sabotage: drop `cpus` and
    // `memory` from `box_plan`, which `build_plan` spreads; the box gets
    // the runtime's defaults and the memory assertion fails.
    let env = TestEnv::new("pi-allow");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);

    // The environment's list replaces the project's, which replaces the
    // built-in one: the box's allowlist is exactly the host PINFOLD_ALLOW
    // names, though the project allows another. Resource and route
    // overrides use that same highest layer.
    let config = project.path().join(".pinfold.toml");
    let project_fixture = HttpFixture::start(Some(("text/plain", "project-route")));
    fs::write(
        &config,
        format!("allow = [\"registry.npmjs.org\"]\ncpus = 2\nmemory = \"1G\"\n[routes]\n'override.internal' = '{}'\n", project_fixture.route()),
    )
    .expect("write .pinfold.toml");
    allow(&env, project.path());
    let fixture = HttpFixture::start(Some(("text/plain", "env-route")));
    let route = format!("override.internal={}", fixture.route());
    let (run, _, name) = PiRpc::start_with_env(
        &env,
        project.path(),
        &[
            ("PINFOLD_ALLOW", "api.github.com"),
            ("PINFOLD_CPUS", "1"),
            ("PINFOLD_MEMORY", "512M"),
            ("PINFOLD_ROUTES", &route),
        ],
    );

    // `cpus` and `memory` reach the box as its kernel sees them: a
    // Firecracker VM has that much memory and that many vCPUs; the Apple
    // guest's cgroup holds the memory limit and the CPU quota.
    #[cfg(target_os = "linux")]
    {
        crate::box_::assert_guest_memory(&env, &name, 512 << 20);
        let cpus = box_exec(&env, &name, &["nproc"]);
        assert_ok(&cpus, "nproc in the box");
        assert_eq!(
            cpus.stdout.trim(),
            "1",
            "the box does not have the environment's 1 cpu"
        );
    }
    #[cfg(target_os = "macos")]
    {
        let memory = box_exec(&env, &name, &["cat", "/sys/fs/cgroup/memory.max"]);
        assert_ok(&memory, "reading memory.max");
        assert_eq!(
            memory.stdout.trim(),
            (512u64 << 20).to_string(),
            "the box's memory.max is not the environment's 512M"
        );
        let cpus = box_exec(&env, &name, &["cat", "/sys/fs/cgroup/cpu.max"]);
        assert_eq!(cpus.code, 0, "reading cpu.max failed: {}", cpus.stderr);
        let mut cpu_max = cpus.stdout.split_whitespace();
        let quota: u64 = cpu_max
            .next()
            .expect("cpu quota")
            .parse()
            .expect("finite CPU quota");
        let period: std::num::NonZeroU64 = cpu_max
            .next()
            .expect("cpu period")
            .parse()
            .expect("nonzero CPU period");
        assert_eq!(
            quota,
            period.get(),
            "the box's cpu quota is not the environment's 1 cpu"
        );
    }

    // Sabotage: omit resources or routes from Layer::from_env. Host
    // fixture input and the kernel's own observations supply expectations.
    let routed = curl(&env, &name, "10", &["http://override.internal/"]);
    assert_ok(&routed, "environment route");
    assert_eq!(routed.stdout, "env-route");

    // The one listed host works.
    let allowed = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "allowlisted host");

    // A host the project's list allows is refused: the environment layer is
    // higher. The proxy decides before dialing, so no request reaches npm.
    let denied = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://registry.npmjs.org/"],
    );
    assert_denied(&denied, "403", "a request to registry.npmjs.org");

    // The log names the refused host and the reason.
    let log = run_dir(&env, &pi_run_id(&env, &name)).join("egress.jsonl");
    let lines = json_lines(&fs::read_to_string(&log).expect("read the pi run's egress log"));
    assert!(
        lines.iter().any(|line| line["host"] == "registry.npmjs.org"
            && line["decision"] == "refused"
            && line["reason"] == "not allowlisted"),
        "no not-allowlisted refusal for registry.npmjs.org: {lines:?}"
    );

    assert!(run.finish().success(), "pinfold pi did not exit cleanly");

    // Sabotage: turn env::var errors into absence with .ok(); malformed
    // overrides then silently use a lower layer. Their bytes stay private.
    for key in [
        "PINFOLD_PROFILE",
        "PINFOLD_ALLOW",
        "PINFOLD_ROUTES",
        "PINFOLD_CPUS",
        "PINFOLD_MEMORY",
    ] {
        let mut bytes = b"private-override-marker".to_vec();
        bytes.push(0xff);
        let invalid = pinfold_in(&env, project.path(), &["config"])
            .env(key, std::ffi::OsString::from_vec(bytes))
            .output()
            .expect("inspect a non-UTF-8 override");
        let stderr = String::from_utf8_lossy(&invalid.stderr);
        assert!(!invalid.status.success(), "ignored {key}");
        assert!(stderr.contains(key), "missing override key: {stderr}");
        assert!(
            !stderr.contains("private-override-marker"),
            "override value leaked: {stderr}"
        );
    }
    run_ok(&mut pinfold_in(&env, project.path(), &["config"]));
}

#[test]
fn the_launch_record_is_the_applied_policy() {
    let _runtime = crate::shared_runtime();
    // Guarantee 39. Sabotage: build `inspect`'s launch section by loading
    // the project's config at query time; the changed allow list then
    // appears. Sabotage: write the plan's env as name and value, or a
    // route's resolved headers, into the launch record; the state-root scan
    // finds the secret or the header value. Sabotage, on Linux: have init
    // write the box's environment to a file in the home disk; the scan
    // reads the disk's data extents and finds the secret. Apple's disks
    // are runtime volumes outside the state root. Sabotage: drop `cpus` from the
    // record; its equality fails.
    let env = TestEnv::new("pi-launch");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);
    let service = HttpFixture::start(Some(("text/plain", "service")));
    let upstream = HttpFixture::start(Some(("text/plain", "upstream")));
    let upstream_origin = format!("http://{}", upstream.route());
    let config = project.path().join(".pinfold.toml");
    fs::write(
        &config,
        format!(
            "allow = [\"api.github.com\"]\ncpus = 2\nmemory = \"1G\"\n[routes]\n'service.internal' = '{}'\n[routes.'inject.internal']\nto = '{upstream_origin}'\nheaders = {{ Authorization = {{ from = \"PINFOLD_E2E_LAUNCH_KEY\", prefix = \"Bearer \" }} }}\n",
            service.route()
        ),
    )
    .expect("write .pinfold.toml");
    allow(&env, project.path());
    let secret = format!("pf-launch-secret-{}", std::process::id());
    let header = format!("pf-launch-header-{}", std::process::id());
    let (run, _, name) = PiRpc::start_with_env(
        &env,
        project.path(),
        &[
            ("PINFOLD_ENV_SECRET", &secret),
            ("PINFOLD_E2E_LAUNCH_KEY", &header),
        ],
    );
    let run_id = pi_run_id(&env, &name);

    // The live box's file changes and is not re-allowed.
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("api.github.com", "example.com"),
    )
    .unwrap();
    let report = config_json(&env, project.path());
    assert_eq!(report["trust"]["ok"], false, "the change was trusted");
    assert_eq!(
        report["egress"]["allow"],
        serde_json::json!(["example.com"])
    );

    let (code, record) = inspect(&env, &run_id);
    assert_eq!(code, 0, "inspect of the live pi run failed");
    assert_eq!(record["launch"]["state"], "available", "{record}");
    let launch = &record["launch"]["record"];
    assert_eq!(
        launch["egress"]["allow"],
        serde_json::json!(["api.github.com"])
    );
    assert_eq!(
        launch["egress"]["routes"],
        serde_json::json!({
            "inject.internal": { "kind": "injecting", "to": upstream_origin },
            "service.internal": { "kind": "host-service", "to": service.route() },
        })
    );
    assert!(
        launch["env"]
            .as_array()
            .is_some_and(|names| names.iter().any(|name| name == "SECRET")),
        "the launch record lacks env name SECRET: {launch}"
    );
    assert_eq!(launch["cpus"], 2.0);
    assert_eq!(launch["memory"], "1G");

    // The started list still decides: the started host passes and the new
    // one is refused.
    let allowed = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "the started list's host");
    let denied = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://example.com/"],
    );
    assert_denied(&denied, "403", "the changed list's host");

    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
    // Guarantee 40: a pi box prints no `down`, so its harness ending is the
    // terminal reason. Sabotage: record a pi run's end as `signal`, or write
    // no terminal record for it; this assertion fails.
    let (_, record) = inspect(&env, &run_id);
    assert_eq!(
        record["terminal"]["record"]["reason"], "harness-exited",
        "{record}"
    );
    // On Linux the scan covers the pi box's sparse disks too, after the
    // box is down: a guest write that kept its environment lands in one.
    for needle in [&secret, &header] {
        holding(&env.state, needle.as_bytes(), &mut Vec::new());
    }
}

#[test]
fn projects_exclude_host_authority() {
    let _runtime = crate::shared_runtime();
    // Guarantee 33. Sabotage: remove trust::validate_host_paths, or compare
    // unresolved XDG paths; allow then records trust inside the project and
    // pi/build accept it. The missing suffix behind a symlink is the case
    // canonicalize(path) alone cannot check. Host fixture paths supply the
    // expected boundary; the allowed sibling runs with the same profile.
    let env = TestEnv::new("host-authority");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);
    let alias = env.root.join("project-alias");
    std::os::unix::fs::symlink(project.path(), &alias).expect("symlink project ancestor");
    for key in ["XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"] {
        for ancestor in [project.path(), alias.as_path()] {
            let unsafe_root = ancestor.join(format!("missing-{key}/nested"));
            for args in [&["allow"][..], &["build"][..], &["pi", "--version"][..]] {
                let output = pinfold_in(&env, project.path(), args)
                    .env(key, &unsafe_root)
                    .output()
                    .expect("refuse project authority");
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(!output.status.success(), "accepted {key}: {stderr}");
                assert!(
                    stderr.contains("host-path-in-project"),
                    "wrong refusal: {stderr}"
                );
            }
            assert!(
                !unsafe_root.join("pinfold/trust").exists(),
                "allow wrote project trust"
            );
        }
    }
    allow(&env, project.path());
    pi_version(&env, project.path());
}

#[test]
fn a_caller_reads_the_effective_configuration_as_data() {
    let _runtime = crate::shared_runtime();
    // Sabotage: report DEFAULT_ALLOW instead of the merged allow in
    // `config_report`; `egress.allow` then names the built-in hosts and the
    // exact-list assertion fails. Sabotage: swap the two names
    // `state::disk_names` returns; `disk list` shows the reported home disk
    // attached at the project, not as $HOME, and the attachment assertion
    // fails. Sabotage: resolve `config_report`'s image with
    // `:latest` replaced by another tag; on both runtimes the built profile
    // image reads as not built.
    let env = TestEnv::new("pi-config");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);
    fs::write(
        project.path().join(".pinfold.toml"),
        "allow = [\"api.github.com\"]\n",
    )
    .expect("write .pinfold.toml");

    // Before `pinfold allow`, the project's config is untrusted; the caller
    // reads that state as data, and the home disk it names.
    let report = config_json(&env, project.path());
    assert_eq!(
        report["egress"]["allow"],
        serde_json::json!(["api.github.com"]),
        "the effective allowlist is not the project's"
    );
    assert_eq!(
        report["trust"]["ok"].as_bool(),
        Some(false),
        "the untrusted config was reported trusted"
    );
    assert_eq!(
        report["image_built"].as_bool(),
        Some(true),
        "the profile image default_image built was reported not built: {report}"
    );
    assert_eq!(report["project"]["home"], "/home/pi", "{report}");
    let home_disk = report["project"]["disks"]["home"]
        .as_str()
        .expect("project.disks.home is a string")
        .to_string();

    // After `pinfold allow` the same command reports the config trusted.
    allow(&env, project.path());
    let report = config_json(&env, project.path());
    assert_eq!(
        report["trust"]["ok"].as_bool(),
        Some(true),
        "the allowed config was reported untrusted"
    );

    // The home disk the report names is the one the box attaches, writable,
    // at its $HOME: a file the box writes there is on that disk.
    let (run, _, name) = PiRpc::start(&env, project.path());
    let listed = run_ok(env.command(pinfold()).args(["disk", "list"])).stdout;
    let listed = json_lines(&String::from_utf8_lossy(&listed));
    let attached = listed
        .iter()
        .find(|disk| disk["name"] == home_disk.as_str())
        .map(|disk| disk["attached"].clone());
    assert_eq!(
        attached,
        Some(serde_json::json!([{ "box": name, "readonly": false }])),
        "{listed:?}"
    );
    let boxed_home = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            "printf %s \"$HOME\" && findmnt -no TARGET --target \"$HOME\"",
        ],
    );
    assert_ok(&boxed_home, "reading HOME and its mount");
    assert_eq!(
        boxed_home.stdout, "/home/pi/home/pi\n",
        "the box's HOME is not its disk"
    );
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");

    // Sabotage: propagate a missing runtime from config_report, treat it as
    // an absent image, or populate embedded profiles while loading metadata.
    // The host's empty PATH makes image existence unknown; the project file
    // still supplies its effective allowlist, and fresh host dirs stay empty.
    let offline = TestEnv::with_private_cache("pi-config-offline");
    let project = TestDir::new(&offline, "project");
    fs::write(
        project.path().join(".pinfold.toml"),
        "allow = [\"api.github.com\"]\n",
    )
    .unwrap();
    let empty = TestDir::new(&offline, "empty-path");
    let output =
        run_ok(pinfold_in(&offline, project.path(), &["config"]).env("PATH", empty.path()));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("read offline config");
    assert_eq!(
        report["egress"]["allow"],
        serde_json::json!(["api.github.com"])
    );
    assert!(
        report["image_built"].is_null(),
        "unobserved image reported as missing: {report}"
    );
    assert!(
        report["image_error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()),
        "missing image observation error: {report}"
    );

    // Sabotage: map `full` to the shared CONFIG in `Profile::builtin`; config
    // then reports only the built-in defaults. The expected list is the
    // spec's default list plus the two GitHub hosts.
    let bare = TestDir::new(&offline, "bare");
    let output = run_ok(
        pinfold_in(&offline, bare.path(), &["config"])
            .env("PATH", empty.path())
            .env("PINFOLD_PROFILE", "full"),
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("read full profile config");
    assert_eq!(
        report["egress"]["allow"],
        serde_json::json!([
            "api.anthropic.com",
            "platform.claude.com",
            "api.openai.com",
            "auth.openai.com",
            "chatgpt.com",
            "openrouter.ai",
            "opencode.ai",
            "registry.npmjs.org",
            "pi.dev",
            "github.com",
            "api.github.com"
        ]),
        "the full profile's allowlist: {report}"
    );
    for directory in [&offline.state, &offline.config, &offline.root.join("cache")] {
        assert_eq!(
            fs::read_dir(directory).unwrap().count(),
            0,
            "config wrote to {}",
            directory.display()
        );
    }

    // Sabotage: remove Layer's deny_unknown_fields, or accept containerfile
    // in a profile layer. The real CLI must refuse the file and key instead
    // of reporting a partially applied policy. Correcting that same file
    // succeeds without a runtime and reports the fixture's allowlist.
    let profile = "e2e-config-refusal";
    let profile_dir = offline.config.join("pinfold/profiles").join(profile);
    fs::create_dir_all(&profile_dir).unwrap();
    fs::write(profile_dir.join("Containerfile"), "FROM scratch\n").unwrap();
    let profile_config = profile_dir.join("pinfold.toml");
    let project_config = project.path().join(".pinfold.toml");
    let valid_project = format!("profile = \"{profile}\"\n");
    let valid_profile = "allow = [\"api.github.com\"]\n";
    for (path, key, invalid, source) in [
        (
            &project_config,
            "alloww",
            format!("{valid_project}alloww = [\"example.com\"]\n"),
            ".pinfold.toml",
        ),
        (
            &profile_config,
            "alloww",
            format!("{valid_profile}alloww = [\"example.com\"]\n"),
            profile,
        ),
        (
            &profile_config,
            "containerfile",
            format!("{valid_profile}containerfile = \"project-only\"\n"),
            profile,
        ),
    ] {
        fs::write(&project_config, &valid_project).unwrap();
        fs::write(&profile_config, valid_profile).unwrap();
        fs::write(path, invalid).unwrap();
        let refused = pinfold_in(&offline, project.path(), &["config"])
            .env("PATH", empty.path())
            .output()
            .expect("read invalid configuration");
        let reason = String::from_utf8_lossy(&refused.stderr);
        assert!(!refused.status.success(), "accepted {source}'s {key}");
        assert!(
            reason.contains(source) && reason.contains("pinfold.toml") && reason.contains(key),
            "refusal did not name its file and key: {reason}"
        );
        fs::write(&project_config, &valid_project).unwrap();
        fs::write(&profile_config, valid_profile).unwrap();
        let corrected =
            run_ok(pinfold_in(&offline, project.path(), &["config"]).env("PATH", empty.path()));
        let report: serde_json::Value = serde_json::from_slice(&corrected.stdout).unwrap();
        assert_eq!(
            report["egress"]["allow"],
            serde_json::json!(["api.github.com"])
        );
    }
}

/// A `pinfold pi --mode rpc` process with a live box.
struct PiRpc {
    child: ChildOwner,
    stdin: Option<ChildStdin>,
    _reader: BufReader<ChildStdout>,
}

impl PiRpc {
    /// Start pi in `project` and wait for its answer; return the run, the
    /// project id, and the name of the one box the run owns.
    fn start(env: &TestEnv, project: &Path) -> (PiRpc, String, String) {
        PiRpc::start_with_env(env, project, &[])
    }

    /// [`PiRpc::start`] with extra variables in `pinfold pi`'s own
    /// environment.
    fn start_with_env(
        env: &TestEnv,
        project: &Path,
        vars: &[(&str, &str)],
    ) -> (PiRpc, String, String) {
        let child = env
            .command(pinfold())
            .args(["pi", "--mode", "rpc"])
            .current_dir(project)
            .env("PINFOLD_ENV_SECRET", SECRET)
            .envs(vars.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn pinfold pi");
        let mut child = ChildOwner::new(child, env, None);
        let stdin = child.stdin.take().expect("pi stdin");
        let stdout = child.stdout.take().expect("pi stdout");
        let mut run = PiRpc {
            child,
            stdin: Some(stdin),
            _reader: BufReader::new(stdout),
        };
        run.stdin
            .as_mut()
            .unwrap()
            .write_all(b"{\"type\":\"get_state\",\"id\":\"1\"}\n")
            .expect("write get_state");
        run.stdin
            .as_mut()
            .unwrap()
            .flush()
            .expect("flush get_state");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        loop {
            let line = read_bounded(
                &mut run.child,
                &mut run._reader,
                deadline.saturating_duration_since(std::time::Instant::now()),
                false,
            )
            .expect("read pi reply");
            assert!(!line.is_empty(), "pi exited before answering get_state");
            if let Ok(reply) = serde_json::from_str::<serde_json::Value>(line.trim())
                && reply["id"] == "1"
            {
                break;
            }
        }
        // The run has created the project state; the id names its directory.
        let id = project_id(env, project);
        let owner = run.child.id();
        let listed: Vec<_> = box_list(env, &format!("dev.pinfold.project={id}"))
            .into_iter()
            .filter(|box_| box_["owner"].as_u64() == Some(u64::from(owner)))
            .collect();
        assert_eq!(
            listed.len(),
            1,
            "expected one pi box for owner {owner}: {listed:?}"
        );
        run.child
            .observe(&serde_json::json!({ "labels": listed[0]["labels"] }));
        let name = listed[0]["name"]
            .as_str()
            .expect("box name is a string")
            .to_string();
        (run, id, name)
    }

    /// Close pi's stdin, wait for it, and return its exit status.
    fn finish(mut self) -> ExitStatus {
        drop(self.stdin.take());
        self.child.wait_bounded()
    }
}

/// Run `pinfold pi --version` in `project` and assert it exits cleanly.
fn pi_version(env: &TestEnv, project: &Path) {
    run_ok(&mut pinfold_in(env, project, &["pi", "--version"]));
}

/// Run `pinfold config` in `project` and parse its JSON object.
fn config_json(env: &TestEnv, project: &Path) -> serde_json::Value {
    let output = run_ok(env.command(pinfold()).arg("config").current_dir(project));
    serde_json::from_slice(&output.stdout).expect("pinfold config output is one JSON object")
}

/// Write one discoverable skill whose description carries `marker`.
fn write_skill(dir: &Path, name: &str, marker: &str) {
    fs::create_dir_all(dir).expect("create the skill directory");
    fs::write(
        dir.join("SKILL.md"),
        format!(
            "---\nname: {name}\ndescription: {marker} identifies this skill in a model request.\n---\n\n# {name}\n"
        ),
    )
    .expect("write SKILL.md");
}

/// A running pi box's run id: its `dev.pinfold.generation`, as `list`
/// reports it.
fn pi_run_id(env: &TestEnv, name: &str) -> String {
    box_list(env, "dev.pinfold.owner")
        .into_iter()
        .find(|box_| box_["name"] == name)
        .and_then(|box_| {
            box_["labels"]["dev.pinfold.generation"]
                .as_str()
                .map(String::from)
        })
        .unwrap_or_else(|| panic!("no listed generation for {name}"))
}
