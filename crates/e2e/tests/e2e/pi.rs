//! End-to-end tests for guarantees 6, 12, 13, 14, 16, 18, 32, 33 and 39 in
//! docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with rootless podman. The harness isolates the XDG dirs, builds the
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
    box_list, box_stat, build_profile, curl, default_image, git, inspect, json_lines, pinfold,
    pinfold_in, project_id, run_egress_log, run_ok,
};

#[test]
fn the_documents_profile_reads_documents_locally() {
    let _runtime = crate::shared_runtime();
    // Guarantee 32. Sabotage: remove poppler-utils from the bundled
    // documents image; `pdftoppm` then fails. Sabotage: omit AnyDoc's native
    // package from that image; the PDF conversion fails.
    // The host fixture lists page objects out of object-number order; the
    // Markdown's page order and the selected page must follow the page tree,
    // also when the input path has spaces. The box has no egress. A fresh
    // image, not the reused default, so an older image's tools cannot hide
    // a removal.
    let env = TestEnv::new("pdf");
    let home = TestDir::new(&env, "home");
    let documents = TestDir::new(&env, "documents");
    fs::write(
        documents.path().join("two pages.pdf"),
        include_bytes!("fixtures/two-pages.pdf"),
    )
    .expect("write the host PDF fixture");
    let profile = "e2e-documents";
    run_ok(env.command(pinfold()).args([
        "profile",
        "new",
        profile,
        "--from",
        "documents",
        "--builtin",
    ]));
    build_profile(&env, profile);
    let _image = ImageCleanup {
        repository: format!("pinfold/profile-{profile}"),
    };
    let name = box_name("pdf");
    let spec = serde_json::json!({
        "name": name,
        "profile": profile,
        "env": { "HOME": "/home/pdf" },
        "mounts": [
            { "host": home.path(), "guest": "/home/pdf" },
            { "host": documents.path(), "guest": "/documents" },
        ],
    });
    let _up = box_up(&env, &spec, &name);

    let converted = box_exec(
        &env,
        &name,
        &[
            "anydoc",
            "/documents/two pages.pdf",
            "-o",
            "/documents/pdf.md",
        ],
    );
    assert_ok(&converted, "converting the PDF to Markdown without egress");
    let markdown =
        fs::read_to_string(documents.path().join("pdf.md")).expect("read the converted PDF");
    let first = markdown.find("first page fixture");
    let second = markdown.find("second page fixture");
    assert!(
        first.is_some() && first < second,
        "the PDF conversion lost a page or the page tree's order: {markdown}"
    );

    let rendered = box_exec(
        &env,
        &name,
        &[
            "pdftoppm",
            "-f",
            "2",
            "-l",
            "2",
            "-r",
            "72",
            "-singlefile",
            "-png",
            "/documents/two pages.pdf",
            "/documents/page",
        ],
    );
    assert_ok(&rendered, "rendering the second PDF page without egress");
    let png = fs::read(documents.path().join("page.png")).expect("read the rendered PNG");
    assert_eq!(
        png.get(16..24),
        Some([0, 0, 0, 200, 0, 0, 0, 100].as_slice())
    );
}

