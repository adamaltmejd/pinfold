//! Framed command stdio shared by vsock and Apple's native exec pipes.

use crate::core::process::StdioRelay;
use crate::core::runtime::terminal::{Raw, Winch, window};
use crate::init::exec_wire::{
    EXEC, EXIT, FAILED, RESIZE, STDERR, STDIN, STDIN_END, STDOUT, read_frame, write_frame,
};
use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) fn put(payload: &mut Vec<u8>, bytes: &[u8]) {
    payload.extend((bytes.len() as u32).to_be_bytes());
    payload.extend(bytes);
}

/// Run `argv` in the box and relay stdio until it exits:
/// its exit code, or `128+n` for signal `n`. The client stays in pinfold's
/// process.
pub fn exec(
    mut connection: UnixStream,
    context: &crate::core::runtime::ExecContext<'_>,
    argv: &[String],
) -> io::Result<i32> {
    let cancel = context.cancel;
    cancelled(cancel)?;
    let tty = context.tty;
    connection.set_nonblocking(true)?;
    let mut metadata = crate::init::ExecMetadata {
        agent: context.agent.map(str::to_string),
        project_directory: context.project_directory()?,
        ..Default::default()
    };
    if tty {
        metadata.window = window()?;
        metadata.term = std::env::var_os("TERM").map(|value| value.as_bytes().to_vec());
        metadata.colorterm = std::env::var_os("COLORTERM").map(|value| value.as_bytes().to_vec());
    }
    let mut payload = Vec::new();
    payload.extend((argv.len() as u32).to_be_bytes());
    for arg in argv {
        put(&mut payload, arg.as_bytes());
    }
    put(
        &mut payload,
        context
            .cwd
            .map_or(&[][..], |dir| dir.as_os_str().as_bytes()),
    );
    payload.push(u8::from(tty));
    put(&mut payload, metadata.encode().as_bytes());
    // Terminal setup must finish before EXEC can start the command.
    let raw = if tty { Raw::enter()? } else { None };
    let winch = if tty { Some(Winch::install()?) } else { None };
    let stdin = StdioRelay::new(0, true)?;
    let mut stdout = StdioRelay::new(1, false)?;
    let mut stderr = StdioRelay::new(2, false)?;
    write_frame(
        &mut Cancellable {
            connection: &connection,
            stop: None,
            cancel,
            completed_first: true,
            deadline: None,
        },
        EXEC,
        &payload,
    )?;

    let (stop, stopping) = crate::core::process::pipe()?;
    let result = std::thread::scope(|scope| {
        let input_connection = connection.try_clone()?;
        let winch = match &winch {
            Some(winch) => Some(winch.read.try_clone()?),
            None => None,
        };
        let input = std::thread::Builder::new()
            .name("exec-input".into())
            .spawn_scoped(scope, move || {
                forward_input(
                    input_connection,
                    &stop,
                    winch.as_ref(),
                    stdin.as_fd(),
                    cancel,
                )
            })?;
        let result = relay_output(
            &mut connection,
            stdout.as_raw_fd(),
            stderr.as_raw_fd(),
            cancel,
        );
        drop(stopping);
        let joined = input.join();
        if joined.is_err() {
            crate::core::process::diagnostic("exec input worker failed");
        }
        result
    });
    let drained = (|| -> io::Result<()> {
        if result.is_ok() {
            stdout.close();
            stderr.close();
            let deadline = Instant::now() + Duration::from_secs(1);
            while !stdout.finished()? || !stderr.finished()? {
                if Instant::now() >= deadline || cancelled(cancel).is_err() {
                    break;
                }
                match nix::poll::poll(&mut [], 10u16) {
                    Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    })();
    drop((winch, raw));
    if let Err(error) = drained {
        crate::core::process::diagnostic(format_args!("stdio relay: {error}"));
    }
    result
}

/// Copy the guest's output to pinfold's own until EXIT.
fn relay_output(
    connection: &mut UnixStream,
    stdout: libc::c_int,
    stderr: libc::c_int,
    cancel: Option<&AtomicBool>,
) -> io::Result<i32> {
    let mut connection = Cancellable {
        connection,
        stop: None,
        cancel,
        completed_first: true,
        deadline: None,
    };
    let mut completed = None;
    loop {
        let (kind, payload) = match read_frame(&mut connection) {
            Ok(frame) => frame,
            Err(_) if completed.is_some() => return Ok(completed.expect("observed exit")),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                return Err(io::Error::other(format!(
                    "the box ended the exec before it exited: {error}"
                )));
            }
        };
        let forwarded = match kind {
            STDOUT => output(stdout, &payload, cancel, connection.deadline),
            STDERR => output(stderr, &payload, cancel, connection.deadline),
            EXIT => {
                let code: [u8; 4] = payload
                    .try_into()
                    .map_err(|_| io::Error::other("the box's EXIT has the wrong length"))?;
                completed = Some(i32::from_be_bytes(code));
                connection.deadline = Some(Instant::now() + Duration::from_secs(1));
                Ok(())
            }
            FAILED => Err(io::Error::other(
                String::from_utf8_lossy(&payload).into_owned(),
            )),
            _ => Err(io::Error::other(format!("the box sent frame {kind}"))),
        };
        if let Err(error) = forwarded.and_then(|()| cancelled(cancel)) {
            return completed.ok_or(error);
        }
    }
}

/// Forward stdin as STDIN frames and its end as STDIN_END, and each window
/// change as RESIZE, until `stop` reads end of file.
fn forward_input(
    connection: UnixStream,
    stop: &io::PipeReader,
    winch: Option<&io::PipeReader>,
    stdin: BorrowedFd<'_>,
    cancel: Option<&AtomicBool>,
) {
    let mut open = true;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        if cancelled(cancel).is_err() {
            return;
        }
        let mut fds = vec![PollFd::new(stop.as_fd(), PollFlags::POLLIN)];
        if let Some(winch) = winch {
            fds.push(PollFd::new(winch.as_fd(), PollFlags::POLLIN));
        }
        if open {
            fds.push(PollFd::new(stdin, PollFlags::POLLIN));
        }
        match poll(&mut fds, PollTimeout::from(100u16)) {
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return,
        }
        let ready: Vec<bool> = fds
            .iter()
            .map(|fd| fd.revents().is_some_and(|events| !events.is_empty()))
            .collect();
        if ready[0] {
            return;
        }
        if let Some(winch) = winch.filter(|_| ready[1]) {
            let _ = (&*winch).read(&mut [0; 64]);
            if let Ok(Some((rows, cols))) = window() {
                let mut payload = rows.to_be_bytes().to_vec();
                payload.extend(cols.to_be_bytes());
                if write_frame(
                    &mut Cancellable {
                        connection: &connection,
                        stop: Some(stop),
                        cancel,
                        completed_first: true,
                        deadline: None,
                    },
                    RESIZE,
                    &payload,
                )
                .is_err()
                {
                    return;
                }
            }
        }
        if open && *ready.last().unwrap() {
            let mut writer = Cancellable {
                connection: &connection,
                stop: Some(stop),
                cancel,
                completed_first: true,
                deadline: None,
            };
            let sent = match read_fd(stdin, &mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Ok(0) | Err(_) => {
                    open = false;
                    write_frame(&mut writer, STDIN_END, &[])
                }
                Ok(size) => write_frame(&mut writer, STDIN, &buffer[..size]),
            };
            if sent.is_err() {
                return;
            }
        }
    }
}

fn read_fd(fd: BorrowedFd, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match nix::unistd::read(fd, buffer) {
            Err(nix::errno::Errno::EINTR) => continue,
            result => return result.map_err(io::Error::from),
        }
    }
}

