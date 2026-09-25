//! The per-box egress proxy.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::core::plan::{Egress, Route, Target};
use crate::core::{network, rfc3339, tls};

/// The loopback URL clients reach through `pinfold init`'s relay.
pub const PROXY_URL: &str = "http://127.0.0.1:3128";

/// One request head. The cap only keeps a client from growing the buffer
/// without bound.
const MAX_HEAD: usize = 8 * 1024;

/// More headers than this get a 400.
const MAX_HEADERS: usize = 64;

/// How long the proxy waits for the next bytes of a request head or
/// ClientHello.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a forwarded body may go without data, or a tunnel with no
/// bytes in either direction, before the proxy closes it.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// The most connections one box may have open at once.
const MAX_CONNECTIONS: usize = 64;

/// Bind the box's socket, create its log, and serve. The accept thread is
/// never stopped: each owner exits right after teardown, which deletes the
/// socket.
pub fn start(socket: &Path, egress: &Egress, log: PathBuf) -> io::Result<()> {
    let rules = Arc::new(Rules::new(egress)?);
    let listener = UnixListener::bind(socket).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("proxy socket {}: {error}", socket.display()),
        )
    })?;
    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent)?;
    }
    // The log exists before the first decision, so it is found from the
    // host even for a box that made no request.
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)?;
    thread::spawn(move || serve(listener, rules, log));
    Ok(())
}

