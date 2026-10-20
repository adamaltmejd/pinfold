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
/// has returned, so a single try races it. Only that holder is waited
/// out; any other, a pinfold straggler included, fails the test.
#[cfg(target_os = "linux")]
fn hold_image_lock(lock: &std::fs::File, held: &nix::libc::flock) {
    use nix::errno::Errno;
    use nix::fcntl::{FcntlArg, fcntl};
    use std::os::unix::fs::MetadataExt;
    let metadata = lock.metadata().unwrap();
    let identity = format!(
        "{:02x}:{:02x}:{}",
        nix::sys::stat::major(metadata.dev()),
        nix::sys::stat::minor(metadata.dev()),
        metadata.ino()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match fcntl(lock, FcntlArg::F_SETLK(held)) {
            Ok(_) => return,
            Err(Errno::EAGAIN | Errno::EACCES) => {}
            Err(error) => panic!("hold Podman's image-store lock: {error}"),
        }
        let locks = std::fs::read_to_string("/proc/locks").unwrap();
        for line in locks.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.contains(&"->") || !fields.contains(&identity.as_str()) {
                continue;
            }
            let pid = fields.get(4).copied().unwrap_or("?");
            // A holder that exited since /proc/locks was read has released.
            let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
                continue;
            };
            if cmdline.is_empty() {
                continue;
            }
            let argv: Vec<_> = cmdline
                .split(|&byte| byte == 0)
                .map(String::from_utf8_lossy)
                .collect();
            let cleanup = argv.first().is_some_and(|arg0| arg0.ends_with("podman"))
                && argv
                    .windows(2)
                    .any(|pair| pair[0] == "container" && pair[1] == "cleanup");
            assert!(
                cleanup,
                "image-store lock held by {pid}, not podman's cleanup: {}",
                argv.join(" ")
            );
        }
        assert!(
            std::time::Instant::now() < deadline,
            "podman's cleanup held the image-store lock past the deadline"
        );
        std::thread::yield_now();
    }
}
