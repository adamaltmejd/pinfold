//! One nonblocking command relay keeps control independent of command I/O.

use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::net::Shutdown;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::process::Child;
use std::time::{Duration, Instant};

use nix::libc;

use super::exec_wire::{
    CHUNK, CREDIT, EXIT, FrameReader, FrameWriter, OUTPUT_END, RESIZE, STDERR, STDIN, STDIN_END,
    STDOUT, WINDOW,
};
use super::session::Session;

pub(super) fn relay(
    connection: UnixStream,
    child: Child,
    pty: Option<File>,
    project: bool,
    departure: Option<BorrowedFd<'_>>,
) -> io::Result<()> {
    let mut session = Some(Session::new(child)?);
    let child = session.as_mut().expect("supervised command").child_mut();
    let mut input = child.stdin.take().map(OwnedFd::from);
    let mut outputs = [
        child.stdout.take().map(OwnedFd::from),
        child.stderr.take().map(OwnedFd::from),
    ];
    let mut terminal = pty.map(OwnedFd::from);
    for fd in input
        .iter()
        .chain(outputs.iter().flatten())
        .chain(terminal.iter())
    {
        nonblocking(fd)?;
    }
    connection.set_nonblocking(true)?;
    let mut reader = FrameReader::default();
    let mut writer = FrameWriter::default();
    let mut bytes = VecDeque::new();
    let mut stdin_open = true;
    let mut credit = [WINDOW; 2];
    let mut consumed = 0usize;
    let mut next_output = 0usize;
    let mut connected = true;
    let mut status = None;
    let mut exit = None;
    let mut drain = None;
    let socket = departure.is_none();
    let departure = departure.unwrap_or(connection.as_fd());
    let lost = libc::POLLHUP | libc::POLLERR | if socket { libc::POLLRDHUP } else { 0 };

    loop {
        observe(&mut session, &mut status, &mut exit, &mut drain)?;
        flush_control(
            &connection,
            &mut connected,
            &mut writer,
            &mut exit,
            &mut consumed,
        )?;
        if status.is_some() && drain.is_some_and(|deadline| Instant::now() >= deadline) && connected
        {
            disconnect(&connection, &mut connected, &mut writer);
        }
        if status.is_some()
            && !connected
            && terminal.is_none()
            && outputs.iter().all(Option::is_none)
        {
            return Ok(());
        }
        if !connected {
            bytes.clear();
            input.take();
            if project && status.is_none() {
                terminal.take();
                if let Some(session) = &mut session {
                    session.depart()?;
                }
            }
        }
        if status.is_some()
            && connected
            && !writer.pending()
            && exit.is_none()
            && terminal.is_none()
            && outputs.iter().all(Option::is_none)
        {
            disconnect(&connection, &mut connected, &mut writer);
            continue;
        }
        if !stdin_open && bytes.is_empty() {
            input.take();
        }
        let input_fd = terminal
            .as_ref()
            .or(input.as_ref())
            .map_or(-1, AsRawFd::as_raw_fd);
        let output_fds = if let Some(terminal) = &terminal {
            [terminal.as_raw_fd(), -1]
        } else {
            [
                outputs[0].as_ref().map_or(-1, AsRawFd::as_raw_fd),
                outputs[1].as_ref().map_or(-1, AsRawFd::as_raw_fd),
            ]
        };
        let mut fds = [
            fd(
                if connected {
                    connection.as_raw_fd()
                } else {
                    -1
                },
                libc::POLLIN | if writer.pending() { libc::POLLOUT } else { 0 },
            ),
            fd(session.as_ref().map_or(-1, Session::fd), libc::POLLIN),
            fd(
                if connected && project && status.is_none() {
                    departure.as_raw_fd()
                } else {
                    -1
                },
                lost,
            ),
            fd(if !bytes.is_empty() { input_fd } else { -1 }, libc::POLLOUT),
            fd(
                if !connected || (!writer.pending() && credit[0] != 0) {
                    output_fds[0]
                } else {
                    -1
                },
                libc::POLLIN,
            ),
            fd(
                if !connected || (!writer.pending() && credit[1] != 0) {
                    output_fds[1]
                } else {
                    -1
                },
                libc::POLLIN,
            ),
        ];
        let timeout = match (
            session.as_ref().map_or(-1, Session::timeout),
            drain.filter(|_| connected),
        ) {
            (timeout, Some(deadline)) => {
                let drain = deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(i32::MAX as u128) as i32;
                if timeout < 0 {
                    drain
                } else {
                    timeout.min(drain)
                }
            }
            (timeout, None) => timeout,
        };
        // Safety: poll only updates this array of live descriptors.
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        // Observe completion before handling a simultaneously ready departure.
        observe(&mut session, &mut status, &mut exit, &mut drain)?;
        if fds[2].revents & lost != 0 {
            disconnect(&connection, &mut connected, &mut writer);
            continue;
        }
        if connected
            && fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLRDHUP)
                != 0
        {
            for _ in 0..8 {
                let frame = match reader.read(&mut &connection) {
                    Ok(Some(frame)) => frame,
                    Ok(None) => break,
                    Err(_) => {
                        disconnect(&connection, &mut connected, &mut writer);
                        break;
                    }
                };
                match frame {
                    (STDIN, payload)
                        if payload.len() <= CHUNK && bytes.len() + payload.len() <= WINDOW =>
                    {
                        if status.is_none() && (terminal.is_some() || input.is_some()) {
                            bytes.extend(payload);
                        } else {
                            consumed += payload.len();
                        }
                    }
                    (STDIN_END, payload) if payload.is_empty() => stdin_open = false,
                    (RESIZE, payload) if payload.len() == 4 => {
                        if let Some(master) = &terminal {
                            let rows = u16::from_be_bytes([payload[0], payload[1]]);
                            let cols = u16::from_be_bytes([payload[2], payload[3]]);
                            if super::set_window(master, rows, cols).is_err() {
                                disconnect(&connection, &mut connected, &mut writer);
                                break;
                            }
                        }
                    }
                    (CREDIT, payload) if payload.len() == 5 => {
                        let Some(index) = stream(payload[0]) else {
                            disconnect(&connection, &mut connected, &mut writer);
                            break;
                        };
                        let count =
                            u32::from_be_bytes(payload[1..].try_into().expect("credit length"))
                                as usize;
                        if count > WINDOW - credit[index] {
                            disconnect(&connection, &mut connected, &mut writer);
                            break;
                        }
                        credit[index] += count;
                    }
                    (OUTPUT_END, payload) if payload.len() == 1 => {
                        if let Some(index) = stream(payload[0]) {
                            if terminal.is_none() {
                                outputs[index].take();
                            }
                        } else {
                            disconnect(&connection, &mut connected, &mut writer);
                            break;
                        }
                    }
                    _ => {
                        disconnect(&connection, &mut connected, &mut writer);
                        break;
                    }
                }
            }
        }
        if connected && fds[3].revents != 0 && !bytes.is_empty() {
            let bytes_to_write = bytes.make_contiguous();
            // Safety: input_fd is the pipe or sole PTY master held above.
            let count = unsafe {
                libc::write(
                    input_fd,
                    bytes_to_write.as_ptr().cast(),
                    bytes_to_write.len(),
                )
            };
            if count > 0 {
                bytes.drain(..count as usize);
                consumed += count as usize;
            } else if count < 0 {
                let error = io::Error::last_os_error();
                if !matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    consumed += bytes.len();
                    bytes.clear();
                    input.take();
                }
            }
        }
        // Completion or input consumption can queue control after poll too.
        flush_control(
            &connection,
            &mut connected,
            &mut writer,
            &mut exit,
            &mut consumed,
        )?;
        let first_output = next_output;
        for offset in 0..2 {
            let index = (first_output + offset) % 2;
            if fds[index + 4].revents == 0 || (connected && writer.pending()) {
                continue;
            }
            // Control may have closed a reader since the poll snapshot.
            let output = if index == 0 {
                terminal.as_ref().or(outputs[index].as_ref())
            } else {
                outputs[index].as_ref()
            };
            let Some(output) = output else { continue };
            let output_fd = output.as_raw_fd();
            if output_fd != output_fds[index] {
                continue;
            }
            let mut buffer = [0; CHUNK];
            let available = if connected {
                credit[index].min(CHUNK)
            } else {
                CHUNK
            };
            // Safety: poll supplied readiness for this owned output descriptor.
            let count = unsafe { libc::read(output_fd, buffer.as_mut_ptr().cast(), available) };
            if count > 0 {
                next_output = 1 - index;
                if connected {
                    credit[index] -= count as usize;
                    writer.start(
                        if index == 0 { STDOUT } else { STDERR },
                        &buffer[..count as usize],
                    )?;
                }
            } else if count == 0
                || (count < 0
                    && !matches!(
                        io::Error::last_os_error().kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ))
            {
                if terminal.is_some() {
                    terminal.take();
                } else {
                    outputs[index].take();
                }
            }
        }
    }
}

