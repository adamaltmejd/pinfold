//! Guarantee 50: a profile with a base applies the base beneath it and is
//! outdated when the base changes.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use e2e::{Exec, Project, TestEnv, run};
#[cfg(target_os = "macos")]
use serde_json::Value;

const BUILD: Duration = Duration::from_secs(900);

/// The chain's root `FROM`. On Linux a small public base, built in the
/// test's own cache root. On macOS images live in the host-wide store, and
/// a build on a public base re-pulls its shared tag; a pinfold-built base
/// is never pulled. So there the root builds on the default image, the one
/// every other Mac test already needs: this test causes no pull of its own.
fn root_base() -> &'static str {
    if cfg!(target_os = "macos") {
        e2e::default_image()
    } else {
        "debian:trixie-slim"
    }
}

#[test]
fn a_profile_with_a_base_applies_it_and_is_outdated_when_it_changes() {
    // Sabotage (run): in pi/launch.rs's freshness walk, give `None` where
    // a base is itself outdated (`builds on an outdated image`). After the
    // root changes, only the root names `image-outdated`.
    // The box from the top link with every link's file and seed, and the
    // root alone not outdated after its own rebuild, are the positive
    // controls.
    let env = TestEnv::own_cache("chain");
    let word = format!("{:08x}", RandomState::new().build_hasher().finish() as u32);
    let prefix = format!("e2e-{}-{word}", std::process::id());
    let [root, mid, top] = ["root", "mid", "top"].map(|link| format!("{prefix}-{link}"));
    #[cfg(target_os = "macos")]
    let _tags = Tags::claim(format!("localhost/pinfold/profile-{prefix}-"));
    let profiles = env.config_home().join("pinfold/profiles");
    let link = |name: &str, base: Option<&str>, recipe: String, seeds: &[(&str, &str)]| {
        let dir = profiles.join(name);
        fs::create_dir_all(dir.join("home")).unwrap();
        let config = base.map_or(String::new(), |base| format!("base = \"{base}\"\n"));
        fs::write(dir.join("pinfold.toml"), config).unwrap();
        fs::write(dir.join("Containerfile"), recipe).unwrap();
        for (path, text) in seeds {
            fs::write(dir.join("home").join(path), text).unwrap();
        }
    };
    let recipe = |from: &str, file: &str, text: &str| {
        format!("FROM {from}\nRUN echo {text} > /etc/e2e-{file}\n")
    };
    let above = |base: &str| format!("localhost/pinfold/profile-{base}:latest");
    // Each lower link seeds a path of its own, and one the top replaces.
    let root_link = |text: &str| {
        link(
            &root,
            None,
            recipe(root_base(), "root", text),
            &[("e2e-root-seed", "root-seed"), ("e2e-seed", "root")],
        );
    };
    root_link("root");
    link(
        &mid,
        Some(&root),
        recipe(&above(&root), "mid", "mid"),
        &[("e2e-mid-seed", "mid-seed")],
    );
    link(
        &top,
        Some(&mid),
        recipe(&above(&mid), "top", "top"),
        &[("e2e-seed", "top")],
    );
    let build = |name: &str| {
        let built = run(env.command().args(["build", "--profile", name]), BUILD);
        assert_eq!(built.code, 0, "build --profile {name}: {built:?}");
    };
    build(&top);

    // A project on the top link.
    let project = Project::new(&env, "chain");
    let file = project.config_dir().join("pinfold.toml");
    let saved = fs::read_to_string(&file).unwrap();
    fs::write(&file, format!("profile = \"{top}\"\n{saved}")).unwrap();
    let launch = || -> Exec {
        project.pinfold(&[
            "--",
            "sh",
            "-c",
            "cd \"$HOME\" && cat /etc/e2e-root /etc/e2e-mid /etc/e2e-top \
             e2e-root-seed e2e-mid-seed e2e-seed",
        ])
    };
    let outdated = |out: &Exec| -> Vec<String> {
        [&root, &mid, &top]
            .into_iter()
            .filter(|name| {
                let reference = above(name);
                out.stderr.lines().any(|line| {
                    tokens(line).any(|token| token == "image-outdated")
                        && tokens(line).any(|token| token == reference)
                })
            })
            .cloned()
            .collect()
    };
    let files = |root_text: &str| format!("{root_text}\nmid\ntop\nroot-seedmid-seedtop");

    // A change at the root names every image above it.
    root_link("root-2");
    let changed = launch();
    assert_eq!(changed.code, 0, "{changed:?}");
    assert_eq!(
        outdated(&changed),
        [root.clone(), mid.clone(), top.clone()],
        "{}",
        changed.stderr
    );

    // The root rebuilt alone is current; the two above it now record a
    // base that is not the root's image, and are named.
    build(&root);
    let root_only = launch();
    assert_eq!(root_only.code, 0, "{root_only:?}");
    assert_eq!(
        outdated(&root_only),
        [mid.clone(), top.clone()],
        "{}",
        root_only.stderr
    );

    // After the chain's rebuild, no image is named at all, and the top's
    // box has every link's file and seed, the new root's among them, and
    // the top's replacement.
    build(&top);
    let rebuilt = launch();
    assert_eq!(
        (rebuilt.code, rebuilt.stdout.clone()),
        (0, files("root-2")),
        "{rebuilt:?}"
    );
    assert!(
        !rebuilt
            .stderr
            .lines()
            .any(|line| tokens(line).any(|token| token == "image-outdated")),
        "{}",
        rebuilt.stderr
    );
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || ",;`'\"()".contains(c))
        .map(|token| token.trim_end_matches(['.', ':']))
}

