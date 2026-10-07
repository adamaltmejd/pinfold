//! The end-to-end suite's one test binary.
//!
//! The modules split it by area: `box_` the box lifecycle and runtime, `pi`
//! the pi run, `cli` the top-level CLI. `cargo test -p e2e` builds the
//! `pinfold` binary and runs tests in parallel except the two orphan
//! scenarios; the harness, fixtures and helpers live in the crate's `e2e::`.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with rootless podman.

use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

mod box_;
mod cli;
mod image_warning;
mod pi;
mod update;

// Daily maintenance can remove another test's dead box. Hold each guard
// before fixtures so cleanup finishes before the runtime is released.
static RUNTIME: RwLock<()> = RwLock::new(());

fn shared_runtime() -> RwLockReadGuard<'static, ()> {
    RUNTIME.read().unwrap_or_else(|poison| poison.into_inner())
}

fn exclusive_runtime() -> RwLockWriteGuard<'static, ()> {
    RUNTIME.write().unwrap_or_else(|poison| poison.into_inner())
}