pub(super) fn cancelled(cancel: Option<&AtomicBool>) -> io::Result<()> {
    if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        Err(io::Error::new(io::ErrorKind::Interrupted, "exec cancelled"))
    } else {
        Ok(())
    }
}

fn wait(
    fd: libc::c_int,
    events: libc::c_short,
    stop: Option<&io::PipeReader>,
    cancel: Option<&AtomicBool>,
    deadline: Option<Instant>,
) -> io::Result<()> {
    loop {
        cancelled(cancel)?;
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut fds = [
            libc::pollfd {
                fd,
                events,
                revents: 0,
            },
            libc::pollfd {
                fd: stop.map_or(-1, AsRawFd::as_raw_fd),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: poll reads and updates the two live pollfd values.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 100) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if fds[1].revents != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "exec input ended",
            ));
        }
        if fds[0].revents != 0 {
            return Ok(());
        }
    }
}

pub(super) struct Cancellable<'a> {
    connection: &'a UnixStream,
    stop: Option<&'a io::PipeReader>,
    cancel: Option<&'a AtomicBool>,
    completed_first: bool,
    deadline: Option<Instant>,
}

impl<'a> Cancellable<'a> {
    #[cfg(target_os = "linux")]
    pub(super) fn startup(connection: &'a UnixStream, cancel: &'a AtomicBool) -> Self {
        Self {
            connection,
            stop: None,
            cancel: Some(cancel),
            completed_first: false,
            deadline: None,
        }
    }

