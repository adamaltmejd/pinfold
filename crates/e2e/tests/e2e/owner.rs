//! Guarantee 8: if the owner dies the box loses its way out, and the
//! leftover can be removed.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use e2e::{
    Exec, Leftover, TestEnv, box_name, default_image, exit_code, hold_maintenance, json_lines, run,
    spawn, wait_until,
};
use serde_json::{Value, json};

const COMMAND: Duration = Duration::from_secs(120);

#[test]
fn losing_the_owner_fails_closed() {
    // Sabotage (run): skip `clean::prune_boxes` in cli.rs's `prune`, or
    // drop `prune` from main.rs's list of commands that skip the daily
    // pass (#377). The prune then prints no `pruned` line for the
    // leftover. The same request before the kill is the positive control.
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

    // A marker, then a CONNECT to an allowlisted host: curl prints the
    // proxy's CONNECT status and exits with its own code.
    let marker = format!("pf-marker-{}", std::process::id());
    let script = format!(
        "echo {marker}; curl -sS --max-time 20 -o /dev/null -w '%{{http_connect}}' https://api.github.com/"
    );
    let request = || -> Exec { env.exec(&name, &["sh", "-c", &script]) };
    let control = request();
    assert_eq!(
        (control.code, control.stdout.as_str()),
        (0, format!("{marker}\n200").as_str()),
        "before the kill: {control:?}"
    );

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
        let orphan = request();
        assert_eq!(
            orphan.stdout,
            format!("{marker}\n000"),
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
    let again = env.up(&spec);
    let again_id = again.ready()["run_id"].as_str().expect("run_id");
    assert_ne!(again_id, run_id);
    drop(again);

    project_client_exits_125(&env);
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
    let project = env.root().join("project");
    std::fs::create_dir(&project).unwrap();
    // Linux takes only Git projects; a Mac share takes one as well.
    let git = run(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project),
        COMMAND,
    );
    assert_eq!(git.code, 0, "git init: {git:?}");
    let in_project = |args: &[&str]| {
        let mut command = env.command();
        command
            .current_dir(&project)
            .env("VISUAL", "true")
            .args(args);
        command
    };
    let edit = run(&mut in_project(&["config", "edit"]), COMMAND);
    assert_eq!(edit.code, 0, "config edit: {edit:?}");
    let config = run(&mut in_project(&["config", "--json"]), COMMAND);
    assert_eq!(config.code, 0, "config --json: {config:?}");
    let config: Value = serde_json::from_str(&config.stdout).expect("config JSON");
    let id = config["project"]["id"].as_str().expect("project id");

    let mut client = Client(spawn(
        in_project(&["--", "sh", "-c", "echo started; exec sleep 600"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()),
    ));
    let stdout = client.0.stdout.take().unwrap();
    let (send, lines) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = send.send(line);
        }
    });
    let started = lines.recv_timeout(COMMAND);
    assert_eq!(started.as_deref(), Ok("started"), "the session");

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
    let kill = run(
        Command::new("kill").args(["-KILL", &leftover.owner().to_string()]),
        COMMAND,
    );
    assert_eq!(kill.code, 0, "kill the project owner: {kill:?}");

    let status = wait_until(&mut client.0, Instant::now() + COMMAND)
        .expect("the client exits after its owner's SIGKILL");
    assert_eq!(
        exit_code(status),
        125,
        "the client after its owner's SIGKILL"
    );
    #[cfg(target_os = "linux")]
    below.wait_gone();
    drop(leftover);
}

/// A project client, killed on drop: its departure ends its session, and
/// with no other session the owner tears the box down.
struct Client(std::process::Child);

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
