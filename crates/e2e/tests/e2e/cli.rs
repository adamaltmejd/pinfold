//! End-to-end tests for the top-level CLI, the paths every caller hits
//! before a verb is chosen.
//!
//! The options need no runtime. Doctor is exercised with the host runtime
//! available and absent from PATH.

use std::fs;

use e2e::{TestEnv, pinfold};

/// A fresh host with no runtime on PATH: `pinfold --version` and `pinfold
/// box list --help` answer from the binary alone, spawn nothing and leave the
/// state dir untouched.
#[test]
fn version_needs_no_runtime() {
    // Sabotage: run `crate::core::clean::maintain()` first in main.rs,
    // before the options and the help check, as the code once did; with the
    // empty PATH the pass writes under the state dir, so both empty-state
    // assertions fail. Sabotage: print `pinfold 0.0.0` in main.rs's
    // `--version` arm instead of `CARGO_PKG_VERSION`; stdout lacks the
    // workspace version and the version assertion fails. Sabotage: in
    // main.rs, answer `--help` after the daily pass instead of before it;
    // `box list --help` then writes the maintenance state first, and its
    // empty-state assertion fails.
    let env = TestEnv::new("version");
    let empty = env.root.join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    // TestEnv creates the state dir, so untouched means still empty.
    let state_entries = || {
        fs::read_dir(&env.state)
            .expect("read the state dir")
            .count()
    };

    let output = env
        .command(pinfold())
        .arg("--version")
        .env("PATH", &empty)
        .output()
        .expect("run pinfold --version");

    assert!(
        output.status.success(),
        "pinfold --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The e2e crate shares the workspace version with pinfold.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "pinfold --version printed no version: {stdout}"
    );
    assert_eq!(state_entries(), 0, "pinfold --version wrote state");

    let help = env
        .command(pinfold())
        .args(["box", "list", "--help"])
        .env("PATH", &empty)
        .output()
        .expect("run pinfold box list --help");
    assert!(
        help.status.success(),
        "pinfold box list --help failed: {}",
        String::from_utf8_lossy(&help.stderr)
    );
    assert_eq!(state_entries(), 0, "pinfold box list --help wrote state");
}

#[test]
fn doctor_reports_without_changing_state() {
    // Guarantee 27. Sabotage: run maintenance before doctor; the fresh
    // state directory gains a maintenance stamp and the old artifact goes.
    // Sabotage: omit host_requirements after a failed preflight; the Linux
    // refusal no longer names the missing runtime directory. Remove its
    // tun check; a host with an inaccessible device loses its named reason.
    let env = TestEnv::with_private_cache("doctor");
    let old = env.root.join("cache/pinfold/artifacts/pi/0.0.0/marker");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, b"keep").unwrap();
    let empty = env.root.join("empty-path");
    fs::create_dir(&empty).unwrap();
    let missing = env.root.join("missing-runtime");

    // The real Podman also fails before reporting its host settings when
    // XDG_RUNTIME_DIR is absent. All reports leave the same fixtures alone.
    for scenario in ["ready", "no-runtime", "missing-runtime-dir"] {
        if scenario == "missing-runtime-dir" && !cfg!(target_os = "linux") {
            continue;
        }
        let mut command = env.command(pinfold());
        command.arg("doctor").current_dir(&env.root);
        if scenario != "ready" {
            command.env("XDG_RUNTIME_DIR", &missing);
        }
        if scenario == "no-runtime" {
            command.env("PATH", &empty);
        }
        let output = command.output().expect("run pinfold doctor");
        assert!(output.status.success(), "doctor failed: {output:?}");
        assert_eq!(
            fs::read(&old).unwrap(),
            b"keep",
            "doctor removed an artifact"
        );
        assert_eq!(
            fs::read_dir(&env.state).unwrap().count(),
            0,
            "doctor wrote state"
        );
        if scenario != "ready" && cfg!(target_os = "linux") {
            let report = String::from_utf8_lossy(&output.stdout);
            assert!(
                report.contains("runtime-dir") && report.contains(missing.to_str().unwrap()),
                "doctor did not identify the missing runtime directory: {report}"
            );
            if fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/net/tun")
                .is_err()
            {
                assert!(
                    report.contains("tun") && report.contains("/dev/net/tun"),
                    "doctor did not identify the unavailable tun device: {report}"
                );
            }
        }
    }
}
