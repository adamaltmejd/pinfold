//! Retain one command's identity through unprivileged group cleanup.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

pub(super) struct Session {
    child: Child,
    process: OwnedFd,
    deadline: Option<Instant>,
}

impl Session {
    pub(super) fn new(mut child: Child) -> io::Result<Self> {
        let process = match super::firecracker::pidfd_open(child.id() as i32) {
            Ok(process) => process,
            Err(error) => {
                let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            child,
            process,
            deadline: None,
        })
    }

    pub(super) fn fd(&self) -> i32 {
        if self.deadline.is_some() {
            -1
        } else {
            self.process.as_raw_fd()
        }
    }

    /// The caller closes its sole PTY master before starting group cleanup.
    pub(super) fn depart(&mut self) -> io::Result<()> {
        if self.deadline.is_none() {
            self.deadline = Some(Instant::now() + Duration::from_secs(2));
            self.signal(Signal::SIGTERM)?;
        }
        Ok(())
    }

    pub(super) fn status(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(deadline) = self.deadline {
            if Instant::now() < deadline {
                return Ok(None);
            }
            // Keep the leader unreaped through escalation, pinning its group id.
            self.signal(Signal::SIGKILL)?;
            return self.child.wait().map(Some);
        }
        self.child.try_wait()
    }

    pub(super) fn timeout(&self) -> i32 {
        self.deadline.map_or(-1, |deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(i32::MAX as u128) as i32
        })
    }

    fn signal(&self, signal: Signal) -> io::Result<()> {
        match killpg(Pid::from_raw(self.child.id() as i32), signal) {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
