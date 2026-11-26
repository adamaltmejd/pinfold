//! Private owner admission framing. Payloads never appear in diagnostics.

use std::io;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::pi::session_snapshot::{OpValues, Roots, Snapshot};

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
        op_values: OpValues,
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
        rex: bool,
    },
    Borrow {
        root: PathBuf,
        generation: String,
        roots: Roots,
        share_only: bool,
    },
    Leave {},
}

impl Request {
    /// Whether a client request addresses this project root and owner generation.
    pub(crate) fn addresses(&self, target_root: &Path, target_generation: &str) -> bool {
        match self {
            Self::Probe {
                root, generation, ..
            }
            | Self::Join {
                root, generation, ..
            }
            | Self::Keep {
                root, generation, ..
            }
            | Self::Borrow {
                root, generation, ..
            } => root == target_root && generation == target_generation,
            _ => false,
        }
    }
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
    /// The owner's box start was refused as `spec` or `login`: the reason
    /// token and the detail `box up`'s `refused` line would carry.
    Refused {
        reason: String,
        detail: String,
    },
}

impl Reply {
    /// An error reply that names only its reason token.
    pub(crate) fn error(reason: impl Into<String>) -> Self {
        Self::Error {
            reason: reason.into(),
            fields: vec![],
        }
    }
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

pub(crate) fn peer(stream: &impl AsFd) -> io::Result<()> {
    let denied = |_| protocol(io::ErrorKind::PermissionDenied);
    #[cfg(target_os = "linux")]
    let uid = nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
        .map_err(denied)?
        .uid();
    #[cfg(target_os = "macos")]
    let uid = nix::unistd::getpeereid(stream).map_err(denied)?.0.as_raw();
    if uid == nix::unistd::geteuid().as_raw() {
        Ok(())
    } else {
        Err(protocol(io::ErrorKind::PermissionDenied))
    }
}
