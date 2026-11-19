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

pub const CREDIT: u8 = b'c';
pub const OUTPUT_END: u8 = b'd';
pub const WINDOW: usize = 64 * 1024;
pub const CHUNK: usize = 16 * 1024;

/// One frame can span readiness notifications, but never another frame.
#[derive(Default)]
pub(crate) struct FrameReader {
    head: [u8; 5],
    header: usize,
    payload: Vec<u8>,
    read: usize,
}

impl FrameReader {
    pub(crate) fn read(&mut self, input: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
        loop {
            let target = if self.header < 5 {
                &mut self.head[self.header..]
            } else {
                &mut self.payload[self.read..]
            };
            if target.is_empty() {
                let frame = (self.head[0], std::mem::take(&mut self.payload));
                self.header = 0;
                self.read = 0;
                return Ok(Some(frame));
            }
            match input.read(target) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(read) if self.header < 5 => {
                    self.header += read;
                    if self.header == 5 {
                        let length =
                            u32::from_be_bytes(self.head[1..].try_into().unwrap()) as usize;
                        if length > FRAME_MAX {
                            return Err(io::Error::other("frame too large"));
                        }
                        self.payload.resize(length, 0);
                    }
                }
                Ok(read) => self.read += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) => return Err(error),
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct FrameWriter {
    bytes: Vec<u8>,
    written: usize,
}

impl FrameWriter {
    pub(crate) fn pending(&self) -> bool {
        self.written < self.bytes.len()
    }

    pub(crate) fn start(&mut self, kind: u8, payload: &[u8]) -> io::Result<()> {
        if self.pending() {
            return Err(io::Error::other("frame already pending"));
        }
        self.bytes.clear();
        self.written = 0;
        write_frame(&mut self.bytes, kind, payload)
    }

    pub(crate) fn flush(&mut self, output: &mut impl Write) -> io::Result<bool> {
        while self.pending() {
            match output.write(&self.bytes[self.written..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => self.written += written,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }
}

pub(crate) fn copy_stream(input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    let mut bytes = [0; 32 * 1024];
    loop {
        let read = match input.read(&mut bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            read => read?,
        };
        if read == 0 {
            return Ok(());
        }
        output.write_all(&bytes[..read])?;
    }
}
