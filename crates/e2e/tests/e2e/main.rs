//! The end-to-end suite's one test binary.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with rootless podman.

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

mod box_;
mod cli;
mod image_warning;
mod pi;
mod runtime;

// Daily maintenance can remove another test's dead box. Hold each guard
// before fixtures so cleanup finishes before the runtime is released.
static RUNTIME: RwLock<()> = RwLock::const_new(());

fn shared_runtime() -> RwLockReadGuard<'static, ()> {
    RUNTIME.blocking_read()
}

fn exclusive_runtime() -> RwLockWriteGuard<'static, ()> {
    RUNTIME.blocking_write()
}
