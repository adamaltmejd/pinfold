//! `pinfold init`: PID 1 in a box.
//!
//! It reports readiness on stdout, has the kernel reap every child
//! reparented to it, and exits on SIGTERM. PID 1 has no default signal
//! actions, so waiting for the blocked signal is what makes a stop possible.
//! When the transport carries the proxy socket in, it also relays
//! `127.0.0.1:3128` to that socket, because clients only know how to reach a
//! proxy over TCP.

use std::fs;
use std::io::{self, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;

use nix::sys::signal::{SigHandler, SigSet, Signal};

/// The loopback port clients use for the proxy.
const RELAY_LISTEN: &str = "127.0.0.1:3128";

/// Run as PID 1 until SIGTERM. `args` is the optional guest path of the
/// carried proxy socket; without it the box has no egress and no relay.
pub fn run(args: &[String]) -> ! {
    if args.first().map(String::as_str) == Some("exec") {
        exec(&args[1..]);
    }

    // Blocked before the relay thread exists, which inherits the mask, so
    // both signals stay pending for `wait` below. A blocked signal reaches
    // PID 1 even with no handler installed.
    let mut terminate = SigSet::empty();
    terminate.add(Signal::SIGTERM);
    terminate.add(Signal::SIGINT);
    terminate.thread_block().expect("block SIGTERM and SIGINT");
    // With SIGCHLD ignored the kernel reaps every exited child, the ones
    // reparented to PID 1 included. init forks nothing, so no wait of its
    // own can lose a status to this.
    // Safety: SIG_IGN runs no code.
    unsafe { nix::sys::signal::signal(Signal::SIGCHLD, SigHandler::SigIgn) }
        .expect("ignore SIGCHLD");

    if let Some(target) = args.first() {
        let target = PathBuf::from(target);
        // Ready is reported after the relay listens, so a caller that sees
        // ready can reach the proxy at once.
        let listener = TcpListener::bind(RELAY_LISTEN).unwrap_or_else(|error| {
            eprintln!("pinfold init: bind {RELAY_LISTEN}: {error}");
            process::exit(1)
        });
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let target = target.clone();
                thread::spawn(move || relay(client, &target));
            }
        });
    }

    println!("ready");
    io::stdout().flush().expect("write ready");

    let _ = terminate.wait();
    process::exit(0);
}

/// `init exec -- ARGV...`: raise this process's `oom_score_adj` to 1000 and
/// become ARGV. Every process pinfold starts in a box other than init runs
/// through here, so the kernel's OOM killer takes one of them before init.
fn exec(args: &[String]) -> ! {
    let (program, args) = match args {
        [separator, program, args @ ..] if separator == "--" => (program, args),
        _ => {
            eprintln!("pinfold init exec: usage: pinfold init exec -- argv...");
            process::exit(1);
        }
    };
    // Raising a process's own OOM score needs no privilege; a rootless
    // container cannot lower it again.
    if let Err(error) = fs::write("/proc/self/oom_score_adj", "1000") {
        eprintln!("pinfold init exec: oom_score_adj: {error}");
        process::exit(1);
    }
    let error = process::Command::new(program).args(args).exec();
    eprintln!("pinfold init exec: {program}: {error}");
    // The shell's convention for a command that could not be run.
    process::exit(if error.kind() == io::ErrorKind::NotFound {
        127
    } else {
        126
    });
}

/// Copy both directions between one client and the proxy socket. Either half
/// closing shuts the other down, so no thread outlives its connection.
fn relay(mut client: TcpStream, target: &Path) -> io::Result<()> {
    let mut server = UnixStream::connect(target)?;
    let mut client_reader = client.try_clone()?;
    let mut server_writer = server.try_clone()?;
    let up = thread::spawn(move || {
        let _ = io::copy(&mut client_reader, &mut server_writer);
        let _ = server_writer.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = up.join();
    Ok(())
}
