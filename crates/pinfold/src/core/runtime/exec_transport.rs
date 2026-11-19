//! Framed command stdio shared by vsock and Apple's native exec pipes.

use crate::core::process::StdioRelay;
use crate::core::runtime::terminal::{Raw, Winch, window};
use crate::formatting::terminal_text;
use crate::init::exec_wire::{
    EXEC, EXIT, FAILED, RESIZE, STDERR, STDIN, STDIN_END, STDOUT, write_frame,
};
use nix::libc;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) fn put(payload: &mut Vec<u8>, bytes: &[u8]) {
    payload.extend((bytes.len() as u32).to_be_bytes());
    payload.extend(bytes);
}

/// An ENV frame's payload.
pub(crate) fn env_payload(vars: &[(OsString, OsString)]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend((vars.len() as u32).to_be_bytes());
    for (name, value) in vars {
        put(&mut payload, name.as_bytes());
        put(&mut payload, value.as_bytes());
    }
    payload
}

/// Run `argv` in the box and relay stdio until it exits:
/// its exit code, or `128+n` for signal `n`. The client stays in pinfold's
/// process.
pub fn exec(
    connection: UnixStream,
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
    let stdout = StdioRelay::new(1, false)?;
    let stderr = StdioRelay::new(2, false)?;
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

    let result = relay(connection, context, stdin, stdout, stderr, winch.as_ref());
    drop((winch, raw));
    result
}

