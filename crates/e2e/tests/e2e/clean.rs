//! Guarantee 15: cleanup removes only pinfold's garbage.

use std::fs;
use std::path::PathBuf;

use e2e::{Exec, Project, TestEnv, default_image, hold_maintenance, json_lines, run, tree};
use serde_json::Value;

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    // Sabotage (run): in cli.rs's `CleanPlan::idle`, drop the
    // `!live.contains(&project.id)` test. `clean` then tries to remove the
    // kept project's attached disk and exits 1, and its state is gone.
    // The idle project's state and disks going is the positive control.
    default_image();
    let env = TestEnv::new("clean");
    let live = Project::new(&env, "live");
    let idle = Project::new(&env, "idle");

    // A kept project with a live box and no clients: its one session wrote
    // a file and ended.
    ok(live.pinfold(&["start"]), "start");
    ok(
        live.pinfold(&["--", "sh", "-c", "echo kept > marker && sync"]),
        "write",
    );
    // An idle project: one run, ended, and a host file in its checkout.
    ok(idle.pinfold(&["--", "true"]), "idle run");
    fs::write(idle.root().join("host-file"), "host\n").unwrap();

    let (live, idle) = (Facts::of(&env, &live), Facts::of(&env, &idle));
    let idle_tree = tree(&idle.root);
    let idle_config = fs::read(&idle.config).unwrap();
    for facts in [&live, &idle] {
        for path in facts.state() {
            assert!(path.exists(), "{} before clean", path.display());
        }
    }
    let generation = live.generation(&env);
    assert!(generation.is_some(), "the kept project has a box");

    // Dead boxes are every root's to prune; hold them for this pass.
    let hold = hold_maintenance();
    let cleaned = run(
        env.command().args(["clean", "--unused", "0s"]),
        std::time::Duration::from_secs(120),
    );
    drop(hold);
    assert_eq!(cleaned.code, 0, "{cleaned:?}");

    for path in idle.state() {
        assert!(!path.exists(), "{} after clean", path.display());
    }
    assert_eq!(tree(&idle.root), idle_tree);
    assert_eq!(fs::read(&idle.config).unwrap(), idle_config);
    for path in live.state() {
        assert!(path.exists(), "{} after clean", path.display());
    }
    assert_eq!(live.generation(&env), generation);
    let read = live.project.pinfold(&["--", "cat", "marker"]);
    assert_eq!(ok(read, "read back"), "kept");
}

/// One project's checkout, configuration file, state directory and disks.
struct Facts<'a> {
    project: &'a Project<'a>,
    root: PathBuf,
    config: PathBuf,
    id: String,
    state_dir: PathBuf,
    disks: Vec<PathBuf>,
}

impl<'a> Facts<'a> {
    fn of(env: &TestEnv, project: &'a Project<'a>) -> Facts<'a> {
        let config: Value =
            serde_json::from_str(&ok(project.pinfold(&["config", "--json"]), "config --json"))
                .expect("config JSON");
        let id = config["project"]["id"].as_str().expect("project id");
        let disk = |kind: &str| {
            let name = config["project"]["disks"][kind]
                .as_str()
                .expect("disk name");
            // Disks: a Linux disk is `<state>/pinfold/disks/<name>.ext4`.
            env.state_home().join(format!("pinfold/disks/{name}.ext4"))
        };
        Facts {
            project,
            root: project.root().to_path_buf(),
            config: project.config_dir().join("pinfold.toml"),
            id: id.to_string(),
            state_dir: env.state_home().join("pinfold/projects").join(id),
            disks: vec![disk("home"), disk("project")],
        }
    }

    fn state(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.state_dir).chain(&self.disks)
    }

    /// The generation of the project's box, if one is listed.
    fn generation(&self, env: &TestEnv) -> Option<String> {
        let label = format!("dev.pinfold.project={}", self.id);
        let listed = ok(
            run(
                env.command().args(["box", "list", "--label", &label]),
                std::time::Duration::from_secs(120),
            ),
            "box list",
        );
        let listed = json_lines(&listed);
        assert!(listed.len() <= 1, "{listed:?}");
        listed.first().map(|line| {
            line["labels"]["dev.pinfold.generation"]
                .as_str()
                .expect("generation")
                .to_string()
        })
    }
}

fn ok(out: Exec, what: &str) -> String {
    assert_eq!(out.code, 0, "{what}: {out:?}");
    out.stdout.trim().to_string()
}