// Finish an older frame and release queued control before another wait or data read.
fn flush_control(
    mut connection: &UnixStream,
    connected: &mut bool,
    writer: &mut FrameWriter,
    exit: &mut Option<i32>,
    consumed: &mut usize,
) -> io::Result<()> {
    while *connected {
        if writer.pending() {
            match writer.flush(&mut connection) {
                Ok(false) => break,
                Ok(true) => {}
                Err(_) => {
                    disconnect(connection, connected, writer);
                    break;
                }
            }
        }
        if let Some(code) = exit.take() {
            writer.start(EXIT, &code.to_be_bytes())?;
        } else if *consumed != 0 {
            let mut payload = [0; 5];
            payload[0] = STDIN;
            payload[1..].copy_from_slice(&(*consumed as u32).to_be_bytes());
            writer.start(CREDIT, &payload)?;
            *consumed = 0;
        } else {
            break;
        }
    }
    Ok(())
}

fn observe(
    session: &mut Option<Session>,
    status: &mut Option<i32>,
    exit: &mut Option<i32>,
    drain: &mut Option<Instant>,
) -> io::Result<()> {
    if let Some(session) = session
        && let Some(completed) = session.status()?
    {
        let code = completed
            .code()
            .or_else(|| completed.signal().map(|signal| 128 + signal))
            .unwrap_or(1);
        *status = Some(code);
        *exit = Some(code);
        *drain = Some(Instant::now() + Duration::from_secs(1));
    }
    if status.is_some() {
        session.take();
    }
    Ok(())
}

fn disconnect(connection: &UnixStream, connected: &mut bool, writer: &mut FrameWriter) {
    let _ = connection.shutdown(Shutdown::Both);
    *connected = false;
    *writer = FrameWriter::default();
}

fn stream(kind: u8) -> Option<usize> {
    match kind {
        STDOUT => Some(0),
        STDERR => Some(1),
        _ => None,
    }
}

fn fd(fd: i32, events: i16) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

fn nonblocking(fd: &OwnedFd) -> io::Result<()> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
    fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    Ok(())
}
