//! Command frames shared by both runtime transports.

use std::io::{self, Read, Write};

pub const EXEC: u8 = b'X';
pub const FAILED: u8 = b'F';
pub const STDIN: u8 = b'i';
pub const STDIN_END: u8 = b'e';
pub const RESIZE: u8 = b'w';
pub const STDOUT: u8 = b'o';
pub const STDERR: u8 = b'r';
pub const EXIT: u8 = b'x';

/// A frame's payload limit: argv and env together stay far below it.
const FRAME_MAX: usize = 64 << 20;

pub(crate) fn write_frame(output: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(payload.len())
        .ok()
        .filter(|&length| length as usize <= FRAME_MAX)
        .ok_or_else(|| io::Error::other("frame too large"))?;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(kind);
    frame.extend(length.to_be_bytes());
    frame.extend(payload);
    output.write_all(&frame)
}

/// Read one frame exactly, never past it: the boot connection's next bytes
/// belong to the exec service.
pub(crate) fn read_frame(input: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0; 5];
    input.read_exact(&mut head)?;
    let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if length > FRAME_MAX {
        return Err(io::Error::other("frame too large"));
    }
    let mut payload = vec![0; length];
    input.read_exact(&mut payload)?;
    Ok((head[0], payload))
}
