//! Guarantee 48: a share reaches only its folder by path, and a project
//! holding a protected path is refused.

use std::fs;
use std::os::unix::fs::symlink;

use e2e::{Project, TestEnv, default_image, tree};

#[test]
fn a_share_reaches_only_its_folder_and_a_protected_path_is_refused() {
    // Sabotage (run): delete `paths.extend(extra)` in core/mounts.rs's
    // `protected_paths`. With the configuration directory symlinked in,
    // `-- true` then runs and exits 0.
    // Sabotage (run): in `protected_paths`, drop the init's own directory
    // (keep `artifacts/init`). With `artifacts/init/<sha>` linked in, `--
    // true` is then refused `spec` after creating project state.
    // The same project running, and `config edit` succeeding, with each
    // symlink removed are the positive controls.
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
    // `artifacts/init`, which the first case's control installs.
    let mut cases = vec!["config", "profile"];
    if cfg!(target_os = "macos") {
        cases.push("init");
    }
    let mut projects: Vec<Project> = Vec::new();
    for case in cases {
        let project = Project::new(&env, &format!("protected-{case}"));
        let file = project.config_dir().join("pinfold.toml");
        let saved = fs::read_to_string(&file).unwrap();
        fs::write(&file, format!("profile = \"e2e-base\"\n{saved}")).unwrap();
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
        let commands = [&["--", "true"][..], &["config", "edit"]];
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

    #[cfg(target_os = "macos")]
    share_reaches_only_its_folder(&env, &projects[1]);
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
