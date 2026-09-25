//! End-to-end tests for guarantees 6, 11, 12, 13, 14, 16 and 18 in
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
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};

use e2e::{
    HttpFixture, ImageCleanup, TestDir, TestEnv, assert_denied, assert_ok, box_exec, box_list,
    box_stat, build_profile, curl, default_image, egress_log_lines, git, pinfold, project_home,
    project_id, run_ok,
};

#[test]
fn the_environment_is_exactly_the_spec() {
    // Sabotage: pass the PINFOLD_ENV_* value on the command line (for
    // example `container exec --env SECRET=shhh`) instead of through the
    // child's environment; `shhh` then appears in host `ps` while the box
    // runs and the ps assertion fails. Sabotage: let the box inherit the
    // host environment; the unprefixed variable is then present and its
    // assertion fails. Sabotage: drop `--http-proxy=false` from podman's
    // run argv; the box inherits the host's proxy variables and the proxy
    // assertions fail. Sabotage: skip the `validate` call in `Box::up`;
    // the `PINFOLD_ENV_BAD-NAME` run does not refuse and the refusal
    // assertions fail.
    let binary = pinfold();
    let env = TestEnv::new("pi-env");
    default_image(binary, &env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);

    // Warm the shared harness cache while no proxy variable is set: pinfold
    // fetches the harness on the host with curl, which honors https_proxy,
    // so the recognisable values below would break the fetch.
    pi_version(binary, &env, project.path());

    let (run, _, name) = PiRpc::start_with_env(
        binary,
        &env,
        project.path(),
        &[
            ("HTTP_PROXY", "http://upper-proxy.invalid:3128"),
            ("https_proxy", "http://lower-proxy.invalid:3128"),
            ("NO_PROXY", "proxy-marker.invalid"),
        ],
    );

    // PINFOLD_ENV_SECRET arrives as SECRET.
    let secret = box_exec(binary, &env, &name, &["sh", "-c", "printf %s \"$SECRET\""]);
    assert_eq!(secret.code, 0, "reading SECRET failed: {}", secret.stderr);
    assert_eq!(secret.stdout, "shhh", "SECRET did not arrive");

    // An unprefixed host variable is absent.
    let host_only = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$E2E_HOST_ONLY\""],
    );
    assert_eq!(host_only.stdout, "", "an unprefixed host variable leaked");

    // The host's proxy variables are absent: podman's run would otherwise
    // copy them in, and curl prefers lowercase https_proxy, so every HTTPS
    // request would go to the host's unreachable proxy.
    let boxed_env = box_exec(binary, &env, &name, &["env"]);
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

    // A host PINFOLD_ENV_* name outside POSIX refuses the run before the
    // box starts, naming the derived name: the shell cannot export such a
    // name, but `Command::env` can set it. Sabotage: skip the `validate`
    // call in `Box::up`; the name reaches the runtime, the run does not
    // refuse, and the exit assertion fails. Sabotage: drop `{name:?}` from
    // `validate`'s env-name message in core/plan.rs; the refusal no longer
    // names BAD-NAME and the naming assertion fails.
    let refused = env
        .command(binary)
        .args(["pi", "--version"])
        .current_dir(project.path())
        .env("PINFOLD_ENV_BAD-NAME", "x")
        .output()
        .expect("run pinfold pi with a bad env name");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "the bad env name ran: {stderr}");
    assert!(
        stderr.contains("BAD-NAME"),
        "the refusal did not name BAD-NAME: {stderr}"
    );
}

