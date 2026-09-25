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
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use e2e::{
    ImageCleanup, TestDir, TestEnv, box_exec, box_list, build_profile, curl, default_image,
    egress_log_lines, exit_code, git_init, git_status, pinfold, project_home, project_id,
};

#[test]
fn the_environment_is_exactly_the_spec() {
    // Sabotage: pass the PINFOLD_ENV_* value on the command line (for
    // example `container exec --env SECRET=shhh`) instead of through the
    // child's environment; `shhh` then appears in host `ps` while the box
    // runs and the ps assertion fails. Sabotage: let the box inherit the
    // host environment; the unprefixed variable is then present and its
    // assertion fails. Sabotage: skip the `validate` call in `Box::up`;
    // the `PINFOLD_ENV_BAD-NAME` run does not refuse and the refusal
    // assertions fail.
    let binary = pinfold();
    let env = TestEnv::new("pi-env");
    default_image(binary, &env);
    let project = TestDir::new(&env, "project");
    git_init(project.path());

    let (run, id, name) = PiRpc::start(binary, &env, project.path());

    // PINFOLD_ENV_SECRET arrives as SECRET; the prefix stays on the host.
    let secret = box_exec(binary, &env, &name, &["sh", "-c", "printf %s \"$SECRET\""]);
    assert_eq!(secret.code, 0, "reading SECRET failed: {}", secret.stderr);
    assert_eq!(secret.stdout, "shhh", "SECRET did not arrive");
    let prefixed = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$PINFOLD_ENV_SECRET\""],
    );
    assert_eq!(
        prefixed.stdout, "",
        "the PINFOLD_ENV_ prefix reached the box"
    );

    // An unprefixed host variable is absent.
    let host_only = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$E2E_HOST_ONLY\""],
    );
    assert_eq!(host_only.stdout, "", "an unprefixed host variable leaked");

    // The fixed environment is the spec's.
    for (variable, want) in [
        ("PI_TELEMETRY", "0"),
        ("PI_SKIP_VERSION_CHECK", "1"),
        ("HERDR_AGENT", "pi"),
    ] {
        let got = box_exec(
            binary,
            &env,
            &name,
            &["sh", "-c", &format!("printf %s \"${variable}\"")],
        );
        assert_eq!(got.stdout, want, "{variable} in the box");
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

    // Normal pi exit removes the box.
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
    let listed = box_list(binary, &env, &format!("dev.pinfold.project={id}"));
    assert!(listed.is_empty(), "the box survived pi exit: {listed:?}");

    // A host PINFOLD_ENV_* name outside POSIX is refused as a spec before
    // the box starts, naming the derived name: the shell cannot export such
    // a name, but `Command::env` can set it. Sabotage: skip the `validate`
    // call in `Box::up`; the name reaches the runtime, the run does not
    // refuse, and these assertions fail.
    let refused = env
        .command(binary)
        .args(["pi", "--version"])
        .current_dir(project.path())
        .env("PINFOLD_ENV_BAD-NAME", "x")
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold pi with a bad env name");
    assert_eq!(
        exit_code(refused.status),
        1,
        "the bad env name ran: {}",
        String::from_utf8_lossy(&refused.stderr)
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains(
            "pinfold pi: invalid box spec: env name \"BAD-NAME\" must match [A-Za-z_][A-Za-z0-9_]*"
        ),
        "the refusal did not name BAD-NAME: {stderr}"
    );
    let listed = box_list(binary, &env, &format!("dev.pinfold.project={id}"));
    assert!(listed.is_empty(), "the refused run left a box: {listed:?}");
}