#[test]
fn the_environment_is_exactly_the_spec() {
    let _runtime = crate::shared_runtime();
    // Sabotage: pass the PINFOLD_ENV_* value on the command line (for
    // example `container exec --env SECRET=shhh`) instead of through the
    // child's environment; `shhh` then appears in host `ps` while the box
    // runs and the ps assertion fails. Sabotage: let the box inherit the
    // host environment; the unprefixed proxy variables are then present and
    // their assertions fail. Sabotage: drop `--http-proxy=false` from podman's
    // run argv; the box inherits the host's proxy variables and the proxy
    // assertions fail.
    let env = TestEnv::new("pi-env");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);

    // Linux pi boxes need disks, which arrive with the Firecracker
    // runtime.
    if cfg!(target_os = "macos") {
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
        assert_eq!(secret.stdout, "shhh", "SECRET did not arrive");

        // The host's proxy variables are absent: podman's run would otherwise
        // copy them in, and curl prefers lowercase https_proxy, so every HTTPS
        // request would go to the host's unreachable proxy.
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
            !ps.contains("shhh"),
            "the secret is in host ps output:\n{ps}"
        );

        assert!(run.finish().success(), "pinfold pi did not exit cleanly");
    }

    // A caller-owned box whose spec PATH is one directory that does not
    // exist, so the runtime's directory is omitted on both macOS and Linux
    // (on Ubuntu /bin is /usr/bin, so a list of system dirs would not show
    // the bug). The runtime is resolved on pinfold's own PATH before the
    // spec's env, so the box still comes up. Sabotage: spawn the runtime by
    // its bare name again; Rust then resolves it against the spec PATH, `up`
    // fails before `ready`, and `box_up` panics on its first line.
    let path = env.root.join("no-such-path");
    let name = box_name("env-path");
    // Sabotage: export guest values under their original names to the
    // runtime client again. Guest HOME and XDG_RUNTIME_DIR then change the
    // client's state/storage, CONTAINER_HOST selects an absent Podman
    // service, and LD_PRELOAD reaches the host loader. Encoding is also
    // needed for a multiline value and a guest name that uses the transport
    // prefix. Expectations are the caller's literal spec values, observed
    // through the real guest's env output. Sabotage: stop removing transport
    // variables in init exec; the extra-prefixed-entry assertion fails.
    let guest = [
        ("HOME", "/pinfold-guest-home-missing"),
        ("XDG_RUNTIME_DIR", "/pinfold-guest-runtime-missing"),
        (
            "CONTAINER_HOST",
            "unix:///pinfold-guest-service-missing.sock",
        ),
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
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "env": variables,
    });
    let up = box_up(&env, &spec, &name);
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
    drop(up);
}

#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Linux disks arrive with the Firecracker runtime"
)]
fn project_state_persists_and_stays_separate() {
    let _runtime = crate::shared_runtime();
    // Sabotage: derive the project id from the directory name alone (drop
    // the root hash in state.rs::project_id); the two checkouts named
    // `checkout` then share a home disk, and the second project sees the
    // first's marker. Sabotage: seed $HOME on every run (drop `--missing`
    // in box.rs's seed copy); the edited settings.json is overwritten and
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
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Linux disks arrive with the Firecracker runtime"
)]
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
    // Sabotage: push under the `pinfold` remote in pi::launch::crossing;
    // the box's merge of host/main fails.
    let env = TestEnv::new("pi-trust");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    let root = project.path();
    let host_git = |args: &[&str]| git(root, args);
    host_git(&["init", "-q", "-b", "main"]);
    host_git(&["config", "user.name", "e2e"]);
    host_git(&["config", "user.email", "e2e@example.test"]);
    host_git(&["commit", "-q", "--allow-empty", "-m", "start"]);
    let config = root.join(".pinfold.toml");
    let in_box = |name: &str, script: &str| {
        let output = box_exec(&env, name, &["sh", "-c", script]);
        assert_ok(&output, script);
    };
    let refused_pi = || {
        let refused = pinfold_in(&env, root, &["pi", "--version"])
            .output()
            .expect("run pinfold pi --version");
        assert!(!refused.status.success(), "the changed project file ran");
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
    assert!(
        !config.exists(),
        "the box's file reached the host unfetched"
    );
    run_ok(&mut pinfold_in(&env, root, &["fetch"]));
    host_git(&["merge", "-q", "--ff-only", "pinfold/main"]);

    // The file that appeared stops the run and the next fetch until
    // `pinfold allow` records it.
    refused_pi();
    let refused = pinfold_in(&env, root, &["fetch"]).output().unwrap();
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "fetch ran untrusted");
    assert!(stderr.contains("untrusted"), "wrong refusal: {stderr}");
    allow(&env, root);
    pi_version(&env, root);

    // The host pushes a commit into the live box, which merges it and adds
    // a domain to the now-trusted file.
    let (run, _, name) = PiRpc::start(&env, root);
    fs::write(root.join("host.txt"), "host\n").unwrap();
    host_git(&["add", "host.txt"]);
    host_git(&["commit", "-q", "-m", "host"]);
    run_ok(&mut pinfold_in(&env, root, &["push"]));
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

    // The change stops the run again, and `pinfold allow` clears it.
    refused_pi();
    allow(&env, root);
    pi_version(&env, root);

    // The project's own Containerfile is trusted the same way: the
    // unchanged file builds and runs, and a change to it stops both the run
    // and the build until `pinfold allow` records the new bytes.
    let _images = ImageCleanup {
        repository: format!("pinfold/project-{}", project_id(&env, root)),
    };
    let containerfile = root.join("Containerfile.pinfold");
    fs::write(
        &config,
        "containerfile = \"Containerfile.pinfold\"\nallow = [\"example.com\"]\n",
    )
    .expect("point .pinfold.toml at the project Containerfile");
    fs::write(&containerfile, "FROM pinfold/profile-default:latest\n")
        .expect("write the project Containerfile");
    allow(&env, root);
    run_ok(&mut pinfold_in(&env, root, &["build"]));
    // Control: with the Containerfile unchanged, the project image runs.
    pi_version(&env, root);

    fs::write(
        &containerfile,
        "FROM pinfold/profile-default:latest\n# changed\n",
    )
    .expect("change the project Containerfile");
    refused_pi();
    let refused = pinfold_in(&env, root, &["build"])
        .output()
        .expect("run pinfold build");
    assert!(!refused.status.success(), "the changed Containerfile built");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("untrusted"), "wrong refusal: {stderr}");
    allow(&env, root);
    run_ok(&mut pinfold_in(&env, root, &["build"]));
    pi_version(&env, root);
}

