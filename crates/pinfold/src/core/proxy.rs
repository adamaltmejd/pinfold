//! The per-box egress proxy.
//!
//! It lives in the `box up` process and listens only on the box's unix
//! socket, so it has no network listener. CONNECT is port 443 to an
//! allowlisted host; plain HTTP is port 80 to an allowlisted host or to a
//! route. The address checks come later.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crate::core::plan::Egress;

/// The loopback URL clients reach through `pinfold init`'s relay.
pub const PROXY_URL: &str = "http://127.0.0.1:3128";

/// macOS `sun_path` is 104 bytes including the terminator.
const SOCKET_PATH_LIMIT: usize = 104;

/// One request head. The cap only keeps a client from growing the buffer
/// without bound.
const MAX_HEAD: usize = 8 * 1024;

/// More headers than this get a 400.
const MAX_HEADERS: usize = 64;

/// A running proxy, owned by the `box up` process.
pub struct Proxy {
    socket: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Proxy {
    /// Bind the box's socket, create its log, and serve until closed.
    pub fn start(socket: PathBuf, egress: &Egress, log: PathBuf) -> io::Result<Proxy> {
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
        let rules = Arc::new(Rules::new(egress));
        let thread = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || serve(listener, stop, rules, log))
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

/// One box's egress rules: the allowlist and the routes.
struct Rules {
    allow: Allow,
    /// Lowercased route names mapped to `host:port` services.
    routes: BTreeMap<String, String>,
}

impl Rules {
    fn new(egress: &Egress) -> Rules {
        let routes = egress
            .routes
            .iter()
            .map(|(name, target)| (name.to_ascii_lowercase(), target.clone()))
            .collect();
        Rules {
            allow: Allow::new(&egress.allow),
            routes,
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

fn serve(listener: UnixListener, stop: Arc<AtomicBool>, rules: Arc<Rules>, log: PathBuf) {
    loop {
        match listener.accept() {
            Ok((client, _)) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let rules = Arc::clone(&rules);
                let log = log.clone();
                thread::spawn(move || handle(client, &rules, &log));
            }
            Err(_) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    }
}

fn handle(mut client: UnixStream, rules: &Rules, log: &Path) {
    let Some(head) = read_head(&mut client) else {
        return;
    };
    if !head_well_formed(&head) {
        let _ = respond(&mut client, 400);
        return;
    }
    let head_text = String::from_utf8_lossy(&head);
    let request_line = head_text.split("\r\n").next().unwrap_or("");
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
    if method == "CONNECT" {
        connect(&mut client, rules, log, target);
    } else {
        plain(&mut client, rules, log, &head);
    }
}

/// Handle one CONNECT: an allowlisted host on port 443 only.
fn connect(client: &mut UnixStream, rules: &Rules, log: &Path, target: &str) {
    let Some((host, port)) = target.rsplit_once(':') else {
        let _ = respond(client, 400);
        return;
    };
    if host.is_empty() {
        let _ = respond(client, 400);
        return;
    }
    if !rules.allow.allows(host) {
        record(log, host, "refused", "not allowlisted");
        let _ = respond(client, 403);
        return;
    }
    if port.parse::<u16>() != Ok(443) {
        record(log, host, "refused", "port not allowed");
        let _ = respond(client, 403);
        return;
    }
    record(log, host, "allowed", "allowlisted");
    // Resolve once and dial the address that was resolved.
    let Some(server) = dial(host, 443) else {
        let _ = respond(client, 502);
        return;
    };
    if respond(client, 200).is_err() {
        return;
    }
    tunnel(client, server);
}

/// Handle one plain HTTP request: an allowlisted host or a route, port 80
/// only, framed by Content-Length, one request per connection.
fn plain(client: &mut UnixStream, rules: &Rules, log: &Path, head: &[u8]) {
    let Some(request) = parse_plain(head) else {
        let _ = respond(client, 400);
        return;
    };
    if request.port != 80 {
        record(log, &request.host, "refused", "port not allowed");
        let _ = respond(client, 403);
        return;
    }
    let host = request.host.to_ascii_lowercase();
    let (server, reason) = if let Some(target) = rules.routes.get(&host) {
        (dial_address(target), "route")
    } else if rules.allow.allows(&host) {
        (dial(&host, 80), "allowlisted")
    } else {
        record(log, &request.host, "refused", "not allowlisted");
        let _ = respond(client, 403);
        return;
    };
    record(log, &request.host, "allowed", reason);
    let Some(mut server) = server else {
        let _ = respond(client, 502);
        return;
    };
    let _ = forward(client, &mut server, &request);
}

/// One parsed plain HTTP request head.
struct Plain {
    method: String,
    version: &'static str,
    /// Origin-form target for the upstream server.
    target: String,
    /// The authority from the absolute-form target, for the Host header.
    authority: String,
    /// The authority's host, without a port.
    host: String,
    /// The authority's port; 80 when it names none.
    port: u16,
    /// The headers to forward, hop-by-hop headers removed.
    headers: Vec<(String, String)>,
    /// The request body length, from the one Content-Length.
    content_length: u64,
}

/// Parse and check a plain HTTP request head. `None` is a 400: not
/// absolute-form, `https`, userinfo, ambiguous framing or an invalid header.
fn parse_plain(head: &[u8]) -> Option<Plain> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    if !request.parse(head).ok()?.is_complete() {
        return None;
    }
    let method = request.method?.to_string();
    let version = match request.version? {
        0 => "HTTP/1.0",
        1 => "HTTP/1.1",
        _ => return None,
    };
    let raw_target = request.path?;
    let (scheme, rest) = raw_target.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    // Userinfo would make the authority ambiguous.
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            (host, port.parse::<u16>().ok()?)
        }
        None => (authority, 80),
    };
    if host.is_empty() {
        return None;
    }
    let path = &rest[authority.len()..];
    // A fragment is client-side only; a query without a path still needs the
    // origin-form's leading slash.
    let path = path.split('#').next().unwrap_or("");
    let target = match path {
        "" => "/".to_string(),
        path if path.starts_with('?') => format!("/{path}"),
        path => path.to_string(),
    };
    let mut content_length = None;
    let mut forwarded = Vec::new();
    for header in request.headers.iter() {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            return None;
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return None;
            }
            let value = std::str::from_utf8(header.value).ok()?;
            content_length = Some(value.parse::<u64>().ok()?);
            forwarded.push((header.name.to_string(), value.to_string()));
            continue;
        }
        if header.name.eq_ignore_ascii_case("host") || hop_by_hop(header.name) {
            continue;
        }
        let value = std::str::from_utf8(header.value).ok()?;
        forwarded.push((header.name.to_string(), value.to_string()));
    }
    Some(Plain {
        method,
        version,
        target,
        authority: authority.to_string(),
        host: host.to_string(),
        port,
        headers: forwarded,
        content_length: content_length.unwrap_or(0),
    })
}