    fn cancellation(&self, error: io::Error) -> io::Error {
        if !self.completed_first && error.kind() == io::ErrorKind::Interrupted {
            // Buffered copy routines retry Interrupted rather than ending BOOT.
            io::Error::new(io::ErrorKind::ConnectionAborted, "startup cancelled")
        } else {
            error
        }
    }
}

impl Read for Cancellable<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(io::ErrorKind::TimedOut.into());
            }
            if !self.completed_first {
                cancelled(self.cancel).map_err(|error| self.cancellation(error))?;
            }
            // A frame already readable, including EXIT, precedes client cancellation.
            match self.connection.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait(
                        self.connection.as_raw_fd(),
                        libc::POLLIN,
                        self.stop,
                        self.cancel,
                        self.deadline,
                    )
                    .map_err(|error| self.cancellation(error))?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }

    fn read_exact(&mut self, mut buffer: &mut [u8]) -> io::Result<()> {
        // The default retries Interrupted, which is this relay's cancellation.
        while !buffer.is_empty() {
            match self.read(buffer)? {
                0 => return Err(io::ErrorKind::UnexpectedEof.into()),
                read => buffer = &mut buffer[read..],
            }
        }
        Ok(())
    }
}

impl Write for Cancellable<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        loop {
            cancelled(self.cancel).map_err(|error| self.cancellation(error))?;
            match self.connection.write(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait(
                        self.connection.as_raw_fd(),
                        libc::POLLOUT,
                        self.stop,
                        self.cancel,
                        self.deadline,
                    )
                    .map_err(|error| self.cancellation(error))?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn write_all(&mut self, mut buffer: &[u8]) -> io::Result<()> {
        // Cancellation must end the frame rather than restart its write.
        while !buffer.is_empty() {
            match self.write(buffer)? {
                0 => return Err(io::ErrorKind::WriteZero.into()),
                written => buffer = &buffer[written..],
            }
        }
        Ok(())
    }
}

fn output(
    fd: libc::c_int,
    mut bytes: &[u8],
    cancel: Option<&AtomicBool>,
    deadline: Option<Instant>,
) -> io::Result<()> {
    while !bytes.is_empty() {
        cancelled(cancel)?;
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(io::ErrorKind::TimedOut.into());
        }
        // SAFETY: write reads from the live byte slice and does not retain it.
        let written = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if written > 0 {
            bytes = &bytes[written as usize..];
            continue;
        }
        if written == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.kind() {
            io::ErrorKind::Interrupted => {}
            io::ErrorKind::WouldBlock => wait(fd, libc::POLLOUT, None, cancel, deadline)?,
            // A closed host stream must not discard the eventual command status.
            _ => return Ok(()),
        }
    }
    Ok(())
}
