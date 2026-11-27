//! What the host and `pinfold init` share: wire formats, constants and
//! guest paths. The PID 1 verbs are in `init/pid1.rs`, built for Linux
//! only; a macOS host embeds the Linux build (`build.rs`).

use std::io::{self, Write};
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "init/builder.rs"]
mod builder;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[path = "init/exec_wire.rs"]
pub(crate) mod exec_wire;
#[cfg(target_os = "linux")]
#[path = "init/export.rs"]
mod export;
#[cfg(target_os = "linux")]
#[path = "init/firecracker.rs"]
pub(crate) mod firecracker;
#[path = "formatting.rs"]
pub(crate) mod formatting;
#[cfg(target_os = "linux")]
#[path = "init/pid1.rs"]
mod pid1;
#[cfg(target_os = "linux")]
#[path = "init/session.rs"]
mod session;
#[cfg(target_os = "linux")]
#[path = "init/session_io.rs"]
mod session_io;

#[cfg(target_os = "linux")]
pub use pid1::run;

/// Private exec metadata is removed before restoring caller-named variables.
pub const EXEC_METADATA: &str = "PINFOLD_EXEC_METADATA";

#[derive(Default)]
pub struct ExecMetadata {
    pub agent: Option<String>,
    pub project_directory: Option<(PathBuf, PathBuf)>,
    pub window: Option<(u16, u16)>,
    pub term: Option<Vec<u8>>,
    pub colorterm: Option<Vec<u8>>,
}

impl ExecMetadata {
    pub fn encode(&self) -> String {
        let optional = |value: Option<&[u8]>| value.map_or_else(|| "-".to_string(), hex);
        let window = self
            .window
            .map_or_else(|| "-".to_string(), |(rows, cols)| format!("{rows},{cols}"));
        format!(
            "{window}:{}:{}:{}:{}:{}",
            optional(self.agent.as_deref().map(str::as_bytes)),
            optional(self.term.as_deref()),
            optional(self.colorterm.as_deref()),
            optional(
                self.project_directory
                    .as_ref()
                    .map(|(root, _)| root.as_os_str().as_bytes())
            ),
            optional(
                self.project_directory
                    .as_ref()
                    .map(|(_, relative)| relative.as_os_str().as_bytes())
            )
        )
    }
}

/// `bytes` as lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn decode_hex(value: &[u8]) -> io::Result<Vec<u8>> {
    let decoded: Option<Vec<u8>> = value
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect();
    decoded
        .filter(|_| value.len().is_multiple_of(2))
        .ok_or_else(|| io::Error::other("invalid encoded environment"))
}

/// The guest path of the CA bundle a box with a routed host trusts: the
/// image's own, with the box CA appended.
pub const BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";

/// The most of an image's bundle pinfold copies; Debian's is about 220 KiB.
pub const MAX_BUNDLE: usize = 4 * 1024 * 1024;

/// One entry of a `from` copy stream: a tag byte, the relative path as a
/// u32 length and bytes, then the payload: nothing for a directory, a u64
/// length and the bytes for a file, a u32 length and the target for a
/// symlink. [`STREAM_END`] ends the stream.
pub const STREAM_DIR: u8 = b'd';
pub const STREAM_FILE: u8 = b'f';
pub const STREAM_EXECUTABLE: u8 = b'x';
pub const STREAM_SYMLINK: u8 = b'l';
pub const STREAM_END: u8 = 0;

/// Limits of a disk export (`init send`) stream, and its abort tag.
pub(crate) const ENTRIES: u64 = 65_536;
pub(crate) const PATH_BYTES: usize = 4096;
pub(crate) const METADATA_BYTES: u64 = 16 << 20;
pub(crate) const CONTENT_BYTES: u64 = 64 << 30;
pub(crate) const ABORT: u8 = b'!';

/// A blocking socket's writes, each failing with `TimedOut` after `timeout`
/// without progress. `SO_SNDTIMEO` is not enough: Linux's timer wheel
/// rounds a five-minute timeout up by up to half a minute, and a partial
/// write returns only at the timeout, restarting it for the rest.
pub(crate) struct Deadline<'a>(pub BorrowedFd<'a>, pub Duration);

impl Write for Deadline<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let began = Instant::now();
        let mut ready = [PollFd::new(self.0, PollFlags::POLLOUT)];
        // Polling only after a refused send keeps Linux at a blocking
        // send's throughput. Darwin ignores MSG_DONTWAIT on a blocking
        // socket, so there every send waits for POLLOUT first.
        let mut wait = cfg!(target_os = "macos");
        loop {
            if wait {
                let remaining = self
                    .1
                    .checked_sub(began.elapsed())
                    .ok_or(io::ErrorKind::TimedOut)?;
                let timeout = PollTimeout::try_from(remaining.max(Duration::from_millis(1)))
                    .expect("five-minute timeout fits poll");
                match poll(&mut ready, timeout) {
                    Ok(0) => return Err(io::ErrorKind::TimedOut.into()),
                    Err(nix::errno::Errno::EINTR) => continue,
                    Ok(_) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            wait = true;
            // Safety: the fd stays borrowed, and buf is readable for its length.
            let sent = unsafe {
                nix::libc::send(
                    self.0.as_raw_fd(),
                    buf.as_ptr().cast(),
                    buf.len(),
                    nix::libc::MSG_DONTWAIT | nix::libc::MSG_NOSIGNAL,
                )
            };
            if sent >= 0 {
                return Ok(sent as usize);
            }
            let error = io::Error::last_os_error();
            if !matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Err(error);
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
