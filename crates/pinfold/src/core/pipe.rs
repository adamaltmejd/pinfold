//! Nonblocking subprocess pipes with absolute deadlines and cancellation.

use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, poll};

const CANCEL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn nonblocking(pipe: &impl AsFd, operation: &str) -> io::Result<()> {
    let failed = |_| io::Error::other(format!("{operation} pipe configuration failed"));
    let flags = OFlag::from_bits_truncate(fcntl(pipe, FcntlArg::F_GETFL).map_err(failed)?);
    fcntl(pipe, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).map_err(failed)?;
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

/// Read what a nonblocking `source` has, waiting for it; 0 at its end.
pub(super) fn read(
    source: &mut (impl Read + AsFd),
    buffer: &mut [u8],
    deadline: Instant,
    cancel: Option<&AtomicBool>,
    operation: &str,
) -> io::Result<usize> {
    loop {
        remaining(deadline, cancel, operation)?;
        match source.read(buffer) {
            Ok(read) => return Ok(read),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_ready(
                    Some(source.as_fd()),
                    PollFlags::POLLIN,
                    deadline,
                    cancel,
                    operation,
                )?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(io::Error::other(format!("{operation} output failed"))),
        }
    }
}

/// Write all of `bytes` to a nonblocking `sink`, waiting for room.
pub(super) fn write_all(
    sink: &mut (impl Write + AsFd),
    mut bytes: &[u8],
    deadline: Instant,
    cancel: Option<&AtomicBool>,
    operation: &str,
) -> io::Result<()> {
    while !bytes.is_empty() {
        remaining(deadline, cancel, operation)?;
        match sink.write(bytes) {
            Ok(0) => return Err(io::Error::other(format!("{operation} input closed"))),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_ready(
                    Some(sink.as_fd()),
                    PollFlags::POLLOUT,
                    deadline,
                    cancel,
                    operation,
                )?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(io::Error::other(format!("{operation} input failed"))),
        }
    }
    Ok(())
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
