//! End-to-end tests for guarantees 6, 12, 13, 18 and 39 in
//! docs/ARCHITECTURE.md.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with Firecracker and its isolated BuildKit jobs. The harness isolates the XDG dirs, builds the
//! default profile image once, and drives `pinfold pi` as a user would. No
//! model is needed: `pi --mode rpc` answers `get_state` while the box runs,
//! and `pi --version` exits on its own.

use std::fs;
use std::io::{BufReader, Write};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, ExitStatus, Stdio};

use crate::box_::{ChildOwner, box_name, box_up_with_env, read_bounded};
use e2e::{
    HttpFixture, ImageCleanup, TestEnv, allow, assert_denied, assert_ok, box_exec, box_list, curl,
    default_image, git, inspect, json_lines, pinfold, pinfold_in, project_id, run_dir, run_ok,
    test_dir,
};

/// The PINFOLD_ENV_ value `PiRpc` launches with: long enough that no disk
/// image holds it by chance.
const SECRET: &str = "pinfold-e2e-secret-4f9c1d7a2be6";

/// Walk `dir`, panic on a regular file whose data holds `needle`, and
/// list every file read. Only data extents are read, so a sparse disk
/// costs what it holds.
pub(crate) fn holding(dir: &Path, needle: &[u8], scanned: &mut Vec<std::path::PathBuf>) {
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
    // Sabotage: let the box inherit the host environment; the unprefixed
    // proxy variables in `up`'s environment are then present and their
    // assertions fail. On Linux, seed the VM's environment from the
    // owner's; the host's proxy variables arrive and the proxy assertions
    // fail. Guarantee 39's pi box shows the secret's path.
    let env = TestEnv::new("pi-env");
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
    let proxies = [
        ("HTTP_PROXY", "http://upper-proxy.invalid:3128"),
        ("https_proxy", "http://lower-proxy.invalid:3128"),
        ("NO_PROXY", "proxy-marker.invalid"),
    ];
    let mut up = box_up_with_env(&env, &spec, &name, &proxies);
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
    // The host's proxy variables are absent: copied in, they would send
    // every HTTPS request to the host's unreachable proxy, since curl
    // prefers lowercase https_proxy.
    for (_, value) in proxies {
        assert!(
            !entries
                .iter()
                .any(|entry| String::from_utf8_lossy(entry).contains(value)),
            "{value} leaked into the box environment"
        );
    }
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
    // first's marker.
    let env = TestEnv::new("pi-state");
    default_image(&env);
    let a = test_dir(&env, "a/checkout");
    let b = test_dir(&env, "b/checkout");
    git(a.as_path(), &["init", "-q", "-b", "work"]);
    git(b.as_path(), &["init", "-q"]);
    let in_box = |name: &str, script: &str| {
        let output = box_exec(&env, name, &["sh", "-c", script]);
        assert_ok(&output, script);
        output.stdout
    };

    // Only branches cross: the first start makes a new repository in the
    // box, named for the host's branch, and checks that branch out clean.
    // Nothing else in the host's `.git` or work tree follows: not an
    // untracked `.env`, the blob of a file staged and then unstaged, a
    // stash, a remote URL holding a token, or a hook.
    // Two tags shadow the host's branch, by its short and its full name.
    // Sabotage: give project_disk the checkout as `from` again; the
    // untracked `.env` makes the box's `git status` unclean.
    // Sabotage: bundle `--all` instead of the branch list in crossing::push;
    // the stash commit's objects arrive and `stash` is printed.
    // Sabotage: drop the INIT_SCRIPT step from launch::sync; the push finds
    // no repository and committed.txt is missing.
    // Sabotage: name the box's branch by `symbolic-ref --short`; it
    // becomes `heads/work`, and committed.txt is missing.
    // Sabotage: bundle by `--branches` in crossing::push; the tag named
    // `work` empties the bundle, and committed.txt is missing. Sabotage:
    // drop `core.warnAmbiguousRefs=false` there; the tag named
    // `refs/heads/work` does the same.
    // Sabotage: exit when FAST_FORWARD_SCRIPT finds no HEAD, as before;
    // the unborn branch stays and committed.txt is missing.
    let write = |path: &str, contents: &str| fs::write(a.join(path), contents).unwrap();
    let id = ["-c", "user.name=e2e", "-c", "user.email=e2e@example.test"];
    write("committed.txt", "committed");
    git(a.as_path(), &["add", "committed.txt"]);
    git(
        a.as_path(),
        &[&id[..], &["commit", "-q", "-m", "start"]].concat(),
    );
    let head = git(a.as_path(), &["rev-parse", "HEAD"]).trim().to_string();
    git(a.as_path(), &["update-ref", "refs/tags/work", "HEAD"]);
    git(
        a.as_path(),
        &["update-ref", "refs/tags/refs/heads/work", "HEAD"],
    );
    write(".env", "FAKE_TOKEN=pinfold-e2e-untracked-env");
    write("staged.txt", "pinfold-e2e-unstaged-blob");
    git(a.as_path(), &["add", "staged.txt"]);
    let blob = git(a.as_path(), &["rev-parse", ":staged.txt"])
        .trim()
        .to_string();
    git(a.as_path(), &["reset", "-q", "--", "staged.txt"]);
    write("committed.txt", "pinfold-e2e-stashed");
    git(a.as_path(), &[&id[..], &["stash", "-q"]].concat());
    let stash = git(a.as_path(), &["rev-parse", "refs/stash"])
        .trim()
        .to_string();
    git(
        a.as_path(),
        &[
            "remote",
            "add",
            "origin",
            "https://x-access-token:pinfold-e2e-fake-token@example.invalid/repo.git",
        ],
    );
    fs::create_dir_all(a.join(".git/hooks")).unwrap();
    write(".git/hooks/pre-commit", "#!/bin/sh\nexit 1\n");
    let (run_a, _, box_a) = PiRpc::start(&env, a.as_path());
    let crossed = in_box(
        &box_a,
        &format!(
            r#"cd {} && cat committed.txt && test "$(git rev-parse HEAD)" = {head} && test -z "$(git status --porcelain)" && {{ test ! -e .env || echo .env; test ! -e staged.txt || echo staged.txt; ! git cat-file -e {blob} 2>/dev/null || echo blob; ! git cat-file -e {stash} 2>/dev/null || echo stash; ! git config --list | grep -q fake-token || echo url; test ! -e .git/hooks/pre-commit || echo hook; }}"#,
            a.display()
        ),
    );
    assert_eq!(crossed, "committed", "the first start is wrong");
    in_box(&box_a, r#"printf project-a > "$HOME/project-a-marker""#);
    assert!(run_a.finish().success(), "the first project failed");

    // A second checkout with the same directory name gets its own home and
    // does not see the first project's marker.
    let (run_b, _, box_b) = PiRpc::start(&env, b.as_path());
    let separate_b = in_box(
        &box_b,
        r#"test ! -e "$HOME/project-a-marker" && printf project-b > "$HOME/project-b-marker" && cat "$HOME/project-b-marker""#,
    );
    assert_eq!(separate_b, "project-b");
    assert!(run_b.finish().success(), "the second project failed");

    // The first project's home kept its marker and never saw the second's.
    let (run_a, _, box_a) = PiRpc::start(&env, a.as_path());
    let kept = in_box(
        &box_a,
        r#"test ! -e "$HOME/project-b-marker" && cat "$HOME/project-a-marker""#,
    );
    assert_eq!(kept, "project-a", "the first project's home changed");
    assert!(run_a.finish().success(), "the returning project failed");

    // A root without `.git` has no branches to cross: it refuses before
    // anything is created, and runs once it has a commit. Sabotage: accept
    // a missing `.git` in launch::require_repo; the plain folder starts.
    // Sabotage: create project state before require_repo refuses; the
    // unchanged projects-tree assertion fails.
    let plain = test_dir(&env, "plain");
    let projects = env.state.join("pinfold/projects");
    let before = crate::cli::host_tree(&projects);
    let output = pinfold_in(&env, plain.as_path(), &["pi", "--version"])
        .output()
        .expect("run pinfold");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "a folder without git started");
    assert!(stderr.contains("not-git"), "wrong refusal: {stderr}");
    assert_eq!(
        crate::cli::host_tree(&projects),
        before,
        "the refusal created project state"
    );
    git(plain.as_path(), &["init", "-q"]);
    git(
        plain.as_path(),
        &[&id[..], &["commit", "-q", "--allow-empty", "-m", "start"]].concat(),
    );
    pi_version(&env, plain.as_path());

    // Sabotage: stop bounding the cosmetic project-id prefix; the trust
    // filename exceeds the host's 255-byte component limit before pi runs.
    let long = test_dir(&env, &"x".repeat(245));
    git(long.as_path(), &["init", "-q"]);
    allow(&env, long.as_path());
    pi_version(&env, long.as_path());
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
    // the dirty box's `host/main` is not the host's commit and its
    // assertion fails. Sabotage: check trust in `build`
    // only when the project names a Containerfile; the build after the
    // domain change runs and its refusal assertion fails.
    // Sabotage: drop the crossing::push call from pi::launch::sync; the
    // behind box never sees its host commit and stays. Sabotage: drop the
    // diff-index line from FAST_FORWARD_SCRIPT; the dirty box is
    // fast-forwarded. Sabotage: drop the merge-base check and replace
    // `merge -q --ff-only` with `reset -q --hard`; the diverged box loses its
    // commit. Sabotage: drop `sync-failed:` from sync's report; the failure
    // assertion fails. Sabotage: remove trust::validate_host_paths, or
    // compare unresolved XDG paths; allow then records trust inside the
    // project and pi/build accept it. The missing suffix behind a symlink is
    // the case canonicalize(path) alone cannot check.
    let env = TestEnv::new("pi-trust");
    default_image(&env);
    let project = test_dir(&env, "project");
    let root = project.as_path();
    let host_git = |args: &[&str]| git(root, args);
    host_git(&["init", "-q", "-b", "main"]);
    // The host identity stays out of the repository's config: the box
    // commits under its image's identity.
    let id = ["-c", "user.name=e2e", "-c", "user.email=e2e@example.test"];
    host_git(&[&id[..], &["commit", "-q", "--allow-empty", "-m", "start"]].concat());
    let root_str = root.to_str().unwrap().to_string();
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

    // State, config or cache roots inside the project, reached through a
    // symlinked ancestor whose suffix does not exist yet, are refused.
    let alias = env.root.join("project-alias");
    std::os::unix::fs::symlink(root, &alias).expect("symlink project ancestor");
    for key in ["XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"] {
        let unsafe_root = alias.join(format!("missing-{key}/nested"));
        for args in [&["allow"][..], &["build"][..], &["pi", "--version"][..]] {
            let output = pinfold_in(&env, root, args)
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

    // A project with no `.pinfold.toml` has nothing to trust: with external
    // roots it runs without `pinfold allow`. The agent creates the file and
    // commits it; the host fetches and merges that commit, after the box
    // has ended, so the fetch runs in a transient box.
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

    // The now-trusted file runs, and the box adds a domain to it.
    let (run, _, name) = PiRpc::start(&env, root);
    in_box(
        &name,
        &format!(
            "cd {} && printf 'allow = [\"example.com\", \"api.github.com\"]\n' > .pinfold.toml && git commit -qam domain",
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

    // Each start pushes the host's branches, then fast-forwards the box's
    // branch only when it is clean and strictly behind. Each host commit
    // adds a new file, so git itself would let every fast-forward through.
    let host_commit = |file: &str| {
        fs::write(root.join(file), file).unwrap();
        host_git(&["add", file]);
        host_git(&[&id[..], &["commit", "-q", "-m", file]].concat());
        host_git(&["rev-parse", "main"]).trim().to_string()
    };
    let box_rev = |name: &str, rev: &str| {
        let output = box_exec(&env, name, &["git", "-C", &root_str, "rev-parse", rev]);
        assert_ok(&output, rev);
        output.stdout.trim().to_string()
    };
    let behind = host_commit("behind.txt");
    let (run, _, name) = PiRpc::start(&env, root);
    assert_eq!(
        box_rev(&name, "HEAD"),
        behind,
        "a clean box behind host stayed"
    );
    // A tracked file the agent edits and leaves uncommitted.
    in_box(&name, &format!("cd {root_str} && printf box >> behind.txt"));
    assert!(run.finish().success(), "the behind run failed");

    let dirty = host_commit("dirty.txt");
    let (run, _, name) = PiRpc::start(&env, root);
    assert_eq!(
        box_rev(&name, "HEAD"),
        behind,
        "a dirty box was fast-forwarded"
    );
    assert_eq!(box_rev(&name, "refs/remotes/host/main"), dirty);
    in_box(
        &name,
        &format!(
            "cd {root_str} && git checkout -q -- behind.txt && git commit -q --allow-empty -m local"
        ),
    );
    let local = box_rev(&name, "HEAD");
    assert!(run.finish().success(), "the dirty run failed");

    let diverged = host_commit("diverged.txt");
    let (run, _, name) = PiRpc::start(&env, root);
    assert_eq!(box_rev(&name, "HEAD"), local, "a diverged box moved");
    assert_eq!(box_rev(&name, "refs/remotes/host/main"), diverged);
    // An untracked file that the next fast-forward would overwrite.
    in_box(
        &name,
        &format!("cd {root_str} && git reset -q --hard host/main && printf box > late.txt"),
    );
    assert!(run.finish().success(), "the diverged run failed");
    host_commit("late.txt");
    let started = run_ok(&mut pinfold_in(&env, root, &["pi", "--version"]));
    let stderr = String::from_utf8_lossy(&started.stderr);
    assert!(stderr.contains("sync-failed"), "no sync failure: {stderr}");

    // Adding an explicit image changes trusted bytes even when it names
    // the image the project already ran. Sabotage: omit `image` from the
    // project trust input, or skip trust for explicit-image launches.
    e2e::profile_containerfile(&env, "default", "FROM scratch\n");
    fs::write(
        &config,
        format!(
            "{}image = {:?}\n",
            fs::read_to_string(&config).unwrap(),
            default_image(&env)
        ),
    )
    .unwrap();
    for args in [&["pi", "--version"][..], &["fetch"][..], &["build"][..]] {
        refused(args);
    }
    allow(&env, root);
    let started = run_ok(&mut pinfold_in(&env, root, &["pi", "--version"]));
    // Sabotage: compare an explicit image with its unrelated profile
    // recipe; the managed-image freshness warning is then false.
    assert!(
        !String::from_utf8_lossy(&started.stderr).contains("image-outdated"),
        "an explicit image was judged against the profile recipe"
    );
    // Trust is checked before the explicit-image build refusal. Sabotage:
    // fall back to the profile recipe; this starts a build instead.
    let no_recipe = pinfold_in(&env, root, &["build"]).output().unwrap();
    assert!(!no_recipe.status.success(), "an explicit image was rebuilt");
    assert!(
        String::from_utf8_lossy(&no_recipe.stderr).contains("image-no-recipe"),
        "{no_recipe:?}"
    );

    // The project's own Containerfile is trusted the same way: a change to
    // it stops both the run and the build until `pinfold allow` records the
    // new bytes.
    let _images = ImageCleanup {
        repository: format!("localhost/pinfold/project-{}", project_id(&env, root)),
    };
    let containerfile = root.join("Containerfile.pinfold");
    // The project recipe also works with a configuration-only profile.
    fs::create_dir_all(env.config.join("pinfold/profiles/e2e-project-recipe")).unwrap();
    fs::write(
        &config,
        "profile = \"e2e-project-recipe\"\ncontainerfile = \"Containerfile.pinfold\"\nallow = [\"example.com\"]\n",
    )
    .expect("point .pinfold.toml at the project Containerfile");
    fs::write(
        &containerfile,
        "FROM localhost/pinfold/profile-default:latest\n",
    )
    .expect("write the project Containerfile");
    allow(&env, root);

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
fn the_launch_record_is_the_applied_policy() {
    let _runtime = crate::shared_runtime();
    // Guarantee 39. Sabotage: build `inspect`'s launch section by loading
    // the project's config at query time; the changed allow list then
    // appears. Sabotage: write the plan's env as name and value, or a
    // route's resolved headers, into the launch record; the state-root scan
    // finds the secret or the header value. Sabotage, on Linux: have init
    // write the box's environment to a file in the home disk; the scan
    // reads the disk's data extents and finds the secret. Apple's disks
    // are runtime volumes outside the state root. Sabotage: drop `cpus` from
    // the record; its equality fails.
    // Sabotage: pass the PINFOLD_ENV_* value on the command line (for
    // example `container exec --env SECRET=...`) instead of through the
    // child's environment; the value then appears in host `ps` while the box
    // runs and the ps assertion fails. On Linux, put the environment in
    // BOOT_ARGS (config.json and /proc/cmdline fail) or write it into the
    // init disk (init.ext4 fails).
    // Sabotage: union DEFAULT_ALLOW under the merged allow in
    // `Config::load`; the default hosts include npm, so the npm 403
    // assertion fails. Sabotage: merge the environment layer below the
    // project's in `Config::load` (or drop `Layer::from_env`'s allow); the
    // allowlist is then the project's registry.npmjs.org, so npm is let
    // through and the npm 403 assertion fails. Sabotage: drop `cpus` and
    // `memory` from `box_plan`, which `build_plan` spreads; the box gets
    // the runtime's defaults and the memory assertion fails. Sabotage: omit
    // routes from Layer::from_env; `config` reports the file's routes.
    // Sabotage: turn env::var errors into absence with .ok(); malformed
    // overrides then silently use a lower layer.
    // Sabotage: report DEFAULT_ALLOW instead of the merged allow in
    // `config_report`; `egress.allow` then names the built-in hosts and the
    // exact-list assertion fails. Sabotage: swap the two names
    // `state::disk_names` returns; `disk list` shows the reported home disk
    // attached at the project, not as $HOME, and the attachment assertion
    // fails. Sabotage: resolve `config_report`'s image with `:latest`
    // replaced by another tag; on both runtimes the built profile image
    // reads as not built.
    let env = TestEnv::new("pi-launch");
    let selected_image = default_image(&env);
    let selected_id = e2e::image_id(selected_image).expect("the runtime holds the selected image");
    // A configuration-only profile supplies native files independently of
    // the explicit image. Sabotage: require its Containerfile, omit its
    // seeds/share, or make the share writable; the real guest fails below.
    let profile_dir = env.config.join("pinfold/profiles/e2e-launch");
    fs::create_dir_all(profile_dir.join("home")).unwrap();
    fs::create_dir_all(profile_dir.join("share")).unwrap();
    fs::write(profile_dir.join("home/seed"), "seed-one").unwrap();
    fs::write(profile_dir.join("share/fixture"), "share-one").unwrap();
    let project = test_dir(&env, "project");
    git(project.as_path(), &["init", "-q"]);
    let service = HttpFixture::start(Some(("text/plain", "service")));
    let upstream = HttpFixture::start(Some(("text/plain", "upstream")));
    let upstream_origin = format!("http://{}", upstream.route());
    let config = project.as_path().join(".pinfold.toml");
    fs::write(
        &config,
        format!(
            "profile = \"e2e-launch\"\nimage = \"{selected_image}\"\nallow = [\"registry.npmjs.org\"]\ncpus = 2\nmemory = \"1G\"\n[routes]\n'service.internal' = '{}'\n[routes.'inject.internal']\nto = '{upstream_origin}'\nheaders = {{ Authorization = {{ from = \"PINFOLD_E2E_LAUNCH_KEY\", prefix = \"Bearer \" }} }}\n",
            service.route()
        ),
    )
    .expect("write .pinfold.toml");

    // Before `pinfold allow`, the project's config is untrusted; the caller
    // reads that state as data, and the home disk it names.
    let report = config_json(&env, project.as_path());
    assert_eq!(
        report["egress"]["allow"],
        serde_json::json!(["registry.npmjs.org"]),
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
        "the selected local image was reported not built: {report}"
    );
    #[cfg(target_os = "linux")]
    {
        // Guarantee 18's present-store control. Sabotage: gate catalog reads
        // on host preflight or an external runtime; this real ioctl refusal
        // and empty PATH then hide the already-built selected image. Expected
        // identity is the host inventory captured before removing those tools.
        let catalog = env.cache.join("pinfold/images/catalog.json");
        let before = fs::read(&catalog).unwrap();
        let empty = test_dir(&env, "config-empty-path");
        let mut command = pinfold_in(&env, project.as_path(), &["config"]);
        command.env("PATH", &empty);
        crate::cli::ioctl_dev_denied(&mut command);
        let output = run_ok(&mut command);
        let offline: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(offline["image_built"], true, "{offline}");
        assert_eq!(offline["image"], selected_image, "{offline}");
        assert_eq!(
            fs::read(catalog).unwrap(),
            before,
            "config changed the existing catalog"
        );
    }
    assert_eq!(report["project"]["home"], "/home/pi", "{report}");
    let home_disk = report["project"]["disks"]["home"]
        .as_str()
        .expect("project.disks.home is a string")
        .to_string();

    // The environment's routes replace the file's whole table.
    let routed = run_ok(
        pinfold_in(&env, project.as_path(), &["config"])
            .env("PINFOLD_ROUTES", "override.internal=127.0.0.1:9"),
    );
    let routed: serde_json::Value = serde_json::from_slice(&routed.stdout).unwrap();
    assert_eq!(
        routed["egress"]["routes"],
        serde_json::json!({ "override.internal": "127.0.0.1:9" }),
        "the environment's routes did not replace the file's"
    );
    // Malformed overrides fail, and their bytes stay private.
    for key in [
        "PINFOLD_PROFILE",
        "PINFOLD_ALLOW",
        "PINFOLD_ROUTES",
        "PINFOLD_CPUS",
        "PINFOLD_MEMORY",
    ] {
        let mut bytes = b"private-override-marker".to_vec();
        bytes.push(0xff);
        let invalid = pinfold_in(&env, project.as_path(), &["config"])
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

    allow(&env, project.as_path());
    let report = config_json(&env, project.as_path());
    assert_eq!(
        report["trust"]["ok"].as_bool(),
        Some(true),
        "the allowed config was reported untrusted"
    );

    // The environment's allow list, cpus and memory replace the file's.
    let secret = format!("pf-launch-secret-{}", std::process::id());
    let header = format!("pf-launch-header-{}", std::process::id());
    let (run, _, name) = PiRpc::start_with_env(
        &env,
        project.as_path(),
        &[
            ("PINFOLD_ENV_SECRET", &secret),
            ("PINFOLD_E2E_LAUNCH_KEY", &header),
            ("PINFOLD_ALLOW", "api.github.com"),
            ("PINFOLD_CPUS", "1"),
            ("PINFOLD_MEMORY", "512M"),
        ],
    );
    let run_id = pi_run_id(&env, &name);
    let files = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            "cat \"$HOME/seed\" /opt/pinfold/profile/fixture && printf user-kept > \"$HOME/seed\" && ! { printf guest-write > /opt/pinfold/profile/fixture; }",
        ],
    );
    assert_ok(&files, "reading seeds and refusing a profile share write");
    assert_eq!(files.stdout, "seed-oneshare-one");
    assert_eq!(
        fs::read_to_string(profile_dir.join("share/fixture")).unwrap(),
        "share-one"
    );

    // PINFOLD_ENV_SECRET arrives as SECRET, and never shows in host `ps`
    // while the box runs.
    let boxed = box_exec(&env, &name, &["sh", "-c", "printf %s \"$SECRET\""]);
    assert_ok(&boxed, "reading SECRET");
    assert_eq!(boxed.stdout, secret, "SECRET did not arrive");
    let ps = Command::new("ps")
        .args(["-ww", "-A", "-o", "command"])
        .output()
        .expect("run ps");
    // A failed ps prints nothing and would pass the absence check whatever
    // argv held.
    assert!(
        ps.status.success(),
        "host ps failed: {}",
        String::from_utf8_lossy(&ps.stderr)
    );
    let ps = String::from_utf8_lossy(&ps.stdout);
    assert!(
        !ps.contains(&secret),
        "the secret is in host ps output:\n{ps}"
    );

    // On Linux the secret reaches the guest only over vsock: not on its
    // kernel command line, nor in any file pinfold writes for the box (the
    // VM configuration, the init disk, mount images, disks).
    #[cfg(target_os = "linux")]
    {
        let cmdline = box_exec(&env, &name, &["cat", "/proc/cmdline"]);
        assert_ok(&cmdline, "reading the guest's /proc/cmdline");
        assert!(
            !cmdline.stdout.contains(&secret),
            "the secret is on the guest's kernel command line"
        );
        let mut scanned = Vec::new();
        holding(&env.state, secret.as_bytes(), &mut scanned);
        holding(
            &env.cache.join("pinfold/root"),
            secret.as_bytes(),
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

    // The home disk the report names is the one the box attaches, writable,
    // at its $HOME: a file the box writes there is on that disk.
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

    // The live box's file changes and is not re-allowed.
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("registry.npmjs.org", "example.com")
            .replace(selected_image, "localhost/pinfold/e2e-unselected:missing"),
    )
    .unwrap();
    let report = config_json(&env, project.as_path());
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
    assert_eq!(launch["cpus"], 1.0);
    assert_eq!(launch["memory"], "512M");
    // Sabotage: resolve image identity from current project config at
    // inspection time. The host's runtime inventory supplies the id.
    assert_eq!(launch["image"]["ref"], selected_image);
    assert_eq!(launch["image"]["id"], selected_id);

    // The started list still decides: the environment's host passes; the
    // file's host at start and the changed file's host are refused before
    // the proxy dials, and the log names why.
    let allowed = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "the started list's host");
    for host in ["registry.npmjs.org", "example.com"] {
        let denied = curl(
            &env,
            &name,
            "30",
            &["-o", "/dev/null", &format!("https://{host}/")],
        );
        assert_denied(&denied, "403", &format!("a request to {host}"));
    }
    let log = run_dir(&env, &run_id).join("egress.jsonl");
    let lines = json_lines(&fs::read_to_string(&log).expect("read the pi run's egress log"));
    for host in ["registry.npmjs.org", "example.com"] {
        assert!(
            lines.iter().any(|line| line["host"] == host
                && line["decision"] == "refused"
                && line["reason"] == "not allowlisted"),
            "no not-allowlisted refusal for {host}: {lines:?}"
        );
    }

    assert!(run.finish().success(), "pinfold pi did not exit cleanly");
    // A pi box prints no `down`, so its harness ending is the
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

    // Host-owned profile changes need no project reapproval and do not
    // reset a persistent home. Sabotage: overwrite existing seeds on
    // startup, cache share bytes across starts, or recreate disks when
    // profile files change. The guest's edited seed must survive.
    fs::write(profile_dir.join("home/seed"), "seed-two").unwrap();
    fs::write(profile_dir.join("share/fixture"), "share-two").unwrap();
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("localhost/pinfold/e2e-unselected:missing", selected_image),
    )
    .unwrap();
    allow(&env, project.as_path());
    fs::write(profile_dir.join("home/seed"), "seed-three").unwrap();
    let (run, _, name) = PiRpc::start_with_env(
        &env,
        project.as_path(),
        &[("PINFOLD_E2E_LAUNCH_KEY", &header)],
    );
    let files = box_exec(
        &env,
        &name,
        &[
            "sh",
            "-c",
            "cat \"$HOME/seed\" /opt/pinfold/profile/fixture",
        ],
    );
    assert_ok(&files, "reading retained seed and current share");
    assert_eq!(files.stdout, "user-keptshare-two");
    assert!(run.finish().success(), "the profile update run failed");
}

#[test]
fn a_caller_reads_the_effective_configuration_as_data() {
    // Guarantee 39's pi box shows the live half.
    //
    // Sabotage: propagate a missing runtime from config_report, treat it as
    // an absent image, or populate embedded profiles while loading metadata.
    // The empty Linux store positively establishes absence. Apple's empty
    // PATH leaves its native image probe unavailable. The project file still
    // supplies its effective allowlist, and fresh host dirs stay empty.
    let offline = TestEnv::with_private_cache("pi-config-offline");
    let project = test_dir(&offline, "project");
    fs::write(
        project.as_path().join(".pinfold.toml"),
        "allow = [\"api.github.com\"]\n",
    )
    .unwrap();
    let empty = test_dir(&offline, "empty-path");
    let output =
        run_ok(pinfold_in(&offline, project.as_path(), &["config"]).env("PATH", empty.as_path()));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("read offline config");
    assert_eq!(
        report["egress"]["allow"],
        serde_json::json!(["api.github.com"])
    );
    if cfg!(target_os = "linux") {
        assert_eq!(
            report["image_built"], false,
            "an empty store reported an image: {report}"
        );
        assert!(
            report["image_error"].is_null(),
            "empty local store is not a probe failure: {report}"
        );
    } else {
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
    }

    // Sabotage: map `full` to the shared CONFIG in `Profile::builtin`; config
    // then reports only the built-in defaults. The expected list is the
    // spec's default list plus the two GitHub hosts.
    let bare = test_dir(&offline, "bare");
    let output = run_ok(
        pinfold_in(&offline, bare.as_path(), &["config"])
            .env("PATH", empty.as_path())
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
    let project_config = project.as_path().join(".pinfold.toml");
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
        let refused = pinfold_in(&offline, project.as_path(), &["config"])
            .env("PATH", empty.as_path())
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
        let corrected = run_ok(
            pinfold_in(&offline, project.as_path(), &["config"]).env("PATH", empty.as_path()),
        );
        let report: serde_json::Value = serde_json::from_slice(&corrected.stdout).unwrap();
        assert_eq!(
            report["egress"]["allow"],
            serde_json::json!(["api.github.com"])
        );
    }

    // Image sources are selected after layering. Sabotage: ignore a
    // profile's image, prefer a recipe over an explicit image, or require
    // every profile to hold a Containerfile. Each expected reference and
    // source below comes from the fixture's own declared inputs.
    let recipe = profile_dir.join("Containerfile");
    fs::write(project.join("recipe"), "FROM scratch\n").unwrap();
    git(project.as_path(), &["init", "-q"]);
    let owned_trees = || {
        [&offline.state, &offline.config, &offline.root.join("cache")]
            .into_iter()
            .map(|directory| crate::cli::host_tree(directory))
            .collect::<Vec<_>>()
    };
    for (project_inputs, profile_inputs, has_recipe, source, image, containerfile) in [
        (
            "",
            "",
            true,
            "profile",
            format!("localhost/pinfold/profile-{profile}:latest"),
            Some("profile"),
        ),
        (
            "",
            "image = 'localhost/fixture-profile:local'\n",
            true,
            "explicit",
            "localhost/fixture-profile:local".to_string(),
            None,
        ),
        (
            "image = 'localhost/fixture-project:local'\n",
            "image = 'localhost/fixture-profile:local'\n",
            false,
            "explicit",
            "localhost/fixture-project:local".to_string(),
            None,
        ),
        (
            "containerfile = 'recipe'\n",
            "",
            false,
            "project",
            String::new(),
            Some("recipe"),
        ),
    ] {
        fs::write(&project_config, format!("{valid_project}{project_inputs}")).unwrap();
        fs::write(&profile_config, format!("{valid_profile}{profile_inputs}")).unwrap();
        if has_recipe {
            fs::write(&recipe, "FROM scratch\n").unwrap();
        } else if recipe.exists() {
            fs::remove_file(&recipe).unwrap();
        }
        let before = owned_trees();
        let output = run_ok(
            pinfold_in(&offline, project.as_path(), &["config"]).env("PATH", empty.as_path()),
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["image_source"], source, "{report}");
        if source != "project" {
            assert_eq!(report["image"], image, "{report}");
        }
        assert_eq!(report["containerfile"], serde_json::json!(containerfile));
        assert_eq!(
            report["image_built"],
            if cfg!(target_os = "linux") {
                serde_json::json!(false)
            } else {
                serde_json::Value::Null
            },
            "{report}"
        );
        assert_eq!(owned_trees(), before, "config wrote pinfold files");
    }

    // A profile image conflicts with a project recipe just as a project
    // image does. Missing and empty selections are distinct refusals.
    // Sabotage: choose silently on conflict, accept no image source, or
    // create box/project state before Config::load rejects. Config changes
    // no files; working commands may perform ordinary daily maintenance.
    for (project_inputs, profile_inputs, reason) in [
        (
            "containerfile = 'recipe'\n",
            "image = 'localhost/fixture-profile:local'\n",
            "image-source-conflict",
        ),
        ("", "", "image-source-missing"),
        ("image = ''\n", "", "image-invalid"),
    ] {
        fs::write(&project_config, format!("{valid_project}{project_inputs}")).unwrap();
        fs::write(&profile_config, format!("{valid_profile}{profile_inputs}")).unwrap();
        let before = owned_trees();
        for args in [
            &["config"][..],
            &["pi", "--version"][..],
            &["build"][..],
            &["allow"][..],
        ] {
            let output = pinfold_in(&offline, project.as_path(), args)
                .env("PATH", empty.as_path())
                .output()
                .expect("refuse an invalid image selection");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{args:?} accepted {reason}");
            assert!(stderr.contains(reason), "wrong refusal: {stderr}");
            if args == ["config"] {
                assert_eq!(owned_trees(), before, "config wrote pinfold files");
            }
        }
        for directory in ["boxes", "projects", "disks", "runs"] {
            assert!(
                !offline.state.join("pinfold").join(directory).exists(),
                "{reason} created {directory} state"
            );
        }
        fs::write(
            &project_config,
            format!("{valid_project}image = 'localhost/fixture-project:local'\n"),
        )
        .unwrap();
        fs::write(&profile_config, valid_profile).unwrap();
        run_ok(pinfold_in(&offline, project.as_path(), &["config"]).env("PATH", empty.as_path()));
    }
}

/// A `pinfold pi --mode rpc` process with a live box.
pub(crate) struct PiRpc {
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
    pub(crate) fn start_with_env(
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
    pub(crate) fn finish(mut self) -> ExitStatus {
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

/// The selected-profile case of guarantee 23, using real local builds.
pub(super) fn selected_profile_freshness(env: &TestEnv, base: &str) {
    let project = test_dir(env, "profile-freshness");
    let root = project.as_path();
    git(root, &["init", "-q"]);
    let containerfile = root.join("Containerfile");
    fs::write(
        root.join(".pinfold.toml"),
        format!(
            "profile = \"e2e-freshness-{}\"\ncontainerfile = \"Containerfile\"\n",
            std::process::id()
        ),
    )
    .unwrap();
    // The selected local profile is the project's freshness dependency on
    // both hosts. Sabotage: omit that identity from project build labels,
    // compare only recipe bytes, or compare the profile with its first FROM;
    // doctor misses the unchanged project's changed selected profile. Rebuild
    // the same project as the allowed counterpart; its warning must clear.
    let selected_profile = &format!("e2e-freshness-{}", std::process::id());
    let _profile_image = ImageCleanup {
        repository: format!("localhost/pinfold/profile-{selected_profile}"),
    };
    let _project_image = ImageCleanup {
        repository: format!("localhost/pinfold/project-{}", project_id(env, root)),
    };
    let profile_recipe = e2e::profile_containerfile(
        env,
        selected_profile,
        &format!("FROM {base}\nENV PROFILE_PHASE=one\n"),
    );
    e2e::build_profile(env, selected_profile);
    fs::write(
        &containerfile,
        format!("FROM localhost/pinfold/profile-{selected_profile}:latest\n"),
    )
    .unwrap();
    allow(env, root);
    run_ok(&mut pinfold_in(env, root, &["build"]));
    fs::write(
        &profile_recipe,
        format!("FROM {base}\nENV PROFILE_PHASE=two\n"),
    )
    .unwrap();
    e2e::build_profile(env, selected_profile);
    let outdated = run_ok(&mut pinfold_in(env, root, &["doctor"]));
    assert!(
        String::from_utf8_lossy(&outdated.stderr).contains("image-outdated"),
        "changed selected profile was missed: {outdated:?}"
    );
    run_ok(&mut pinfold_in(env, root, &["build"]));
    let rebuilt = run_ok(&mut pinfold_in(env, root, &["doctor"]));
    assert!(
        !String::from_utf8_lossy(&rebuilt.stderr).contains("image-outdated"),
        "rebuilt project remains outdated: {rebuilt:?}"
    );
}