#[test]
fn project_state_persists_and_stays_separate() {
    // Sabotage: derive the project id from the directory name alone (drop
    // the root hash in state.rs::project_id); the two checkouts named
    // `checkout` then share a home, the second's settings.json is the first's
    // edited marker, and the second-seed assertions fail. Sabotage: seed
    // $HOME on every run instead of only when missing; the edited marker is
    // overwritten and the survives-a-run assertion fails. Sabotage: copy
    // every agent entry in `profile new --from-project` (drop
    // `agent_entry_excluded`); `auth.json` lands in the profile and its
    // assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("pi-state");
    default_image(binary, &env);
    let a = TestDir::new(&env, "a/checkout");
    let b = TestDir::new(&env, "b/checkout");
    git(a.path(), &["init", "-q"]);
    git(b.path(), &["init", "-q"]);

    // The first run seeds the default profile's settings.json.
    pi_version(binary, &env, a.path());
    let settings_a = project_home(&env, a.path()).join(".pi/agent/settings.json");

    // An edit survives the next run: a seed is copied only when missing.
    let marker = "{\"marker\":\"project-a\"}\n";
    fs::write(&settings_a, marker).expect("edit settings.json");
    pi_version(binary, &env, a.path());
    assert_eq!(
        fs::read_to_string(&settings_a).expect("read edited settings.json"),
        marker,
        "a run reseeded an edited file"
    );

    // A second checkout with the same directory name gets its own home and
    // does not see the first project's marker. Take the first home before
    // the second run: the sabotage overwrites the shared state.json's root.
    let home_a = project_home(&env, a.path());
    pi_version(binary, &env, b.path());
    let home_b = project_home(&env, b.path());
    let seeded_b =
        fs::read_to_string(home_b.join(".pi/agent/settings.json")).expect("read second seed");
    assert!(
        seeded_b.contains("defaultProjectTrust"),
        "second seed content: {seeded_b}"
    );
    assert!(
        !seeded_b.contains("project-a"),
        "the second project sees the first's marker"
    );

    // `profile new --from-project` copies the project's agent config, the
    // edited settings included, and leaves the login behind.
    let agent_a = home_a.join(".pi/agent");
    fs::write(agent_a.join("auth.json"), "{\"secret\":true}\n").expect("plant auth.json");
    let output = env
        .command(binary)
        .args(["profile", "new", "from-a", "--from-project"])
        .arg(a.path())
        .output()
        .expect("run pinfold profile new --from-project");
    assert!(
        output.status.success(),
        "pinfold profile new --from-project failed: {}",
        String::from_utf8_lossy(&output.stderr)
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
    pi_version(binary, &env, a.path());
    let reseeded = fs::read_to_string(&settings_a).expect("read reseeded settings.json");
    assert!(
        reseeded.contains("defaultProjectTrust"),
        "reseed content: {reseeded}"
    );
}

#[test]
fn a_changed_project_file_stops_the_run() {
    // Sabotage: drop the trust::check call from pi::launch; the created and
    // the changed `.pinfold.toml` then run and both refusal assertions fail.
    // Sabotage: hash an absent `.pinfold.toml` as the empty file instead of
    // recording its absence; the first run, with no `.pinfold.toml` and no
    // record, refuses as "not trusted" and the bare-run control fails.
    // Sabotage: record no Containerfile hash in trust::current
    // (`containerfile: None`); the changed Containerfile then runs and
    // builds, and the two refusal assertions after the change fail.
    let binary = pinfold();
    let env = TestEnv::new("pi-trust");
    default_image(binary, &env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);
    let config = project.path().join(".pinfold.toml");

    // A project with no `.pinfold.toml` has nothing to trust: it runs
    // without `pinfold allow`.
    pi_version(binary, &env, project.path());

    // `pinfold allow` records the absence, so the file the agent creates in
    // the live box is a change.
    allow(binary, &env, project.path());
    let (run, id, name) = PiRpc::start(binary, &env, project.path());
    let created = box_exec(
        binary,
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
    let refused = pi_version_output(binary, &env, project.path());
    assert!(!refused.status.success(), "the new .pinfold.toml ran");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("pinfold allow"),
        "the refusal did not name `pinfold allow`: {stderr}"
    );
    allow(binary, &env, project.path());
    pi_version(binary, &env, project.path());

    // The agent adds a domain to the now-trusted file in a live box.
    let (run, _, name) = PiRpc::start(binary, &env, project.path());
    let changed = box_exec(
        binary,
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
    let refused = pi_version_output(binary, &env, project.path());
    assert!(!refused.status.success(), "the changed .pinfold.toml ran");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("pinfold allow"),
        "the refusal did not name `pinfold allow`: {stderr}"
    );
    allow(binary, &env, project.path());
    pi_version(binary, &env, project.path());

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
    allow(binary, &env, project.path());
    build(binary, &env, project.path());
    // Control: with the Containerfile unchanged, the project image runs.
    pi_version(binary, &env, project.path());

    // The agent changes the Containerfile in a live box.
    let (run, _, name) = PiRpc::start(binary, &env, project.path());
    let changed = box_exec(
        binary,
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
    let refused = pi_version_output(binary, &env, project.path());
    assert!(!refused.status.success(), "the changed Containerfile ran");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("pinfold allow"),
        "the refusal did not name `pinfold allow`: {stderr}"
    );
    let refused = build_output(binary, &env, project.path());
    assert!(!refused.status.success(), "the changed Containerfile built");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("pinfold allow"),
        "the build refusal did not name `pinfold allow`: {stderr}"
    );
    allow(binary, &env, project.path());
    build(binary, &env, project.path());
    pi_version(binary, &env, project.path());
}

