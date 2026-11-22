//! Guarantee 15: cleanup removes only pinfold's garbage.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use e2e::{
    COMMAND, Client, Exec, Project, TestEnv, detached_children, hold_maintenance, run, tree,
    wait_exited,
};
use serde_json::Value;

#[test]
fn cleanup_removes_only_pinfolds_garbage() {
    // Sabotage (run): in cli.rs, drop the `!live.contains(&project.id)`
    // test in `CleanPlan::idle` and the `locks.contains_key` skip in
    // `CleanPlan::remove`. The kept project's state directory is then gone.
    // Dropping only the live test passes: the owner's project lock still
    // keeps the project.
    // The idle project's state and disks going is the positive control.
    // Its own cache root, so its `clean` empties only its own builder
    // cache, and a small image of its own: the default image's tools are
    // not needed.
    let env = TestEnv::own_cache("clean");
    let image = small_image(&env);
    let live = Project::new(&env, "live");
    let idle = Project::new(&env, "idle");
    for project in [&live, &idle] {
        let file = project.config_dir().join("pinfold.toml");
        let saved = fs::read_to_string(&file).unwrap();
        fs::write(&file, format!("image = \"{image}\"\n{saved}")).unwrap();
    }

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
    // An idle project: one session, ended, and a host file in its
    // checkout. Its detached owner, the client's child while the session
    // runs, has exited before `clean`.
    let client =
        Client::spawn(idle.command(&["--", "sh", "-c", "echo started; read line; exit 0"]));
    assert_eq!(client.line(), "started");
    let owners = detached_children(client.id());
    assert_eq!(owners.len(), 1, "the idle client's children: {owners:?}");
    assert_eq!(client.finish(), 0, "the idle session's exit");
    wait_exited(owners[0]);
    fs::write(idle.root().join("host-file"), "host\n").unwrap();

    let (live, idle) = (Facts::of(&env, &live), Facts::of(&env, &idle));
    let idle_tree = tree(&idle.root);
    let idle_config = fs::read(&idle.config).unwrap();
    for facts in [&live, &idle] {
        for path in facts.state() {
            assert!(path.exists(), "{} before clean", path.display());
        }
    }
    let live_record = contents(&live.state_dir);

    // Dead boxes are every root's to prune; hold them for this pass.
    let hold = hold_maintenance();
    let cleaned = run(env.command().args(["clean", "--unused", "0s"]), COMMAND);
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
    assert_eq!(contents(&live.state_dir), live_record);
    let read = live
        .project
        .pinfold(&["--", "cat", "marker", "/tmp/e2e-box"]);
    assert_eq!(ok(read, "read back"), format!("kept\n{box_mark}"));
}

/// A caller image of the Debian base alone, built in `env`'s cache root.
fn small_image(env: &TestEnv) -> String {
    let dir = env.root().join("image");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("Containerfile"), "FROM debian:trixie-slim\n").unwrap();
    let built = run(
        env.command()
            .args(["image", "build", "e2e-clean", "--containerfile"])
            .arg(dir.join("Containerfile"))
            .arg("--context")
            .arg(&dir),
        Duration::from_secs(600),
    );
    assert_eq!(built.code, 0, "image build: {built:?}");
    "localhost/pinfold/image-e2e-clean:latest".to_string()
}

/// Every file under `dir` and its bytes.
fn contents(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    tree(dir)
        .into_iter()
        .filter(|(path, _, _)| fs::symlink_metadata(path).unwrap().is_file())
        .map(|(path, _, _)| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

/// One project's checkout, configuration file, state directory and disks.
struct Facts<'a> {
    project: &'a Project<'a>,
    root: PathBuf,
    config: PathBuf,
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
            state_dir: env.state_home().join("pinfold/projects").join(id),
            disks: vec![disk("home"), disk("project")],
        }
    }

    fn state(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.state_dir).chain(&self.disks)
    }
}

fn ok(out: Exec, what: &str) -> String {
    assert_eq!(out.code, 0, "{what}: {out:?}");
    out.stdout.trim().to_string()
}
