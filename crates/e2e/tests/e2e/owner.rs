//! Guarantee 8: if the owner dies the box loses its way out, and the
//! leftover can be removed.

#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
use e2e::Exec;
use e2e::{
    COMMAND, Client, Leftover, Project, TestEnv, box_name, default_image, hold_maintenance,
    json_lines, run,
};
use serde_json::{Value, json};

#[test]
fn losing_the_owner_fails_closed() {
    // Sabotage (run): skip `clean::prune_boxes` in cli.rs's `prune`, or
    // drop `prune` from main.rs's list of commands that skip the daily
    // pass (#377). The prune then prints no `pruned` line for the
    // leftover. The other root seeing the owner alive before the kill, and
    // on Apple the same request before it, are the positive controls.
    let image = default_image();
    let env = TestEnv::new("owner");
    // A second state root, created before the hold below.
    let other = TestEnv::new("owner-other");
    let name = box_name("owner");
    let spec = json!({
        "name": name,
        "image": image,
        "egress": { "allow": ["api.github.com"] },
    });
    let up = env.up(&spec);

    // Control, on Apple: the request the orphan makes after the kill
    // succeeds before it.
    #[cfg(target_os = "macos")]
    {
        let control = request(&env, &name);
        assert_eq!(
            (control.code, control.stdout.as_str()),
            (0, format!("{}\n200", marker()).as_str()),
            "before the kill: {control:?}"
        );
    }

    // Control: before the kill the other root sees the owner alive.
    let run_id = up.ready()["run_id"].as_str().expect("run_id").to_string();
    let owner = json!(up.pid());
    let listed = other.listed(&run_id);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(
        (&listed[0]["owner"], &listed[0]["owner_alive"]),
        (&owner, &json!(true)),
        "{}",
        listed[0]
    );

    let hold = hold_maintenance();
    let leftover = up.kill_owner();

    // On Apple the guest survives its owner. A script still runs in it,
    // but the proxy is gone: the CONNECT gets no answer. The guest's relay
    // may accept the connection (curl 56) or refuse it (7); a timeout is
    // not a pass. On Linux the VM ended with its owner (`kill_owner`).
    #[cfg(target_os = "macos")]
    {
        let orphan = request(&env, &name);
        assert_eq!(
            orphan.stdout,
            format!("{}\n000", marker()),
            "after the kill: {orphan:?}"
        );
        assert!(
            matches!(orphan.code, 7 | 56),
            "curl after the kill: {orphan:?}"
        );
    }

    // Another state root sees the owner dead, and its prune removes the
    // leftover, naming the owner pid the test killed.
    let listed = other.listed(leftover.run_id());
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(
        (
            &listed[0]["name"],
            &listed[0]["owner"],
            &listed[0]["owner_alive"]
        ),
        (&json!(name), &json!(leftover.owner()), &json!(false)),
        "{}",
        listed[0]
    );
    // A state root whose daily pass is due, as when `box prune` is a
    // root's first command. Nothing else runs in it, so its pass never
    // runs outside the hold.
    let fresh = other.root().join("fresh-state");
    let pruned = run(
        other
            .command()
            .env("XDG_STATE_HOME", &fresh)
            .args(["box", "prune"]),
        COMMAND,
    );
    assert_eq!(pruned.code, 0, "{pruned:?}");
    assert!(
        json_lines(&pruned.stdout).contains(&json!({
            "event": "pruned",
            "box": name,
            "owner": leftover.owner(),
        })),
        "box prune: {pruned:?}"
    );
    drop(hold);
    #[cfg(target_os = "macos")]
    assert!(!apple_lists(&name), "the runtime still lists {name}");
    assert!(other.listed(leftover.run_id()).is_empty());
    drop(leftover);

    // The name is reusable.
    drop(env.up(&spec));

    project_client_exits_125(&env);
}

#[cfg(target_os = "macos")]
fn marker() -> String {
    format!("pf-marker-{}", std::process::id())
}

/// In box `name`, a marker, then a CONNECT to an allowlisted host: curl
/// prints the proxy's CONNECT status and exits with its own code.
#[cfg(target_os = "macos")]
fn request(env: &TestEnv, name: &str) -> Exec {
    let script = format!(
        "echo {}; curl -sS --max-time 20 -o /dev/null -w '%{{http_connect}}' https://api.github.com/",
        marker()
    );
    env.exec(name, &["sh", "-c", &script])
}

/// Whether Apple's runtime lists a container with id `name`.
#[cfg(target_os = "macos")]
fn apple_lists(name: &str) -> bool {
    let out = run(
        Command::new("container").args(["list", "--all", "--format", "json"]),
        COMMAND,
    );
    assert_eq!(out.code, 0, "container list: {out:?}");
    let listed: Value = serde_json::from_str(&out.stdout).expect("container list JSON");
    listed
        .as_array()
        .expect("a list")
        .iter()
        .any(|container| container["configuration"]["id"] == name)
}

/// SIGKILL of a detached project owner while a session runs: the client
/// exits 125.
fn project_client_exits_125(env: &TestEnv) {
    let project = Project::new(env, "project");
    let config = project.pinfold(&["config", "--json"]);
    assert_eq!(config.code, 0, "config --json: {config:?}");
    let config: Value = serde_json::from_str(&config.stdout).expect("config JSON");
    let id = config["project"]["id"].as_str().expect("project id");

    let client =
        Client::spawn(project.command(&["--", "sh", "-c", "echo started; exec sleep 600"]));
    assert_eq!(client.line(), "started", "the session");

    // The one box labeled with this project's id; its owner is detached.
    let label = format!("dev.pinfold.project={id}");
    let listed = run(
        env.command().args(["box", "list", "--label", &label]),
        COMMAND,
    );
    let listed = json_lines(&listed.stdout);
    assert_eq!(listed.len(), 1, "boxes of project {id}: {listed:?}");
    let leftover = Leftover::listed(env, &listed[0]);
    #[cfg(target_os = "linux")]
    let below = e2e::Below::of(leftover.owner());
    let kill = leftover.kill();
    assert_eq!(kill.code, 0, "kill the project owner: {kill:?}");

    // `finish` closes stdin, which `sleep` never reads.
    assert_eq!(client.finish(), 125, "the client after its owner's SIGKILL");
    // The client can exit before the VMM has; removing the leftover
    // before then fails on a still-running orphan VMM.
    #[cfg(target_os = "linux")]
    below.wait_gone();
    drop(leftover);
}