#[test]
fn the_box_cannot_write_git_or_protected_config() {
    // Sabotage: omit the `.git` read-only mount from pi::git (or mount it
    // writable); the `core.fsmonitor` write and the rename then succeed, and
    // host `git status` runs the planted fsmonitor, so those assertions fail.
    // Sabotage: skip the absent protect directories; the box's `mkdir
    // .vscode` then succeeds and its refusal assertion fails. Sabotage: skip
    // the symlink check in pi::git's `path_kind` (classify with fs::metadata
    // and drop the canonical comparison); a symlinked `.vscode` is followed,
    // the run starts, and the exit assertion fails.
    // Sabotage: resolve `core.hooksPath` from `.git/config` only (`git
    // config --file .git/config core.hooksPath` in pi::git's `hooks_path`);
    // the global config's `.husky/_` is not protected, and the
    // `.husky/_/pre-commit` write succeeds, so its assertion fails.
    // Sabotage: ignore the configured `protect` list in `Git::prepare`; the
    // `tooling/hooks.sh` write succeeds and its assertion fails. Sabotage:
    // drop the GIT_CONFIG_* entries from the pi box's env. The sabotage
    // bites only while Apple presents the mount top as root-owned; the Y-55
    // gate saw that on 2026-09-24, when the positive control's `git status`
    // and `git log` exited 128 with `dubious ownership`, and a later probe
    // did not see it, so the failure is not deterministic. On podman keep-id
    // makes the mount top the box user's, so it never bites.
    let binary = pinfold();
    let env = TestEnv::new("pi-git");
    default_image(binary, &env);

    // A project with no `.vscode/` yet: pinfold creates the protected
    // directories empty before the run, so the box cannot create them.
    let bare = TestDir::new(&env, "bare");
    git(bare.path(), &["init", "-q"]);
    let root = bare.path();
    // A commit for the box's `git log` positive control below. The author is
    // on the command line, so the fixture does not need host git config.
    git(
        root,
        &[
            "-c",
            "user.name=pinfold-e2e",
            "-c",
            "user.email=pinfold-e2e@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "pinfold-e2e",
        ],
    );
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

    let (run, _, name) =
        PiRpc::start_with_env(binary, &env, root, &[("GIT_CONFIG_GLOBAL", global)]);

    // Positive control: the box reads the project's git state. Apple
    // `container` shows a mount's top directory as root-owned inside the
    // box, so without the `safe.directory` entry git refuses the project as
    // dubious ownership.
    let root_str = root.to_str().expect("project root is UTF-8");
    let status = box_exec(
        binary,
        &env,
        &name,
        &["git", "-C", root_str, "status", "--porcelain"],
    );
    assert_eq!(status.code, 0, "git status failed: {}", status.stderr);
    let log = box_exec(
        binary,
        &env,
        &name,
        &["git", "-C", root_str, "log", "--oneline"],
    );
    assert_eq!(log.code, 0, "git log failed: {}", log.stderr);
    assert!(
        log.stdout.contains("pinfold-e2e"),
        "the box's git log is missing the commit: {}",
        log.stdout
    );

    // The box cannot create the protected directory it did not have...
    let denied = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", &format!("mkdir '{}'", vscode.display())],
    );
    assert_denied(&denied, "File exists", "mkdir .vscode");
    // ...nor write into it.
    let denied = box_exec(
        binary,
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
        binary,
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
        binary,
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
        binary,
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
        binary,
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

    // Host git runs nothing the box planted: the fsmonitor was never set.
    git(root, &["status", "--porcelain"]);
    assert!(
        !root.join("pwned-fsmonitor").exists(),
        "host git status ran the box's fsmonitor"
    );

    // A project that already has `.vscode/`: its settings stay read-only,
    // and so does a directory the project's `protect` list names.
    let with = TestDir::new(&env, "with-vscode");
    git(with.path(), &["init", "-q"]);
    let settings = with.path().join(".vscode/settings.json");
    fs::create_dir(with.path().join(".vscode")).unwrap();
    fs::write(&settings, "{\"keep\":true}\n").unwrap();
    let tooling = with.path().join("tooling/hooks.sh");
    fs::create_dir(with.path().join("tooling")).unwrap();
    fs::write(&tooling, "#!/bin/sh\n").unwrap();
    fs::write(
        with.path().join(".pinfold.toml"),
        "protect = [\"tooling\"]\n",
    )
    .unwrap();
    allow(binary, &env, with.path());
    let (run, _, name) = PiRpc::start(binary, &env, with.path());
    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("printf 'pwned' > '{}'", tooling.display()),
        ],
    );
    assert_denied(&denied, "Read-only file system", "the tooling write");
    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!("printf '{{}}' > '{}'", settings.display()),
        ],
    );
    assert_denied(&denied, "Read-only file system", "the settings write");
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
    assert_eq!(
        fs::read_to_string(&settings).expect("read settings.json"),
        "{\"keep\":true}\n",
        "the host's .vscode/settings.json changed"
    );
    assert_eq!(
        fs::read_to_string(&tooling).expect("read tooling/hooks.sh"),
        "#!/bin/sh\n",
        "the host's protected tooling/hooks.sh changed"
    );

    // A protected path the host planted as a symlink refuses the run, naming
    // the link: the runtime resolves a bind-mount source on the host, so
    // following it would mount the target into the box. Sabotage: drop the
    // path from pi::git's `not_real_dir` message; the refusal no longer
    // names the link and the naming assertion fails.
    let outside = TestDir::new(&env, "outside");
    let symlinked = TestDir::new(&env, "symlinked");
    git(symlinked.path(), &["init", "-q"]);
    let link = symlinked.path().join(".vscode");
    std::os::unix::fs::symlink(outside.path(), &link).expect("create .vscode symlink");
    let refused = pi_version_output(binary, &env, symlinked.path());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success(),
        "pinfold pi started with a symlinked .vscode: {stderr}"
    );
    assert!(
        stderr.contains(&link.display().to_string()),
        "the refusal did not name {}: {stderr}",
        link.display()
    );

    // `pinfold pi` started inside the repository's `.git` refuses before it
    // creates a box or a project state: the fallback root would be `.git`
    // itself, mounted writable, so host git would run what the box writes
    // there. Sabotage: drop the `--is-inside-git-dir` check from
    // `project_root`; the run starts, exits 0, and the exit assertion fails.
    let dot_git = fs::canonicalize(root.join(".git")).expect("canonicalize .git");
    let refused = pi_version_output(binary, &env, &dot_git);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success(),
        "pinfold pi started inside .git: {stderr}"
    );
    assert!(
        stderr.contains(&dot_git.display().to_string()),
        "the refusal did not name {}: {stderr}",
        dot_git.display()
    );
    // No project state dir names `.git` as its root: the refusal came before
    // `record_run`. A state dir is `<name>-<hash>`, so the `.git` root's
    // would be `.git-<hash>`.
    let mut leftover = Vec::new();
    if let Ok(entries) = fs::read_dir(env.state.join("pinfold/projects")) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(".git-") {
                leftover.push(entry.path());
            }
        }
    }
    assert!(
        leftover.is_empty(),
        "the refused run left project state for .git: {leftover:?}"
    );

    // A top level whose name is a space is the root: only git's trailing
    // newline comes off, so trimming would resolve to the parent and leave
    // the repository's `.git` inside the parent unprotected. Sabotage: trim
    // git's `--show-toplevel` output again; `pinfold config` reports the
    // parent as the root and its id starts with the parent's name.
    let parent = TestDir::new(&env, "space-parent");
    let spaced = parent.path().join(" ");
    fs::create_dir(&spaced).expect("create the space-named top level");
    git(&spaced, &["init", "-q"]);
    let report = config_json(binary, &env, &spaced);
    let id = report["project"]["id"]
        .as_str()
        .expect("project.id is a string");
    let parent_name = parent
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the parent's name is UTF-8");
    assert!(
        !id.starts_with(parent_name),
        "pinfold config resolved the space-named top level to its parent {parent_name}: {id}"
    );
}

