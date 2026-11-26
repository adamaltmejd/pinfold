//! Retain one command's identity through unprivileged group cleanup.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

pub(super) struct Session {
    child: Option<Child>,
    process: OwnedFd,
    deadline: Option<Instant>,
}

/// A poll timeout until `deadline`; -1 for none.
pub(super) fn millis(deadline: Option<Instant>) -> i32 {
    deadline.map_or(-1, |deadline| {
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(i32::MAX as u128) as i32
    })
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
            child: Some(child),
            process,
            deadline: None,
        })
    }

    pub(super) fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("unreaped command")
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
        let result = if let Some(deadline) = self.deadline {
            if Instant::now() < deadline {
                return Ok(None);
            }
            // Keep the leader unreaped through escalation, pinning its group id.
            self.signal(Signal::SIGKILL)?;
            self.child_mut().wait().map(Some)
        } else {
            self.child_mut().try_wait()
        };
        if matches!(result, Ok(Some(_)))
            || result
                .as_ref()
                .is_err_and(|error| error.raw_os_error() == Some(nix::libc::ECHILD))
        {
            self.child.take();
        }
        result
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn signal(&self, signal: Signal) -> io::Result<()> {
        match killpg(
            Pid::from_raw(self.child.as_ref().expect("unreaped command").id() as i32),
            signal,
        ) {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        // Relay-owned stdio, including the PTY master, drops before this guard.
        let _ = self.depart();
        if let Some(deadline) = self.deadline {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
        let _ = self.signal(Signal::SIGKILL);
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}
