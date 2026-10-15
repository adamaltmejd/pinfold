//! The end-to-end suite's one test binary.
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

#[track_caller]
fn shared_runtime() -> RwLockReadGuard<'static, ()> {
    let waiting = e2e::timing::Span::new("runtime.shared.WAIT");
    let guard = RUNTIME.read().unwrap_or_else(|poison| poison.into_inner());
    waiting.finish("ACQUIRED");
    guard
}

#[track_caller]
fn exclusive_runtime() -> RwLockWriteGuard<'static, ()> {
    let waiting = e2e::timing::Span::new("runtime.exclusive.WAIT");
    let guard = RUNTIME.write().unwrap_or_else(|poison| poison.into_inner());
    waiting.finish("ACQUIRED");
    guard
}