#[test]
fn both_pi_config_levels_load_behind_a_route() {
    // Sabotage: drop the route (leave PINFOLD_ROUTES empty, or point it at
    // another name); the proxy refuses fake.model with a 403 and the
    // "pi -p failed" assertion fails before any request reaches the model.
    // Sabotage: skip the profile skill under the profile's share/pi/skills;
    // the profile marker is absent from the request. Sabotage: stop the
    // project from being trusted (remove defaultProjectTrust from the seeded
    // settings); the project marker is absent.
    let binary = pinfold();
    let env = TestEnv::new("pi-levels");
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);

    // The profile carries the user level: a skill in its share/pi package,
    // and the agent dir's models.json, where pi reads provider settings
    // (pi's models.md and custom-provider.md). The seeded settings.json from
    // `default` keeps defaultProjectTrust, so the project level loads too.
    let profile = "e2e-fake";
    let output = env
        .command(binary)
        .args(["profile", "new", profile, "--from", "default"])
        .output()
        .expect("run pinfold profile new");
    assert!(
        output.status.success(),
        "pinfold profile new {profile} failed: {}",
        String::from_utf8_lossy(&output.stderr)
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
    build_profile(binary, &env, profile);

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
    std::os::unix::fs::symlink(binary, &shim).expect("symlink pi to pinfold");
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
        .env("PINFOLD_ROUTES", format!("fake.model={}", model.route()))
        .env("PINFOLD_ENV_OPENAI_API_KEY", "sk-fake")
        .output()
        .expect("run pi -p");
    assert!(
        output.status.success(),
        "pi -p failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let requests = model.requests();
    let (_, request) = requests.first().expect("the fake model got no request");
    assert!(
        request.contains(profile_marker),
        "the profile skill never reached the model; the profile config level did not load"
    );
    assert!(
        request.contains(project_marker),
        "the project skill never reached the model; the project config level did not load"
    );
}

#[test]
fn the_highest_layer_sets_the_allowlist() {
    // Sabotage: union DEFAULT_ALLOW under the merged allow in
    // `Config::load`; the default hosts stay in the box's PINFOLD_ALLOW and
    // npm is let through, so the exact-list assertion fails. Sabotage: merge
    // the environment layer below the project's in `Config::load` (or drop
    // `Layer::from_env`'s allow); the box's PINFOLD_ALLOW is then the
    // project's registry.npmjs.org, so the exact-list assertion fails and npm
    // is let through. Sabotage: drop `cpus` and `memory` from the Plan in
    // `build_plan`; the box gets the runtime's defaults and the memory-limit
    // assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("pi-allow");
    default_image(binary, &env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);

    // No `.pinfold.toml`: the built-in defaults are the box's allowlist.
    let (run, _, name) = PiRpc::start(binary, &env, project.path());
    let default_allow = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$PINFOLD_ALLOW\""],
    );
    assert!(
        default_allow.stdout.contains("registry.npmjs.org"),
        "the built-in allowlist is missing registry.npmjs.org: {}",
        default_allow.stdout
    );
    assert!(
        run.finish().success(),
        "the default run did not exit cleanly"
    );

    // The environment's list replaces the project's, which replaces the
    // built-in one: the box's allowlist is exactly the host PINFOLD_ALLOW
    // names, though the project allows another. The project file still sets
    // the box's resources.
    let config = project.path().join(".pinfold.toml");
    fs::write(
        &config,
        "allow = [\"registry.npmjs.org\"]\ncpus = 2\nmemory = \"1G\"\n",
    )
    .expect("write .pinfold.toml");
    allow(binary, &env, project.path());
    let (run, _, name) = PiRpc::start_with_env(
        binary,
        &env,
        project.path(),
        &[("PINFOLD_ALLOW", "api.github.com")],
    );

    // `cpus` and `memory` reach the box: stat reports the memory limit, and
    // the box's cgroup shows the CPU quota, on both runtimes (the Apple
    // guest kernel exposes it too).
    let stat = box_stat(binary, &env, &name);
    assert_eq!(
        stat["memory"]["limit"].as_u64(),
        Some(1024 * 1024 * 1024),
        "the box's memory limit is not the project's 1G: {stat}"
    );
    let cpus = box_exec(binary, &env, &name, &["cat", "/sys/fs/cgroup/cpu.max"]);
    assert_eq!(cpus.code, 0, "reading cpu.max failed: {}", cpus.stderr);
    assert_eq!(
        cpus.stdout.trim(),
        "200000 100000",
        "the box's cpu quota is not the project's 2 cpus"
    );
    let env_allow = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$PINFOLD_ALLOW\""],
    );
    assert_eq!(
        env_allow.stdout, "api.github.com",
        "PINFOLD_ALLOW did not replace the project's allowlist"
    );

    // The one listed host works.
    let allowed = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "allowlisted host");

    // A host the project's list allows is refused: the environment layer is
    // higher. The proxy decides before dialing, so no request reaches npm.
    let denied = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://registry.npmjs.org/"],
    );
    assert_denied(&denied, "403", "a request to registry.npmjs.org");

    // The log names the refused host and the reason.
    let lines = egress_log_lines(&env, &name);
    assert!(
        lines.iter().any(|line| line["host"] == "registry.npmjs.org"
            && line["decision"] == "refused"
            && line["reason"] == "not allowlisted"),
        "no not-allowlisted refusal for registry.npmjs.org: {lines:?}"
    );

    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
}

