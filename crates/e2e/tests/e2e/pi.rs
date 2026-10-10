//! End-to-end tests for guarantees 6, 11, 12, 13, 14, 16, 18, 32 and 33 in
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

use crate::box_::{ChildOwner, assert_left_nothing, box_name, box_up, read_bounded};
use e2e::{
    HttpFixture, ImageCleanup, TestDir, TestEnv, allow, assert_denied, assert_ok, box_exec,
    box_list, box_stat, build_profile, curl, default_image, egress_log, git, json_lines, pinfold,
    pinfold_in, project_id, project_state_dir, run_ok,
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
    let ps = String::from_utf8_lossy(&ps.stdout);
    assert!(
        !ps.contains("shhh"),
        "the secret is in host ps output:\n{ps}"
    );

    assert!(run.finish().success(), "pinfold pi did not exit cleanly");

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
fn project_state_persists_and_stays_separate() {
    let _runtime = crate::shared_runtime();
    // Sabotage: derive the project id from the directory name alone (drop
    // the root hash in state.rs::project_id); the two checkouts named
    // `checkout` then share a home, the second's settings.json is the first's
    // edited marker, and the second-seed assertions fail. Sabotage: seed
    // $HOME on every run instead of only when missing; the edited marker is
    // overwritten and the survives-a-run assertion fails. Sabotage: copy
    // every agent entry in `profile new --from-project` (drop
    // `agent_entry_excluded`); `auth.json` lands in the profile and its
    // assertion fails. Mount the other project's home in the box while
    // keeping separate host settings; the guest markers reveal the mix-up.
    let env = TestEnv::new("pi-state");
    default_image(&env);
    let a = TestDir::new(&env, "a/checkout");
    let b = TestDir::new(&env, "b/checkout");
    git(a.path(), &["init", "-q"]);
    git(b.path(), &["init", "-q"]);

    // The first run seeds the default profile's settings.json.
    pi_version(&env, a.path());
    let settings_a = project_state_dir(&env, a.path()).join("home/.pi/agent/settings.json");

    // An edit survives the next run: a seed is copied only when missing.
    let marker = "{\"marker\":\"project-a\"}\n";
    fs::write(&settings_a, marker).expect("edit settings.json");
    let (run_a, _, box_a) = PiRpc::start(&env, a.path());
    let written_a = box_exec(
        &env,
        &box_a,
        &["sh", "-c", r#"printf project-a > "$HOME/project-a-marker""#],
    );
    assert_ok(&written_a, "writing the first project's guest home");
    assert!(
        run_a.finish().success(),
        "the first project did not exit cleanly"
    );
    assert_eq!(
        fs::read_to_string(&settings_a).expect("read edited settings.json"),
        marker,
        "a run reseeded an edited file"
    );

    // A second checkout with the same directory name gets its own home and
    // does not see the first project's marker. Take the first home before
    // the second run: the sabotage overwrites the shared state.json's root.
    let home_a = project_state_dir(&env, a.path()).join("home");
    let (run_b, _, box_b) = PiRpc::start(&env, b.path());
    let separate_b = box_exec(
        &env,
        &box_b,
        &[
            "sh",
            "-c",
            r#"test ! -e "$HOME/project-a-marker" && printf project-b > "$HOME/project-b-marker" && cat "$HOME/project-b-marker""#,
        ],
    );
    assert_ok(&separate_b, "the second project's separate guest home");
    assert_eq!(separate_b.stdout, "project-b");
    assert!(
        run_b.finish().success(),
        "the second project did not exit cleanly"
    );
    // `profile new --from-project` copies the project's agent config, the
    // edited settings included, and leaves the login behind.
    let agent_a = home_a.join(".pi/agent");
    fs::write(agent_a.join("auth.json"), "{\"secret\":true}\n").expect("plant auth.json");
    run_ok(
        env.command(pinfold())
            .args(["profile", "new", "from-a", "--from-project"])
            .arg(a.path()),
    );
    let profile_agent = env.config.join("pinfold/profiles/from-a/home/.pi/agent");
    assert_eq!(
        fs::read_to_string(profile_agent.join("settings.json")).expect("read the copied settings"),
        marker,
        "the profile did not take the project's settings"
    );
    assert!(
        !profile_agent.join("auth.json").exists(),
        "the profile took the project's auth.json"
    );

    // A deleted seed comes back on the next run.
    fs::remove_file(&settings_a).expect("delete settings.json");
    let (run_a, _, box_a) = PiRpc::start(&env, a.path());
    let separate_a = box_exec(
        &env,
        &box_a,
        &[
            "sh",
            "-c",
            r#"test ! -e "$HOME/project-b-marker" && cat "$HOME/project-a-marker""#,
        ],
    );
    assert_ok(
        &separate_a,
        "the first project's persisted separate guest home",
    );
    assert_eq!(separate_a.stdout, "project-a");

    // Sabotage: ignore attach's requested box and choose the first project
    // box; selecting the later name reads or edits the wrong /tmp marker.
    // Each marker is a host-chosen value in a separate box filesystem.
    let (second_a, _, second_box_a) = PiRpc::start(&env, a.path());
    let (selected, other) = if box_a > second_box_a {
        (&box_a, &second_box_a)
    } else {
        (&second_box_a, &box_a)
    };
    for (name, marker) in [(selected, "selected"), (other, "other")] {
        assert_ok(
            &box_exec(
                &env,
                name,
                &[
                    "sh",
                    "-c",
                    "printf %s \"$1\" > /tmp/attach-marker",
                    "sh",
                    marker,
                ],
            ),
            "writing the attach fixture marker",
        );
    }
    let attached = run_ok(&mut pinfold_in(
        &env,
        a.path(),
        &[
            "attach",
            "--box",
            selected,
            "--",
            "sh",
            "-c",
            "cat /tmp/attach-marker && printf attached > /tmp/attach-marker",
        ],
    ));
    assert_eq!(String::from_utf8(attached.stdout).unwrap(), "selected");
    for (name, expected) in [(selected, "attached"), (other, "other")] {
        let marker = box_exec(&env, name, &["cat", "/tmp/attach-marker"]);
        assert_ok(&marker, "reading the marker after attach");
        assert_eq!(marker.stdout, expected, "attach changed the wrong box");
    }
    assert!(second_a.finish().success(), "the second pi run failed");
    assert!(
        run_a.finish().success(),
        "the returning project did not exit cleanly"
    );
    let reseeded = fs::read_to_string(&settings_a).expect("read reseeded settings.json");
    assert!(
        reseeded.contains("defaultProjectTrust"),
        "reseed content: {reseeded}"
    );

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
    // Sabotage: drop the trust::check call from pi::launch; the created and
    // the changed `.pinfold.toml` then run and both refusal assertions fail.
    // Sabotage: hash an absent `.pinfold.toml` as the empty file instead of
    // recording its absence; the first run, with no `.pinfold.toml` and no
    // record, refuses as "not trusted" and the bare-run control fails.
    // Sabotage: record no Containerfile hash in trust::current
    // (`containerfile: None`); the changed Containerfile then runs and
    // builds, and the two refusal assertions after the change fail.
    let env = TestEnv::new("pi-trust");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);
    let config = project.path().join(".pinfold.toml");

    // A project with no `.pinfold.toml` has nothing to trust: it runs
    // without `pinfold allow`.
    pi_version(&env, project.path());

    // `pinfold allow` records the absence, so the file the agent creates in
    // the live box is a change.
    allow(&env, project.path());
    let (run, id, name) = PiRpc::start(&env, project.path());
    let created = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf 'allow = [\"example.com\"]\\n' > {}",
                config.display()
            ),
        ],
    );
    assert_ok(&created, "writing .pinfold.toml in the box");
    assert!(run.finish().success(), "the bare run did not exit cleanly");

    // The file that appeared stops the run until `pinfold allow` records it.
    let refused = pinfold_in(&env, project.path(), &["pi", "--version"])
        .output()
        .expect("run pinfold pi --version");
    assert!(!refused.status.success(), "the new .pinfold.toml ran");
    allow(&env, project.path());
    pi_version(&env, project.path());

    // The agent adds a domain to the now-trusted file in a live box.
    let (run, _, name) = PiRpc::start(&env, project.path());
    let changed = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf 'allow = [\"example.com\", \"api.github.com\"]\\n' > {}",
                config.display()
            ),
        ],
    );
    assert_ok(&changed, "changing .pinfold.toml in the box");
    assert!(
        run.finish().success(),
        "the trusted run did not exit cleanly"
    );

    // The change stops the run again, and `pinfold allow` clears it.
    let refused = pinfold_in(&env, project.path(), &["pi", "--version"])
        .output()
        .expect("run pinfold pi --version");
    assert!(!refused.status.success(), "the changed .pinfold.toml ran");
    allow(&env, project.path());
    pi_version(&env, project.path());

    // The project's own Containerfile is trusted the same way: the
    // unchanged file builds and runs, and a change to it stops both the run
    // and the build until `pinfold allow` records the new bytes.
    let _images = ImageCleanup {
        repository: format!("pinfold/project-{id}"),
    };
    let containerfile = project.path().join("Containerfile.pinfold");
    fs::write(
        &config,
        "containerfile = \"Containerfile.pinfold\"\nallow = [\"example.com\"]\n",
    )
    .expect("point .pinfold.toml at the project Containerfile");
    fs::write(&containerfile, "FROM pinfold/profile-default:latest\n")
        .expect("write the project Containerfile");
    allow(&env, project.path());
    run_ok(&mut pinfold_in(&env, project.path(), &["build"]));
    // Control: with the Containerfile unchanged, the project image runs.
    pi_version(&env, project.path());

    // The agent changes the Containerfile in a live box.
    let (run, _, name) = PiRpc::start(&env, project.path());
    let changed = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf 'FROM pinfold/profile-default:latest\\n# changed\\n' > '{}'",
                containerfile.display()
            ),
        ],
    );
    assert_ok(&changed, "changing the Containerfile in the box");
    assert!(
        run.finish().success(),
        "the trusted run did not exit cleanly"
    );

    // The change stops the run and the build until `pinfold allow` records
    // the new bytes.
    let refused = pinfold_in(&env, project.path(), &["pi", "--version"])
        .output()
        .expect("run pinfold pi --version");
    assert!(!refused.status.success(), "the changed Containerfile ran");
    let refused = pinfold_in(&env, project.path(), &["build"])
        .output()
        .expect("run pinfold build");
    assert!(!refused.status.success(), "the changed Containerfile built");
    allow(&env, project.path());
    run_ok(&mut pinfold_in(&env, project.path(), &["build"]));
    pi_version(&env, project.path());
}

