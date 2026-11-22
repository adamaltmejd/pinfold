//! Guarantee 47: a project's sessions share one box under its startup
//! configuration. One session ending does not end another, and a client
//! with different configuration is refused.

use std::fs;
use std::process::Command;
use std::time::Duration;

use e2e::{
    Client, Project, READER, TestEnv, default_image, hold_maintenance, json_lines, run, tree,
};
use serde_json::Value;

const COMMAND: Duration = Duration::from_secs(120);
/// The caller variable the project's `env` reads with `from`.
const SOURCE: &str = "E2E_SESSION_SOURCE";

#[test]
fn a_projects_sessions_share_one_box() {
    // Sabotage (run): in pi/session_snapshot.rs's `differences`, drop the
    // `env.values` comparison. The client holding another value then joins
    // and `true` exits 0, not 125.
    // The same value joining, and the refused invocation running after
    // `config edit`, are the positive controls.
    default_image();
    let env = TestEnv::new("sessions");
    let mut project = Project::unconfigured(&env, "sessions");

    // With no configuration and no terminal, `-- CMD` is refused and
    // creates nothing under the state or config roots.
    let argv = ["--", "printf", "[%s]", "", "--help", "a b"];
    let roots = || (tree(env.state_home()), tree(env.config_home()));
    let before = roots();
    let refused = project.pinfold(&argv);
    assert_eq!(refused.code, 125, "{refused:?}");
    assert!(
        tokens(&refused.stderr).any(|token| token == "no-config"),
        "{}",
        refused.stderr
    );
    assert_eq!(roots(), before, "the refused launch changed the roots");

    // Sabotage (run): in pi/session.rs, call `clean::maintain()` at the
    // top of `borrow` again. The refused fetch then writes the daily
    // pass's stamp into the fresh state root.
    // The Git remote is refused `no-config` too, before automatic
    // maintenance: from a state root no command has used, nothing appears
    // in it or the config root. Control: a configured disk project's fetch
    // from that root passes admission and runs the pass, then fails for
    // want of a project disk; a fetch that succeeds needs a disk, and any
    // command that makes one spends the pass first. Held, since a pass
    // prunes host-wide.
    let fresh = env.root().join("fresh-state");
    fs::create_dir(&fresh).unwrap();
    let fetch_fresh = |project: &Project| {
        let added = project.git(&[
            "remote",
            "add",
            "box",
            &format!("pinfold::{}", project.root().display()),
        ]);
        assert_eq!(added.code, 0, "{added:?}");
        let mut fetch = project.git_command(&["fetch", "box"]);
        fetch.env("XDG_STATE_HOME", &fresh);
        run(&mut fetch, COMMAND)
    };
    let disk = Project::new(&env, "sessions-disk");
    fs::write(
        disk.config_dir().join("pinfold.toml"),
        "mounts = [{ host = \".\", kind = \"disk\" }]\n",
    )
    .unwrap();
    {
        let _hold = hold_maintenance();
        let config = tree(env.config_home());
        let fetched = fetch_fresh(&project);
        assert_eq!(fetched.code, 128, "{fetched:?}");
        assert!(
            tokens(&fetched.stderr).any(|token| token == "no-config"),
            "{}",
            fetched.stderr
        );
        assert_eq!(tree(&fresh), Vec::new(), "the refused fetch wrote state");
        assert_eq!(tree(env.config_home()), config);
        let admitted = fetch_fresh(&disk);
        assert_eq!(admitted.code, 128, "{admitted:?}");
        assert!(
            !tokens(&admitted.stderr).any(|token| token == "no-config"),
            "{}",
            admitted.stderr
        );
        assert_ne!(tree(&fresh), Vec::new(), "the admitted fetch ran no pass");
    }

    // Control: after `config edit` the same invocation runs, with its
    // arguments literal, the empty one and `--help` included.
    project.configure();
    let ran = project.pinfold(&argv);
    assert_eq!(
        (ran.code, ran.stdout.as_str()),
        (0, "[][--help][a b]"),
        "{ran:?}"
    );

    // The startup configuration: one allowed host and a `from` variable.
    let file = project.config_dir().join("pinfold.toml");
    let saved = fs::read_to_string(&file).unwrap();
    let configure = |allow: &str| {
        fs::write(
            &file,
            format!(
                "{saved}allow = [\"{allow}\"]\nenv = {{ E2E_SESSION = {{ from = \"{SOURCE}\" }} }}\n"
            ),
        )
        .unwrap();
    };
    configure("api.github.com");
    let session = |value: &str, args: &[&str]| -> Command {
        let mut command = project.command(args);
        command.env(SOURCE, value);
        command
    };

    // Two simultaneous first launches. A holds a background child in its
    // command group; B runs each line it reads and then prints `done`.
    let a = Client::spawn(session(
        "one",
        &[
            "--",
            "sh",
            "-c",
            "sleep 600 & echo $! > /tmp/e2e-a-child; echo $$ > /tmp/e2e-a; echo started; wait",
        ],
    ));
    let mut b = Client::spawn(session(
        "one",
        &["--", "sh", "-c", &format!("echo $$ > /tmp/e2e-b; {READER}")],
    ));
    assert_eq!(a.line(), "started");
    assert_eq!(b.line(), "started");

    // One box serves the project: one owner, one generation.
    let id = project
        .config_dir()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the project id")
        .to_string();
    let label = format!("dev.pinfold.project={id}");
    let boxes = || -> Vec<Value> {
        let out = run(
            env.command().args(["box", "list", "--label", &label]),
            COMMAND,
        );
        assert_eq!(out.code, 0, "box list: {out:?}");
        json_lines(&out.stdout)
    };
    let listed = boxes();
    assert_eq!(listed.len(), 1, "boxes of project {id}: {listed:?}");
    let generation = listed[0]["labels"]["dev.pinfold.generation"]
        .as_str()
        .expect("a generation")
        .to_string();
    let owner = listed[0]["owner"].clone();
    assert!(
        owner.as_u64().is_some_and(|pid| pid > 0),
        "an owner pid: {}",
        listed[0]
    );

    // Control: a client whose `from` variable holds the same value joins,
    // and sees both sessions' files in that box's own /tmp.
    let both = ["--", "cat", "/tmp/e2e-a", "/tmp/e2e-b"];
    let joined = run(&mut session("one", &both), COMMAND);
    assert_eq!(joined.code, 0, "{joined:?}");
    assert_eq!(joined.stdout.lines().count(), 2, "{joined:?}");

    // The same command with another value is refused, naming the differing
    // field.
    let other = run(&mut session("two", &both), COMMAND);
    assert_eq!(other.code, 125, "{other:?}");
    let named: Vec<&str> = tokens(&other.stderr).collect();
    assert!(
        named.contains(&"config-changed") && named.contains(&"env.values"),
        "{}",
        other.stderr
    );

    // Exit codes pass through; one pinfold uses for itself (125, a setup
    // failure: here the executable lookup) is distinct from the command's.
    for code in [23, 126, 127] {
        let out = run(
            &mut session("one", &["--", "sh", "-c", &format!("exit {code}")]),
            COMMAND,
        );
        assert_eq!(out.code, code, "{out:?}");
    }
    let missing = run(&mut session("one", &["--", "e2e-no-such-command"]), COMMAND);
    assert_eq!(missing.code, 125, "{missing:?}");

    // Killing A's client ends A's command group, background child
    // included, while B keeps working in the same box. Both are alive
    // before.
    assert_eq!(
        b.ask("kill -0 \"$(cat /tmp/e2e-a)\" \"$(cat /tmp/e2e-a-child)\"; echo $?"),
        ["0"]
    );
    a.kill();
    assert_eq!(
        b.ask(
            "timeout 60 tail --pid=\"$(cat /tmp/e2e-a)\" -f /dev/null && \
             timeout 60 tail --pid=\"$(cat /tmp/e2e-a-child)\" -f /dev/null; echo $?"
        ),
        ["0"]
    );
    let listed = boxes();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(
        (
            &listed[0]["labels"]["dev.pinfold.generation"],
            &listed[0]["owner"]
        ),
        (&Value::from(generation.as_str()), &owner),
        "{}",
        listed[0]
    );

    // After the file's allowlist changes, the started one still decides:
    // the newly listed host is refused and the started one goes through.
    configure("example.com");
    let log = env
        .state_home()
        .join("pinfold/runs")
        .join(&generation)
        .join("egress.jsonl");
    for (host, connect, decision, reason) in [
        ("example.com", "403 56", "refused", "not allowlisted"),
        ("api.github.com", "200 0", "allowed", ""),
    ] {
        let before = json_lines(&fs::read_to_string(&log).unwrap()).len();
        let out = b.ask(&format!(
            "curl -sS --max-time 20 -o /dev/null -w '%{{http_connect}}' https://{host}/; echo \" $?\""
        ));
        assert_eq!(out, [connect], "{host}");
        let lines = json_lines(&fs::read_to_string(&log).unwrap())[before..].to_vec();
        assert_eq!(lines.len(), 1, "{host}: {lines:?}");
        assert_eq!(
            (&lines[0]["host"], &lines[0]["decision"]),
            (&Value::from(host), &Value::from(decision)),
            "{}",
            lines[0]
        );
        if decision == "refused" {
            assert_eq!(lines[0]["reason"], reason, "{}", lines[0]);
        }
    }

    assert_eq!(b.finish(), 0, "B after its stdin closed");
}

/// The words of `text`, a word being letters, digits, `-`, `.` and `_`,
/// without a sentence's final period.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || "-._".contains(c)))
        .map(|token| token.trim_end_matches('.'))
}