#[test]
fn a_caller_reads_the_effective_configuration_as_data() {
    // Sabotage: report DEFAULT_ALLOW instead of the merged allow in
    // `run_config`; `egress.allow` then names the built-in hosts and the
    // exact-list assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("pi-config");
    default_image(binary, &env);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);
    fs::write(
        project.path().join(".pinfold.toml"),
        "allow = [\"api.github.com\"]\n",
    )
    .expect("write .pinfold.toml");

    // Before `pinfold allow`, the project's config is untrusted; the caller
    // reads that state as data, and the home it names.
    let report = config_json(binary, &env, project.path());
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
    allow(binary, &env, project.path());
    let report = config_json(binary, &env, project.path());
    assert_eq!(
        report["trust"]["ok"].as_bool(),
        Some(true),
        "the allowed config was reported untrusted"
    );

    // The home the report names is the one the box mounts as HOME.
    let (run, _, name) = PiRpc::start(binary, &env, project.path());
    let boxed_home = box_exec(binary, &env, &name, &["sh", "-c", "printf %s \"$HOME\""]);
    assert_eq!(
        boxed_home.stdout, home,
        "the box's HOME is not project.home"
    );
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
}

/// A `pinfold pi --mode rpc` process with a live box.
struct PiRpc {
    child: Child,
    stdin: Option<ChildStdin>,
    _reader: BufReader<ChildStdout>,
}