#[test]
fn the_box_cannot_write_git_or_protected_config() {
    let _runtime = crate::shared_runtime();
    // Sabotage: omit the `.git` read-only mount from pi::git (or mount it
    // writable); the `core.fsmonitor` write and the rename then succeed, so
    // those assertions fail.
    // Sabotage: skip the absent protect directories; the box's `.vscode`
    // write then fails as a missing directory, not a read-only one, and its
    // reason assertion fails. Sabotage: skip
    // the symlink check in pi::git's `path_kind` (classify with fs::metadata
    // and drop the canonical comparison); a symlinked `.vscode` is followed,
    // the run starts, and the exit assertion fails.
    // Sabotage: resolve `core.hooksPath` from `.git/config` only (`git
    // config --file .git/config core.hooksPath` in pi::git's `hooks_path`);
    // the global config's `.husky/_` is not protected, and the
    // `.husky/_/pre-commit` write succeeds, so its assertion fails.
    // Sabotage: ignore the configured `protect` list in `Git::prepare`; the
    // `tooling/hooks.sh` write succeeds and its assertion fails.
    let env = TestEnv::new("pi-git");
    default_image(&env);

    // A project with no `.vscode/` yet: pinfold creates the protected
    // directories empty before the run, so the box cannot create them.
    let bare = TestDir::new(&env, "bare");
    git(bare.path(), &["init", "-q"]);
    let root = bare.path();
    // Host git runs the hooks named by `core.hooksPath`, as host git
    // resolves it: here from the global config the `pinfold pi` run's
    // GIT_CONFIG_GLOBAL names, not the repo's `.git/config`. A value inside
    // the project must be read-only like `.git` itself.
    let husky = root.join(".husky/_");
    fs::create_dir_all(&husky).expect("create .husky/_");
    let global = env.root.join("gitconfig");
    fs::write(
        &global,
        format!("[core]\n\thooksPath = {}\n", husky.display()),
    )
    .expect("write the global git config");
    let global = global.to_str().expect("the global config path is UTF-8");
    let vscode = root.join(".vscode");
    assert!(!vscode.exists(), "the fixture already has .vscode");

    // Guarantee 11 admission. Sabotage: omit the core alias check. Pi's
    // real prepared plan must refuse both Git and protected configuration
    // aliases before RPC or guest commands. Host bytes are the expectation.
    let config = root.join(".git/config");
    let protected = root.join(".idea/workspace.xml");
    fs::create_dir(root.join(".idea")).unwrap();
    fs::write(&protected, b"<project/>\n").unwrap();
    for source in [&config, &protected] {
        let expected = fs::read(source).unwrap();
        let alias = root.join("config-alias");
        fs::hard_link(source, &alias).unwrap();
        let refused = pinfold_in(&env, root, &["pi", "--mode", "rpc"])
            .env("GIT_CONFIG_GLOBAL", global)
            .output()
            .expect("run pi alias admission");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert_eq!(refused.status.code(), Some(1), "{stderr}");
        assert!(stderr.contains("mount-alias"), "wrong refusal: {stderr}");
        assert!(stderr.contains(source.to_str().unwrap()), "{stderr}");
        assert!(stderr.contains(alias.to_str().unwrap()), "{stderr}");
        assert!(refused.stdout.is_empty(), "refused Pi emitted RPC output");
        let project_label = format!("dev.pinfold.project={}", project_id(&env, root));
        assert_left_nothing(&env, "pi alias fixture", &project_label, "refused");
        assert_eq!(fs::read(source).unwrap(), expected);
        // Sabotage: release Git's cleanup guard before core startup. The
        // absent protected directories must still be absent after refusal.
        for absent in [".vscode", ".claude"] {
            assert!(!root.join(absent).exists(), "left {absent} after refusal");
        }
        assert!(husky.is_dir(), "removed existing protected directory");
        assert!(protected.is_file(), "removed existing protected config");
        fs::remove_file(&alias).unwrap();
    }

    let (run, _, name) = PiRpc::start_with_env(&env, root, &[("GIT_CONFIG_GLOBAL", global)]);

    // The box cannot write into the protected directory it did not have.
    let denied = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("printf '{{}}' > '{}/settings.json'", vscode.display()),
        ],
    );
    assert_denied(&denied, "Read-only file system", "the .vscode write");

    // Host git runs `.husky/_/pre-commit` on the next commit because the
    // global `core.hooksPath` names it; the box cannot write it.
    let husky_hook = husky.join("pre-commit");
    let denied = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf '#!/bin/sh\\ntouch {}/pwned-husky\\n' > '{}'",
                root.display(),
                husky_hook.display()
            ),
        ],
    );
    assert_denied(&denied, "Read-only file system", "the hooksPath write");

    // `.git` is read-only: the box writes a script in the writable project,
    // and the config that would make host git run it is what must fail.
    let fsmonitor = root.join("fsmonitor.sh");
    let denied = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf '#!/bin/sh\\ntouch {}/pwned-fsmonitor\\n' > '{}' && chmod +x '{}' && git -C '{}' config core.fsmonitor '{}'",
                root.display(),
                fsmonitor.display(),
                fsmonitor.display(),
                root.display(),
                fsmonitor.display()
            ),
        ],
    );
    assert_denied(&denied, "Read-only file system", "git config");

    // The `.git` mount point cannot be renamed.
    let dot_git = root.join(".git");
    let denied = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("mv '{}' '{}-moved'", dot_git.display(), dot_git.display()),
        ],
    );
    assert_denied(&denied, "Device or resource busy", "renaming .git");

    // Positive control: the project itself is writable.
    let control = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("printf 'ok\\n' > '{}/control.txt'", root.display()),
        ],
    );
    assert_ok(&control, "writing a project file");
    assert_eq!(
        fs::read(root.join("control.txt")).expect("read control.txt"),
        b"ok\n"
    );

    assert!(run.finish().success(), "pinfold pi did not exit cleanly");

    // A directory the project's `protect` list names is read-only.
    let with = TestDir::new(&env, "with-protect");
    git(with.path(), &["init", "-q"]);
    let tooling = with.path().join("tooling/hooks.sh");
    fs::create_dir(with.path().join("tooling")).unwrap();
    fs::write(&tooling, "#!/bin/sh\n").unwrap();
    fs::write(
        with.path().join(".pinfold.toml"),
        "protect = [\"tooling\"]\n",
    )
    .unwrap();
    allow(&env, with.path());
    let (run, _, name) = PiRpc::start(&env, with.path());
    let denied = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("printf 'pwned' > '{}'", tooling.display()),
        ],
    );
    assert_denied(&denied, "Read-only file system", "the tooling write");
    let control = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf 'ok\\n' > '{}'",
                with.path().join("control.txt").display()
            ),
        ],
    );
    assert_ok(&control, "writing an unprotected sibling of tooling");
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");

    // A protected path the host planted as a symlink refuses the run: the
    // runtime resolves a bind-mount source on the host, so following it
    // would mount the target into the box. Sabotage: skip pi::git's symlink
    // check; the run starts and the exit assertion fails. Sabotage: remove
    // `protected-path-invalid` from `not_real_dir`; the reason assertion fails.
    let outside = TestDir::new(&env, "outside");
    let symlinked = TestDir::new(&env, "symlinked");
    git(symlinked.path(), &["init", "-q"]);
    fs::write(
        symlinked.path().join(".pinfold.toml"),
        "protect = [\".cleanup\", \".cleanup/nested\"]\n",
    )
    .expect("configure nested protected directories");
    allow(&env, symlinked.path());
    let link = symlinked.path().join(".vscode");
    std::os::unix::fs::symlink(outside.path(), &link).expect("create .vscode symlink");
    let refused = pinfold_in(&env, symlinked.path(), &["pi", "--version"])
        .output()
        .expect("run pinfold pi --version");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success(),
        "pinfold pi started with a symlinked .vscode: {stderr}"
    );
    assert!(
        stderr.contains("protected-path-invalid"),
        "wrong refusal: {stderr}"
    );
    // Sabotage: keep cleanup only on the successful launch path. The earlier
    // .claude and .idea mounts leave empty directories after .vscode fails.
    for earlier in [".claude", ".cleanup", ".idea"] {
        assert!(
            !symlinked.path().join(earlier).exists(),
            "failed preparation left {earlier} behind"
        );
    }
    fs::remove_file(&link).expect("remove .vscode symlink");
    fs::create_dir(&link).expect("create real .vscode directory");
    pi_version(&env, symlinked.path());

    // Sabotage: release the Git cleanup guard before build_plan, or keep
    // manual cleanup after it. A non-UTF-8 state path passes trust and
    // preparation but its project home cannot enter the JSON spec. The
    // named path-not-utf8 refusal identifies that later guard; an earlier
    // failure cannot pass just because no protected directory exists.
    // APFS rejects these filenames, so this scenario is Linux-only.
    #[cfg(target_os = "linux")]
    {
        let invalid_state = env.root.join(std::ffi::OsString::from_vec(vec![0xff]));
        fs::create_dir(&invalid_state).expect("create non-UTF-8 state directory");
        run_ok(
            pinfold_in(&env, symlinked.path(), &["allow"]).env("XDG_STATE_HOME", &invalid_state),
        );
        let refused = pinfold_in(&env, symlinked.path(), &["pi", "--version"])
            .env("XDG_STATE_HOME", &invalid_state)
            .output()
            .expect("refuse the non-UTF-8 project home");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(!refused.status.success(), "accepted non-UTF-8 project home");
        assert!(stderr.contains("path-not-utf8"), "wrong refusal: {stderr}");
        for created in [".claude", ".cleanup", ".idea"] {
            assert!(
                !symlinked.path().join(created).exists(),
                "failed plan left {created} behind"
            );
        }
        assert!(
            link.is_dir(),
            "cleanup removed the existing .vscode directory"
        );
        pi_version(&env, symlinked.path());
    }

    // `pinfold pi` started inside the repository's `.git` refuses before it
    // creates a box or a project state: the fallback root would be `.git`
    // itself, mounted writable, so host git would run what the box writes
    // there. Sabotage: drop the `--is-inside-git-dir` check from
    // `project_root`; the run starts, exits 0, and the exit assertion fails.
    // Sabotage: remove `project-in-git` from that refusal; the reason assertion fails.
    let dot_git = fs::canonicalize(root.join(".git")).expect("canonicalize .git");
    let refused = pinfold_in(&env, &dot_git, &["pi", "--version"])
        .output()
        .expect("run pinfold pi --version");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success(),
        "pinfold pi started inside .git: {stderr}"
    );
    assert!(stderr.contains("project-in-git"), "wrong refusal: {stderr}");
    pi_version(&env, root);

    // A top level whose name is a space is the root: only git's trailing
    // newline comes off, so trimming would resolve to the parent and leave
    // the repository's `.git` inside it unprotected. Sabotage: trim git's
    // `--show-toplevel` output again; the parent becomes the root, so no
    // project state records the space-named root and `PiRpc::start` panics
    // finding its box. Its `.git`, under the parent's mount and not
    // protected, would take the hook write.
    let parent = TestDir::new(&env, "space-parent");
    let spaced = parent.path().join(" ");
    fs::create_dir(&spaced).expect("create the space-named top level");
    git(&spaced, &["init", "-q"]);
    let (run, _, name) = PiRpc::start(&env, &spaced);
    let denied = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf '#!/bin/sh\\n' > '{}'",
                spaced.join(".git/hooks/pre-commit").display()
            ),
        ],
    );
    assert_denied(
        &denied,
        "Read-only file system",
        "the space-named .git write",
    );
    // Positive control: the space-named project itself is writable.
    let control = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf 'ok\\n' > '{}'",
                spaced.join("control.txt").display()
            ),
        ],
    );
    assert_ok(&control, "writing a file in the space-named project");
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
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
    // Sabotage: omit the full profile's document skill or live package;
    // the skill never reaches the model. Save full's resource paths in the
    // project's settings, or keep using full's package after selecting
    // default; the returning default request still advertises documents.
    // Reseed existing settings on a profile switch; the host marker changes.
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
    build_profile(&env, profile);
    let _image = ImageCleanup {
        repository: format!("pinfold/profile-{profile}"),
    };
    default_image(&env);
    build_profile(&env, "full");

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
    // The first profile seeded models.json and settings.json. All later
    // selections reuse this home; only the live mounted pi package changes.
    let settings_path =
        project_state_dir(&env, project.path()).join("home/.pi/agent/settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_slice(&fs::read(&settings_path).unwrap()).unwrap();
    settings["e2eSavedSettingsMarker"] = serde_json::json!("keep-across-profile-switches");
    fs::write(&settings_path, serde_json::to_vec(&settings).unwrap()).unwrap();
    let saved = fs::read(&settings_path).unwrap();
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
        assert_eq!(
            fs::read(&settings_path).unwrap(),
            saved,
            "selecting {selected} overwrote saved pi settings"
        );
    }
}

