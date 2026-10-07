//! Nonblocking subprocess pipes with absolute deadlines and cancellation.

use std::io;
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, poll};

const CANCEL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn nonblocking(pipe: &impl AsFd, operation: &str) -> io::Result<()> {
    let flags = fcntl(pipe, FcntlArg::F_GETFL)
        .map_err(|_| io::Error::other(format!("{operation} pipe configuration failed")))?;
    fcntl(
        pipe,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map_err(|_| io::Error::other(format!("{operation} pipe configuration failed")))?;
    Ok(())
}

pub(super) fn remaining(
    deadline: Instant,
    cancel: Option<&AtomicBool>,
    operation: &str,
) -> io::Result<Duration> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!("{operation} cancelled"),
        ));
    }
    deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{operation} deadline exceeded"),
            )
        })
}

/// With no fd, wait for the next cancellation check or deadline.
pub(super) fn wait_ready(
    fd: Option<std::os::fd::BorrowedFd<'_>>,
    events: PollFlags,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
    operation: &str,
) -> io::Result<()> {
    loop {
        let timeout = remaining(deadline, cancel, operation)?.min(CANCEL_INTERVAL);
        let mut fds: Vec<_> = fd.into_iter().map(|fd| PollFd::new(fd, events)).collect();
        let millis = timeout.as_millis().max(1) as u16;
        match poll(&mut fds, millis) {
            Ok(n) if n > 0 || fd.is_none() => return Ok(()),
            Ok(_) | Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return Err(io::Error::other(format!("{operation} pipe wait failed"))),
        }
    }
}
