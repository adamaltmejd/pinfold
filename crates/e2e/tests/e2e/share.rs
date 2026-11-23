//! Guarantee 48: a share reaches only its folder by path, a read-only
//! folder is read-only, and a project holding a protected path is refused.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::os::unix::fs::symlink;
use std::path::Path;

use e2e::{Project, TestEnv, default_image, tree};

#[test]
fn a_share_reaches_only_its_folder_and_a_protected_path_is_refused() {
    // Sabotage (run): delete `paths.extend(extra)` in core/mounts.rs's
    // `protected_paths`. With the configuration directory symlinked in,
    // `-- true` then runs and exits 0.
    // Sabotage (run): in `protected_paths`, drop the init's own directory
    // (keep `artifacts/init`). With `artifacts/init/<sha>` linked in, `--
    // true` is then refused `spec` after creating project state.
    // `config edit` succeeding and the same project running, with each
    // symlink removed, are the positive controls.
    default_image();
    // On macOS the init case moves the supplied init, so the cache root is
    // the test's own; it holds no image there.
    let env = if cfg!(target_os = "macos") {
        TestEnv::own_cache("protected")
    } else {
        TestEnv::new("protected")
    };
    // A user profile on default's image, which each project selects.
    let profile = env.config_home().join("pinfold/profiles/e2e-base");
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join("pinfold.toml"), "base = \"default\"\n").unwrap();

    // For each protected directory, a new project that has never run: the
    // directory is moved into its checkout and linked from where pinfold
    // reads it. Both commands are refused and change nothing under the
    // state or config roots or the moved directory; with the link replaced
    // by the directory, both succeed.
    // On macOS also the supplied init's own directory, below
    // `artifacts/init`, which the first case's control installs, in a
    // project with no configuration yet: refused, `config edit` saves none.
    let mut cases = vec!["config", "profile"];
    if cfg!(target_os = "macos") {
        cases.push("init");
    }
    let mut projects: Vec<Project> = Vec::new();
    for case in cases {
        let name = format!("protected-{case}");
        let project = if case == "init" {
            Project::unconfigured(&env, &name)
        } else {
            let project = Project::new(&env, &name);
            let file = project.config_dir().join("pinfold.toml");
            let saved = fs::read_to_string(&file).unwrap();
            fs::write(&file, format!("profile = \"e2e-base\"\n{saved}")).unwrap();
            project
        };
        let protected = match case {
            "config" => project.config_dir().to_path_buf(),
            "profile" => profile.clone(),
            _ => {
                // No box of the earlier cases still attaches it.
                for earlier in &projects {
                    let stop = earlier.pinfold(&["stop"]);
                    assert_eq!(stop.code, 0, "{stop:?}");
                }
                let inits: Vec<_> = fs::read_dir(env.cache_home().join("pinfold/artifacts/init"))
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect();
                assert_eq!(inits.len(), 1, "{inits:?}");
                inits[0].clone()
            }
        };
        let protected = protected.as_path();
        let inside = project.root().join("linked");
        fs::rename(protected, &inside).unwrap();
        symlink(&inside, protected).unwrap();
        // `config edit` first: the control creates a configuration.
        let commands = [&["config", "edit"][..], &["--", "true"]];
        for args in commands {
            // The config root's tree holds the link, not what it names.
            let trees = || {
                (
                    tree(env.state_home()),
                    tree(env.config_home()),
                    tree(&inside),
                )
            };
            let before = trees();
            let out = project.pinfold(args);
            // A setup failure exits 125; the spec names no code for `config
            // edit`.
            match args[0] {
                "--" => assert_eq!(out.code, 125, "{protected:?} linked in: {out:?}"),
                _ => assert_ne!(out.code, 0, "{protected:?} linked in: {out:?}"),
            }
            assert!(
                tokens(&out.stderr).any(|token| token == "host-path-in-project"),
                "{args:?}: {}",
                out.stderr
            );
            assert_eq!(trees(), before, "{args:?} with {protected:?} linked in");
        }
        fs::remove_file(protected).unwrap();
        fs::rename(&inside, protected).unwrap();
        for args in commands {
            let out = project.pinfold(args);
            assert_eq!(out.code, 0, "{args:?} with {protected:?} in place: {out:?}");
        }
        projects.push(project);
    }

    read_only_folder(&env, &projects[0]);

    #[cfg(target_os = "macos")]
    share_reaches_only_its_folder(&env, &projects[1]);
}

