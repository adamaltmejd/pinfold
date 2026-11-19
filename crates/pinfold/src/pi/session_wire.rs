//! Private owner admission framing. Payloads never appear in diagnostics.

use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

use nix::libc;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::pi::session_snapshot::{Roots, Snapshot};

const MAX_FRAME: usize = 16 * 1024 * 1024;
const FRAME_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Intent {
    Join,
    Keep,
    /// Keep, and relay the box's Rex server to the host.
    KeepRex,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Request {
    Bootstrap {
        root: PathBuf,
        intent: Intent,
        snapshot: Snapshot,
    },
    Probe {
        root: PathBuf,
        generation: String,
        roots: Roots,
    },
    Join {
        root: PathBuf,
        generation: String,
        snapshot: Snapshot,
    },
    Keep {
        root: PathBuf,
        generation: String,
        snapshot: Snapshot,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        rex: bool,
    },
    Borrow {
        root: PathBuf,
        generation: String,
        roots: Roots,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        share_only: bool,
    },
    Leave {},
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Reply {
    Starting {},
    Compatible {
        mount: crate::config::ProjectMount,
    },
    Admitted {
        name: String,
        generation: String,
    },
    Kept {
        name: String,
        generation: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rex: Option<crate::pi::rex::Status>,
    },
    Left {},
    Draining {
        generation: String,
    },
    Ended {
        generation: String,
        reason: String,
        teardown_ok: bool,
    },
    Error {
        reason: String,
        fields: Vec<String>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestFrame {
    version: u8,
    request: Request,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyFrame {
    version: u8,
    reply: Reply,
}

fn protocol(kind: io::ErrorKind) -> io::Error {
    io::Error::new(kind, "session-protocol")
}

fn io_protocol(error: io::Error) -> io::Error {
    protocol(error.kind())
}

async fn read_frame<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
    idle: bool,
) -> io::Result<T> {
    let mut length = [0u8; 4];
    if idle {
        stream.read_exact(&mut length[..1]).await.map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                protocol(io::ErrorKind::ConnectionAborted)
            } else {
                io_protocol(error)
            }
        })?;
    }
    let body = tokio::time::timeout(FRAME_TIMEOUT, async {
        stream
            .read_exact(&mut length[usize::from(idle)..])
            .await
            .map_err(io_protocol)?;
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > MAX_FRAME {
            return Err(protocol(io::ErrorKind::InvalidData));
        }
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).await.map_err(io_protocol)?;
        Ok(body)
    })
    .await
    .map_err(|_| protocol(io::ErrorKind::TimedOut))??;
    serde_json::from_slice(&body).map_err(|_| protocol(io::ErrorKind::InvalidData))
}

async fn write_frame<T: Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    frame: &T,
) -> io::Result<()> {
    let body = serde_json::to_vec(frame).map_err(|_| protocol(io::ErrorKind::InvalidData))?;
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err(protocol(io::ErrorKind::InvalidData));
    }
    let length = (body.len() as u32).to_be_bytes();
    tokio::time::timeout(FRAME_TIMEOUT, async {
        stream.write_all(&length).await.map_err(io_protocol)?;
        stream.write_all(&body).await.map_err(io_protocol)
    })
    .await
    .map_err(|_| protocol(io::ErrorKind::TimedOut))?
}

pub(crate) async fn read_request(
    stream: &mut (impl AsyncRead + Unpin),
    idle: bool,
) -> io::Result<Request> {
    let frame: RequestFrame = read_frame(stream, idle).await?;
    if frame.version != 1 {
        return Err(protocol(io::ErrorKind::InvalidData));
    }
    Ok(frame.request)
}

pub(crate) async fn read_reply(
    stream: &mut (impl AsyncRead + Unpin),
    idle: bool,
) -> io::Result<Reply> {
    let frame: ReplyFrame = read_frame(stream, idle).await?;
    if frame.version != 1 {
        return Err(protocol(io::ErrorKind::InvalidData));
    }
    Ok(frame.reply)
}

pub(crate) async fn write_request(
    stream: &mut (impl AsyncWrite + Unpin),
    request: Request,
) -> io::Result<()> {
    write_frame(
        stream,
        &RequestFrame {
            version: 1,
            request,
        },
    )
    .await
}

pub(crate) async fn write_reply(
    stream: &mut (impl AsyncWrite + Unpin),
    reply: Reply,
) -> io::Result<()> {
    write_frame(stream, &ReplyFrame { version: 1, reply }).await
}

pub(crate) fn peer(stream: &impl AsRawFd) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let uid = {
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
        // SAFETY: getsockopt writes into the sized live credentials value.
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut length,
            )
        };
        if result != 0 || length as usize != std::mem::size_of_val(&credentials) {
            return Err(protocol(io::ErrorKind::PermissionDenied));
        }
        credentials.uid
    };
    #[cfg(target_os = "macos")]
    let uid = {
        let mut uid = 0;
        let mut gid = 0;
        // SAFETY: getpeereid writes the two live credential outputs.
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(protocol(io::ErrorKind::PermissionDenied));
        }
        uid
    };
    if uid == nix::unistd::geteuid().as_raw() {
        Ok(())
    } else {
        Err(protocol(io::ErrorKind::PermissionDenied))
    }
}
