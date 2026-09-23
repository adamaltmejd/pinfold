//! `pinfold init`: PID 1 in a box.
//!
//! It reports readiness on stdout, reaps every child reparented to it, and
//! exits on SIGTERM. PID 1 has no default signal actions, so the handlers
//! are what make a stop possible.

use std::io::{self, Write};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};

use nix::errno::Errno;
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;

static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_terminate(_signal: i32) {
    TERMINATE.store(true, Ordering::SeqCst);
}

extern "C" fn on_child(_signal: i32) {}

/// Run as PID 1 until SIGTERM.
pub fn run() -> ! {
    let unblocked = install_handlers();

    println!("ready");
    io::stdout().flush().expect("write ready");

    loop {
        if TERMINATE.load(Ordering::SeqCst) {
            break;
        }
        reap();
        if TERMINATE.load(Ordering::SeqCst) {
            break;
        }
        // TERM, INT and CHLD are blocked, so one that arrives between the
        // reap and this wait stays pending and wakes it immediately.
        unblocked.suspend().expect("sigsuspend");
    }
    process::exit(0);
}

/// Install the handlers and block the signals they serve, returning the mask
/// `suspend` waits with (the mask from before they were blocked).
fn install_handlers() -> SigSet {
    let terminate = SigAction::new(
        SigHandler::Handler(on_terminate),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let child = SigAction::new(
        SigHandler::Handler(on_child),
        SaFlags::empty(),
        SigSet::empty(),
    );
    // Safety: the handlers only store to an atomic.
    unsafe {
        nix::sys::signal::sigaction(Signal::SIGTERM, &terminate).expect("sigaction SIGTERM");
        nix::sys::signal::sigaction(Signal::SIGINT, &terminate).expect("sigaction SIGINT");
        nix::sys::signal::sigaction(Signal::SIGCHLD, &child).expect("sigaction SIGCHLD");
    }

    let mut blocked = SigSet::empty();
    blocked.add(Signal::SIGTERM);
    blocked.add(Signal::SIGINT);
    blocked.add(Signal::SIGCHLD);
    let mut previous = SigSet::empty();
    nix::sys::signal::sigprocmask(SigmaskHow::SIG_BLOCK, Some(&blocked), Some(&mut previous))
        .expect("block signals");
    previous
}

/// Reap every child that has exited.
fn reap() {
    loop {
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => break,
            Ok(_) => continue,
            Err(Errno::ECHILD) => break,
            Err(Errno::EINTR) => continue,
            Err(_) => break,
        }
    }
}