#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Linux disks arrive with the Firecracker runtime"
)]
fn both_pi_config_levels_load_behind_a_route() {
    let _runtime = crate::shared_runtime();
    // Sabotage: drop the route (leave PINFOLD_ROUTES empty, or point it at
    // another name); the proxy refuses fake.model with a 403 and the
    // "pi -p failed" assertion fails before any request reaches the model.
    // Sabotage: skip the profile skill under the profile's share/pi/skills;
    // the profile marker is absent from the request. Sabotage: stop the
    // project from being trusted (remove defaultProjectTrust from the seeded
    // settings); the project marker is absent.
    // Sabotage: omit the full profile's document skill or live package;
    // the skill never reaches the model. Save full's resource paths in the
    // project's settings, or keep using full's package after selecting
    // default; the returning default request still advertises documents.
    // Reseed existing settings on a profile switch; the saved marker changes.
    // Return a forced systemPrompt from operating-context instead of adding
    // its section; Pi projects that text over Ponytail's later section, so
    // full loses its active rules while the allowlist still reaches the model.
    // Omit the operating-context extension; its host-chosen allowlist never
    // reaches the model. Cache that allowlist across starts; the returning
    // default request reports the earlier marker.
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
        repository: format!("pinfold/profile-{profile}"),
    };
    build_profile(&env, "full");
    // The pinned Ponytail instruction hook emits this marker for explicit full input.
    let ponytail_rule = "PONYTAIL MODE ACTIVE — level: full";

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
    let prompt = |selected: &str, allow: &str| {
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
            .env("PINFOLD_PROFILE", selected)
            .env("PINFOLD_ALLOW", allow)
            .env("PINFOLD_ROUTES", format!("fake.model={}", model.route()))
            .env("PINFOLD_ENV_OPENAI_API_KEY", "sk-fake")
            .env("PINFOLD_ENV_PONYTAIL_DEFAULT_MODE", "full")
            .output()
            .expect("run pi -p");
        assert!(
            output.status.success(),
            "pi -p with {selected} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        model
            .requests()
            .last()
            .expect("the fake model got no request")
            .1
            .clone()
    };
    let request = prompt(profile, "custom-profile-allow.invalid");

    assert!(
        request.contains(profile_marker),
        "the profile skill never reached the model; the profile config level did not load"
    );
    // The bundled skill's own source supplies its marker: the description
    // its frontmatter declares.
    let bundled = include_str!("../../../../profile/share/pi/skills/read-documents/SKILL.md");
    let description = bundled
        .lines()
        .find_map(|line| line.strip_prefix("description: "))
        .expect("read-documents declares a description");
    // The first profile seeded models.json and settings.json on the home
    // disk. All later selections reuse this home; only the live mounted pi
    // package changes. The box marks the saved settings.
    let settings = |script: &str| {
        let (run, _, name) = PiRpc::start(&env, project.path());
        let output = box_exec(&env, &name, &["sh", "-c", script]);
        assert_ok(&output, script);
        assert!(run.finish().success(), "the settings run failed");
        output.stdout
    };
    let saved = settings(
        r#"f=$HOME/.pi/agent/settings.json && sed -i 's/^{/{"e2eSavedSettingsMarker":"keep",/' "$f" && cat "$f""#,
    );
    assert!(saved.contains("e2eSavedSettingsMarker"), "{saved}");
    let selections = [
        ("default", "default-first-allow.invalid"),
        ("full", "full-allow.invalid"),
        ("default", "default-return-allow.invalid"),
    ];
    for (selected, allow) in selections {
        let request = prompt(selected, allow);
        assert_eq!(
            request.contains(description),
            selected == "full",
            "{selected} advertised the wrong live document skills: {request}"
        );
        let body: serde_json::Value = serde_json::from_str(&request).unwrap();
        let system = body["messages"]
            .as_array()
            .expect("the model request has messages")
            .iter()
            .find(|message| message["role"] == "system")
            .and_then(|message| message["content"].as_str())
            .expect("the model request has a system prompt");
        assert_eq!(
            system.contains(ponytail_rule),
            selected == "full",
            "{selected} carried the wrong active Ponytail rules: {system}"
        );
        assert!(
            system.contains(allow)
                && selections
                    .iter()
                    .filter(|(_, other)| *other != allow)
                    .all(|(_, other)| !system.contains(other)),
            "{selected} did not state the selected allowlist: {system}"
        );
        assert!(
            request.contains(project_marker),
            "{selected} lost the project skill"
        );
    }
    assert_eq!(
        settings(r#"cat "$HOME/.pi/agent/settings.json""#),
        saved,
        "a profile switch overwrote saved pi settings"
    );
}

#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Linux disks arrive with the Firecracker runtime"
)]
fn the_highest_layer_sets_the_allowlist() {
    let _runtime = crate::shared_runtime();
    // Sabotage: union DEFAULT_ALLOW under the merged allow in
    // `Config::load`; the default hosts include npm, so the npm 403
    // assertion fails. Sabotage: merge the environment layer below the
    // project's in `Config::load` (or drop `Layer::from_env`'s allow); the
    // allowlist is then the project's registry.npmjs.org, so npm is let
    // through and the npm 403 assertion fails. Sabotage: drop `cpus` and
    // `memory` from the Plan in `build_plan`; the box gets the runtime's
    // defaults and the memory-limit assertion fails.
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

    // `cpus` and `memory` reach the box: stat reports the memory limit, and
    // the box's cgroup shows the CPU quota, on both runtimes (the Apple
    // guest kernel exposes it too).
    let stat = box_stat(&env, &name);
    assert_eq!(
        stat["memory"]["limit"].as_u64(),
        Some(512 * 1024 * 1024),
        "the box's memory limit is not the environment's 512M: {stat}"
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
    let lines = json_lines(&run_egress_log(&env, &pi_run_id(&env, &name)));
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
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Linux disks arrive with the Firecracker runtime"
)]
fn the_launch_record_is_the_applied_policy() {
    let _runtime = crate::shared_runtime();
    // Guarantee 39. Sabotage: build `inspect`'s launch section by loading
    // the project's config at query time; the changed allow list then
    // appears. Sabotage: write the plan's env as name and value, or a
    // route's resolved headers, into the launch record; the state-root scan
    // finds the secret or the header value. Sabotage: drop `cpus` from the
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
    let (_, record) = inspect(&env, &run_id);
    assert_eq!(
        record["terminal"]["record"]["reason"], "harness-exited",
        "{record}"
    );
    let mut leaked = Vec::new();
    let mut dirs = vec![env.state.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                dirs.push(entry.path());
            } else if kind.is_file() {
                let bytes = fs::read(entry.path()).unwrap_or_default();
                let text = String::from_utf8_lossy(&bytes);
                if text.contains(&secret) || text.contains(&header) {
                    leaked.push(entry.path());
                }
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "the state root holds a secret: {leaked:?}"
    );
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
    // Linux pi boxes need disks, which arrive with the Firecracker runtime.
    if cfg!(target_os = "macos") {
        pi_version(&env, project.path());
    }
}

#[test]
fn a_caller_reads_the_effective_configuration_as_data() {
    let _runtime = crate::shared_runtime();
    // Sabotage: report DEFAULT_ALLOW instead of the merged allow in
    // `run_config`; `egress.allow` then names the built-in hosts and the
    // exact-list assertion fails. Sabotage: swap the two names
    // `state::disk_names` returns; `disk list` shows the reported home disk
    // attached at the project, not as $HOME, and the attachment assertion
    // fails.
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
    // Linux pi boxes need disks, which arrive with the Firecracker runtime.
    if cfg!(target_os = "macos") {
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
    }

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
            .env("PINFOLD_ENV_SECRET", "shhh")
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
