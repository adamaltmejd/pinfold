//! End-to-end tests for the top-level CLI, the paths every caller hits
//! before a verb is chosen.
//!
//! They need no runtime, so they run on both hosts; the rest of the suite
//! needs the Apple `container` CLI or podman.

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
