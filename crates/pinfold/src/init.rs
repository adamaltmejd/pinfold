//! `pinfold init`: PID 1 in a box.
//!
//! It reports readiness on stdout, reaps every child reparented to it, and
//! exits on SIGTERM. PID 1 has no default signal actions, so the handlers
//! are what make a stop possible. When the transport carries the proxy socket
//! in, it also relays `127.0.0.1:3128` to that socket, because clients only
//! know how to reach a proxy over TCP.

use std::ffi::OsString;
use std::io::{self, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use nix::errno::Errno;
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;

static TERMINATE: AtomicBool = AtomicBool::new(false);

/// The loopback port clients use for the proxy.
const RELAY_LISTEN: &str = "127.0.0.1:3128";

extern "C" fn on_terminate(_signal: i32) {
    TERMINATE.store(true, Ordering::SeqCst);
}

extern "C" fn on_child(_signal: i32) {}

/// Run as PID 1 until SIGTERM. `args` is the optional guest path of the
/// carried proxy socket; without it the box has no egress and no relay.
pub fn run(args: &[OsString]) -> ! {
    let unblocked = install_handlers();

    if let Some(target) = args.first() {
        let target = PathBuf::from(target);
        // Ready is reported after the relay listens, so a caller that sees
        // ready can reach the proxy at once.
        match TcpListener::bind(RELAY_LISTEN) {
            Ok(listener) => {
                thread::spawn(move || relay_loop(listener, target));
            }
            Err(error) => {
                eprintln!("pinfold init: bind {RELAY_LISTEN}: {error}");
                process::exit(1);
            }
        }
    }

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
        // sigsuspend returns when any unblocked signal is delivered; a
        // signal is the normal wakeup, not an error.
        if let Err(error) = unblocked.suspend() {
            eprintln!("pinfold init: sigsuspend: {error}");
            break;
        }
    }
    process::exit(0);
}

/// Accept relay clients and give each its own thread.
fn relay_loop(listener: TcpListener, target: PathBuf) {
    for client in listener.incoming() {
        let Ok(client) = client else {
            continue;
        };
        let target = target.clone();
        thread::spawn(move || relay(client, &target));
    }
}

/// Copy both directions between one client and the proxy socket. Either half
/// closing shuts the other down, so no thread outlives its connection.
fn relay(client: TcpStream, target: &Path) {
    let mut client = client;
    let Ok(mut server) = UnixStream::connect(target) else {
        return;
    };
    let Ok(mut client_reader) = client.try_clone() else {
        return;
    };
    let Ok(mut server_writer) = server.try_clone() else {
        return;
    };
    let up = thread::spawn(move || {
        let _ = io::copy(&mut client_reader, &mut server_writer);
        let _ = server_writer.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = up.join();
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