#[test]
fn the_highest_layer_sets_the_allowlist() {
    let _runtime = crate::shared_runtime();
    // Sabotage: union DEFAULT_ALLOW under the merged allow in
    // `Config::load`; the default hosts stay in the box's PINFOLD_ALLOW and
    // npm is let through, so the exact-list assertion fails. Sabotage: merge
    // the environment layer below the project's in `Config::load` (or drop
    // `Layer::from_env`'s allow); the box's PINFOLD_ALLOW is then the
    // project's registry.npmjs.org, so the exact-list assertion fails and npm
    // is let through. Sabotage: drop `cpus` and `memory` from the Plan in
    // `build_plan`; the box gets the runtime's defaults and the memory-limit
    // assertion fails.
    let env = TestEnv::new("pi-allow");
    default_image(&env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);

    // The environment's list replaces the project's, which replaces the
    // built-in one: the box's allowlist is exactly the host PINFOLD_ALLOW
    // names, though the project allows another. Resource, protection and
    // route overrides use that same highest layer.
    let config = project.path().join(".pinfold.toml");
    let project_fixture = HttpFixture::start(Some(("text/plain", "project-route")));
    fs::write(
        &config,
        format!("allow = [\"registry.npmjs.org\"]\ncpus = 2\nmemory = \"1G\"\nprotect = [\"project-only\"]\n[routes]\n'override.internal' = '{}'\n", project_fixture.route()),
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
            ("PINFOLD_PROTECT", "host-config"),
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
    let env_allow = box_exec(&env, &name, &["sh", "-c", "printf %s \"$PINFOLD_ALLOW\""]);
    assert_eq!(
        env_allow.stdout, "api.github.com",
        "PINFOLD_ALLOW did not replace the project's allowlist"
    );

    // Sabotage: omit resource/protect/routes from Layer::from_env. Host
    // fixture input and the kernel's own observations supply expectations.
    let protected = project.path().join("host-config");
    let project_only = project.path().join("project-only");
    fs::create_dir(&project_only).expect("prepare lower-layer protection control");
    let superseded = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("echo writable > '{}/file'", project_only.display()),
        ],
    );
    assert_ok(
        &superseded,
        "environment protection replaces the project list",
    );
    let write = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("echo denied > '{}/file'", protected.display()),
        ],
    );
    assert_denied(
        &write,
        "Read-only file system",
        "environment-protected directory",
    );
    let writable = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("echo allowed > '{}/project-file'", project.path().display()),
        ],
    );
    assert_ok(&writable, "unprotected project file");
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
    let lines = json_lines(&egress_log(&env, &name));
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
        "PINFOLD_PROTECT",
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
fn writable_projects_exclude_host_authority() {
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
    // `run_config`; `egress.allow` then names the built-in hosts and the
    // exact-list assertion fails.
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
    // reads that state as data, and the home it names.
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
    let home = report["project"]["home"]
        .as_str()
        .expect("project.home is a string")
        .to_string();

    // After `pinfold allow` the same command reports the config trusted.
    allow(&env, project.path());
    let report = config_json(&env, project.path());
    assert_eq!(
        report["trust"]["ok"].as_bool(),
        Some(true),
        "the allowed config was reported untrusted"
    );

    // The home the report names is the one the box mounts as HOME.
    let (run, _, name) = PiRpc::start(&env, project.path());
    let boxed_home = box_exec(&env, &name, &["sh", "-c", "printf %s \"$HOME\""]);
    assert_eq!(
        boxed_home.stdout, home,
        "the box's HOME is not project.home"
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