impl PiRpc {
    /// Start pi in `project` and wait for its answer; return the run, the
    /// project id, and the name of the one box the run owns.
    fn start(binary: &Path, env: &TestEnv, project: &Path) -> (PiRpc, String, String) {
        PiRpc::start_with_env(binary, env, project, &[])
    }

    /// [`PiRpc::start`] with extra variables in `pinfold pi`'s own
    /// environment.
    fn start_with_env(
        binary: &Path,
        env: &TestEnv,
        project: &Path,
        vars: &[(&str, &str)],
    ) -> (PiRpc, String, String) {
        let mut child = env
            .command(binary)
            .args(["pi", "--mode", "rpc"])
            .current_dir(project)
            .env("PINFOLD_ENV_SECRET", "shhh")
            .env("E2E_HOST_ONLY", "host-value")
            .envs(vars.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn pinfold pi");
        let mut stdin = child.stdin.take().expect("pi stdin");
        stdin
            .write_all(b"{\"type\":\"get_state\",\"id\":\"1\"}\n")
            .expect("write get_state");
        stdin.flush().expect("flush get_state");
        let stdout = child.stdout.take().expect("pi stdout");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let read = reader.read_line(&mut line).expect("read pi reply");
            assert!(read > 0, "pi exited before answering get_state");
            if let Ok(reply) = serde_json::from_str::<serde_json::Value>(line.trim())
                && reply["id"] == "1"
            {
                break;
            }
        }
        let run = PiRpc {
            child,
            stdin: Some(stdin),
            _reader: reader,
        };
        // The run has created the project state; the id names its directory.
        let id = project_id(env, project);
        let listed = box_list(binary, env, &format!("dev.pinfold.project={id}"));
        assert_eq!(listed.len(), 1, "expected one pi box for {id}: {listed:?}");
        let name = listed[0]["name"]
            .as_str()
            .expect("box name is a string")
            .to_string();
        (run, id, name)
    }

