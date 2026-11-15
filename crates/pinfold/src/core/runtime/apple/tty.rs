//! Apple's native client owns only this private terminal, never the caller's.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use nix::libc;

use crate::core::runtime::terminal::{Raw, Winch, window};

pub(super) fn exec(
    mut command: Command,
    size: Option<(u16, u16)>,
    cancel: Option<&AtomicBool>,
) -> io::Result<ExitStatus> {
    let cancelled = || cancel.is_some_and(|flag| flag.load(Ordering::Acquire));
    let interrupted = || io::Error::new(io::ErrorKind::Interrupted, "terminal execution cancelled");
    if cancelled() {
        return Err(interrupted());
    }
    let (master, slave) = private_pty(size)?;
    let winch = Winch::install()?;
    let _raw = Raw::enter()?;
    crate::core::pipe::nonblocking(&master, "private terminal")?;
    let master_fd = master.as_raw_fd();
    let mut flags = Flags::nonblocking(&[0, 1, 2])?;
    let mut status = None;
    let result = (|| {
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave))
            .stderr(Stdio::piped());
        // SAFETY: only async-signal-safe syscalls run between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0
                    || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0
                    || libc::tcsetpgrp(0, libc::getpgrp()) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        if cancelled() {
            return Err(interrupted());
        }
        let mut client = Client(
            command
                .spawn()
                .map_err(|error| super::spawn_error("container", error))?,
            Some(master),
        );
        drop(command);
        let stderr = client.0.stderr.take().expect("native stderr is piped");
        crate::core::pipe::nonblocking(&stderr, "native stderr")?;
        let mut input = Buffer::new();
        let mut output = Buffer::new();
        let mut errors = Buffer::new();
        let (mut stdin_open, mut master_open, mut stderr_open) = (true, true, true);
        loop {
            if cancelled() {
                return Err(interrupted());
            }
            if status.is_none() {
                status = client.0.try_wait()?;
            }
            if let Some(status) = status
                && !master_open
                && !stderr_open
                && output.empty()
                && errors.empty()
            {
                return Ok(status);
            }
            let mut fds = [
                pollfd(0, stdin_open && master_open && input.empty(), libc::POLLIN),
                pollfd(master_fd, master_open && output.empty(), libc::POLLIN),
                pollfd(
                    stderr.as_raw_fd(),
                    stderr_open && errors.empty(),
                    libc::POLLIN,
                ),
                pollfd(master_fd, master_open && !input.empty(), libc::POLLOUT),
                pollfd(1, !output.empty(), libc::POLLOUT),
                pollfd(2, !errors.empty(), libc::POLLOUT),
                pollfd(winch.read.as_raw_fd(), true, libc::POLLIN),
            ];
            // A finite poll lets cancellation interrupt even a full host output pipe.
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 100) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if fds[6].revents != 0 {
                let mut bytes = [0; 64];
                unsafe {
                    libc::read(
                        winch.read.as_raw_fd(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                    )
                };
                if let Some(size) = window()? {
                    resize(master_fd, size)?;
                }
            }
            if fds[0].revents != 0 {
                stdin_open = input.read(0)?;
            }
            if fds[1].revents != 0 {
                master_open = output.read(master_fd)?;
            }
            if fds[2].revents != 0 {
                stderr_open = errors.read(stderr.as_raw_fd())?;
            }
            if fds[3].revents != 0 {
                input.write(master_fd)?;
            }
            if fds[4].revents != 0 {
                output.write(1)?;
            }
            if fds[5].revents != 0 {
                errors.write(2)?;
            }
        }
    })();
    // The client is reaped before restoring shared caller file flags.
    let restored = flags.restore();
    let result = match (result, status) {
        (Err(error), Some(status)) if !status.success() => {
            if error.kind() != io::ErrorKind::Interrupted {
                eprintln!("pinfold: terminal transport: {error}");
            }
            Ok(status)
        }
        (result, _) => result,
    };
    match restored {
        Ok(()) => result,
        Err(error) => {
            let error = io::Error::other(format!("restore terminal file flags: {error}"));
            if result.as_ref().is_ok_and(ExitStatus::success) {
                Err(error)
            } else {
                eprintln!("pinfold: {error}");
                result
            }
        }
    }
}

fn pollfd(fd: RawFd, enabled: bool, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd: if enabled { fd } else { -1 },
        events,
        revents: 0,
    }
}

fn resize(fd: RawFd, (rows, cols): (u16, u16)) -> io::Result<()> {
    let size = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &size) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn private_pty(size: Option<(u16, u16)>) -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: openpty fills two descriptors; no name or inherited termios is requested.
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&master, &slave] {
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    if let Some(size) = size {
        resize(master.as_raw_fd(), size)?;
    }
    Ok((master, slave))
}

struct Client(Child, Option<OwnedFd>);

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.0.kill();
        // Closing the master releases a native client draining its slave on exit.
        self.1.take();
        let _ = self.0.wait();
    }
}

struct Flags(Vec<(RawFd, libc::c_int)>);

impl Flags {
    fn nonblocking(fds: &[RawFd]) -> io::Result<Self> {
        let mut flags = Self(Vec::new());
        // Capture all first: stdin/stdout/stderr may share an open description.
        for &fd in fds {
            let saved = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if saved < 0 {
                return Err(io::Error::last_os_error());
            }
            flags.0.push((fd, saved));
        }
        let configured = flags.0.iter().try_for_each(|&(fd, saved)| {
            if unsafe { libc::fcntl(fd, libc::F_SETFL, saved | libc::O_NONBLOCK) } < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
        if let Err(error) = configured {
            if let Err(restore) = flags.restore() {
                eprintln!("pinfold: restore terminal file flags: {restore}");
            }
            return Err(error);
        }
        Ok(flags)
    }

    fn restore(&mut self) -> io::Result<()> {
        let mut failure = None;
        for (fd, flags) in self.0.drain(..) {
            if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } < 0 && failure.is_none() {
                failure = Some(io::Error::last_os_error());
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for Flags {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

struct Buffer {
    bytes: [u8; 16384],
    start: usize,
    end: usize,
}

impl Buffer {
    fn new() -> Self {
        Self {
            bytes: [0; 16384],
            start: 0,
            end: 0,
        }
    }
    fn empty(&self) -> bool {
        self.start == self.end
    }

    fn read(&mut self, fd: RawFd) -> io::Result<bool> {
        let count = unsafe { libc::read(fd, self.bytes.as_mut_ptr().cast(), self.bytes.len()) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Ok(true);
            }
            // A PTY master reports EIO when its last slave closes.
            if error.raw_os_error() == Some(libc::EIO) {
                return Ok(false);
            }
            return Err(error);
        }
        self.start = 0;
        self.end = count as usize;
        Ok(count != 0)
    }

    fn write(&mut self, fd: RawFd) -> io::Result<()> {
        let bytes = &self.bytes[self.start..self.end];
        let count = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Ok(());
            }
            return Err(error);
        }
        self.start += count as usize;
        Ok(())
    }
}
