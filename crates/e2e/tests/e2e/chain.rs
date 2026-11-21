//! Guarantee 50: a profile with a base applies the base beneath it and is
//! outdated when the base changes.

use std::fs;
use std::thread;
use std::time::Duration;

use e2e::{BoxUp, Exec, Project, TestEnv, box_name, run};
use serde_json::{Value, json};

const BUILD: Duration = Duration::from_secs(900);

/// The chain's root `FROM`. On Linux a small public base, built in the
/// test's own cache root. On macOS images live in the host-wide store, and
/// building on a public base re-pulls its shared tag; a pinfold-built base
/// is never pulled, so the root builds on the default image.
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
    // The box from the top link with every link's file and the top's seed,
    // and the corrected cycle coming ready, are the positive controls.
    let env = TestEnv::own_cache("chain");
    let pid = std::process::id();
    let [root, mid, top] = ["root", "mid", "top"].map(|link| format!("e2e-{pid}-{link}"));
    #[cfg(target_os = "macos")]
    let _tags = Tags(format!("localhost/pinfold/profile-e2e-{pid}-"));
    let profiles = env.config_home().join("pinfold/profiles");
    let link = |name: &str, base: Option<&str>, recipe: String, seed: Option<&str>| {
        let dir = profiles.join(name);
        fs::create_dir_all(dir.join("home")).unwrap();
        let config = base.map_or(String::new(), |base| format!("base = \"{base}\"\n"));
        fs::write(dir.join("pinfold.toml"), config).unwrap();
        fs::write(dir.join("Containerfile"), recipe).unwrap();
        if let Some(seed) = seed {
            fs::write(dir.join("home/e2e-seed"), seed).unwrap();
        }
    };
    let recipe = |from: &str, file: &str, text: &str| {
        format!("FROM {from}\nRUN echo {text} > /etc/e2e-{file}\n")
    };
    let above = |base: &str| format!("localhost/pinfold/profile-{base}:latest");
    let root_recipe = |text: &str| recipe(root_base(), "root", text);

    // A cycle: the root names the top as its base.
    link(&root, Some(&top), root_recipe("root"), Some("root"));
    link(&mid, Some(&root), recipe(&above(&root), "mid", "mid"), None);
    link(
        &top,
        Some(&mid),
        recipe(&above(&mid), "top", "top"),
        Some("top"),
    );
    // Seeds need `$HOME` on a disk.
    let spec = json!({
        "name": box_name("chain"),
        "profile": top,
        "env": { "HOME": "/home/e2e" },
        "disks": [{ "name": format!("e2e-{pid}-chain"), "guest": "/home/e2e", "size": "1G" }],
    });
    let refused = match env.start(&spec) {
        Ok(up) => panic!("a cycle came ready: {}", up.ready()),
        Err(line) => line,
    };
    assert_eq!(
        (&refused["event"], &refused["reason"]),
        (&json!("refused"), &json!("profile")),
        "{refused}"
    );

    // Corrected, the chain builds root first and a box of the top runs.
    link(&root, None, root_recipe("root"), Some("root"));
    let build = || {
        let built = run(env.command().args(["build", "--profile", &top]), BUILD);
        assert_eq!(built.code, 0, "build --profile {top}: {built:?}");
    };
    build();
    drop(env.up(&spec));

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
            "cat /etc/e2e-root /etc/e2e-mid /etc/e2e-top \"$HOME/e2e-seed\"",
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

    // Control: every link's file and the top's replacement seed.
    let first = launch();
    assert_eq!(
        (first.code, first.stdout.as_str(), outdated(&first)),
        (0, "root\nmid\ntop\ntop", vec![]),
        "{first:?}"
    );

    // A change at the root names every image above it.
    link(&root, None, root_recipe("root-2"), Some("root"));
    let changed = launch();
    assert_eq!(changed.code, 0, "{changed:?}");
    assert_eq!(
        outdated(&changed),
        [root.clone(), mid.clone(), top.clone()],
        "{}",
        changed.stderr
    );

    // After the rebuild, none, and the top's box has the new root.
    build();
    let rebuilt = launch();
    assert_eq!(
        (rebuilt.code, rebuilt.stdout.as_str(), outdated(&rebuilt)),
        (0, "root-2\nmid\ntop\ntop", vec![]),
        "{rebuilt:?}"
    );

    // A second, unchanged rebuild keeps every image identity.
    let before = identities(&env, [&root, &mid, &top]);
    build();
    assert_eq!(identities(&env, [&root, &mid, &top]), before);
}

/// Each link's image id, from the `ready` line of a box of its latest.
fn identities(env: &TestEnv, links: [&String; 3]) -> Vec<Value> {
    let ups: Vec<BoxUp> = thread::scope(|scope| {
        let started: Vec<_> = links
            .iter()
            .map(|link| {
                scope.spawn(move || {
                    env.up(&json!({
                        "name": box_name(&format!("chain-{}", link.rsplit('-').next().unwrap())),
                        "image": format!("localhost/pinfold/profile-{link}:latest"),
                    }))
                })
            })
            .collect();
        started.into_iter().map(|up| up.join().unwrap()).collect()
    });
    let ids: Vec<Value> = ups
        .iter()
        .map(|up| up.ready()["image"]["id"].clone())
        .collect();
    for (index, id) in ids.iter().enumerate() {
        assert!(id.as_str().is_some_and(|id| !id.is_empty()), "{id}");
        assert!(!ids[..index].contains(id), "two links share {id}");
    }
    ids
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || ",;`'\"()".contains(c))
        .map(|token| token.trim_end_matches(['.', ':']))
}

/// On macOS, the test's own tags in the host-wide store, removed by name.
#[cfg(target_os = "macos")]
struct Tags(String);

#[cfg(target_os = "macos")]
impl Drop for Tags {
    fn drop(&mut self) {
        let listed = run(
            std::process::Command::new("container").args(["image", "list", "--format", "json"]),
            Duration::from_secs(120),
        );
        let listed: Value = serde_json::from_str(&listed.stdout).unwrap_or_default();
        for image in listed.as_array().into_iter().flatten() {
            let Some(reference) = image["configuration"]["name"].as_str() else {
                continue;
            };
            if reference.starts_with(&self.0) {
                let deleted = run(
                    std::process::Command::new("container").args(["image", "delete", reference]),
                    Duration::from_secs(120),
                );
                if deleted.code != 0 {
                    eprintln!("e2e: TAG LEFT {reference}: {deleted:?}");
                }
            }
        }
    }
}
