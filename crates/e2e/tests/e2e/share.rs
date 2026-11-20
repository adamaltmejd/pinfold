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
    // The same project running with each symlink removed is the positive
    // control.
    default_image();
    let env = TestEnv::new("protected");
    let project = Project::new(&env, "protected");
    // A user profile on default's image, which the project selects.
    let profile = env.config_home().join("pinfold/profiles/e2e-base");
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join("pinfold.toml"), "base = \"default\"\n").unwrap();
    let file = project.config_dir().join("pinfold.toml");
    let saved = fs::read_to_string(&file).unwrap();
    fs::write(&file, format!("profile = \"e2e-base\"\n{saved}")).unwrap();

    // Each protected directory in turn is moved into the checkout and
    // linked from where pinfold reads it. Both commands are refused and
    // change nothing under the state or config roots; with the link
    // replaced by the directory, the project runs.
    for protected in [project.config_dir(), profile.as_path()] {
        let inside = project.root().join("linked");
        fs::rename(protected, &inside).unwrap();
        symlink(&inside, protected).unwrap();
        for args in [&["--", "true"][..], &["config", "edit"]] {
            let before = (tree(env.state_home()), tree(env.config_home()));
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
            assert_eq!(
                (tree(env.state_home()), tree(env.config_home())),
                before,
                "{args:?} with {protected:?} linked in"
            );
        }
        fs::remove_file(protected).unwrap();
        fs::rename(&inside, protected).unwrap();
        let out = project.pinfold(&["--", "true"]);
        assert_eq!(out.code, 0, "{protected:?} in place: {out:?}");
    }

    #[cfg(target_os = "macos")]
    share_reaches_only_its_folder(&env, &project);
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
    assert_eq!(git(&["status", "--porcelain", "--", "edit"]), "");
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
}