#[test]
fn project_state_persists_and_stays_separate() {
    // Sabotage: derive the project id from the directory name alone (drop
    // the root hash in state.rs::id_for); the two checkouts named `checkout`
    // then share a home, and the assert_ne "two checkouts share a home"
    // fails. Sabotage: seed $HOME on every run instead of only when missing;
    // the edited marker is overwritten and the survives-a-run assertion
    // fails.
    let binary = pinfold();
    let env = TestEnv::new("pi-state");
    default_image(binary, &env);
    let a = TestDir::new(&env, "a/checkout");
    let b = TestDir::new(&env, "b/checkout");
    git_init(a.path());
    git_init(b.path());

    // The first run seeds the default profile's settings.json.
    pi_version(binary, &env, a.path());
    let settings_a = project_home(&env, a.path()).join(".pi/agent/settings.json");
    let seeded = fs::read_to_string(&settings_a).expect("read seeded settings.json");
    assert!(
        seeded.contains("defaultProjectTrust"),
        "seed content: {seeded}"
    );

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
    assert_ne!(home_a, home_b, "two checkouts share a home");
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

    // A deleted seed comes back on the next run.
    fs::remove_file(&settings_a).expect("delete settings.json");
    pi_version(binary, &env, a.path());
    let reseeded = fs::read_to_string(&settings_a).expect("read reseeded settings.json");
    assert!(
        reseeded.contains("defaultProjectTrust"),
        "reseed content: {reseeded}"
    );
    assert!(!reseeded.contains("project-a"), "the marker came back");
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
    git_init(project.path());
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
    assert_eq!(
        created.code, 0,
        "writing .pinfold.toml in the box failed: {}",
        created.stderr
    );
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
    assert_eq!(
        changed.code, 0,
        "changing .pinfold.toml in the box failed: {}",
        changed.stderr
    );
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
    assert_eq!(
        changed.code, 0,
        "changing the Containerfile in the box failed: {}",
        changed.stderr
    );
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
    // writable); the hook, `core.fsmonitor` and `commondir` writes and the
    // rename then succeed, and host `git status` runs the planted fsmonitor,
    // so those assertions fail. Sabotage: skip the absent protect
    // directories; the "pinfold did not create the absent .vscode" assertion
    // fails before the box starts. Sabotage: classify protected paths with
    // fs::metadata instead of the symlink check; a symlinked `.vscode` is
    // followed, the run starts, and the refusal assertion fails.
    // Sabotage: skip reading `core.hooksPath` in pi::git; the
    // `.husky/_/pre-commit` write succeeds, the hook exists on the host, and
    // those assertions fail. Sabotage: drop the GIT_CONFIG_* entries from
    // the pi box's env. The sabotage bites only while Apple presents the
    // mount top as root-owned; the Y-55 gate saw that on 2026-09-24, when
    // the positive control's `git status` and `git log` exited 128 with
    // `dubious ownership`, and a later probe did not see it, so the failure
    // is not deterministic. On podman keep-id makes the mount top the box
    // user's, so it never bites.
    let binary = pinfold();
    let env = TestEnv::new("pi-git");
    default_image(binary, &env);

    // A project with no `.vscode/` yet: pinfold creates the protected
    // directories empty before the run, so the box cannot create them, and
    // removes them after the run when they are still empty.
    let bare = TestDir::new(&env, "bare");
    git_init(bare.path());
    let root = bare.path();
    // A commit for the box's `git log` positive control below. The author is
    // on the command line, so the fixture does not need host git config.
    let status = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args([
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
        ])
        .status()
        .expect("run git commit");
    assert!(status.success(), "git commit failed");
    // Host git runs the hooks named by `core.hooksPath`; a value inside the
    // project must be read-only like `.git` itself.
    let husky = root.join(".husky/_");
    let status = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["config", "core.hooksPath", ".husky/_"])
        .status()
        .expect("run git config core.hooksPath");
    assert!(status.success(), "git config core.hooksPath failed");
    fs::create_dir_all(&husky).expect("create .husky/_");
    let vscode = root.join(".vscode");
    assert!(!vscode.exists(), "the fixture already has .vscode");

    let (run, _, name) = PiRpc::start(binary, &env, root);
    assert!(vscode.is_dir(), "pinfold did not create the absent .vscode");
    assert_eq!(
        fs::read_dir(&vscode).expect("read .vscode").count(),
        0,
        ".vscode was not created empty"
    );

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
    assert_ne!(denied.code, 0, "the box created .vscode");
    assert!(
        denied.stderr.contains("File exists"),
        "mkdir .vscode failed for another reason: {}",
        denied.stderr
    );
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
    assert_ne!(denied.code, 0, "the box wrote .vscode/settings.json");
    assert!(
        denied.stderr.contains("Read-only file system"),
        "the .vscode write failed for another reason: {}",
        denied.stderr
    );

    // `.git` is read-only: a hook, the config, and commondir.
    let hook = root.join(".git/hooks/pre-commit");
    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf '#!/bin/sh\\ntouch {}/pwned-hook\\n' > '{}'",
                root.display(),
                hook.display()
            ),
        ],
    );
    assert_ne!(denied.code, 0, "the box wrote a git hook");
    assert!(
        denied.stderr.contains("Read-only file system"),
        "the hook write failed for another reason: {}",
        denied.stderr
    );

    // Host git runs `.husky/_/pre-commit` on the next commit because the
    // repo's `core.hooksPath` names it; the box cannot write it.
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
    assert_ne!(denied.code, 0, "the box wrote a core.hooksPath hook");
    assert!(
        denied.stderr.contains("Read-only file system"),
        "the core.hooksPath hook write failed for another reason: {}",
        denied.stderr
    );

    // A script in the writable project; the config that would make host git
    // run it is what must fail.
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
    assert_ne!(denied.code, 0, "the box set core.fsmonitor");
    assert!(
        denied.stderr.contains("Read-only file system"),
        "git config failed for another reason: {}",
        denied.stderr
    );

    let denied = box_exec(
        binary,
        &env,
        &name,
        &[
            "sh",
            "-c",
            &format!(
                "printf '../evilgit\\n' > '{}/.git/commondir'",
                root.display()
            ),
        ],
    );
    assert_ne!(denied.code, 0, "the box wrote .git/commondir");
    assert!(
        denied.stderr.contains("Read-only file system"),
        "the commondir write failed for another reason: {}",
        denied.stderr
    );

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
    assert_ne!(denied.code, 0, "the box renamed .git");
    assert!(
        denied.stderr.contains("Device or resource busy"),
        "renaming .git failed for another reason: {}",
        denied.stderr
    );

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
    assert_eq!(
        control.code, 0,
        "the box could not write a project file: {}",
        control.stderr
    );
    assert_eq!(
        fs::read(root.join("control.txt")).expect("read control.txt"),
        b"ok\n"
    );

    assert!(run.finish().success(), "pinfold pi did not exit cleanly");

    // Host git runs nothing the box planted: the fsmonitor was never set,
    // no hook exists, and .git was not renamed.
    git_status(root);
    assert!(
        !root.join("pwned-fsmonitor").exists(),
        "host git status ran the box's fsmonitor"
    );
    assert!(!root.join("pwned-hook").exists(), "the box planted a hook");
    assert!(
        !husky_hook.exists(),
        "the box planted a core.hooksPath hook"
    );
    assert!(!root.join(".git-moved").exists(), "the box renamed .git");
    assert!(
        !vscode.exists(),
        "the created .vscode was not removed after the run"
    );

    // A project that already has `.vscode/`: its settings stay read-only.
    let with = TestDir::new(&env, "with-vscode");
    git_init(with.path());
    let settings = with.path().join(".vscode/settings.json");
    fs::create_dir(with.path().join(".vscode")).unwrap();
    fs::write(&settings, "{\"keep\":true}\n").unwrap();
    let (run, _, name) = PiRpc::start(binary, &env, with.path());
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
    assert_ne!(denied.code, 0, "the box overwrote .vscode/settings.json");
    assert!(
        denied.stderr.contains("Read-only file system"),
        "the settings write failed for another reason: {}",
        denied.stderr
    );
    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
    assert_eq!(
        fs::read_to_string(&settings).expect("read settings.json"),
        "{\"keep\":true}\n",
        "the host's .vscode/settings.json changed"
    );
    assert!(
        with.path().join(".vscode").is_dir(),
        "the existing .vscode was removed"
    );

    // A protected path the host planted as a symlink is refused: the runtime
    // resolves a bind-mount source on the host, so following it would mount
    // the target into the box.
    let outside = TestDir::new(&env, "outside");
    let symlinked = TestDir::new(&env, "symlinked");
    git_init(symlinked.path());
    let link = symlinked.path().join(".vscode");
    std::os::unix::fs::symlink(outside.path(), &link).expect("create .vscode symlink");
    let refused = pi_version_output(binary, &env, symlinked.path());
    assert!(
        !refused.status.success(),
        "pinfold pi started with a symlinked .vscode"
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("not a real directory") && stderr.contains(&link.display().to_string()),
        "the refusal did not name {} as not a real directory: {stderr}",
        link.display()
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
    git_init(project.path());

    // The profile carries the user level: a skill in its share/pi package,
    // and the agent dir's models.json, where pi reads provider settings
    // (pi's models.md and custom-provider.md). The seeded settings.json from
    // `default` keeps defaultProjectTrust, so the project level loads too.
    let profile = "e2e-fake";
    let output = env
        .command(binary)
        .args(["profile", "new", profile, "--from", "default"])
        .stdin(Stdio::null())
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

    // The fake model listens on the host; only the route can reach it.
    let model = FakeModel::start();

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
        .env(
            "PINFOLD_ROUTES",
            format!("fake.model=127.0.0.1:{}", model.port()),
        )
        .env("PINFOLD_ENV_OPENAI_API_KEY", "sk-fake")
        .stdin(Stdio::null())
        .output()
        .expect("run pi -p");
    assert!(
        output.status.success(),
        "pi -p failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let request = model.request();
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
    // npm is let through, so the exact-list assertion fails.
    let binary = pinfold();
    let env = TestEnv::new("pi-allow");
    default_image(binary, &env);
    let project = TestDir::new(&env, "project");
    git_init(project.path());

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
        default_allow.stdout.contains("pi.dev"),
        "the built-in allowlist is missing pi.dev: {}",
        default_allow.stdout
    );
    assert!(
        run.finish().success(),
        "the default run did not exit cleanly"
    );

    // A project list replaces the built-in one: the box's allowlist is
    // exactly the host the file names.
    let config = project.path().join(".pinfold.toml");
    fs::write(&config, "allow = [\"api.github.com\"]\n").expect("write .pinfold.toml");
    allow(binary, &env, project.path());
    let (run, _, name) = PiRpc::start(binary, &env, project.path());
    let project_allow = box_exec(
        binary,
        &env,
        &name,
        &["sh", "-c", "printf %s \"$PINFOLD_ALLOW\""],
    );
    assert_eq!(
        project_allow.stdout, "api.github.com",
        "the project's allowlist did not replace the built-in one"
    );

    // The one listed host works.
    let allowed = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_eq!(
        allowed.code, 0,
        "allowlisted host failed: {}",
        allowed.stderr
    );

    // A host the built-in list allowed is refused now; the proxy decides
    // before dialing, so no request reaches npm.
    let denied = curl(
        binary,
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://registry.npmjs.org/"],
    );
    assert_ne!(denied.code, 0, "registry.npmjs.org was allowed through");
    assert!(
        denied.stderr.contains("403"),
        "expected a proxy 403: {}",
        denied.stderr
    );

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
    git_init(project.path());
    fs::write(
        project.path().join(".pinfold.toml"),
        "allow = [\"api.github.com\"]\n",
    )
    .expect("write .pinfold.toml");

    // Before `pinfold allow`, the project's config is untrusted; the caller
    // reads that state as data and the home is named but not created.
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
    assert!(
        !Path::new(&home).exists(),
        "pinfold config created the project home {home}"
    );

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
        let mut child = env
            .command(binary)
            .args(["pi", "--mode", "rpc"])
            .current_dir(project)
            .env("PINFOLD_ENV_SECRET", "shhh")
            .env("E2E_HOST_ONLY", "host-value")
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
    let output = pi_version_output(binary, env, project);
    assert!(
        output.status.success(),
        "pinfold pi --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run `pinfold pi --version` in `project` and return its output.
fn pi_version_output(binary: &Path, env: &TestEnv, project: &Path) -> Output {
    env.command(binary)
        .args(["pi", "--version"])
        .current_dir(project)
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold pi --version")
}

/// Run `pinfold allow` in `project` and assert it succeeds.
fn allow(binary: &Path, env: &TestEnv, project: &Path) {
    let output = env
        .command(binary)
        .arg("allow")
        .current_dir(project)
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold allow");
    assert!(
        output.status.success(),
        "pinfold allow failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run `pinfold config` in `project` and parse its JSON object.
fn config_json(binary: &Path, env: &TestEnv, project: &Path) -> serde_json::Value {
    let output = env
        .command(binary)
        .arg("config")
        .current_dir(project)
        .stdin(Stdio::null())
        .output()
        .expect("run pinfold config");
    assert!(
        output.status.success(),
        "pinfold config failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("pinfold config output is one JSON object")
}

/// Run `pinfold build` in `project` and assert it exits cleanly.
fn build(binary: &Path, env: &TestEnv, project: &Path) {
    let output = build_output(binary, env, project);
    assert!(
        output.status.success(),
        "pinfold build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run `pinfold build` in `project` and return its output.
fn build_output(binary: &Path, env: &TestEnv, project: &Path) -> Output {
    env.command(binary)
        .arg("build")
        .current_dir(project)
        .stdin(Stdio::null())
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

/// A fake OpenAI-compatible chat-completions server on the host. It answers
/// the one streaming request pi sends and keeps the body, so the test can
/// assert what pi loaded.
struct FakeModel {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeModel {
    fn start() -> FakeModel {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the fake model");
        let port = listener.local_addr().expect("fake model address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let captured = Arc::clone(&captured);
                thread::spawn(move || answer(stream, &captured));
            }
        });
        FakeModel { port, requests }
    }

    fn port(&self) -> u16 {
        self.port
    }

    /// The first request body the model answered.
    fn request(&self) -> String {
        self.requests
            .lock()
            .expect("lock the fake model's requests")
            .first()
            .cloned()
            .expect("the fake model got no request")
    }
}

/// Read one Content-Length-framed request, keep its body, and answer with a
/// streaming chat completion.
fn answer(mut stream: TcpStream, requests: &Mutex<Vec<String>>) {
    let mut data = Vec::new();
    let mut buffer = [0u8; 4096];
    let head_end = loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        data.extend_from_slice(&buffer[..read]);
        if let Some(end) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    while data.len() < head_end + content_length {
        let read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        data.extend_from_slice(&buffer[..read]);
    }
    requests
        .lock()
        .expect("lock the fake model's requests")
        .push(String::from_utf8_lossy(&data[head_end..]).into_owned());
    let body = concat!(
        "data: {\"id\":\"chatcmpl-e2e\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"fake-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-e2e\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"fake-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
        "data: [DONE]\n\n"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}
