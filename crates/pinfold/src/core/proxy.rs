//! The per-box egress proxy.
//!
//! It lives in the `box up` process and listens only on the box's unix
//! socket, so it has no network listener. CONNECT is port 443 to an
//! allowlisted host; plain HTTP is port 80 to an allowlisted host or to a
//! route.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::core::plan::{Egress, Route, Target};
use crate::core::{network, tls};

/// The loopback URL clients reach through `pinfold init`'s relay.
pub const PROXY_URL: &str = "http://127.0.0.1:3128";

/// macOS `sun_path` is 104 bytes including the terminator.
const SOCKET_PATH_LIMIT: usize = 104;

/// One request head. The cap only keeps a client from growing the buffer
/// without bound.
const MAX_HEAD: usize = 8 * 1024;

/// More headers than this get a 400.
const MAX_HEADERS: usize = 64;

/// How long the proxy waits for the next bytes of a request head or
/// ClientHello.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a tunnel or forwarded body may go without data before the
/// proxy closes it.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// The most connections one box may have open at once.
const MAX_CONNECTIONS: usize = 64;

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
        // Before anything is created, so a route that cannot be served
        // leaves nothing behind.
        let rules = Arc::new(Rules::new(egress)?);
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
    /// Lowercased route names mapped to where they lead.
    routes: BTreeMap<String, Upstream>,
    /// The TLS client for `https` routes, with the host's roots. Present when
    /// one exists.
    tls: Option<Arc<ClientConfig>>,
}

/// A route's upstream. No `Debug`: an injected header holds a credential.
enum Upstream {
    /// A host service at `host:port`.
    Address(String),
    /// An injecting route's target and its headers, values already read.
    Inject {
        target: Target,
        headers: Vec<(String, String)>,
    },
}

impl Rules {
    /// The rules for one box. Injected header values are read here, once,
    /// from this process's environment.
    fn new(egress: &Egress) -> io::Result<Rules> {
        let mut routes = BTreeMap::new();
        for (name, route) in &egress.routes {
            let upstream = match route {
                Route::Address(address) => Upstream::Address(address.clone()),
                Route::Inject(inject) => {
                    let (target, headers) = inject.resolve().map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("route {name}: {error}"),
                        )
                    })?;
                    Upstream::Inject { target, headers }
                }
            };
            routes.insert(name.to_ascii_lowercase(), upstream);
        }
        let https = routes
            .values()
            .any(|upstream| matches!(upstream, Upstream::Inject { target, .. } if target.https));
        let tls = if https { Some(tls_config()?) } else { None };
        Ok(Rules {
            allow: Allow::new(&egress.allow),
            routes,
            tls,
        })
    }
}

/// A TLS client that verifies against the host's roots. Roots that fail to
/// load are skipped; with none, every `https` route fails verification.
fn tls_config() -> io::Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(io::Error::other)?
            .with_root_certificates(roots)
            .with_no_client_auth();
    Ok(Arc::new(config))
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
    let active = Arc::new(AtomicUsize::new(0));
    loop {
        match listener.accept() {
            Ok((mut client, _)) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    record(&log, "", "refused", "connection cap");
                    let _ = respond(&mut client, 503);
                    continue;
                }
                let rules = Arc::clone(&rules);
                let log = log.clone();
                let active = Arc::clone(&active);
                thread::spawn(move || {
                    handle(client, &rules, &log);
                    active.fetch_sub(1, Ordering::SeqCst);
                });
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
    if client.set_read_timeout(Some(HEADER_TIMEOUT)).is_err() {
        return;
    }
    let head = match read_head(&mut client) {
        Ok(head) => head,
        Err(HeadError::Timeout) => {
            record(log, "", "refused", "header timeout");
            return;
        }
        Err(HeadError::Closed) => return,
    };
    if !head_well_formed(&head) {
        record(log, "", "refused", "ambiguous framing");
        let _ = respond(&mut client, 400);
        return;
    }
    let head_text = String::from_utf8_lossy(&head);
    let request_line = head_text.split("\r\n").next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(_version)) = (parts.next(), parts.next(), parts.next())
    else {
        record(log, "", "refused", "malformed request");
        let _ = respond(&mut client, 400);
        return;
    };
    if parts.next().is_some() {
        record(log, "", "refused", "malformed request");
        let _ = respond(&mut client, 400);
        return;
    }
    if method == "CONNECT" {
        connect(&mut client, rules, log, target);
    } else {
        plain(&mut client, rules, log, &head);
    }
}

