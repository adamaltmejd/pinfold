//! The end-to-end suite's one test binary.
//!
//! They run on a macOS host with the Apple `container` CLI, or a Linux host
//! with rootless podman.

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

mod box_;
mod cli;
mod image_warning;
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

/// Write-lock Podman's image-store lock, as `held` describes. conmon's
/// detached exit command, `podman container cleanup`, write-locks it for a
/// moment after a container or exec session ends, often after `podman rm`
/// has returned, so a single try races it.
#[cfg(target_os = "linux")]
fn hold_image_lock(lock: &std::fs::File, held: &nix::libc::flock) {
    use nix::errno::Errno;
    use nix::fcntl::{FcntlArg, fcntl};
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match fcntl(lock, FcntlArg::F_SETLK(held)) {
            Ok(_) => return,
            Err(Errno::EAGAIN | Errno::EACCES) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Podman's image-store lock stayed held"
                );
                std::thread::yield_now();
            }
            Err(error) => panic!("hold Podman's image-store lock: {error}"),
        }
    }
}