/// A read-only folder linked into the state root is refused before a box
/// exists. With a fixture folder in its place, the same project's box reads
/// it and cannot write it.
// Sabotage (not run): in config.rs, pass `false` instead of `true` in both
// read-only folder checks (`Config::check_folders`, `admit_folder`). The
// linked folder, which lies inside the state root and contains no
// protected path, then runs and exits 0. This block proves only that
// direction of the overlap check: a folder inside a protected path.
// Sabotage (not run): in `build_plan`, push each folder with
// `MountKind::Share` on macOS. The write then lands and changes the host
// file. Dropping the folder from the plan fails the read.
fn read_only_folder(env: &TestEnv, project: &Project) {
    // A configuration change applies only to a box not running.
    let stop = project.pinfold(&["stop"]);
    assert_eq!(stop.code, 0, "{stop:?}");
    // A guest path and content no image holds.
    let token = format!("{:016x}", RandomState::new().build_hasher().finish());
    let guest = format!("/folder-{token}");
    let content = format!("note {token}\n");
    let folder = env.root().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("note"), &content).unwrap();
    let file = project.config_dir().join("pinfold.toml");
    let kind = if cfg!(target_os = "macos") {
        "share"
    } else {
        "disk"
    };
    // The settings `config edit` and the profile case left, and one folder.
    let settings = |host: &Path| {
        fs::write(
            &file,
            format!(
                "profile = \"e2e-base\"\nmounts = [{{ host = \".\", kind = \"{kind}\" }}, {{ host = {:?}, guest = {guest:?} }}]\n",
                host.to_str().unwrap()
            ),
        )
        .unwrap();
    };

    // A link outside every protected path to a directory the earlier runs
    // made inside the state root: refused, with no state written.
    let inside = env.state_home().join("pinfold/projects");
    assert!(inside.is_dir(), "{inside:?}");
    let link = env.root().join("linked-state");
    symlink(&inside, &link).unwrap();
    settings(&link);
    let before = tree(env.state_home());
    let out = project.pinfold(&["--", "true"]);
    assert_eq!(out.code, 125, "{out:?}");
    assert!(
        tokens(&out.stderr).any(|token| token == "host-path-in-project"),
        "{}",
        out.stderr
    );
    assert_eq!(tree(env.state_home()), before);

    // Control: the fixture folder in its place. The read prints the file;
    // the write of bytes prints its errno, EROFS.
    settings(&folder);
    let out = project.pinfold(&[
        "--",
        "perl",
        "-e",
        "open(my $f, '<', \"$ARGV[0]/note\") or die \"read: $!\\n\"; print <$f>;
         if (open(my $w, '>>', \"$ARGV[0]/note\") and print {$w} \"box\\n\" and close($w)) { print \"written\\n\" } else { print 0 + $!, \"\\n\" }",
        &guest,
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("{content}30\n"));
    assert_eq!(fs::read_to_string(folder.join("note")).unwrap(), content);
}

/// In a running share, an outward symlink and paths outside it reach no
/// host file; shared edits and a box commit land on the host's branch.
#[cfg(target_os = "macos")]
fn share_reaches_only_its_folder(env: &TestEnv, project: &Project) {
    let outside = env.root().join("outside");
    fs::create_dir(&outside).unwrap();
    let secret = outside.join("secret");
    fs::write(&secret, "secret\n").unwrap();
    symlink(&secret, project.root().join("out")).unwrap();
    fs::write(project.root().join("inside"), "inside\n").unwrap();
    let git = |args: &[&str]| {
        let out = project.git(args);
        assert_eq!(out.code, 0, "git {args:?}: {out:?}");
        out.stdout.trim().to_string()
    };
    git(&["add", "inside"]);
    git(&["commit", "-q", "-m", "host"]);
    let host_commit = git(&["rev-parse", "HEAD"]);

    // Each path's open errno, 0 with the bytes it read, then a commit.
    let out = project.pinfold(&[
        "--",
        "sh",
        "-c",
        "readlink out
         perl -e 'for (@ARGV) { if (open(my $f, \"<\", $_)) { print \"0 \", <$f> } else { print 0 + $!, \"\\n\" } }' \
           out \"$1\" ../outside/secret inside
         echo edit > edit && git add edit && git commit -q -m box",
        "sh",
        secret.to_str().unwrap(),
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    // ENOENT for each way out; the share's own file reads.
    assert_eq!(
        out.stdout,
        format!("{}\n2\n2\n2\n0 inside\n", secret.display())
    );
    assert_eq!(
        fs::read_to_string(project.root().join("edit")).unwrap(),
        "edit\n"
    );
    assert_eq!(git(&["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(git(&["rev-parse", "HEAD^"]), host_commit);
    assert_eq!(git(&["show", "HEAD:edit"]), "edit");
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
}