/// On macOS, the test's own tags in the host-wide store: none may exist
/// before it builds, and every one is removed by name afterwards.
#[cfg(target_os = "macos")]
struct Tags(String);

#[cfg(target_os = "macos")]
impl Tags {
    fn claim(prefix: String) -> Tags {
        let taken = tagged(&prefix).unwrap_or_else(|error| panic!("{error}"));
        assert!(taken.is_empty(), "the store already holds {taken:?}");
        Tags(prefix)
    }
}

/// The store's references starting with `prefix`.
#[cfg(target_os = "macos")]
fn tagged(prefix: &str) -> Result<Vec<String>, String> {
    let listed = run(
        std::process::Command::new("container").args(["image", "list", "--format", "json"]),
        e2e::COMMAND,
    );
    // The listing holds other images' labels and environment: an error
    // names neither.
    if listed.code != 0 {
        return Err(format!("container image list exited {}", listed.code));
    }
    let images: Value = serde_json::from_str(&listed.stdout)
        .map_err(|error| format!("container image list: {error}"))?;
    let images = images
        .as_array()
        .ok_or("container image list: not an array")?;
    images
        .iter()
        .map(|image| {
            image["configuration"]["name"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "container image list: an image without a name".to_string())
        })
        .filter(|name| name.as_ref().map_or(true, |name| name.starts_with(prefix)))
        .collect()
}

#[cfg(target_os = "macos")]
impl Drop for Tags {
    fn drop(&mut self) {
        // On its own thread: a panic there never aborts an unwinding test.
        let prefix = self.0.clone();
        let removed = e2e::in_drop(move || {
            let mut left = Vec::new();
            for reference in tagged(&prefix)? {
                let deleted = run(
                    std::process::Command::new("container").args(["image", "delete", &reference]),
                    e2e::COMMAND,
                );
                if deleted.code != 0 {
                    left.push(format!("{reference}: {deleted:?}"));
                }
            }
            match left.is_empty() {
                true => Ok(()),
                false => Err(format!("{left:?}")),
            }
        });
        if let Err(error) = removed {
            eprintln!(
                "e2e: TAGS LEFT in the image store under {}: {error}",
                self.0
            );
            if !std::thread::panicking() {
                panic!("tags left in the image store (see stderr)");
            }
        }
    }
}
