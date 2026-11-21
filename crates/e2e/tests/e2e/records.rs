//! Guarantee 38: every egress decision is recorded, or the box stops. Each
//! run has its own egress log and launch record.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use e2e::{BoxUp, Fixture, TestEnv, box_name, default_image, run};
use serde_json::{Value, json};

const COMMAND: Duration = Duration::from_secs(120);
/// Each box's egress log cap (Egress proxy, Log).
const CAP: u64 = 128 << 20;

/// An allowed route request and a refused request, in that order: the
/// lines they added to `up`'s log, as `(host, decision, reason)`, the
/// reason a refusal's only.
fn two_decisions(env: &TestEnv, up: &BoxUp, name: &str) -> Vec<(String, String, String)> {
    let before = up.egress_log().len();
    for (url, status) in [
        ("http://fixture.internal/", "418"),
        ("http://example.com/", "403"),
    ] {
        let out = env.exec(
            name,
            &[
                "curl",
                "-sS",
                "--max-time",
                "20",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                url,
            ],
        );
        assert_eq!(
            (out.code, out.stdout.as_str()),
            (0, status),
            "{url}: {out:?}"
        );
    }
    up.egress_log()[before..]
        .iter()
        .map(|line| {
            let field = |key: &str| line[key].as_str().unwrap_or_default().to_string();
            let reason = match line["decision"] == "refused" {
                true => field("reason"),
                false => String::new(),
            };
            (field("host"), field("decision"), reason)
        })
        .collect()
}

/// The first `len` bytes of the file at `path` and its length.
fn head(path: &str, len: usize) -> (Vec<u8>, u64) {
    let mut file = fs::File::open(path).unwrap_or_else(|error| panic!("open {path}: {error}"));
    let mut bytes = vec![0; len];
    file.read_exact(&mut bytes).unwrap();
    (bytes, file.metadata().unwrap().len())
}

#[test]
fn every_egress_decision_is_recorded_or_the_box_stops() {
    // Sabotage (run): in proxy.rs's `record`, append even when the line
    // would pass MAX_LOG_BYTES. The padded log then grows and the box stays
    // up: `box up` does not end.
    // The second run's own two decisions are the positive control.
    let env = TestEnv::new("records");
    let fixture = Fixture::start(418);
    let name = box_name("records");
    let literal = format!("pf-literal-{}", std::process::id());
    let spec = json!({
        "name": name,
        "image": default_image(),
        "env": { "E2E_LITERAL": literal },
        "egress": {
            "allow": ["api.github.com"],
            "routes": { "fixture.internal": fixture.route() },
        },
        "cpus": 2,
        "memory": "1G",
    });
    let expected = |host: &str, decision: &str, reason: &str| {
        (host.to_string(), decision.to_string(), reason.to_string())
    };
    let own = vec![
        expected("fixture.internal", "allowed", ""),
        expected("example.com", "refused", "not allowlisted"),
    ];

    // The first run records its decisions, then its log is padded to one
    // byte below the cap without writing the bytes: the cap is on the
    // file's length.
    let first = env.up(&spec);
    let first_id = first.ready()["run_id"]
        .as_str()
        .expect("run_id")
        .to_string();
    let first_log = first.ready()["egress_log"]
        .as_str()
        .expect("egress_log")
        .to_string();
    assert_eq!(
        two_decisions(&env, &first, &name),
        own,
        "the first run's log"
    );
    // The log `ready` names is a regular file under the test's state root.
    assert!(
        Path::new(&first_log).starts_with(env.state_home())
            && fs::symlink_metadata(&first_log).is_ok_and(|metadata| metadata.is_file()),
        "{first_log}"
    );
    let recorded = fs::read(&first_log).unwrap();
    fs::File::options()
        .write(true)
        .open(&first_log)
        .and_then(|file| file.set_len(CAP - 1))
        .unwrap();

    // The next route request cannot be recorded: the owner stops with
    // `audit-log` and exit 1, and the log is as the test left it. The
    // request may already have reached the fixture (Egress proxy), so
    // neither its answer nor the fixture is asserted.
    let _ = env.exec(
        &name,
        &[
            "curl",
            "-sS",
            "--max-time",
            "20",
            "http://fixture.internal/",
        ],
    );
    let (down, code) = first.ended();
    assert_eq!(
        (
            &down["event"],
            &down["box"],
            &down["run_id"],
            &down["reason"]
        ),
        (
            &json!("down"),
            &json!(name),
            &json!(first_id),
            &json!("audit-log")
        ),
        "{down}"
    );
    assert_eq!(code, 1, "box up after {down}");
    assert_eq!(
        head(&first_log, recorded.len()),
        (recorded.clone(), CAP - 1)
    );

    // `inspect` reads the first run's records with no runtime on PATH. The
    // launch record is the spec's policy, naming the box's variables
    // without values: the spec's and pinfold's default PINFOLD_ALLOW (Run
    // records).
    let empty = env.root().join("empty-path");
    fs::create_dir(&empty).unwrap();
    let inspect = |run_id: &str| -> Value {
        let out = run(
            env.command()
                .env("PATH", &empty)
                .args(["box", "inspect", run_id]),
            COMMAND,
        );
        assert_eq!(out.code, 0, "{out:?}");
        serde_json::from_str(&out.stdout).expect("inspect JSON")
    };
    let first_inspect = inspect(&first_id);
    let launch = &first_inspect["launch"]["record"];
    assert_eq!(
        first_inspect["launch"]["state"], "available",
        "{first_inspect}"
    );
    assert_eq!(
        (
            &launch["run_id"],
            &launch["box"],
            &launch["egress"],
            &launch["env"],
            &launch["memory"]
        ),
        (
            &json!(first_id),
            &json!(name),
            &json!({
                "allow": ["api.github.com"],
                "routes": { "fixture.internal": { "kind": "host-service", "to": fixture.route() } },
            }),
            &json!(["E2E_LITERAL", "PINFOLD_ALLOW"]),
            &json!("1G"),
        ),
        "{launch}"
    );
    assert_eq!(launch["cpus"].as_f64(), Some(2.0), "{launch}");
    assert!(
        !first_inspect.to_string().contains(&literal),
        "{first_inspect}"
    );
    assert_eq!(
        (
            &first_inspect["terminal"]["state"],
            &first_inspect["terminal"]["record"]["reason"],
            &first_inspect["egress_log"]["bytes"],
        ),
        (&json!("available"), &json!("audit-log"), &json!(CAP - 1)),
        "{first_inspect}"
    );

    // Control: a second run of the same name, under another policy, has its
    // own log holding its own two decisions once each. The first run's log
    // and launch record are unchanged.
    let mut second_spec = spec.clone();
    second_spec["egress"]["allow"] = json!(["api.github.com", "example.org"]);
    second_spec["memory"] = json!("2G");
    let second = env.up(&second_spec);
    let second_id = second.ready()["run_id"].as_str().expect("run_id");
    assert_ne!(second_id, first_id);
    assert_eq!(
        two_decisions(&env, &second, &name),
        own,
        "the second run's log"
    );
    assert_eq!(second.egress_log().len(), 2);
    assert_eq!(head(&first_log, recorded.len()), (recorded, CAP - 1));
    assert_eq!(
        inspect(&first_id)["launch"],
        first_inspect["launch"],
        "the first run's launch record after the second run"
    );
}