/// One box's egress rules: the allowlist and the routes.
struct Rules {
    /// Lowercased allowlist entries: a name, or `.name` for the name and
    /// its subdomains.
    allow: Vec<String>,
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
                    let (target, headers) = inject.resolve().map_err(io::Error::other)?;
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
            allow: egress
                .allow
                .iter()
                .map(|entry| entry.to_ascii_lowercase())
                .collect(),
            routes,
            tls,
        })
    }

    fn allows(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.allow
            .iter()
            .any(|entry| match entry.strip_prefix('.') {
                Some(name) if !name.is_empty() => host == name || host.ends_with(entry.as_str()),
                _ => host == *entry,
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

fn serve(listener: UnixListener, rules: Arc<Rules>, log: PathBuf) {
    let active = Arc::new(AtomicUsize::new(0));
    for client in listener.incoming() {
        let Ok(mut client) = client else {
            continue;
        };
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            refuse(&mut client, &log, "", 503, "connection cap");
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
}

fn handle(mut client: UnixStream, rules: &Rules, log: &Path) {
    if client.set_read_timeout(Some(HEADER_TIMEOUT)).is_err() {
        return;
    }
    let head = match read_head(&mut client) {
        Ok(head) => head,
        Err(error) if timed_out(&error) => {
            record(log, "", "refused", "header timeout");
            return;
        }
        Err(_) => return,
    };
    if !head_well_formed(&head) {
        return refuse(&mut client, log, "", 400, "ambiguous framing");
    }
    // One parse for both methods; httparse rejects a folded header, a
    // fourth token on the request line and more than MAX_HEADERS headers.
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    let (Ok(httparse::Status::Complete(_)), Some(method), Some(target)) =
        (request.parse(&head), request.method, request.path)
    else {
        return refuse(&mut client, log, "", 400, "malformed request");
    };
    if method == "CONNECT" {
        connect(&mut client, rules, log, target);
    } else {
        plain(&mut client, rules, log, &request);
    }
}

/// Handle one CONNECT: an allowlisted host on port 443 only, with the
/// ClientHello's SNI checked before the server is dialed.
fn connect(client: &mut UnixStream, rules: &Rules, log: &Path, target: &str) {
    let Some((host, port)) = target.rsplit_once(':') else {
        return refuse(client, log, "", 400, "malformed request");
    };
    if host.is_empty() {
        return refuse(client, log, "", 400, "malformed request");
    }
    if network::literal(host).is_some() {
        return refuse(client, log, host, 403, "ip literal");
    }
    if rules.routes.contains_key(&host.to_ascii_lowercase()) {
        return refuse(client, log, host, 403, "route");
    }
    if !rules.allows(host) {
        return refuse(client, log, host, 403, "not allowlisted");
    }
    if port.parse::<u16>() != Ok(443) {
        return refuse(client, log, host, 403, "port not allowed");
    }
    let Some(address) = resolve_checked(client, log, host, host, 443) else {
        return;
    };
    if respond(client, 200).is_err() {
        return;
    }
    // The client sends its ClientHello only after the 200, so it is read
    // here and forwarded unchanged once the SNI checks out.
    let hello = match tls::read_client_hello(client) {
        Ok(hello) => hello,
        Err(reason) => {
            record(log, host, "refused", reason);
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
        tunnel(client, server, log, host);
    }
}

/// Handle one plain HTTP request: an allowlisted host or a route, port 80
/// only, framed by Content-Length, one request per connection.
fn plain(client: &mut UnixStream, rules: &Rules, log: &Path, request: &httparse::Request) {
    let request = match parse_plain(request) {
        Ok(request) => request,
        Err(reason) => return refuse(client, log, "", 400, reason),
    };
    if request.port != 80 {
        return refuse(client, log, &request.host, 403, "port not allowed");
    }
    if network::literal(&request.host).is_some() {
        return refuse(client, log, &request.host, 403, "ip literal");
    }
    let host = request.host.to_ascii_lowercase();
    if let Some(upstream) = rules.routes.get(&host) {
        route(client, rules, log, &request, upstream);
        return;
    }
    if !rules.allows(&host) {
        return refuse(client, log, &request.host, 403, "not allowlisted");
    }
    let Some(address) = resolve_checked(client, log, &request.host, &host, 80) else {
        return;
    };
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

/// Serve one request to a route. A host service is dialed unchecked; an
/// `https` target is resolved and checked like an allowlisted host, then
/// dialed over TLS.
fn route(client: &mut UnixStream, rules: &Rules, log: &Path, request: &Plain, upstream: &Upstream) {
    let (server, authority, headers) = match upstream {
        Upstream::Address(address) => (
            dial_address(address.as_str()),
            request.authority.as_str(),
            [].as_slice(),
        ),
        Upstream::Inject { target, headers } if !target.https => (
            dial_address((target.host.as_str(), target.port)),
            target.authority.as_str(),
            headers.as_slice(),
        ),
        Upstream::Inject { target, headers } => {
            let Some(address) =
                resolve_checked(client, log, &request.host, &target.host, target.port)
            else {
                return;
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
            return;
        }
    };
    record(log, &request.host, "allowed", "route");
    let Some(mut server) = server else {
        let _ = respond(client, 502);
        return;
    };
    let _ = forward(client, &mut server, request, authority, headers);
}

/// Resolve once and check every address before anything is dialed. A
/// forbidden address is refused 403 with its reason, an unresolved name 502.
fn resolve_checked(
    client: &mut UnixStream,
    log: &Path,
    log_host: &str,
    host: &str,
    port: u16,
) -> Option<SocketAddr> {
    match network::resolve(host, port) {
        Ok(address) => Some(address),
        Err(network::ResolveError::Forbidden(reason)) => {
            refuse(client, log, log_host, 403, reason);
            None
        }
        Err(network::ResolveError::Unresolved) => {
            refuse(client, log, log_host, 502, "resolve failed");
            None
        }
    }
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
    /// The minor version: httparse yields 0 or 1.
    version: u8,
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
fn parse_plain(request: &httparse::Request) -> Result<Plain, &'static str> {
    let method = request.method.ok_or("malformed request")?.to_string();
    let version = request.version.ok_or("malformed request")?;
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
        let length = header.name.eq_ignore_ascii_case("content-length");
        if length && content_length.is_some() {
            return Err("ambiguous framing");
        }
        if header.name.eq_ignore_ascii_case("host") || hop_by_hop(header.name) {
            continue;
        }
        let value = std::str::from_utf8(header.value).map_err(|_| "malformed request")?;
        if length {
            content_length = Some(value.parse::<u64>().map_err(|_| "malformed request")?);
        }
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
        "{} {} HTTP/1.{}\r\n",
        request.method, request.target, request.version
    )?;
    write!(head, "Host: {host}\r\n")?;
    for (name, value) in request
        .headers
        .iter()
        .filter(|(name, _)| {
            !inject
                .iter()
                .any(|(injected, _)| injected.eq_ignore_ascii_case(name))
        })
        .chain(inject)
    {
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
/// also ends the read, so the caller can refuse it instead of blocking. EOF
/// and a head over the cap are `UnexpectedEof`; a timeout keeps its kind.
fn read_head(client: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        if client.read(&mut byte)? == 0 || head.len() >= MAX_HEAD {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
            return Ok(head);
        }
    }
}

/// Whether a read failed because its socket timeout expired.
fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// A head with only CRLF line endings. httparse accepts bare LF, so this
/// framing check is done on the bytes; a folded header it rejects itself.
fn head_well_formed(head: &[u8]) -> bool {
    head.ends_with(b"\r\n\r\n")
        && head.iter().enumerate().all(|(index, byte)| match byte {
            b'\n' => index > 0 && head[index - 1] == b'\r',
            b'\r' => head.get(index + 1) == Some(&b'\n'),
            _ => true,
        })
}

/// Resolve a route's address once and connect to the first. A route's
/// target is a host service, so its address is not checked.
fn dial_address(address: impl ToSocketAddrs) -> Option<TcpStream> {
    let address = address.to_socket_addrs().ok()?.next()?;
    TcpStream::connect(address).ok()
}

/// The socket calls a tunnel direction needs besides reading and writing.
trait TunnelSocket {
    fn set_timeout(&self, timeout: Option<Duration>);
    fn close(&self, how: Shutdown);
}

impl TunnelSocket for UnixStream {
    fn set_timeout(&self, timeout: Option<Duration>) {
        let _ = self.set_read_timeout(timeout);
    }
    fn close(&self, how: Shutdown) {
        let _ = self.shutdown(how);
    }
}

impl TunnelSocket for TcpStream {
    fn set_timeout(&self, timeout: Option<Duration>) {
        let _ = self.set_read_timeout(timeout);
    }
    fn close(&self, how: Shutdown) {
        let _ = self.shutdown(how);
    }
}

impl<T: TunnelSocket + ?Sized> TunnelSocket for &mut T {
    fn set_timeout(&self, timeout: Option<Duration>) {
        (**self).set_timeout(timeout);
    }
    fn close(&self, how: Shutdown) {
        (**self).close(how);
    }
}

/// Copy one direction for the life of the tunnel. Every read that moves
/// bytes stamps the shared activity. A read timeout waits out the rest of
/// `IDLE_TIMEOUT` while either direction has moved bytes; when none has,
/// both sockets close and the tunnel is marked idle. EOF or an error
/// half-closes the peer's write side.
fn copy_direction<R, W>(mut reader: R, mut writer: W, activity: &Mutex<Instant>, idle: &AtomicBool)
where
    R: Read + TunnelSocket,
    W: Write + TunnelSocket,
{
    let mut buffer = [0u8; 8 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                writer.close(Shutdown::Write);
                return;
            }
            Ok(n) => {
                if let Ok(mut last) = activity.lock() {
                    *last = Instant::now();
                }
                if writer.write_all(&buffer[..n]).is_err() {
                    writer.close(Shutdown::Write);
                    return;
                }
            }
            Err(error) if timed_out(&error) => {
                let elapsed = activity
                    .lock()
                    .map_or(Duration::ZERO, |last| last.elapsed());
                let Some(remaining) = IDLE_TIMEOUT.checked_sub(elapsed) else {
                    idle.store(true, Ordering::SeqCst);
                    reader.close(Shutdown::Both);
                    writer.close(Shutdown::Both);
                    return;
                };
                reader.set_timeout(Some(remaining.max(Duration::from_millis(1))));
            }
            Err(_) => {
                writer.close(Shutdown::Write);
                return;
            }
        }
    }
}

/// Copy both directions for the life of the tunnel. It is idle, and logged
/// closed, only when neither direction has moved bytes for `IDLE_TIMEOUT`.
fn tunnel(client: &mut UnixStream, server: TcpStream, log: &Path, host: &str) {
    let _ = client.set_read_timeout(Some(IDLE_TIMEOUT));
    let _ = server.set_read_timeout(Some(IDLE_TIMEOUT));
    let Ok(client_reader) = client.try_clone() else {
        return;
    };
    let Ok(server_writer) = server.try_clone() else {
        return;
    };
    let activity = Arc::new(Mutex::new(Instant::now()));
    let idle = Arc::new(AtomicBool::new(false));
    let up = {
        let activity = Arc::clone(&activity);
        let idle = Arc::clone(&idle);
        thread::spawn(move || {
            copy_direction(client_reader, server_writer, &activity, &idle);
        })
    };
    copy_direction(server, client, &activity, &idle);
    let _ = up.join();
    if idle.load(Ordering::SeqCst) {
        record(log, host, "closed", "idle timeout");
    }
}

fn respond(client: &mut UnixStream, code: u16) -> io::Result<()> {
    let reason = match code {
        200 => "Connection Established",
        400 => "Bad Request",
        403 => "Forbidden",
        502 => "Bad Gateway",
        _ => "Service Unavailable",
    };
    let framing = if code == 200 {
        ""
    } else {
        "Content-Length: 0\r\nConnection: close\r\n"
    };
    client.write_all(format!("HTTP/1.1 {code} {reason}\r\n{framing}\r\n").as_bytes())
}

/// Record one refusal and answer with its status.
fn refuse(client: &mut UnixStream, log: &Path, host: &str, code: u16, reason: &str) {
    record(log, host, "refused", reason);
    let _ = respond(client, code);
}

/// Append one decision as a JSON line. No header value is ever written.
fn record(log: &Path, host: &str, decision: &str, reason: &str) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let mut line = serde_json::json!({
        "time": rfc3339(seconds),
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
