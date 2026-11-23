//! Guarantee 47: a project's sessions share one box under its startup
//! configuration. One session ending does not end another, and a client
//! with different configuration is refused.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use e2e::{
    COMMAND, Client, Project, READER, TestEnv, default_image, first_on_path, hold_maintenance,
    json_lines, op_calls, run, spawn, stand_in_op, tree,
};
use serde_json::Value;

/// The caller variable the project's `env` reads with `from`.
const SOURCE: &str = "E2E_SESSION_SOURCE";
/// The 1Password reference the project's `env` reads with `op`, and the
/// value the stand-in `op` gives it.
const REFERENCE: &str = "op://e2e/sessions/one";
const OP_VALUE: &str = "e2esessionop";
/// The box's PATH: a home directory the box writes first, then the image's.
const BOX_PATH: &str =
    "/home/pinfold/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

#[test]
fn a_projects_sessions_share_one_box() {
    // Sabotage (run): in pi/session_snapshot.rs's `differences`, drop the
    // `env.values` comparison. The client holding another value then joins
    // and `true` exits 0, not 125.
    // Sabotage (run): in core/crossing.rs's `BoxChild::spawn`, set
    // `project_directory` to None; or in init/session.rs's `status`, drop
    // the SIGKILL sent once the departure deadline passes; or in
    // core/box.rs's `Signals::recv`, drop the `closed` branch. The box's hung
    // `git` then outlives the killed Git and `tail --pid` times out with
    // 124.
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
        format!(
            "mounts = [{{ host = \".\", kind = \"disk\" }}]\nenv = {{ PATH = \"{BOX_PATH}\" }}\n"
        ),
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

    // The startup configuration: one allowed host, a `from` variable and
    // an `op` reference. The two first launches have a stand-in `op`
    // first on PATH that gives the value; every later client has one
    // that refuses every read.
    let file = project.config_dir().join("pinfold.toml");
    let saved = fs::read_to_string(&file).unwrap();
    let configure = |allow: &str, reference: &str| {
        fs::write(
            &file,
            format!(
                "{saved}allow = [\"{allow}\"]\nenv = {{ E2E_SESSION = {{ from = \"{SOURCE}\" }}, \
                 E2E_SESSION_OP = {{ op = \"{reference}\" }} }}\n"
            ),
        )
        .unwrap();
    };
    configure("api.github.com", REFERENCE);
    let op = stand_in_op(&env, "op", &[(REFERENCE, OP_VALUE)], "e2edecoy");
    let refusing = stand_in_op(&env, "op-refusing", &[], "e2edecoy");
    let session = |value: &str, args: &[&str], op: &Path| -> Command {
        let mut command = project.command(args);
        command.env(SOURCE, value);
        first_on_path(&mut command, op);
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
        &op,
    ));
    let mut b = Client::spawn(session(
        "one",
        &["--", "sh", "-c", &format!("echo $$ > /tmp/e2e-b; {READER}")],
        &op,
    ));
    assert_eq!(a.line(), "started");
    assert_eq!(b.line(), "started");
    // Sabotage (run): in pi/session.rs, read `op` in `resolve_spec`,
    // which a join runs too. The first launch that joins then asks `op` as well,
    // and the stand-in logs more than one call.
    // The one that started the box asked `op` once; the other joined
    // without it. The value is the box's.
    assert_eq!(op_calls(&op), [REFERENCE], "op calls");
    assert_eq!(b.ask("printenv E2E_SESSION_OP"), [OP_VALUE]);

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
    let joined = run(&mut session("one", &both, &refusing), COMMAND);
    assert_eq!(joined.code, 0, "{joined:?}");
    assert_eq!(joined.stdout.lines().count(), 2, "{joined:?}");

    // The same command with another value is refused, naming the differing
    // field.
    let other = run(&mut session("two", &both, &refusing), COMMAND);
    assert_eq!(other.code, 125, "{other:?}");
    let named: Vec<&str> = tokens(&other.stderr).collect();
    assert!(
        named.contains(&"config-changed") && named.contains(&"env.values"),
        "{}",
        other.stderr
    );

    // Sabotage (run): in pi/session_snapshot.rs's `differences`, compare
    // `env.names` with each `op` source's reference blanked. The client
    // naming another reference then joins and exits 0, not 125.
    // An `op` source compares by its reference.
    configure("api.github.com", "op://e2e/sessions/two");
    let changed = run(&mut session("one", &both, &refusing), COMMAND);
    configure("api.github.com", REFERENCE);
    assert_eq!(changed.code, 125, "{changed:?}");
    let named: Vec<&str> = tokens(&changed.stderr).collect();
    assert!(
        named.contains(&"config-changed") && named.contains(&"env.names"),
        "{}",
        changed.stderr
    );

    // Exit codes pass through; one pinfold uses for itself (125, a setup
    // failure: here the executable lookup) is distinct from the command's.
    for code in [23, 126, 127] {
        let out = run(
            &mut session(
                "one",
                &["--", "sh", "-c", &format!("exit {code}")],
                &refusing,
            ),
            COMMAND,
        );
        assert_eq!(out.code, code, "{out:?}");
    }
    let missing = run(
        &mut session("one", &["--", "e2e-no-such-command"], &refusing),
        COMMAND,
    );
    assert_eq!(missing.code, 125, "{missing:?}");
    // No join asked `op`.
    assert_eq!(
        op_calls(&refusing),
        Vec::<String>::new(),
        "op calls on joins"
    );

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
    configure("example.com", REFERENCE);
    let log = env
        .state_home()
        .join("pinfold/runs")
        .join(&generation)
        .join("egress.jsonl");
    let before = json_lines(&fs::read_to_string(&log).unwrap()).len();
    for (host, connect) in [("example.com", "403 56"), ("api.github.com", "200 0")] {
        let out = b.ask(&format!(
            "curl -sS --max-time 20 -o /dev/null -w '%{{http_connect}}' https://{host}/; echo \" $?\""
        ));
        assert_eq!(out, [connect], "{host}");
    }
    let lines = json_lines(&fs::read_to_string(&log).unwrap())[before..].to_vec();
    assert!(
        lines.iter().any(|line| line["host"] == "example.com"
            && line["decision"] == "refused"
            && line["reason"] == "not allowlisted"),
        "{lines:?}"
    );

    assert_eq!(b.finish(), 0, "B after its stdin closed");

    // A crossing's box command ends when Git does, even one that ignores
    // TERM. In the disk project's box, kept by session C, the box's `git`
    // reports its pid through a FIFO and hangs in the fetch's first git
    // call. It is alive before SIGKILL reaches `git fetch box` alone, not
    // the remote helper, and gone after: the helper sees Git's pipe close.
    let mut c = Client::spawn(disk.command(&["--", "sh", "-c", READER]));
    assert_eq!(c.line(), "started");
    c.ask(
        "mkdir -p /home/pinfold/bin && mkfifo /tmp/e2e-git && \
         printf '#!/bin/sh\\ntrap \"\" TERM\\necho $$ > /tmp/e2e-git\\nexec sleep 600\\n' \
         > /home/pinfold/bin/git && chmod 755 /home/pinfold/bin/git",
    );
    let mut fetch = disk.git_command(&["fetch", "box"]);
    let mut fetch = spawn(
        fetch
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
    let hung = c.ask("timeout 60 cat /tmp/e2e-git");
    let [hung] = <[String; 1]>::try_from(hung).expect("the box's git pid");
    assert_eq!(c.ask(&format!("kill -0 {hung}; echo $?")), ["0"]);
    fetch.kill().expect("SIGKILL git fetch");
    fetch.wait().expect("reap git fetch");
    assert_eq!(
        c.ask(&format!(
            "timeout 60 tail --pid={hung} -f /dev/null; echo $?"
        )),
        ["0"]
    );
    assert_eq!(c.finish(), 0, "C after its stdin closed");
}

/// The words of `text`, a word being letters, digits, `-`, `.` and `_`,
/// without a sentence's final period.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || "-._".contains(c)))
        .map(|token| token.trim_end_matches('.'))
}