fn relay(
    mut connection: UnixStream,
    context: &crate::core::runtime::ExecContext<'_>,
    mut stdin: StdioRelay,
    mut stdout: StdioRelay,
    mut stderr: StdioRelay,
    winch: Option<&Winch>,
) -> io::Result<i32> {
    use crate::init::exec_wire::{CHUNK, CREDIT, FrameReader, FrameWriter, OUTPUT_END, WINDOW};
    use std::collections::VecDeque;
    let mut reader = FrameReader::default();
    let mut writer = FrameWriter::default();
    let mut output: [VecDeque<u8>; 2] = std::array::from_fn(|_| VecDeque::new());
    let mut closed = [false; 2];
    let mut close_pending = [false; 2];
    let mut credit_pending = [0u32; 2];
    let mut next_credit = 0usize;
    let mut credit_turn = true;
    let mut input_credit = WINDOW;
    let mut input = Vec::new();
    let mut input_open = true;
    let mut input_end = false;
    let mut resize: Option<(u16, u16)> = None;
    let mut completed = None;
    let mut deadline: Option<Instant> = None;
    let mut connected = true;
    let mut writable = true;
    let mut send_failure = None;
    let result = (|| -> io::Result<i32> {
        loop {
            // Read control and status even while the caller's output is full.
            for _ in 0..16 {
                if !connected {
                    break;
                }
                let frame = match reader.read(&mut connection) {
                    Ok(Some(frame)) => frame,
                    Ok(None) => break,
                    Err(_) if completed.is_some() => {
                        connected = false;
                        break;
                    }
                    Err(error) => {
                        return Err(send_failure.take().unwrap_or_else(|| {
                            io::Error::other(format!(
                                "the box ended the exec before it exited: {error}"
                            ))
                        }));
                    }
                };
                let (kind, payload) = frame;
                match kind {
                    STDOUT | STDERR => {
                        let index = usize::from(kind == STDERR);
                        if !closed[index] {
                            if payload.len() > WINDOW - output[index].len() {
                                return Err(io::Error::other("output exceeds credit"));
                            }
                            output[index].extend(payload);
                        }
                    }
                    CREDIT if payload.len() == 5 && payload[0] == STDIN => {
                        let count = u32::from_be_bytes(payload[1..].try_into().unwrap()) as usize;
                        if count > WINDOW - input_credit {
                            return Err(io::Error::other("invalid input credit"));
                        }
                        input_credit += count;
                    }
                    EXIT if payload.len() == 4 => {
                        if completed.is_some() {
                            return Err(io::Error::other("duplicate exit"));
                        }
                        completed = Some(i32::from_be_bytes(payload.try_into().unwrap()));
                        deadline = Some(Instant::now() + Duration::from_secs(1));
                        input_open = false;
                        input.clear();
                        stdin.close();
                    }
                    FAILED => {
                        return Err(io::Error::other(terminal_text(&String::from_utf8_lossy(
                            &payload,
                        ))));
                    }
                    _ => return Err(io::Error::other(format!("the box sent frame {kind}"))),
                }
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return completed.ok_or_else(|| send_failure.take().expect("failure deadline"));
            }
            if cancelled(context.cancel).is_err() {
                return completed
                    .ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "exec cancelled"));
            }
            let relays = [&mut stdout, &mut stderr];
            for (index, relay) in relays.into_iter().enumerate() {
                if closed[index] {
                    continue;
                }
                let (bytes, _) = output[index].as_slices();
                if !bytes.is_empty() {
                    let count = unsafe {
                        libc::write(relay.as_raw_fd(), bytes.as_ptr().cast(), bytes.len())
                    };
                    if count > 0 {
                        output[index].drain(..count as usize);
                        credit_pending[index] += count as u32;
                    } else if count < 0 {
                        let error = io::Error::last_os_error();
                        match error.kind() {
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => {}
                            io::ErrorKind::BrokenPipe if !context.tty => {
                                relay.close();
                                closed[index] = true;
                                close_pending[index] = true;
                                output[index].clear();
                                credit_pending[index] = 0;
                            }
                            _ => return Err(error),
                        }
                    }
                }
            }
            if let Some(code) = completed
                && !connected
                && output.iter().all(VecDeque::is_empty)
            {
                stdout.close();
                stderr.close();
                if stdout.finished()? && stderr.finished()? {
                    return Ok(code);
                }
            }
            let credit_index = [next_credit, 1 - next_credit]
                .into_iter()
                .find(|&index| credit_pending[index] != 0);
            if connected && writable && !writer.pending() {
                if let Some(index) = close_pending.iter().position(|pending| *pending) {
                    writer.start(OUTPUT_END, &[if index == 0 { STDOUT } else { STDERR }])?;
                    close_pending[index] = false;
                } else if let Some((rows, cols)) = resize.take() {
                    let mut bytes = rows.to_be_bytes().to_vec();
                    bytes.extend(cols.to_be_bytes());
                    writer.start(RESIZE, &bytes)?;
                } else if input_end && input.is_empty() {
                    writer.start(STDIN_END, &[])?;
                    input_end = false;
                } else if let Some(index) = credit_index
                    && (credit_turn || input.is_empty())
                {
                    let mut bytes = vec![if index == 0 { STDOUT } else { STDERR }];
                    bytes.extend(credit_pending[index].to_be_bytes());
                    writer.start(CREDIT, &bytes)?;
                    credit_pending[index] = 0;
                    next_credit = 1 - index;
                    credit_turn = false;
                } else if !input.is_empty() {
                    writer.start(STDIN, &input)?;
                    input.clear();
                    credit_turn = true;
                }
            }
            if connected
                && writable
                && writer.pending()
                && let Err(error) = writer.flush(&mut connection)
            {
                // A guest can close input after sending EXIT, before it reaches us.
                writable = false;
                input_open = false;
                input.clear();
                stdin.close();
                writer = FrameWriter::default();
                send_failure = Some(error);
                deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(1));
            }
            let mut fds = [
                libc::pollfd {
                    fd: if connected {
                        connection.as_raw_fd()
                    } else {
                        -1
                    },
                    events: libc::POLLIN
                        | if writable && writer.pending() {
                            libc::POLLOUT
                        } else {
                            0
                        },
                    revents: 0,
                },
                libc::pollfd {
                    fd: if input_open && input.is_empty() && input_credit != 0 {
                        stdin.as_raw_fd()
                    } else {
                        -1
                    },
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: if !closed[0] && !output[0].is_empty() {
                        stdout.as_raw_fd()
                    } else {
                        -1
                    },
                    events: libc::POLLOUT,
                    revents: 0,
                },
                libc::pollfd {
                    fd: if !closed[1] && !output[1].is_empty() {
                        stderr.as_raw_fd()
                    } else {
                        -1
                    },
                    events: libc::POLLOUT,
                    revents: 0,
                },
                libc::pollfd {
                    fd: winch.map_or(-1, |winch| winch.read.as_raw_fd()),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let timeout = deadline.map_or(100, |end| {
                end.saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(100) as i32
            });
            let polled = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
            if polled < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if fds[4].revents != 0 {
                let _ = (&winch.expect("watched window").read).read(&mut [0; 64]);
                resize = window()?;
            }
            if fds[1].revents != 0 {
                let mut bytes = [0; CHUNK];
                match nix::unistd::read(stdin.as_fd(), &mut bytes[..CHUNK.min(input_credit)]) {
                    Ok(0) => {
                        input_open = false;
                        input_end = true;
                        stdin.close();
                    }
                    Ok(count) => {
                        input.extend(&bytes[..count]);
                        input_credit -= count;
                    }
                    Err(nix::errno::Errno::EAGAIN | nix::errno::Errno::EINTR) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
    })();
    // Once EXIT was observed, later drain or terminal-query errors cannot replace it.
    result.or_else(|error| completed.ok_or(error))
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
