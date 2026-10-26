//! Run records: guarantees 38 and 41. Guarantee 39 needs a pi box and lives
//! in `pi.rs`; 40 and 42 need the runtime exclusively and live in
//! `runtime.rs`.

use std::process::Stdio;

use crate::box_::{Up, box_name, box_up};
use e2e::{
    HttpFixture, TestDir, TestEnv, assert_denied, assert_ok, curl, default_image, egress_log,
    pinfold, run_query,
};

/// `ready`'s run id, checked against the generation label it must equal.
pub(super) fn run_id(up: &Up) -> String {
    let ready = up.ready();
    let id = ready["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("ready names no run_id: {ready}"))
        .to_string();
    assert_eq!(
        ready["labels"]["dev.pinfold.generation"], id,
        "run_id is not the box's generation: {ready}"
    );
    id
}

fn events(env: &TestEnv, run_id: &str, after: Option<&str>) -> (i32, serde_json::Value) {
    let mut command = env.command(pinfold());
    command.args(["box", "events", run_id]);
    if let Some(after) = after {
        command.args(["--after", after]);
    }
    run_query(&mut command)
}

#[test]
fn each_runs_history_is_its_own() {
    let _runtime = crate::shared_runtime();
    // Guarantee 38. Sabotage: key the egress log by box name again; the
    // second run's log already holds the first's lines, and the first run's
    // events gain the second's request. Sabotage: emit a fresh random
    // `run_id` instead of the generation; the label equality fails.
    // Sabotage: print the first run's id in the second `down`; its run_id
    // assertion fails.
    let env = TestEnv::new("run-history");
    let name = box_name("run-history");
    let spec = serde_json::json!({
        "name": name,
        "image": default_image(&env),
        "egress": { "allow": ["api.github.com"] },
    });
    let mut first = box_up(&env, &spec, &name);
    let first_run = run_id(&first);
    let allowed = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&allowed, "the first run's allowlisted request");
    let denied = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://example.com/"],
    );
    assert_denied(&denied, "403", "the first run's request to example.com");
    let down = first.close_stdin();
    assert_eq!(
        down.last().expect("the first run's down line")["run_id"],
        first_run.as_str()
    );

    let mut second = box_up(&env, &spec, &name);
    let second_run = run_id(&second);
    assert_ne!(second_run, first_run, "a reused name reused its run_id");
    assert_eq!(
        egress_log(second.ready()),
        "",
        "the second run's log is not empty before its first request"
    );
    let again = curl(
        &env,
        &name,
        "30",
        &["-o", "/dev/null", "https://api.github.com/"],
    );
    assert_ok(&again, "the second run's request");

    let (code, page) = events(&env, &first_run, None);
    assert_eq!(code, 0, "events for the first run failed");
    let decisions: Vec<(&str, &str)> = page["events"]
        .as_array()
        .expect("events is an array")
        .iter()
        .map(|event| {
            (
                event["host"].as_str().unwrap_or_default(),
                event["decision"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        decisions,
        [("api.github.com", "allowed"), ("example.com", "refused")],
        "the first run's events: {page}"
    );
    assert_eq!(page["more"], false, "the first run's events: {page}");
    let down = second.close_stdin();
    assert_eq!(
        down.last().expect("the second run's down line")["run_id"],
        second_run.as_str()
    );
}

#[test]
fn run_queries_are_bounded_and_need_no_runtime() {
    let _runtime = crate::shared_runtime();
    // Guarantee 41. Sabotage: list the runtime's images in `inspect`; with
    // no `podman` or `container` on PATH it exits 1. (Listing boxes runs no
    // program on Linux, so it would pass there.)
    // Sabotage: resume after the cursor's line rather than at it; each
    // page boundary skips a line and the order assertion fails. Counting
    // an incomplete final line into `next` is not caught every run: the
    // proxy appends each line in one write, so a partial line is rare.
    // Sabotage: drop the run from the cursor; another run's cursor exits 0.
    // Sabotage: report a no-egress run's log `unavailable`; the
    // not-applicable assertions fail.
    let env = TestEnv::new("run-query");
    let image = default_image(&env);

    let quiet = box_name("run-query-quiet");
    let mut up = box_up(
        &env,
        &serde_json::json!({ "name": quiet, "image": image }),
        &quiet,
    );
    let quiet_run = run_id(&up);
    up.close_stdin();

    let fixture = HttpFixture::start(None);
    let name = box_name("run-query");
    let spec = serde_json::json!({
        "name": name,
        "image": image,
        "egress": { "routes": { "fixture.internal": fixture.route() } },
    });
    let mut up = box_up(&env, &spec, &name);
    let run = run_id(&up);
    let requests = |prefix: &str, count: usize| {
        format!(
            "i=1; while [ $i -le {count} ]; do curl -sS -o /dev/null http://fixture.internal/{prefix}/$i || exit 1; i=$((i+1)); done"
        )
    };
    let first = e2e::box_exec(&env, &name, &["sh", "-c", &requests("n", 150)]);
    assert_ok(&first, "the first 150 route requests");
    // The box keeps appending while the log is paged.
    let mut appender = env
        .command(pinfold())
        .args([
            "box",
            "exec",
            &name,
            "--",
            "sh",
            "-c",
            &requests("extra", 50),
        ])
        .stdin(Stdio::null())
        .spawn()
        .expect("spawn the appending exec");
    let mut paths = Vec::new();
    let mut cursor: Option<String> = None;
    let mut appended = false;
    loop {
        let (code, page) = events(&env, &run, cursor.as_deref());
        assert_eq!(code, 0, "events failed");
        assert_eq!(page["state"], "available", "{page}");
        let page_events = page["events"].as_array().expect("events is an array");
        for event in page_events {
            assert_eq!(event["reason"], "route", "{event}");
            paths.push(event["path"].as_str().unwrap_or_default().to_string());
        }
        if let Some(next) = page["next"].as_str() {
            cursor = Some(next.to_string());
        }
        if page["more"] == false {
            if appended {
                break;
            }
            assert!(appender.wait().unwrap().success(), "the appending exec");
            appended = true;
        }
    }
    // Reaped above; the status is cached.
    appender.wait().expect("reap the appending exec");
    let expected: Vec<String> = (1..=150)
        .map(|i| format!("/n/{i}"))
        .chain((1..=50).map(|i| format!("/extra/{i}")))
        .collect();
    assert_eq!(paths, expected, "paging skipped, repeated or reordered");
    let cursor = cursor.expect("paging returned a cursor");
    up.close_stdin();

    // With no runtime on PATH, both queries answer for finished runs.
    let empty = TestDir::new(&env, "empty-path");
    let offline = |args: &[&str]| {
        run_query(
            env.command(pinfold())
                .arg("box")
                .args(args)
                .env("PATH", empty.path()),
        )
    };
    let (code, record) = offline(&["inspect", &run]);
    assert_eq!(code, 0, "inspect with no runtime failed");
    assert_eq!(record["terminal"]["record"]["reason"], "stdin-closed");
    assert_eq!(record["egress_log"]["state"], "available", "{record}");
    let (code, page) = offline(&["events", &run]);
    assert_eq!(code, 0, "events with no runtime failed");
    assert_eq!(page["events"].as_array().map(Vec::len), Some(100));
    let (code, record) = offline(&["inspect", &quiet_run]);
    assert_eq!(code, 0, "inspect of the no-egress run failed");
    assert_eq!(record["egress_log"]["state"], "not-applicable", "{record}");
    let (code, page) = offline(&["events", &quiet_run]);
    assert_eq!(code, 0, "events of the no-egress run failed");
    assert_eq!(page["state"], "not-applicable", "{page}");

    // Malformed arguments exit 1 and an unknown run exits 3.
    let upper = run.to_ascii_uppercase();
    for (args, want) in [
        (vec!["inspect", upper.as_str()], 1),
        (vec!["events", run.as_str(), "--limit", "101"], 1),
        (
            vec!["events", quiet_run.as_str(), "--after", cursor.as_str()],
            1,
        ),
        (vec!["inspect", &"0".repeat(64)], 3),
        (vec!["events", &"0".repeat(64)], 3),
        (
            vec![
                "events",
                run.as_str(),
                "--limit",
                "100",
                "--after",
                cursor.as_str(),
            ],
            0,
        ),
    ] {
        let (code, _) = run_query(env.command(pinfold()).arg("box").args(&args));
        assert_eq!(code, want, "box {args:?}");
    }
}
