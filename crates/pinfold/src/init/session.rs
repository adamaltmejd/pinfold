//! Unprivileged cleanup of one departed project command's process group.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

use nix::poll::poll;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

pub(super) fn wait(
    child: &mut Child,
    departure: BorrowedFd<'_>,
    socket: bool,
) -> io::Result<ExitStatus> {
    let process = match super::firecracker::pidfd_open(child.id() as i32) {
        Ok(process) => process,
        Err(error) => {
            let _ = end(child);
            return Err(error);
        }
    };
    let mut disconnected = nix::libc::POLLHUP | nix::libc::POLLERR;
    if socket {
        disconnected |= nix::libc::POLLRDHUP;
    }
    loop {
        let mut fds = [
            nix::libc::pollfd {
                fd: process.as_raw_fd(),
                events: nix::libc::POLLIN,
                revents: 0,
            },
            nix::libc::pollfd {
                fd: departure.as_raw_fd(),
                events: disconnected,
                revents: 0,
            },
        ];
        // Safety: both poll descriptors remain open for this call.
        if unsafe { nix::libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            let _ = end(child);
            return Err(error);
        }
        // Completion wins a simultaneous disconnect. Normal background work
        // retains its lifetime; only departure starts group cleanup.
        if fds[0].revents != 0 {
            return child.wait();
        }
        if fds[1].revents & disconnected != 0 {
            break;
        }
    }

    end(child)
}

fn end(child: &mut Child) -> io::Result<ExitStatus> {
    // Keep the leader unreaped through escalation, pinning its group id even
    // when TERM ends the leader before its remaining group members.
    let group = Pid::from_raw(child.id() as i32);
    let term = signal(group, Signal::SIGTERM);
    let deadline = Instant::now() + Duration::from_secs(2);
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match poll(
            &mut [],
            remaining.as_millis().clamp(1, u16::MAX as u128) as u16,
        ) {
            Ok(_) | Err(nix::errno::Errno::EINTR) => {}
            Err(_) => {
                let _ = signal(group, Signal::SIGKILL);
                return child.wait();
            }
        }
    }
    let kill = signal(group, Signal::SIGKILL);
    let status = child.wait();
    term?;
    kill?;
    status
}

fn signal(group: Pid, signal: Signal) -> io::Result<()> {
    match killpg(group, signal) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
