//! The end-to-end suite's one test binary.
//!
//! The modules split it by area: `box_` the box lifecycle and runtime, `pi`
//! the pi run, `cli` the top-level CLI. `cargo test -p e2e` builds the
//! `pinfold` binary and runs all of them in parallel; the harness, fixtures
//! and helpers live in the crate's `e2e::`.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with rootless podman.

use std::sync::Mutex;

mod box_;
mod cli;
mod image_warning;
mod pi;
mod update;

/// The two owner-gone tests share one hazard: either one's removal can take
/// the other's dead box before the other expects it. Hold this from killing
/// an owner until that test's removal has run.
static DEAD_BOX_RACE: Mutex<()> = Mutex::new(());
