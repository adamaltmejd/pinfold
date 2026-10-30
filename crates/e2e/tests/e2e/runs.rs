//! Run records: guarantee 38. Guarantee 39 needs a pi box and lives in
//! `pi.rs`; 42 needs the runtime exclusively and lives in `runtime.rs`.

use crate::box_::{Up, box_name, box_up};
use e2e::{
    HttpFixture, TestEnv, assert_denied, assert_ok, curl, default_image, egress_log, inspect,
    pinfold, run_query, test_dir,
};

/// `ready`'s run id.
pub(super) fn run_id(up: &Up) -> String {
    let ready = up.ready();
    ready["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("ready names no run_id: {ready}"))
        .to_string()
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
    // events gain the second's requests. Sabotage: emit a fresh random
    // `run_id` instead of the generation; the label equality fails.
    // Sabotage: print the first run's id in the second `down`; its run_id
    // assertion fails. Sabotage: skip the terminal record in `Box::down`
    // for a requested stop; the inspect after `box down` reports terminal
    // `not-observed`. Writing it after `finish`'s ack is caught only when
    // `inspect` wins the race: the owner waits on nothing after the ack, so
    // no outside observer can hold it there. Sabotage: drop `detail` from
    // the terminal record; the 137 assertion fails. Sabotage: list the
    // Apple runtime's images in `inspect`; with no `container` on PATH it
    // exits 1. Linux catalog reads need no helper. Sabotage: resume after
    // the cursor's line rather than at
    // it; each page boundary skips a line and the order assertion fails.
    // Sabotage: drop the run from the cursor; another run's cursor
    // exits 0. Sabotage: report a no-egress run's log `unavailable`; the
    // not-applicable assertions fail. Sabotage: a default limit of 1000;
    // the first page then holds all 150 lines and its length assertion
    // fails.
    let env = TestEnv::new("run-history");
    let fixture = HttpFixture::start(None);
    let image = default_image(&env);
    let name = box_name("run-history");
    let spec = serde_json::json!({
        "name": name,
        "image": image,
        "egress": {
            "allow": ["api.github.com"],
            "routes": { "fixture.internal": fixture.route() },
        },
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
    // `box down` returns only after the owner acknowledged teardown, so the
    // terminal record is already there.
    first.down(&env);
    let (code, record) = inspect(&env, &first_run);
    assert_eq!(code, 0, "inspect after down failed");
    assert_eq!(record["terminal"]["state"], "available", "{record}");
    assert_eq!(record["terminal"]["record"]["reason"], "signal", "{record}");
    let down = first.close_stdin();
    assert_eq!(
        down.last().expect("the first run's down line")["run_id"],
        first_run.as_str()
    );

    let mut second = box_up(&env, &spec, &name);
    let run = run_id(&second);
    assert_eq!(
        second.ready()["labels"]["dev.pinfold.generation"],
        run.as_str(),
        "run_id is not the box's generation: {}",
        second.ready()
    );
    assert_ne!(run, first_run, "a reused name reused its run_id");
    assert_eq!(
        egress_log(second.ready()),
        "",
        "the second run's log is not empty before its first request"
    );
    let requests = |prefix: &str, count: usize| {
        let script = format!(
            "i=1; while [ $i -le {count} ]; do curl -sS -o /dev/null http://fixture.internal/{prefix}/$i || exit 1; i=$((i+1)); done"
        );
        let output = e2e::box_exec(&env, &name, &["sh", "-c", &script]);
        assert_ok(&output, &format!("the {prefix} route requests"));
    };
    requests("n", 150);
    // The box appends after the first page, so `next` must resume across
    // lines that did not exist when it was issued.
    let mut paths = Vec::new();
    let mut cursor: Option<String> = None;
    let mut appended = false;
    loop {
        let (code, page) = events(&env, &run, cursor.as_deref());
        assert_eq!(code, 0, "events failed");
        assert_eq!(page["state"], "available", "{page}");
        let page_events = page["events"].as_array().expect("events is an array");
        // 150 lines exist before the first page, so it is full at the
        // spec's default limit of 100.
        if cursor.is_none() {
            assert_eq!(page_events.len(), 100, "the first page's length");
        }
        for event in page_events {
            paths.push(event["path"].as_str().unwrap_or_default().to_string());
        }
        if let Some(next) = page["next"].as_str() {
            cursor = Some(next.to_string());
        }
        if !appended {
            requests("extra", 50);
            appended = true;
        } else if page["more"] == false {
            break;
        }
    }
    let expected: Vec<String> = (1..=150)
        .map(|i| format!("/n/{i}"))
        .chain((1..=50).map(|i| format!("/extra/{i}")))
        .collect();
    assert_eq!(paths, expected, "paging skipped, repeated or reordered");
    let cursor = cursor.expect("paging returned a cursor");

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
        run.as_str()
    );

    // SIGKILL of a no-egress box from the host: of its init through Apple's
    // runtime, of its VMM on Linux. The box exits 137.
    let quiet = box_name("run-history-quiet");
    let mut up = box_up(
        &env,
        &serde_json::json!({ "name": quiet, "image": image }),
        &quiet,
    );
    let quiet_run = run_id(&up);
    #[cfg(target_os = "linux")]
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(super::box_::vmm_pid(up.pid()) as i32),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("SIGKILL the box's VMM");
    #[cfg(not(target_os = "linux"))]
    e2e::run_ok(
        std::process::Command::new(e2e::image_cli()).args(["kill", "--signal", "KILL", &quiet]),
    );
    assert_eq!(up.wait().code(), Some(1), "an exited box's up exit code");
    let down = up.close_stdin();
    let down = down.last().expect("down line");
    assert_eq!(down["reason"], "exited", "{down}");

    // With no runtime on PATH, both queries answer for finished runs.
    let empty = test_dir(&env, "empty-path");
    let offline = |args: &[&str]| {
        run_query(
            env.command(pinfold())
                .arg("box")
                .args(args)
                .env("PATH", empty.as_path()),
        )
    };
    let (code, record) = offline(&["inspect", &run]);
    assert_eq!(code, 0, "inspect with no runtime failed");
    assert_eq!(record["egress_log"]["state"], "available", "{record}");
    let (code, page) = offline(&["events", &run]);
    assert_eq!(code, 0, "events with no runtime failed");
    assert_eq!(page["events"].as_array().map(Vec::len), Some(100));
    let (code, record) = offline(&["inspect", &quiet_run]);
    assert_eq!(code, 0, "inspect of the no-egress run failed");
    assert_eq!(record["egress_log"]["state"], "not-applicable", "{record}");
    let terminal = &record["terminal"]["record"];
    assert_eq!(terminal["reason"], "exited", "{record}");
    assert!(
        terminal["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("137")),
        "the terminal detail names no exit status 137: {record}"
    );
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
