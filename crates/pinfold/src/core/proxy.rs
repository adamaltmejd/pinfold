//! The per-box egress proxy.
//!
//! It lives in the `box up` process and listens only on the box's unix
//! socket, so it has no network listener. This is the CONNECT half: port 443
//! to an allowlisted host. Plain HTTP and the address checks come later.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

/// The loopback URL clients reach through `pinfold init`'s relay.
pub const PROXY_URL: &str = "http://127.0.0.1:3128";

/// macOS `sun_path` is 104 bytes including the terminator.
const SOCKET_PATH_LIMIT: usize = 104;

/// One request head. The framing checks come with the next ticket; the cap
/// only keeps a client from growing the buffer without bound.
const MAX_HEAD: usize = 8 * 1024;

/// A running proxy, owned by the `box up` process.
pub struct Proxy {
    socket: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Proxy {
    /// Bind the box's socket, create its log, and serve until closed.
    pub fn start(socket: PathBuf, allow: &[String], log: PathBuf) -> io::Result<Proxy> {
        if socket.as_os_str().len() >= SOCKET_PATH_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "proxy socket path is {} bytes; macOS allows {}: {}",
                    socket.as_os_str().len(),
                    SOCKET_PATH_LIMIT - 1,
                    socket.display()
                ),
            ));
        }
        if let Some(parent) = log.parent() {
            fs::create_dir_all(parent)?;
        }
        // The log exists before the first decision, so it is found from the
        // host even for a box that made no request.
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)?;
        let listener = UnixListener::bind(&socket)?;
        let stop = Arc::new(AtomicBool::new(false));
        let allow = Arc::new(Allow::new(allow));
        let thread = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || serve(listener, stop, allow, log))
        };
        Ok(Proxy {
            socket,
            stop,
            thread: Some(thread),
        })
    }

    /// The host path of the socket, for the transport to carry in.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Stop accepting and wait for the accept loop to drop the listener.
    pub fn close(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocked accept so it sees the flag. When the socket is
        // already gone the accept thread cannot be woken; leave it to the
        // process exit rather than joining forever.
        if UnixStream::connect(&self.socket).is_ok()
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

/// One box's allowlist.
struct Allow {
    /// Names that match only themselves.
    exact: Vec<String>,
    /// Names that match themselves and any subdomain.
    suffix: Vec<String>,
}

impl Allow {
    fn new(entries: &[String]) -> Allow {
        let mut exact = Vec::new();
        let mut suffix = Vec::new();
        for entry in entries {
            let entry = entry.to_ascii_lowercase();
            match entry.strip_prefix('.') {
                Some(name) if !name.is_empty() => suffix.push(name.to_string()),
                _ => exact.push(entry),
            }
        }
        Allow { exact, suffix }
    }

    fn allows(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.exact.iter().any(|name| name == &host)
            || self.suffix.iter().any(|name| {
                host.strip_suffix(name)
                    .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('.'))
            })
    }
}

fn serve(listener: UnixListener, stop: Arc<AtomicBool>, allow: Arc<Allow>, log: PathBuf) {
    loop {
        match listener.accept() {
            Ok((client, _)) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let allow = Arc::clone(&allow);
                let log = log.clone();
                thread::spawn(move || handle(client, &allow, &log));
            }
            Err(_) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    }
}

fn handle(mut client: UnixStream, allow: &Allow, log: &Path) {
    let Some(head) = read_head(&mut client) else {
        return;
    };
    let head = String::from_utf8_lossy(&head);
    let request_line = head.split("\r\n").next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(_version)) = (parts.next(), parts.next(), parts.next())
    else {
        let _ = respond(&mut client, 400);
        return;
    };
    if parts.next().is_some() {
        let _ = respond(&mut client, 400);
        return;
    }
    if method != "CONNECT" {
        // Plain HTTP arrives on this port and is the next ticket's.
        let _ = respond(&mut client, 405);
        return;
    }
    let Some((host, port)) = target.rsplit_once(':') else {
        let _ = respond(&mut client, 400);
        return;
    };
    if host.is_empty() {
        let _ = respond(&mut client, 400);
        return;
    }
    if !allow.allows(host) {
        record(log, host, "refused", "not allowlisted");
        let _ = respond(&mut client, 403);
        return;
    }
    if port.parse::<u16>() != Ok(443) {
        record(log, host, "refused", "port not allowed");
        let _ = respond(&mut client, 403);
        return;
    }
    record(log, host, "allowed", "allowlisted");
    // Resolve once and dial the address that was resolved.
    let Some(server) = dial(host) else {
        let _ = respond(&mut client, 502);
        return;
    };
    if respond(&mut client, 200).is_err() {
        return;
    }
    tunnel(client, server);
}

/// Read one request head, ending at the blank line.
fn read_head(client: &mut UnixStream) -> Option<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        match client.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    return Some(head);
                }
                if head.len() > MAX_HEAD {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

/// Resolve `host` once and connect to the first address.
fn dial(host: &str) -> Option<TcpStream> {
    let address = (host, 443).to_socket_addrs().ok()?.next()?;
    TcpStream::connect(address).ok()
}

/// Copy both directions for the life of the tunnel.
fn tunnel(client: UnixStream, server: TcpStream) {
    let mut client = client;
    let mut server = server;
    let Ok(mut client_reader) = client.try_clone() else {
        return;
    };
    let Ok(mut server_writer) = server.try_clone() else {
        return;
    };
    let up = thread::spawn(move || {
        let _ = io::copy(&mut client_reader, &mut server_writer);
        let _ = server_writer.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = up.join();
}

fn respond(client: &mut UnixStream, code: u16) -> io::Result<()> {
    let response = match code {
        200 => "HTTP/1.1 200 Connection Established\r\n\r\n".to_string(),
        _ => {
            let reason = match code {
                400 => "Bad Request",
                403 => "Forbidden",
                405 => "Method Not Allowed",
                502 => "Bad Gateway",
                _ => "Error",
            };
            format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        }
    };
    client.write_all(response.as_bytes())
}

/// Append one decision as a JSON line. No header value is ever written.
fn record(log: &Path, host: &str, decision: &str, reason: &str) {
    let mut line = serde_json::json!({
        "host": host,
        "decision": decision,
        "reason": reason,
    })
    .to_string();
    line.push('\n');
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(log) {
        let _ = file.write_all(line.as_bytes());
    }
}
