//! The end-to-end suite's one test binary.
//!
//! Its tests run on a macOS host with the Apple `container` CLI, or a Linux host
//! with Firecracker and rootless podman for images.

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

mod box_;
mod cli;
mod pi;
mod runs;
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
