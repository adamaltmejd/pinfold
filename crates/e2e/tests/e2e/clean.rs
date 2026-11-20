//! Guarantee 15: cleanup removes only pinfold's garbage.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use e2e::{Exec, Project, TestEnv, default_image, hold_maintenance, json_lines, run, tree};
use serde_json::Value;

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    // Sabotage (run): in cli.rs, drop the `!live.contains(&project.id)`
    // test in `CleanPlan::idle` and the `locks.contains_key` skip in
    // `CleanPlan::remove`. The kept project's state directory is then gone.
    // Dropping only the live test passes: the owner's project lock still
    // keeps the project.
    // The idle project's state and disks going is the positive control.
    default_image();
    let env = TestEnv::new("clean");
    let live = Project::new(&env, "live");
    let idle = Project::new(&env, "idle");

    // A kept project with a live box and no clients: its one session wrote
    // a file to its disk and one to the box's own tmpfs /tmp, which a
    // replacement box would not have, and ended.
    let box_mark = format!("box-{}", std::process::id());
    ok(live.pinfold(&["start"]), "start");
    ok(
        live.pinfold(&[
            "--",
            "sh",
            "-c",
            "echo kept > marker && echo \"$1\" > /tmp/e2e-box && sync",
            "sh",
            &box_mark,
        ]),
        "write",
    );
    // An idle project: one run, ended, and a host file in its checkout. Its
    // detached owner is gone before `clean`: the only processes left with
    // this test's state root are the ones the kept project had before.
    let kept = processes_of(env.state_home());
    assert!(!kept.is_empty(), "the kept project's owner");
    ok(idle.pinfold(&["--", "true"]), "idle run");
    let deadline = Instant::now() + Duration::from_secs(120);
    while !processes_of(env.state_home()).is_subset(&kept) {
        assert!(
            Instant::now() < deadline,
            "the idle project's owner outlived its run"
        );
        thread::sleep(Duration::from_millis(10));
    }
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
        Duration::from_secs(120),
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
    let read = live
        .project
        .pinfold(&["--", "cat", "marker", "/tmp/e2e-box"]);
    assert_eq!(ok(read, "read back"), format!("kept\n{box_mark}"));
}

/// The host processes whose environment sets `XDG_STATE_HOME` to `state`.
fn processes_of(state: &Path) -> BTreeSet<u32> {
    let wanted = format!("XDG_STATE_HOME={}", state.display()).into_bytes();
    let mut found = BTreeSet::new();
    for entry in fs::read_dir("/proc").expect("read /proc").flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        // A process that exited meanwhile, or a zombie, has no environment.
        let Ok(environ) = fs::read(entry.path().join("environ")) else {
            continue;
        };
        if environ.split(|byte| *byte == 0).any(|var| var == wanted) {
            found.insert(pid);
        }
    }
    found
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
                Duration::from_secs(120),
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
