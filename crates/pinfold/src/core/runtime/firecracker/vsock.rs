//! The host's half of the guest protocol (`init/firecracker.rs`): requests
//! over the VM's vsock socket, and the exec client.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use crate::core::process::StdioRelay;
use crate::core::runtime::sockets::Sockets;
use crate::core::runtime::terminal::{Raw, Winch, window};

use crate::init::firecracker::{
    CONTROL_PORT, ENV, ENV_SET, EXEC, EXEC_PORT, EXIT, FAILED, RESIZE, STAT, STDERR, STDIN,
    STDIN_END, STDOUT, UNMOUNT, read_frame, write_frame,
};

/// A host-initiated connection to the guest's `port`.
pub fn connect(sockets: &Sockets, port: u32) -> io::Result<UnixStream> {
    let mut stream = sockets.connect()?;
    // Box work runs as the exec service's user and can stop it from
    // answering, so every request has a deadline; `exec` lifts it after
    // the handshake.
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(format!("CONNECT {port}\n").as_bytes())?;
    // One byte at a time: what follows the line is the guest's.
    let mut line = Vec::new();
    let mut byte = [0];
    while line.len() < 64 {
        match stream.read(&mut byte)? {
            0 => break,
            _ if byte[0] == b'\n' => break,
            _ => line.push(byte[0]),
        }
    }
    if !line.starts_with(b"OK ") {
        return Err(io::Error::other(format!(
            "the box does not answer on vsock port {port}"
        )));
    }
    Ok(stream)
}

/// Read the answer to a request: the payload of a frame of `kind`, or the
/// guest's FAILED text as the error.
fn answer(stream: &mut impl Read, kind: u8) -> io::Result<Vec<u8>> {
    let (got, payload) = read_frame(stream)?;
    match got {
        _ if got == kind => Ok(payload),
        FAILED => Err(io::Error::other(
            String::from_utf8_lossy(&payload).into_owned(),
        )),
        _ => Err(io::Error::other(format!("the box answered frame {got}"))),
    }
}

pub(super) fn put(payload: &mut Vec<u8>, bytes: &[u8]) {
    payload.extend((bytes.len() as u32).to_be_bytes());
    payload.extend(bytes);
}

fn put_pairs(payload: &mut Vec<u8>, pairs: &[(OsString, OsString)]) {
    payload.extend((pairs.len() as u32).to_be_bytes());
    for (name, value) in pairs {
        put(payload, name.as_bytes());
        put(payload, value.as_bytes());
    }
}

/// Give the exec service the box's environment, once.
pub fn set_env(
    sockets: &Sockets,
    vars: &[(OsString, OsString)],
    cancel: &AtomicBool,
) -> io::Result<()> {
    cancelled(Some(cancel))?;
    let stream = exec_connection(sockets, Some(cancel))?;
    let mut transport = Cancellable::startup(&stream, cancel);
    let mut payload = Vec::new();
    put_pairs(&mut payload, vars);
    write_frame(&mut transport, ENV, &payload)?;
    answer(&mut transport, ENV_SET).map(drop)
}

/// The work cgroup's memory current and peak, pids current and max, and
/// OOM kills.
pub fn stat(sockets: &Sockets) -> io::Result<[Option<u64>; 5]> {
    let mut stream = connect(sockets, EXEC_PORT)?;
    write_frame(&mut stream, STAT, &[])?;
    let payload = answer(&mut stream, STAT)?;
    let mut values = [None; 5];
    let (fields @ [_, _, _, _, _], []) = payload.as_chunks::<9>() else {
        return Err(io::Error::other("the box's STAT has the wrong length"));
    };
    for (value, field) in values.iter_mut().zip(fields) {
        if field[0] != 0 {
            *value = Some(u64::from_be_bytes(field[1..].try_into().unwrap()));
        }
    }
    Ok(values)
}

/// Ask PID 1 to kill the work cgroup, sync and unmount every attachment.
pub fn unmount(sockets: &Sockets) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut stream = sockets.connect_until(deadline)?;
    stream.set_nonblocking(true)?;
    let mut transport = Bounded {
        stream: &mut stream,
        deadline,
    };
    transport.write_all(format!("CONNECT {CONTROL_PORT}\n").as_bytes())?;
    let mut byte = [0];
    let mut line = Vec::new();
    while line.len() < 64 {
        match transport.read(&mut byte)? {
            0 => break,
            _ if byte[0] == b'\n' => break,
            _ => line.push(byte[0]),
        }
    }
    if !line.starts_with(b"OK ") {
        return Err(io::Error::other("the box does not answer on control vsock"));
    }
    write_frame(&mut transport, UNMOUNT, &[])?;
    answer(&mut transport, crate::init::firecracker::CLEAN).map(drop)
}

