//! The host's half of the guest protocol (`init/firecracker.rs`): requests
//! over the VM's vsock socket, and the exec client.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;

use std::time::Duration;

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

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
fn answer(stream: &mut UnixStream, kind: u8) -> io::Result<Vec<u8>> {
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
pub fn set_env(sockets: &Sockets, vars: &[(OsString, OsString)]) -> io::Result<()> {
    let mut stream = connect(sockets, EXEC_PORT)?;
    let mut payload = Vec::new();
    put_pairs(&mut payload, vars);
    write_frame(&mut stream, ENV, &payload)?;
    answer(&mut stream, ENV_SET).map(drop)
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
    let mut stream = connect(sockets, CONTROL_PORT)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    write_frame(&mut stream, UNMOUNT, &[])?;
    answer(&mut stream, crate::init::firecracker::CLEAN).map(drop)
}

/// Run `argv` in the box and relay stdio until it exits:
/// its exit code, or `128+n` for signal `n`. The client stays in pinfold's
/// process.
pub fn exec(
    sockets: &Sockets,
    context: &crate::core::runtime::ExecContext<'_>,
    argv: &[String],
) -> io::Result<i32> {
    let tty = context.tty;
    let mut connection = connect(sockets, EXEC_PORT)?;
    // A command runs as long as it runs.
    connection.set_read_timeout(None)?;
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
    write_frame(&mut connection, EXEC, &payload)?;

    let (stop, stopping) = io::pipe()?;
    let input = {
        let connection = connection.try_clone()?;
        let winch = match &winch {
            Some(winch) => Some(winch.read.try_clone()?),
            None => None,
        };
        std::thread::Builder::new()
            .name("exec-input".into())
            .spawn(move || forward_input(connection, &stop, winch.as_ref()))?
    };
    let result = relay_output(&mut connection);
    // Ends the input thread's poll; it never blocks in a read.
    drop(stopping);
    let _ = input.join();
    drop((winch, raw));
    result
}

/// Copy the guest's output to pinfold's own until EXIT.
fn relay_output(connection: &mut UnixStream) -> io::Result<i32> {
    let (mut stdout, mut stderr) = (io::stdout(), io::stderr());
    loop {
        let (kind, payload) = read_frame(connection).map_err(|error| {
            io::Error::other(format!("the box ended the exec before it exited: {error}"))
        })?;
        // A closed host output never stops the relay: the exit code still
        // has to arrive.
        match kind {
            STDOUT => {
                let _ = stdout.write_all(&payload).and_then(|()| stdout.flush());
            }
            STDERR => {
                let _ = stderr.write_all(&payload).and_then(|()| stderr.flush());
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
    }
}

/// Forward stdin as STDIN frames and its end as STDIN_END, and each window
/// change as RESIZE, until `stop` reads end of file.
fn forward_input(
    mut connection: UnixStream,
    stop: &io::PipeReader,
    winch: Option<&io::PipeReader>,
) {
    let stdin = io::stdin();
    let mut open = true;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let mut fds = vec![PollFd::new(stop.as_fd(), PollFlags::POLLIN)];
        if let Some(winch) = winch {
            fds.push(PollFd::new(winch.as_fd(), PollFlags::POLLIN));
        }
        if open {
            fds.push(PollFd::new(stdin.as_fd(), PollFlags::POLLIN));
        }
        match poll(&mut fds, PollTimeout::NONE) {
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
                if write_frame(&mut connection, RESIZE, &payload).is_err() {
                    return;
                }
            }
        }
        if open && *ready.last().unwrap() {
            let sent = match read_fd(stdin.as_fd(), &mut buffer) {
                Ok(0) | Err(_) => {
                    open = false;
                    write_frame(&mut connection, STDIN_END, &[])
                }
                Ok(size) => write_frame(&mut connection, STDIN, &buffer[..size]),
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