/// Forward one parsed request and its Content-Length body, then stream the
/// response back until the upstream closes. One request per connection.
fn forward(client: &mut UnixStream, server: &mut TcpStream, request: &Plain) -> io::Result<()> {
    let mut head = Vec::with_capacity(256);
    write!(
        head,
        "{} {} {}\r\n",
        request.method, request.target, request.version
    )?;
    write!(head, "Host: {}\r\n", request.authority)?;
    for (name, value) in &request.headers {
        write!(head, "{name}: {value}\r\n")?;
    }
    // Keep-alive is out of scope, so the upstream closes after answering.
    head.extend_from_slice(b"Connection: close\r\n\r\n");
    server.write_all(&head)?;
    if request.content_length > 0 {
        let mut body = Read::take(&mut *client, request.content_length);
        io::copy(&mut body, server)?;
    }
    let _ = server.shutdown(Shutdown::Write);
    io::copy(server, client)?;
    let _ = client.shutdown(Shutdown::Write);
    Ok(())
}

/// Whether a header is hop-by-hop and must not be forwarded.
fn hop_by_hop(name: &str) -> bool {
    const HOP_BY_HOP: [&str; 9] = [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ];
    HOP_BY_HOP.iter().any(|hop| name.eq_ignore_ascii_case(hop))
}

/// Read one request head, ending at the blank line. A bare-LF blank line
/// also ends the read, so the caller can refuse it instead of blocking.
fn read_head(client: &mut UnixStream) -> Option<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        match client.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
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

/// A head with only CRLF line endings and no folded header. httparse accepts
/// bare LF and continuations, so the framing checks are done on the bytes.
fn head_well_formed(head: &[u8]) -> bool {
    if !head.ends_with(b"\r\n\r\n") {
        return false;
    }
    for (index, byte) in head.iter().enumerate() {
        match byte {
            b'\n' if index == 0 || head[index - 1] != b'\r' => return false,
            b'\r' if head.get(index + 1) != Some(&b'\n') => return false,
            _ => {}
        }
    }
    head.split(|byte| *byte == b'\n')
        .skip(1)
        .all(|line| !matches!(line.first(), Some(b' ' | b'\t')))
}

/// Resolve `host` once and connect to the first address on `port`.
fn dial(host: &str, port: u16) -> Option<TcpStream> {
    let address = (host, port).to_socket_addrs().ok()?.next()?;
    TcpStream::connect(address).ok()
}

/// Resolve a route's `host:port` once and connect to the first address.
fn dial_address(address: &str) -> Option<TcpStream> {
    let address = address.to_socket_addrs().ok()?.next()?;
    TcpStream::connect(address).ok()
}

/// Copy both directions for the life of the tunnel.
fn tunnel(client: &mut UnixStream, server: TcpStream) {
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
    let _ = io::copy(&mut server, client);
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

