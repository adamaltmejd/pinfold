//! The host's half of the guest protocol (`init/firecracker.rs`): requests
//! over the VM's vsock socket, and the exec client.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use crate::core::runtime::sockets::Sockets;
use crate::formatting::terminal_text;

use crate::init::exec_wire::{ENV, FAILED, read_frame, write_frame};
use crate::init::firecracker::{CONTROL_PORT, ENV_SET, EXEC_PORT, STAT, UNMOUNT};

/// Read the answer to a request: the payload of a frame of `kind`, or the
/// guest's FAILED text as the error.
fn answer(stream: &mut impl Read, kind: u8) -> io::Result<Vec<u8>> {
    let (got, payload) = read_frame(stream)?;
    match got {
        _ if got == kind => Ok(payload),
        FAILED => Err(io::Error::other(terminal_text(&String::from_utf8_lossy(
            &payload,
        )))),
        _ => Err(io::Error::other(format!("the box answered frame {got}"))),
    }
}

use super::super::exec_transport::{Cancellable, cancelled, env_payload};

/// Box work runs as the exec service's user and can stop it from
/// answering, so every exec-service handshake and request has a deadline.
const EXEC_DEADLINE: Duration = Duration::from_secs(10);

/// Give the exec service the box's environment, once.
pub fn set_env(
    sockets: &Sockets,
    vars: &[(OsString, OsString)],
    cancel: &AtomicBool,
) -> io::Result<()> {
    cancelled(Some(cancel))?;
    let deadline = Instant::now() + EXEC_DEADLINE;
    let stream = connect(sockets, EXEC_PORT, deadline, Some(cancel), "exec handshake")?;
    let mut transport = Cancellable::startup(&stream, cancel);
    write_frame(&mut transport, ENV, &env_payload(vars))?;
    answer(&mut transport, ENV_SET).map(drop)
}

/// The work cgroup's memory current and peak, pids current and max, and
/// OOM kills.
pub fn stat(sockets: &Sockets) -> io::Result<[Option<u64>; 5]> {
    let deadline = Instant::now() + EXEC_DEADLINE;
    let mut stream = connect(sockets, EXEC_PORT, deadline, None, "box stat")?;
    // After the handshake, each read gets its own timeout.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(EXEC_DEADLINE))?;
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
    let deadline = Instant::now() + super::UNMOUNT_TIMEOUT;
    let mut stream = connect(sockets, CONTROL_PORT, deadline, None, "box unmount")?;
    let mut transport = Bounded {
        stream: &mut stream,
        deadline,
    };
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

/// A host-initiated connection to the guest's `port`, bounded by
/// `deadline` and `cancel`; the stream it returns is nonblocking.
fn connect(
    sockets: &Sockets,
    port: u32,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
    operation: &str,
) -> io::Result<UnixStream> {
    let mut connection = sockets.connect(deadline, cancel)?;
    connection.set_nonblocking(true)?;
    crate::core::pipe::write_all(
        &mut connection,
        format!("CONNECT {port}\n").as_bytes(),
        deadline,
        cancel,
        operation,
    )?;
    // One byte at a time: what follows the line is the guest's.
    let mut line = Vec::new();
    let mut byte = [0];
    while line.len() < 64 {
        match crate::core::pipe::read(&mut connection, &mut byte, deadline, cancel, operation)? {
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
    Ok(connection)
}

/// Run a command over the box's existing exec connection.
pub fn exec(
    sockets: &Sockets,
    context: &crate::core::runtime::ExecContext<'_>,
    argv: &[String],
) -> io::Result<i32> {
    let deadline = Instant::now() + EXEC_DEADLINE;
    let connection = connect(
        sockets,
        EXEC_PORT,
        deadline,
        context.cancel,
        "exec handshake",
    )?;
    super::super::exec_transport::exec(connection, context, argv)
}
