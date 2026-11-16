//! Native exec uses private endpoints; pinfold alone holds caller stdio.

use crate::core::process::{CommandGuard, StdioRelay};

use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::libc;

use crate::core::runtime::terminal::{Raw, Winch, window};

const EXIT_DRAIN: Duration = Duration::from_secs(1);

pub(super) fn exec(
    mut command: Command,
    size: Option<(u16, u16)>,
    tty: bool,
    cancel: Option<&AtomicBool>,
) -> io::Result<ExitStatus> {
    let cancelled = || cancel.is_some_and(|flag| flag.load(Ordering::SeqCst));
    let interrupted = || io::Error::new(io::ErrorKind::Interrupted, "terminal execution cancelled");
    if cancelled() {
        return Err(interrupted());
    }
    let private = tty.then(|| private_pty(size)).transpose()?;
    let winch = tty.then(Winch::install).transpose()?;
    let _raw = if tty { Raw::enter()? } else { None };
    let mut caller = CallerStdio::new()?;
    let mut status = None;
    let result = (|| {
        let master = if let Some((master, slave)) = private {
            command
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave));
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
            Some(master)
        } else {
            command.stdin(Stdio::piped()).stdout(Stdio::piped());
            None
        };
        command.stderr(Stdio::piped());
        if cancelled() {
            return Err(interrupted());
        }
        let mut client = Client(
            command
                .spawn_guarded()
                .map_err(|error| super::spawn_error("container", error))?,
            Vec::new(),
        );
        drop(command);
        let (mut input_fd, output_fd) = if let Some(master) = master {
            let fd = master.as_raw_fd();
            client.1.push(master);
            (fd, fd)
        } else {
            let input = OwnedFd::from(client.0.stdin.take().expect("native stdin is piped"));
            let output = OwnedFd::from(client.0.stdout.take().expect("native stdout is piped"));
            let fds = (input.as_raw_fd(), output.as_raw_fd());
            client.1.extend([input, output]);
            fds
        };
        let stderr = OwnedFd::from(client.0.stderr.take().expect("native stderr is piped"));
        let error_fd = stderr.as_raw_fd();
        client.1.push(stderr);
        for endpoint in &client.1 {
            crate::core::pipe::nonblocking(endpoint, "native exec")?;
        }
        let mut input = Buffer::new();
        let mut output = Buffer::new();
        let mut errors = Buffer::new();
        let (mut stdin_open, mut output_open, mut stderr_open) = (true, true, true);
        let mut drain = None;
        loop {
            if status.is_none() {
                status = client.0.try_wait()?;
                if status.is_some() {
                    drain = Some(Instant::now() + EXIT_DRAIN);
                    stdin_open = false;
                    input.discard();
                    if !tty {
                        client.close(input_fd);
                    }
                    input_fd = -1;
                }
            }
            if cancelled() {
                return status.map_or_else(|| Err(interrupted()), Ok);
            }
            if drain.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(status.expect("drain follows observed exit"));
            }
            if !output_open && output.empty() {
                caller.close(1);
            }
            if !stderr_open && errors.empty() {
                caller.close(2);
            }
            if let Some(status) = status
                && !output_open
                && !stderr_open
                && output.empty()
                && errors.empty()
                && caller.outputs_finished()?
            {
                return Ok(status);
            }
            let mut fds = [
                pollfd(
                    caller.fds[0],
                    stdin_open && input_fd >= 0 && (!tty || output_open) && input.empty(),
                    libc::POLLIN,
                ),
                pollfd(output_fd, output_open && output.empty(), libc::POLLIN),
                pollfd(error_fd, stderr_open && errors.empty(), libc::POLLIN),
                pollfd(input_fd, input_fd >= 0 && !input.empty(), libc::POLLOUT),
                pollfd(caller.fds[1], !output.empty(), libc::POLLOUT),
                pollfd(caller.fds[2], !errors.empty(), libc::POLLOUT),
                pollfd(
                    winch.as_ref().map_or(-1, |winch| winch.read.as_raw_fd()),
                    winch.is_some(),
                    libc::POLLIN,
                ),
            ];
            // A finite poll lets cancellation interrupt even a full host output pipe.
            let timeout = drain.map_or(100, |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(100) as i32
            });
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if let Some(winch) = winch.as_ref().filter(|_| fds[6].revents != 0) {
                let mut bytes = [0; 64];
                unsafe {
                    libc::read(
                        winch.read.as_raw_fd(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                    )
                };
                if let Some(size) = window()? {
                    resize(output_fd, size)?;
                }
            }
            if fds[0].revents != 0 {
                stdin_open = input.read(caller.fds[0])?;
            }
            if fds[1].revents != 0 {
                output_open = output.read(output_fd)?;
            }
            if fds[2].revents != 0 {
                stderr_open = errors.read(error_fd)?;
            }
            if fds[3].revents != 0
                && let Err(error) = input.write(input_fd)
            {
                if matches!(error.raw_os_error(), Some(libc::EPIPE | libc::EIO)) {
                    stdin_open = false;
                    input.discard();
                    if !tty {
                        client.close(input_fd);
                    }
                    input_fd = -1;
                } else {
                    return Err(error);
                }
            }
            if fds[4].revents != 0
                && let Err(error) = output.write(caller.fds[1])
            {
                if !tty && error.raw_os_error() == Some(libc::EPIPE) {
                    output.discard();
                    output_open = false;
                    client.close(output_fd);
                } else {
                    return Err(error);
                }
            }
            if fds[5].revents != 0
                && let Err(error) = errors.write(caller.fds[2])
            {
                if !tty && error.raw_os_error() == Some(libc::EPIPE) {
                    errors.discard();
                    stderr_open = false;
                    client.close(error_fd);
                } else {
                    return Err(error);
                }
            }
            if tty && !output_open {
                stdin_open = false;
                input.discard();
                input_fd = -1;
            } else if !tty && !stdin_open && input.empty() && input_fd >= 0 {
                client.close(input_fd);
                input_fd = -1;
            }
        }
    })();
    match (result, status) {
        (Err(error), Some(status)) => {
            if error.kind() != io::ErrorKind::Interrupted {
                notice(format_args!("terminal transport: {error}"));
            }
            Ok(status)
        }
        (result, _) => result,
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
    // CLOEXEC is part of each open, including before another thread can fork.
    let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if master < 0 {
        return Err(io::Error::last_os_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    if unsafe { libc::grantpt(master.as_raw_fd()) } < 0
        || unsafe { libc::unlockpt(master.as_raw_fd()) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    // Darwin has no ptsname_r; the ioctl fills a caller-owned name buffer.
    let mut name = [0 as libc::c_char; 128];
    if unsafe {
        libc::ioctl(
            master.as_raw_fd(),
            libc::TIOCPTYGNAME as _,
            name.as_mut_ptr(),
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let slave = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if slave < 0 {
        return Err(io::Error::last_os_error());
    }
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let mut raw: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut raw) } < 0 {
        return Err(io::Error::last_os_error());
    }
    unsafe { libc::cfmakeraw(&mut raw) };
    // Early input must not echo, transform, or signal the private foreground group.
    if unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &raw) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if let Some(size) = size {
        resize(master.as_raw_fd(), size)?;
    }
    Ok((master, slave))
}

struct Client(Child, Vec<OwnedFd>);

impl Client {
    fn close(&mut self, fd: RawFd) {
        self.1.retain(|endpoint| endpoint.as_raw_fd() != fd);
    }
}

fn notice(message: impl std::fmt::Display) {
    // The caller may have made stderr nonblocking or closed it.
    let _ = writeln!(io::stderr().lock(), "pinfold: {message}");
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.0.kill();
        // Closing the master releases a native client draining its slave on exit.
        self.1.clear();
        let _ = self.0.wait();
    }
}

struct CallerStdio {
    fds: [RawFd; 3],
    relays: [Option<StdioRelay>; 3],
}

impl CallerStdio {
    fn new() -> io::Result<Self> {
        let mut caller = Self {
            fds: [0, 1, 2],
            relays: [None, None, None],
        };
        for index in 0..3 {
            let relay = StdioRelay::new(index as RawFd, index == 0)?;
            caller.fds[index] = relay.as_raw_fd();
            caller.relays[index] = Some(relay);
        }
        Ok(caller)
    }

    fn close(&mut self, index: usize) {
        if let Some(relay) = self.relays[index].as_mut() {
            relay.close();
            self.fds[index] = -1;
        }
    }

    fn outputs_finished(&mut self) -> io::Result<bool> {
        let mut finished = true;
        for relay in self.relays[1..].iter_mut().flatten() {
            finished &= relay.finished()?;
        }
        Ok(finished)
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

    fn discard(&mut self) {
        self.start = self.end;
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