    /// Close pi's stdin, wait for it, and return its exit status.
    fn finish(mut self) -> ExitStatus {
        drop(self.stdin.take());
        self.child.wait().expect("wait for pinfold pi")
    }
}

impl Drop for PiRpc {
    fn drop(&mut self) {
        // A panicking test must not leave a live `pinfold pi` behind; the
        // next pinfold command prunes the box once its owner is gone.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `pinfold pi --version` in `project` and assert it exits cleanly.
fn pi_version(binary: &Path, env: &TestEnv, project: &Path) {
    run_ok(
        env.command(binary)
            .args(["pi", "--version"])
            .current_dir(project),
    );
}

/// Run `pinfold pi --version` in `project` and return its output.
fn pi_version_output(binary: &Path, env: &TestEnv, project: &Path) -> Output {
    env.command(binary)
        .args(["pi", "--version"])
        .current_dir(project)
        .output()
        .expect("run pinfold pi --version")
}

/// Run `pinfold allow` in `project` and assert it succeeds.
fn allow(binary: &Path, env: &TestEnv, project: &Path) {
    run_ok(env.command(binary).arg("allow").current_dir(project));
}

/// Run `pinfold config` in `project` and parse its JSON object.
fn config_json(binary: &Path, env: &TestEnv, project: &Path) -> serde_json::Value {
    let output = run_ok(env.command(binary).arg("config").current_dir(project));
    serde_json::from_slice(&output.stdout).expect("pinfold config output is one JSON object")
}

/// Run `pinfold build` in `project` and assert it exits cleanly.
fn build(binary: &Path, env: &TestEnv, project: &Path) {
    run_ok(env.command(binary).arg("build").current_dir(project));
}

/// Run `pinfold build` in `project` and return its output.
fn build_output(binary: &Path, env: &TestEnv, project: &Path) -> Output {
    env.command(binary)
        .arg("build")
        .current_dir(project)
        .output()
        .expect("run pinfold build")
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