struct Bounded<'a> {
    stream: &'a mut UnixStream,
    deadline: Instant,
}

impl Read for Bounded<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        crate::core::pipe::read(self.stream, bytes, self.deadline, None, "box unmount")
    }
}

impl Write for Bounded<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        crate::core::pipe::write_all(self.stream, bytes, self.deadline, None, "box unmount")?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Run `argv` in the box and relay stdio until it exits:
/// its exit code, or `128+n` for signal `n`. The client stays in pinfold's
/// process.
pub fn exec(
    sockets: &Sockets,
    context: &crate::core::runtime::ExecContext<'_>,
    argv: &[String],
) -> io::Result<i32> {
    let cancel = context.cancel;
    cancelled(cancel)?;
    let tty = context.tty;
    let mut connection = exec_connection(sockets, cancel)?;
    // A command runs as long as it runs.
    connection.set_read_timeout(None)?;
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
        if joined.is_err() && matches!(result, Ok(0)) {
            return Err(io::Error::other("exec input worker failed"));
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
        if matches!(result, Ok(0)) {
            return Err(error);
        }
        let _ = writeln!(io::stderr(), "pinfold: stdio relay: {error}");
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
    };
    loop {
        let (kind, payload) = read_frame(&mut connection).map_err(|error| {
            if error.kind() == io::ErrorKind::Interrupted {
                return error;
            }
            io::Error::other(format!("the box ended the exec before it exited: {error}"))
        })?;
        // A closed host output never stops the relay: the exit code still
        // has to arrive.
        match kind {
            STDOUT => {
                output(stdout, &payload, cancel)?;
            }
            STDERR => {
                output(stderr, &payload, cancel)?;
            }
            EXIT => {
                let code: [u8; 4] = payload
                    .try_into()
                    .map_err(|_| io::Error::other("the box's EXIT has the wrong length"))?;
                return Ok(i32::from_be_bytes(code));
            }
            FAILED => {
                return Err(io::Error::other(
                    String::from_utf8_lossy(&payload).into_owned(),
                ));
            }
            _ => return Err(io::Error::other(format!("the box sent frame {kind}"))),
        }
        cancelled(cancel)?;
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

fn cancelled(cancel: Option<&AtomicBool>) -> io::Result<()> {
    if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        Err(io::Error::new(io::ErrorKind::Interrupted, "exec cancelled"))
    } else {
        Ok(())
    }
}

fn exec_connection(sockets: &Sockets, cancel: Option<&AtomicBool>) -> io::Result<UnixStream> {
    if cancel.is_none() {
        return connect(sockets, EXEC_PORT);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut connection = sockets.connect_until(deadline)?;
    connection.set_nonblocking(true)?;
    crate::core::pipe::write_all(
        &mut connection,
        format!("CONNECT {EXEC_PORT}\n").as_bytes(),
        deadline,
        cancel,
        "exec handshake",
    )?;
    let mut line = Vec::new();
    let mut byte = [0];
    while line.len() < 64 {
        match crate::core::pipe::read(
            &mut connection,
            &mut byte,
            deadline,
            cancel,
            "exec handshake",
        )? {
            0 => break,
            _ if byte[0] == b'\n' => break,
            _ => line.push(byte[0]),
        }
    }
    if !line.starts_with(b"OK ") {
        return Err(io::Error::other("the box does not answer on exec vsock"));
    }
    Ok(connection)
}

fn wait(
    fd: libc::c_int,
    events: libc::c_short,
    stop: Option<&io::PipeReader>,
    cancel: Option<&AtomicBool>,
) -> io::Result<()> {
    loop {
        cancelled(cancel)?;
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
}

impl<'a> Cancellable<'a> {
    pub(super) fn startup(connection: &'a UnixStream, cancel: &'a AtomicBool) -> Self {
        Self {
            connection,
            stop: None,
            cancel: Some(cancel),
            completed_first: false,
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

fn output(fd: libc::c_int, mut bytes: &[u8], cancel: Option<&AtomicBool>) -> io::Result<()> {
    while !bytes.is_empty() {
        cancelled(cancel)?;
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
            io::ErrorKind::WouldBlock => wait(fd, libc::POLLOUT, None, cancel)?,
            // A closed host stream must not discard the eventual command status.
            _ => return Ok(()),
        }
    }
    Ok(())
}
