//! End-to-end tests for the top-level CLI, the paths every caller hits
//! before a verb is chosen.
//!
//! They need no runtime, so they run on both hosts; the rest of the suite
//! needs the Apple `container` CLI or podman.

use std::fs;

use e2e::{TestEnv, pinfold};

/// A fresh host with no runtime on PATH: `pinfold --version` answers from
/// the binary alone, spawns nothing and leaves the state dir untouched.
#[test]
fn version_needs_no_runtime() {
    // Sabotage: run `pinfold::core::clean::maintain()` before dispatch as
    // the code did; with the empty PATH the pass prints the
    // missing-runtime sentence on stderr and writes the `maintenance` stamp,
    // failing both assertions.
    let binary = pinfold();
    let env = TestEnv::new("version");
    let empty = env.root.join("empty-path");
    fs::create_dir_all(&empty).unwrap();

    let output = env
        .command(binary)
        .arg("--version")
        .env("PATH", &empty)
        .output()
        .expect("run pinfold --version");

    assert!(
        output.status.success(),
        "pinfold --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.starts_with(b"pinfold "),
        "pinfold --version printed no version: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.is_empty(),
        "pinfold --version wrote to stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !env.state.join("pinfold").join("maintenance").exists(),
        "pinfold --version left a maintenance stamp"
    );
}