/// Handle one CONNECT: an allowlisted host on port 443 only, with the
/// ClientHello's SNI checked before the server is dialed.
fn connect(client: &mut UnixStream, rules: &Rules, log: &Path, target: &str) {
    let Some((host, port)) = target.rsplit_once(':') else {
        let _ = respond(client, 400);
        return;
    };
    if host.is_empty() {
        let _ = respond(client, 400);
        return;
    }
    if network::literal(host).is_some() {
        record(log, host, "refused", "ip literal");
        let _ = respond(client, 403);
        return;
    }
    if rules.routes.contains_key(&host.to_ascii_lowercase()) {
        record(log, host, "refused", "route");
        let _ = respond(client, 403);
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
    // Resolve once and check every address before anything is dialed.
    let address = match network::resolve(host, 443) {
        Ok(address) => address,
        Err(network::ResolveError::Forbidden(reason)) => {
            record(log, host, "refused", reason);
            let _ = respond(client, 403);
            return;
        }
        Err(network::ResolveError::Unresolved) => {
            record(log, host, "refused", "resolve failed");
            let _ = respond(client, 502);
            return;
        }
    };
    if respond(client, 200).is_err() {
        return;
    }
    // The client sends its ClientHello only after the 200, so it is read
    // here and forwarded unchanged once the SNI checks out.
    let hello = match tls::read_client_hello(client) {
        Ok(hello) => hello,
        Err(error) => {
            record(log, host, "refused", error.reason());
            return;
        }
    };
    match hello.sni.as_deref() {
        None => {
            record(log, host, "refused", "sni missing");
            return;
        }
        Some(sni) if !sni.eq_ignore_ascii_case(host) => {
            record(log, host, "refused", "sni mismatch");
            return;
        }
        Some(_) => {}
    }
    record(log, host, "allowed", "allowlisted");
    if let Ok(mut server) = TcpStream::connect(address) {
        if server.write_all(&hello.bytes).is_err() {
            return;
        }
        tunnel(client, server);
    }
}

/// Handle one plain HTTP request: an allowlisted host or a route, port 80
/// only, framed by Content-Length, one request per connection.
fn plain(client: &mut UnixStream, rules: &Rules, log: &Path, head: &[u8]) {
    let request = match parse_plain(head) {
        Ok(request) => request,
        Err(reason) => {
            record(log, "", "refused", reason);
            let _ = respond(client, 400);
            return;
        }
    };
    if request.port != 80 {
        record(log, &request.host, "refused", "port not allowed");
        let _ = respond(client, 403);
        return;
    }
    if network::literal(&request.host).is_some() {
        record(log, &request.host, "refused", "ip literal");
        let _ = respond(client, 403);
        return;
    }
    let host = request.host.to_ascii_lowercase();
    if let Some(upstream) = rules.routes.get(&host) {
        route(client, rules, log, &request, upstream);
        return;
    }
    if !rules.allow.allows(&host) {
        record(log, &request.host, "refused", "not allowlisted");
        let _ = respond(client, 403);
        return;
    }
    match network::resolve(&host, 80) {
        Ok(address) => {
            record(log, &request.host, "allowed", "allowlisted");
            match TcpStream::connect(address) {
                Ok(mut server) => {
                    let _ = forward(client, &mut server, &request, &request.authority, &[]);
                }
                Err(_) => {
                    let _ = respond(client, 502);
                }
            }
        }
        Err(network::ResolveError::Forbidden(reason)) => {
            record(log, &request.host, "refused", reason);
            let _ = respond(client, 403);
        }
        Err(network::ResolveError::Unresolved) => {
            record(log, &request.host, "refused", "resolve failed");
            let _ = respond(client, 502);
        }
    }
}

/// Serve one request to a route. A host service is dialed unchecked; an
/// `https` target is resolved and checked like an allowlisted host, then
/// dialed over TLS.
fn route(client: &mut UnixStream, rules: &Rules, log: &Path, request: &Plain, upstream: &Upstream) {
    let (target, headers) = match upstream {
        Upstream::Address(address) => {
            record(log, &request.host, "allowed", "route");
            let Some(mut server) = dial_address(address.as_str()) else {
                let _ = respond(client, 502);
                return;
            };
            let _ = forward(client, &mut server, request, &request.authority, &[]);
            return;
        }
        Upstream::Inject { target, headers } => (target, headers),
    };
    if !target.https {
        record(log, &request.host, "allowed", "route");
        let Some(mut server) = dial_address((target.host.as_str(), target.port)) else {
            let _ = respond(client, 502);
            return;
        };
        let _ = forward(client, &mut server, request, &target.authority, headers);
        return;
    }
    let address = match network::resolve(&target.host, target.port) {
        Ok(address) => address,
        Err(network::ResolveError::Forbidden(reason)) => {
            record(log, &request.host, "refused", reason);
            let _ = respond(client, 403);
            return;
        }
        Err(network::ResolveError::Unresolved) => {
            record(log, &request.host, "refused", "resolve failed");
            let _ = respond(client, 502);
            return;
        }
    };
    record(log, &request.host, "allowed", "route");
    let Some(mut server) = rules
        .tls
        .as_ref()
        .and_then(|config| dial_tls(address, &target.host, config))
    else {
        let _ = respond(client, 502);
        return;
    };
    let _ = forward_tls(client, &mut server, request, &target.authority, headers);
}

/// Connect to a checked address and complete the TLS handshake, with SNI
/// and the certificate checked against `host`.
fn dial_tls(
    address: SocketAddr,
    host: &str,
    config: &Arc<ClientConfig>,
) -> Option<StreamOwned<ClientConnection, TcpStream>> {
    let name = ServerName::try_from(host.to_string()).ok()?;
    let mut connection = ClientConnection::new(Arc::clone(config), name).ok()?;
    let mut server = TcpStream::connect(address).ok()?;
    server.set_read_timeout(Some(IDLE_TIMEOUT)).ok()?;
    while connection.is_handshaking() {
        connection.complete_io(&mut server).ok()?;
    }
    Some(StreamOwned::new(connection, server))
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

/// Parse and check a plain HTTP request head. The error is the 400's log
/// reason: not absolute-form, `https`, userinfo, ambiguous framing or an
/// invalid header.
fn parse_plain(head: &[u8]) -> Result<Plain, &'static str> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    match request.parse(head) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return Err("malformed request"),
    }
    let method = request.method.ok_or("malformed request")?.to_string();
    let version = match request.version.ok_or("malformed request")? {
        0 => "HTTP/1.0",
        1 => "HTTP/1.1",
        _ => return Err("malformed request"),
    };
    let raw_target = request.path.ok_or("malformed request")?;
    let (scheme, rest) = raw_target.split_once("://").ok_or("malformed request")?;
    if !scheme.eq_ignore_ascii_case("http") {
        return Err("malformed request");
    }
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .ok_or("malformed request")?;
    // Userinfo would make the authority ambiguous.
    if authority.is_empty() || authority.contains('@') {
        return Err("malformed request");
    }
    let (host, port) = network::authority_host(authority, 80).ok_or("malformed request")?;
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
            return Err("ambiguous framing");
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("ambiguous framing");
            }
            let value = std::str::from_utf8(header.value).map_err(|_| "malformed request")?;
            content_length = Some(value.parse::<u64>().map_err(|_| "malformed request")?);
            forwarded.push((header.name.to_string(), value.to_string()));
            continue;
        }
        if header.name.eq_ignore_ascii_case("host") || hop_by_hop(header.name) {
            continue;
        }
        let value = std::str::from_utf8(header.value).map_err(|_| "malformed request")?;
        forwarded.push((header.name.to_string(), value.to_string()));
    }
    Ok(Plain {
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
fn forward(
    client: &mut UnixStream,
    server: &mut TcpStream,
    request: &Plain,
    host: &str,
    inject: &[(String, String)],
) -> io::Result<()> {
    let _ = server.set_read_timeout(Some(IDLE_TIMEOUT));
    send(client, server, request, host, inject)?;
    let _ = server.shutdown(Shutdown::Write);
    relay(server, client)
}

/// [`forward`] over TLS. There is no half-close: a close_notify before the
/// response would end the exchange. The server's close ends the response,
/// with or without its own close_notify.
fn forward_tls(
    client: &mut UnixStream,
    server: &mut StreamOwned<ClientConnection, TcpStream>,
    request: &Plain,
    host: &str,
    inject: &[(String, String)],
) -> io::Result<()> {
    send(client, server, request, host, inject)?;
    server.flush()?;
    relay(server, client)
}

/// Write the request head with `host` as its Host header, then the body.
/// Each injected header replaces any the box sent under the same name.
fn send(
    client: &mut UnixStream,
    server: &mut impl Write,
    request: &Plain,
    host: &str,
    inject: &[(String, String)],
) -> io::Result<()> {
    let _ = client.set_read_timeout(Some(IDLE_TIMEOUT));
    let mut head = Vec::with_capacity(256);
    write!(
        head,
        "{} {} {}\r\n",
        request.method, request.target, request.version
    )?;
    write!(head, "Host: {host}\r\n")?;
    for (name, value) in &request.headers {
        if !inject
            .iter()
            .any(|(injected, _)| injected.eq_ignore_ascii_case(name))
        {
            write!(head, "{name}: {value}\r\n")?;
        }
    }
    for (name, value) in inject {
        write!(head, "{name}: {value}\r\n")?;
    }
    // Keep-alive is out of scope, so the upstream closes after answering.
    head.extend_from_slice(b"Connection: close\r\n\r\n");
    server.write_all(&head)?;
    if request.content_length > 0 {
        let mut body = Read::take(&mut *client, request.content_length);
        io::copy(&mut body, server)?;
    }
    Ok(())
}

/// Stream the response back until the upstream closes.
fn relay(server: &mut impl Read, client: &mut UnixStream) -> io::Result<()> {
    io::copy(server, client)?;
    let _ = client.shutdown(Shutdown::Write);
    Ok(())
}

/// Whether a header is hop-by-hop and must not be forwarded.
pub fn hop_by_hop(name: &str) -> bool {
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
fn read_head(client: &mut UnixStream) -> Result<Vec<u8>, HeadError> {
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        match client.read(&mut byte) {
            Ok(0) => return Err(HeadError::Closed),
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
                    return Ok(head);
                }
                if head.len() > MAX_HEAD {
                    return Err(HeadError::Closed);
                }
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                return Err(HeadError::Timeout);
            }
            Err(_) => return Err(HeadError::Closed),
        }
    }
}

/// Why the request head could not be read.
enum HeadError {
    /// The peer closed or the socket failed.
    Closed,
    /// The header timeout expired.
    Timeout,
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

/// Resolve a route's address once and connect to the first. A route's
/// target is a host service, so its address is not checked.
fn dial_address(address: impl ToSocketAddrs) -> Option<TcpStream> {
    let address = address.to_socket_addrs().ok()?.next()?;
    TcpStream::connect(address).ok()
}

/// Copy both directions for the life of the tunnel.
fn tunnel(client: &mut UnixStream, server: TcpStream) {
    let mut server = server;
    let _ = client.set_read_timeout(Some(IDLE_TIMEOUT));
    let _ = server.set_read_timeout(Some(IDLE_TIMEOUT));
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
                503 => "Service Unavailable",
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
