//! Run records: guarantees 38, 40, 41 and 42. Guarantee 39 needs a pi box
//! and lives in `pi.rs`.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::box_::{Up, box_name, box_up};
use e2e::{
    HttpFixture, TestDir, TestEnv, assert_denied, assert_ok, curl, default_image, egress_log,
    image_cli, inspect, pinfold, run_ok, run_query,
};

/// `ready`'s run id, checked against the generation label it must equal.
fn run_id(up: &Up) -> String {
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

/// The run's directory, at the path the spec names.
fn run_dir(env: &TestEnv, run_id: &str) -> PathBuf {
    env.state.join("pinfold/runs").join(run_id)
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
fn a_runs_end_is_recorded_apart_from_its_start() {
    // `box prune` removes every dead box on the host.
    let _runtime = crate::exclusive_runtime();
    // Guarantee 40. Sabotage: drop `detail` from the terminal record; the
    // 137 assertion fails. Sabotage: write a `signal` record from `prune`
    // (or from orphan removal); the killed owner's terminal is then
    // available after prune. Sabotage: skip the terminal record in
    // `Box::down` for a requested stop; the first inspect reports terminal
    // `not-observed`. Writing it after `finish`'s ack is caught only when
    // `inspect` wins the race: the owner waits on nothing after the ack, so
    // no outside observer can hold it there.
    let env = TestEnv::new("run-end");
    let image = default_image(&env);
    let spec = |name: &str| serde_json::json!({ "name": name, "image": image });

    // `box down` returns only after the owner acknowledged teardown, so the
    // terminal record is already there.
    let name = box_name("run-end-down");
    let mut up = box_up(&env, &spec(&name), &name);
    let run = run_id(&up);
    up.down(&env);
    let (code, record) = inspect(&env, &run);
    assert_eq!(code, 0, "inspect after down failed");
    assert_eq!(record["launch"]["state"], "available", "{record}");
    assert_eq!(record["launch"]["record"]["run_id"], run.as_str());
    assert_eq!(record["terminal"]["state"], "available", "{record}");
    assert_eq!(record["terminal"]["record"]["reason"], "signal", "{record}");
    assert_eq!(
        record["terminal"]["record"]["detail"],
        serde_json::Value::Null
    );
    let down = up.close_stdin();
    assert_eq!(down.last().expect("down line")["reason"], "signal");

    // SIGKILL of the box's init from the host runtime: the box exits 137.
    let name = box_name("run-end-killed");
    let mut up = box_up(&env, &spec(&name), &name);
    let run = run_id(&up);
    run_ok(Command::new(image_cli()).args(["kill", "--signal", "KILL", &name]));
    assert_eq!(up.wait().code(), Some(1), "an exited box's up exit code");
    let down = up.close_stdin();
    let down = down.last().expect("down line");
    assert_eq!(down["reason"], "exited", "{down}");
    let (code, record) = inspect(&env, &run);
    assert_eq!(code, 0, "inspect after an init kill failed");
    let terminal = &record["terminal"]["record"];
    assert_eq!(terminal["reason"], "exited", "{record}");
    assert!(
        terminal["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("137")),
        "the terminal detail names no exit status 137: {record}"
    );
    assert_eq!(terminal["detail"], down["detail"], "{record}");

    // An owner that dies without teardown leaves no end, and nothing infers
    // one later.
    let name = box_name("run-end-owner");
    let mut up = box_up(&env, &spec(&name), &name);
    let run = run_id(&up);
    up.kill();
    up.wait();
    for when in ["after the owner's SIGKILL", "after prune"] {
        let (code, record) = inspect(&env, &run);
        assert_eq!(code, 0, "inspect {when} failed");
        assert_eq!(record["launch"]["state"], "available", "{when}: {record}");
        assert_eq!(
            record["terminal"]["state"], "not-observed",
            "{when}: {record}"
        );
        assert_eq!(record["terminal"]["record"], serde_json::Value::Null);
        if when == "after the owner's SIGKILL" {
            let pruned = run_ok(env.command(pinfold()).args(["box", "prune"]));
            assert!(
                String::from_utf8_lossy(&pruned.stdout).contains(&name),
                "prune did not remove the dead owner's box"
            );
        }
    }
}

/// Hold Podman's real image-store lock, so its clients stall before they
/// return inventory. The lock is released when the file closes.
#[cfg(target_os = "linux")]
fn hold_image_store_lock(env: &TestEnv) -> File {
    let info = run_ok(
        env.command(Path::new("podman"))
            .args(["info", "--format", "json"]),
    );
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    let store = &info["store"];
    let path = Path::new(store["graphRoot"].as_str().unwrap()).join(format!(
        "{}-images/images.lock",
        store["graphDriverName"].as_str().unwrap()
    ));
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open Podman's image-store lock");
    let held = nix::libc::flock {
        l_type: nix::libc::F_WRLCK as i16,
        l_whence: nix::libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    nix::fcntl::fcntl(&lock, nix::fcntl::FcntlArg::F_SETLK(&held))
        .expect("hold Podman's image-store lock");
    lock
}

#[test]
fn run_queries_are_bounded_and_need_no_runtime() {
    // The held image-store lock stalls every podman client.
    #[cfg(target_os = "linux")]
    let _runtime = crate::exclusive_runtime();
    #[cfg(not(target_os = "linux"))]
    let _runtime = crate::shared_runtime();
    // Guarantee 41. Sabotage: list the runtime's boxes (or take the name
    // lock) in `inspect`; with no runtime on PATH it exits 1, and on Linux
    // it stalls behind the image-store lock past the list it should beat.
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
        assert!(page_events.len() <= 100, "a page over the default limit");
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

    #[cfg(target_os = "linux")]
    {
        let lock = hold_image_store_lock(&env);
        let mut list = env
            .command(pinfold())
            .args(["box", "list", "--label", "dev.pinfold.owner"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the stalled list");
        let (code, _) = inspect(&env, &run);
        assert_eq!(code, 0, "inspect behind the image-store lock failed");
        let (code, _) = events(&env, &run, None);
        assert_eq!(code, 0, "events behind the image-store lock failed");
        assert!(
            list.try_wait().unwrap().is_none(),
            "the concurrent list was not blocked by the image-store lock"
        );
        drop(lock);
        list.wait().expect("wait for the list");
    }

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

/// Set the modification time of the run's directory and every file in it.
fn age(dir: &Path, age: Duration) {
    let time = SystemTime::now() - age;
    for entry in fs::read_dir(dir).expect("read the run dir") {
        let path = entry.expect("run dir entry").path();
        File::open(&path).unwrap().set_modified(time).unwrap();
    }
    File::open(dir).unwrap().set_modified(time).unwrap();
}

/// Append `bytes` of valid decision lines to the run's egress log.
fn pad(dir: &Path, bytes: usize) {
    let line = b"{\"time\":\"2026-10-01T00:00:00Z\",\"host\":\"pad.invalid\",\"decision\":\"refused\",\"reason\":\"not allowlisted\"}\n";
    let padding = line.repeat(bytes.div_ceil(line.len()));
    fs::OpenOptions::new()
        .append(true)
        .open(dir.join("egress.jsonl"))
        .expect("open the run's egress log")
        .write_all(&padding)
        .expect("pad the run's egress log");
}

fn log_bytes(env: &TestEnv, run_id: &str) -> u64 {
    let (code, record) = inspect(env, run_id);
    assert_eq!(code, 0, "inspect {run_id} failed");
    record["egress_log"]["bytes"]
        .as_u64()
        .unwrap_or_else(|| panic!("no log bytes: {record}"))
}

#[test]
fn run_records_are_retained_within_bounds() {
    // `clean` prunes dead boxes host-wide.
    let _runtime = crate::exclusive_runtime();
    // Guarantee 42. Sabotage: age out runs without checking the lifetime
    // lock; the live run is removed. Sabotage: drop the 14-day rule; the
    // aged run survives the first clean, which is under the quota.
    // Sabotage: evict the newest inactive run first; the newer padded run
    // goes and the older stays. Sabotage:
    // truncate the live run's log to meet the quota; its bytes shrink.
    // Sabotage: count live runs toward the quota; the newer padded run goes
    // too. The 128 MiB pad and the backdated mtimes act on host paths the
    // spec names, as a user's disk and clock would.
    let env = TestEnv::new("run-retention");
    let fixture = HttpFixture::start(None);
    let image = default_image(&env);
    let spec = |name: &str| {
        serde_json::json!({
            "name": name,
            "image": image,
            "egress": { "routes": { "fixture.internal": fixture.route() } },
        })
    };
    let finished = |test: &str| {
        let name = box_name(test);
        let mut up = box_up(&env, &spec(&name), &name);
        let run = run_id(&up);
        up.close_stdin();
        run
    };
    let aged = finished("run-retention-aged");
    let older = finished("run-retention-older");
    let newer = finished("run-retention-newer");
    let name = box_name("run-retention-live");
    let mut live_up = box_up(&env, &spec(&name), &name);
    let live = run_id(&live_up);

    // Under the quota, only age retires: the aged run goes, the live one
    // stays however old.
    let day = Duration::from_secs(24 * 60 * 60);
    age(&run_dir(&env, &aged), 15 * day);
    age(&run_dir(&env, &live), 15 * day);
    run_ok(env.command(pinfold()).arg("clean"));
    assert_eq!(inspect(&env, &aged).0, 3, "the aged inactive run was kept");
    let (code, record) = inspect(&env, &live);
    assert_eq!(code, 0, "the aged live run was removed");
    assert_eq!(record["launch"]["state"], "available", "{record}");

    // Inactive runs together hold just over 128 MiB; the live one would
    // tip the newer over the quota if it counted.
    pad(&run_dir(&env, &older), (64 << 20) + (256 << 10));
    pad(&run_dir(&env, &newer), (64 << 20) + (256 << 10));
    pad(&run_dir(&env, &live), 64 << 20);
    age(&run_dir(&env, &live), 15 * day);
    age(&run_dir(&env, &older), 3 * day);
    age(&run_dir(&env, &newer), 2 * day);
    let live_bytes = log_bytes(&env, &live);

    run_ok(env.command(pinfold()).arg("clean"));

    assert_eq!(inspect(&env, &older).0, 3, "the older padded run was kept");
    let (code, record) = inspect(&env, &newer);
    assert_eq!(code, 0, "the newer padded run was removed");
    assert_eq!(record["launch"]["state"], "available", "{record}");
    let (code, record) = inspect(&env, &live);
    assert_eq!(code, 0, "the live run was removed");
    assert_eq!(record["launch"]["state"], "available", "{record}");
    assert_eq!(
        log_bytes(&env, &live),
        live_bytes,
        "the live run's log changed"
    );
    let request = curl(&env, &name, "5", &["http://fixture.internal/after-clean"]);
    assert_ok(&request, "the live run's request after clean");
    assert!(
        log_bytes(&env, &live) > live_bytes,
        "the live run's log did not keep appending"
    );
    live_up.close_stdin();
}
