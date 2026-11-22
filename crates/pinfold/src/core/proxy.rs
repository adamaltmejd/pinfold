//! The per-box egress proxy.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak, mpsc};

use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::socket::{MsgFlags, recv};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::core::login;
use crate::core::plan::{Egress, Route, Target};
use crate::core::{network, now, rfc3339, runs, tls};
use crate::init::Deadline;

/// The loopback URL clients reach through `pinfold init`'s relay.
pub const PROXY_URL: &str = "http://127.0.0.1:3128";

/// One request head, or an upstream's status line. The cap only keeps a
/// client or upstream from growing the buffer without bound.
const MAX_HEAD: usize = 8 * 1024;

const MAX_HEADERS: usize = 64;

/// How long the proxy waits for the next bytes of a request head or
/// ClientHello.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);

const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a forwarded body may go without data, or a tunnel with no
/// bytes in either direction, before the proxy closes it.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

const MAX_CONNECTIONS: usize = 64;

static LOOKUPS: AtomicUsize = AtomicUsize::new(0);

struct LookupSlot;

impl Drop for LookupSlot {
    fn drop(&mut self) {
        LOOKUPS.fetch_sub(1, Ordering::AcqRel);
    }
}

const MAX_LOG_BYTES: u64 = 128 * 1024 * 1024;

/// A proxy's listener, request workers and in-memory credentials.
pub struct Server {
    pub failure: oneshot::Receiver<()>,
    control: Arc<Control>,
    worker: Option<thread::JoinHandle<io::Result<()>>>,
}

impl Server {
    /// Stop admission and interrupt every registered stream before joining.
    pub fn stop(&mut self) -> io::Result<()> {
        self.control.stop();
        self.worker.take().map_or(Ok(()), |worker| {
            worker
                .join()
                .map_err(|_| io::Error::other("proxy worker panicked"))?
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[derive(Default)]
struct Control {
    cancelled: AtomicBool,
    connections: Mutex<Vec<Weak<Connection>>>,
}

impl Control {
    fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
        for connection in self
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
        {
            connection.stop();
        }
    }
}

struct Connection {
    control: Arc<Control>,
    client: UnixStream,
    servers: Mutex<Vec<TcpStream>>,
}

impl Connection {
    fn cancelled(&self) -> bool {
        self.control.cancelled.load(Ordering::Acquire)
    }

    fn stop(&self) {
        let _ = self.client.shutdown(Shutdown::Both);
        for server in self
            .servers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            let _ = server.shutdown(Shutdown::Both);
        }
    }

    fn server(&self, server: &TcpStream) -> io::Result<()> {
        let mut servers = self.servers.lock().unwrap_or_else(PoisonError::into_inner);
        if self.cancelled() {
            let _ = server.shutdown(Shutdown::Both);
            return Err(io::ErrorKind::Interrupted.into());
        }
        servers.push(server.try_clone()?);
        Ok(())
    }
}

/// Serve the bound box socket and create its log. All request workers,
/// including a synchronous token refresh, are joined by `Server::stop`.
pub fn start(
    listener: UnixListener,
    egress: &Egress,
    codex: Option<login::Token>,
    log: PathBuf,
) -> io::Result<Server> {
    let rules = Arc::new(Rules::new(egress, codex)?);
    runs::open_log(&log)?;
    listener.set_nonblocking(true)?;
    let (failure, failed) = oneshot::channel();
    let log = Arc::new(AuditLog {
        path: log,
        failure: Mutex::new(Some(failure)),
    });
    let control = Arc::new(Control::default());
    let worker_control = control.clone();
    let worker =
        thread::Builder::new().spawn(move || serve(listener, rules, log, worker_control))?;
    Ok(Server {
        failure: failed,
        control,
        worker: Some(worker),
    })
}

struct Rules {
    /// Lowercased allowlist entries: a name, or `.name` for the name and
    /// its subdomains.
    allow: Vec<String>,
    /// Lowercased route names mapped to where they lead.
    routes: BTreeMap<String, Upstream>,
}

/// A route's upstream. No `Debug`: an injected header holds a credential.
enum Upstream {
    /// A host service at `host:port`, dialed unchecked. The box's own
    /// authority stays the Host header.
    Address(Target),
    /// An injecting route's target and its headers, values already read.
    /// Static login routes apply the login credential policy.
    Inject {
        target: Target,
        headers: Vec<(String, String)>,
        login: bool,
    },
    /// The target of a login route on the host's Codex login; its headers
    /// come from the live token on each request. Every such route in a box
    /// shares one token and its refresh.
    Codex {
        target: Target,
        login: Arc<login::Codex>,
    },
}

impl Rules {
    /// The rules for one box. Injected header values are read here, once,
    /// from this process's environment; a codex login route takes `codex`.
    fn new(egress: &Egress, codex: Option<login::Token>) -> io::Result<Rules> {
        let codex = codex.map(|token| Arc::new(login::Codex::new(token)));
        let mut routes = BTreeMap::new();
        for (name, route) in &egress.routes {
            let upstream = match route {
                Route::Address(address) => {
                    let (host, port) = network::authority_host(address, 0)
                        .expect("validate refuses a route target that is not host:port");
                    Upstream::Address(Target {
                        https: false,
                        host: host.to_string(),
                        port,
                        authority: address.clone(),
                    })
                }
                Route::Inject(inject) => {
                    let (target, headers) = inject.resolve().map_err(io::Error::other)?;
                    Upstream::Inject {
                        target,
                        headers,
                        login: false,
                    }
                }
                Route::Login(login) if login.uses_host_codex() => Upstream::Codex {
                    target: login.target().map_err(io::Error::other)?,
                    login: codex.clone().expect("up resolves a Codex login's token"),
                },
                Route::Login(login) => {
                    let (target, headers) = login.resolve().map_err(io::Error::other)?;
                    Upstream::Inject {
                        target,
                        headers,
                        login: true,
                    }
                }
            };
            routes.insert(name.to_ascii_lowercase(), upstream);
        }
        Ok(Rules {
            allow: egress
                .allow
                .iter()
                .map(|entry| entry.to_ascii_lowercase())
                .collect(),
            routes,
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

/// The TLS client for `https` routes, built on the first one. It verifies
/// against the host's roots; roots that fail to load are skipped, and with
/// none every `https` route fails verification. ring is rustls's only
/// provider feature, so `builder` picks it.
fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let mut roots = RootCertStore::empty();
        roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    });
    Arc::clone(config)
}

fn serve(
    listener: UnixListener,
    rules: Arc<Rules>,
    log: Arc<AuditLog>,
    control: Arc<Control>,
) -> io::Result<()> {
    let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
    let accepted = (|| {
        while !control.cancelled.load(Ordering::Acquire) && !log.failed() {
            let mut index = 0;
            while index < workers.len() {
                if workers[index].is_finished() {
                    workers
                        .swap_remove(index)
                        .join()
                        .map_err(|_| io::Error::other("proxy request worker panicked"))?;
                } else {
                    index += 1;
                }
            }
            let mut ready = [PollFd::new(listener.as_fd(), PollFlags::POLLIN)];
            match poll(&mut ready, 100u16) {
                Ok(0) | Err(nix::errno::Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
            let accepted = {
                let _guard = crate::core::process::fork_guard();
                listener.accept()
            };
            let (mut client, _) = match accepted {
                Ok(client) => client,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(error) => return Err(error),
            };
            // Darwin may inherit the listener's nonblocking flag.
            client.set_nonblocking(false)?;
            if client.set_write_timeout(Some(IDLE_TIMEOUT)).is_err() {
                continue;
            }
            let connection = Arc::new(Connection {
                control: control.clone(),
                client: client.try_clone()?,
                servers: Mutex::new(Vec::new()),
            });
            {
                let mut connections = control
                    .connections
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                connections.retain(|connection| connection.strong_count() > 0);
                connections.push(Arc::downgrade(&connection));
                if connection.cancelled() {
                    connection.stop();
                }
            }
            if workers.len() >= MAX_CONNECTIONS {
                refuse(&mut client, &log, "", 503, "connection cap");
                continue;
            }
            let rules = Arc::clone(&rules);
            let log = log.clone();
            workers.push(
                thread::Builder::new().spawn(move || handle(client, &rules, &log, &connection))?,
            );
        }
        Ok(())
    })();
    drop(listener);
    control.stop();
    let mut joined = Ok(());
    for worker in workers {
        if worker.join().is_err() {
            joined = Err(io::Error::other("proxy request worker panicked"));
        }
    }
    accepted.and(joined)
}

fn handle(mut client: UnixStream, rules: &Rules, log: &AuditLog, connection: &Connection) {
    if connection.cancelled()
        || log.failed()
        || client.set_read_timeout(Some(HEADER_TIMEOUT)).is_err()
    {
        return;
    }
    let head = match read_head(&mut client) {
        Ok(head) => head,
        Err(error) if timed_out(&error) => {
            let _ = record(log, "", "refused", "header timeout", None);
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
    let (Ok(httparse::Status::Complete(_)), Some(method), Some(version), Some(target)) = (
        request.parse(&head),
        request.method,
        request.version,
        request.path,
    ) else {
        return refuse(&mut client, log, "", 400, "malformed request");
    };
    if method == "CONNECT" {
        return connect(&mut client, rules, log, target, connection);
    }
    match parse_plain(method, version, target, request.headers) {
        Ok(request) => plain(&mut client, rules, log, &request, connection),
        Err(reason) => refuse(&mut client, log, "", 400, reason),
    }
}

/// Handle one CONNECT: an allowlisted host on port 443 only, with the
/// ClientHello's SNI checked before the server is dialed.
fn connect(
    client: &mut UnixStream,
    rules: &Rules,
    log: &AuditLog,
    target: &str,
    connection: &Connection,
) {
    let Some((host, port)) = target.rsplit_once(':').filter(|(host, _)| !host.is_empty()) else {
        return refuse(client, log, "", 400, "malformed request");
    };
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
    let Some(address) = resolve_checked(client, log, host, host, 443, connection) else {
        return;
    };
    if respond(client, 200).is_err() {
        return;
    }
    // The client sends its ClientHello only after the 200, so it is read
    // here and forwarded unchanged once the SNI checks out.
    let hello = match tls::read_client_hello(client, host) {
        Ok(hello) => hello,
        Err(reason) => {
            let _ = record(log, host, "refused", reason, None);
            return;
        }
    };
    if record(log, host, "allowed", "allowlisted", None).is_err() {
        return;
    }
    if let Some(server) = dial(address, connection) {
        if Deadline(server.as_fd(), IDLE_TIMEOUT)
            .write_all(&hello)
            .is_err()
        {
            return;
        }
        tunnel(client, server, log, host);
    }
}

/// Handle one plain HTTP request: an allowlisted host or a route, port 80
/// only, framed by Content-Length, one request per connection.
fn plain(
    client: &mut UnixStream,
    rules: &Rules,
    log: &AuditLog,
    request: &Plain,
    connection: &Connection,
) {
    if request.port != 80 {
        return refuse(client, log, &request.host, 403, "port not allowed");
    }
    if network::literal(&request.host).is_some() {
        return refuse(client, log, &request.host, 403, "ip literal");
    }
    let host = request.host.to_ascii_lowercase();
    if let Some(upstream) = rules.routes.get(&host) {
        route(client, log, request, upstream, connection);
        return;
    }
    if !rules.allows(&host) {
        return refuse(client, log, &request.host, 403, "not allowlisted");
    }
    let Some(address) = resolve_checked(client, log, &request.host, &host, 80, connection) else {
        return;
    };
    if record(log, &request.host, "allowed", "allowlisted", None).is_err() {
        return;
    }
    let _ = forward(
        client,
        dial(address, connection).map(DeadlineStream::new),
        request,
        &request.authority,
        &[],
        &[],
        &mut |_| Ok(()),
    );
}

/// Serve one request to a route. A host service is dialed unchecked; an
/// `https` target is resolved and checked like an allowlisted host, then
/// dialed over TLS. A codex login whose token has lapsed is refused before
/// anything is dialed.
fn route(
    client: &mut UnixStream,
    log: &AuditLog,
    request: &Plain,
    upstream: &Upstream,
    connection: &Connection,
) {
    let mut report = |status: Option<u16>| {
        let path = request
            .target
            .split_once('?')
            .map_or(request.target.as_str(), |(path, _)| path);
        let fields = serde_json::json!({
            "method": request.method,
            "path": path,
            "status": status,
        });
        record(log, &request.host, "allowed", "route", Some(fields))
    };
    let login_policy = matches!(
        upstream,
        Upstream::Inject { login: true, .. } | Upstream::Codex { .. }
    );
    let remove = if login_policy {
        match login_removals(request) {
            Ok(remove) => remove,
            Err((status, reason)) => return refuse(client, log, &request.host, status, reason),
        }
    } else {
        Vec::new()
    };
    let (target, host, headers) = match upstream {
        Upstream::Address(target) => (target, &request.authority, Cow::Borrowed(&[][..])),
        Upstream::Inject {
            target, headers, ..
        } => (target, &target.authority, Cow::Borrowed(headers.as_slice())),
        Upstream::Codex { target, login } => match login.headers(&connection.control.cancelled) {
            Some(headers) => (target, &target.authority, Cow::Owned(headers)),
            None => return refuse(client, log, &request.host, 403, "login lapsed"),
        },
    };
    if !target.https {
        let server = lookup(&target.host, target.port, false, connection)
            .ok()
            .and_then(|address| dial(address, connection))
            .map(DeadlineStream::new);
        let _ = forward(
            client,
            server,
            request,
            host,
            &headers,
            &remove,
            &mut report,
        );
        return;
    }
    let Some(address) = resolve_checked(
        client,
        log,
        &request.host,
        &target.host,
        target.port,
        connection,
    ) else {
        return;
    };
    let server = dial_tls(address, &target.host, connection);
    let _ = forward(
        client,
        server,
        request,
        host,
        &headers,
        &remove,
        &mut report,
    );
}

/// Resolve once and check every address before anything is dialed. A
/// forbidden address is refused 403 with its reason, an unresolved name 502.
fn resolve_checked(
    client: &mut UnixStream,
    log: &AuditLog,
    log_host: &str,
    host: &str,
    port: u16,
    connection: &Connection,
) -> Option<SocketAddr> {
    let resolved = lookup(host, port, true, connection);
    if connection.cancelled() {
        return None;
    }
    match resolved {
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

// Keep the deadline below TLS: rustls's ciphertext writes must use it,
// including writes while reading or flushing the TLS stream.
struct DeadlineStream(Option<TcpStream>);

impl DeadlineStream {
    fn new(stream: TcpStream) -> Self {
        Self(Some(stream))
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0
            .as_mut()
            .ok_or(io::ErrorKind::NotConnected)?
            .read(buffer)
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let stream = self.0.as_ref().ok_or(io::ErrorKind::NotConnected)?;
        // Darwin ignores MSG_DONTWAIT on blocking sockets. This socket
        // has no parallel reader or cloned handle; restore before reading.
        #[cfg(target_os = "macos")]
        let result = stream.set_nonblocking(true).and_then(|()| {
            let sent = Deadline(stream.as_fd(), IDLE_TIMEOUT).write(buffer);
            stream.set_nonblocking(false)?;
            sent
        });
        #[cfg(not(target_os = "macos"))]
        let result = Deadline(stream.as_fd(), IDLE_TIMEOUT).write(buffer);
        // rustls can defer a write error, then wait for more guest bytes.
        // Close the sole upstream handle before returning that error.
        if result.is_err() {
            drop(self.0.take());
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.as_mut().ok_or(io::ErrorKind::NotConnected)?.flush()
    }
}

/// Connect to a checked address and complete the TLS handshake, with SNI
/// and the certificate checked against `host`.
fn dial_tls(
    address: SocketAddr,
    host: &str,
    request: &Connection,
) -> Option<StreamOwned<ClientConnection, DeadlineStream>> {
    let name = ServerName::try_from(host.to_string()).ok()?;
    let mut connection = ClientConnection::new(tls_config(), name).ok()?;
    let mut server = DeadlineStream::new(dial(address, request)?);
    while connection.is_handshaking() {
        connection.complete_io(&mut server).ok()?;
    }
    Some(StreamOwned::new(connection, server))
}

struct Plain {
    method: String,
    /// The minor version: httparse yields 0 or 1.
    version: u8,
    /// Origin-form target for the upstream server.
    target: String,
    /// The authority from the absolute-form target, for the Host header.
    authority: String,
    host: String,
    /// The authority's port; 80 when it names none.
    port: u16,
    /// The headers to forward, hop-by-hop headers removed.
    headers: Vec<(String, String)>,
    /// Raw Connection fields, before hop-by-hop removal.
    connection: Vec<Vec<u8>>,
    /// The request body length, from the one Content-Length.
    content_length: u64,
}

/// Parse and check a plain HTTP request head. The error is the 400's log
/// reason: not absolute-form, `https`, userinfo, ambiguous framing or an
/// invalid header.
fn parse_plain(
    method: &str,
    version: u8,
    raw_target: &str,
    headers: &[httparse::Header],
) -> Result<Plain, &'static str> {
    let (url, path) = network::absolute_url(raw_target).ok_or("malformed request")?;
    if url.https {
        return Err("malformed request");
    }
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
    let mut connection = Vec::new();
    for header in headers {
        if header.name.eq_ignore_ascii_case("connection") {
            connection.push(header.value.to_vec());
        }
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
            // RFC 9110's 1*DIGIT: `parse` alone would also take a leading `+`.
            let digits = value.bytes().all(|byte| byte.is_ascii_digit());
            let parsed = value.parse::<u64>().ok().filter(|_| digits);
            content_length = Some(parsed.ok_or("ambiguous framing")?);
        }
        forwarded.push((header.name.to_string(), value.to_string()));
    }
    Ok(Plain {
        method: method.to_string(),
        version,
        target,
        authority: url.authority,
        host: url.host,
        port: url.port,
        headers: forwarded,
        connection,
        content_length: content_length.unwrap_or(0),
    })
}

/// Validate login channels before asking the helper or contacting upstream.
/// Only guest fields are removed; trusted headers are appended afterward.
fn login_removals(request: &Plain) -> Result<Vec<&str>, (u16, &'static str)> {
    let mut remove = Vec::new();
    for value in &request.connection {
        let value = std::str::from_utf8(value).map_err(|_| (400, "malformed request"))?;
        for option in value
            .split(',')
            .map(|option| option.trim_matches([' ', '\t']))
        {
            if option.is_empty() {
                continue;
            }
            if !option
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
            {
                return Err((400, "malformed request"));
            }
            if option.eq_ignore_ascii_case("content-length") {
                return Err((400, "ambiguous framing"));
            }
            remove.push(option);
        }
    }
    if let Some((_, query)) = request.target.split_once('?') {
        let mut forbidden = false;
        for field in query.split(['&', ';']) {
            let (name, value) = field.split_once('=').unwrap_or((field, ""));
            let mut name = query_decode(name).ok_or((400, "malformed request"))?;
            query_decode(value).ok_or((400, "malformed request"))?;
            for byte in &mut name {
                *byte = match byte {
                    b'_' => b'-',
                    _ => byte.to_ascii_lowercase(),
                };
            }
            forbidden |= [
                "key",
                "api-key",
                "apikey",
                "x-api-key",
                "access-token",
                "refresh-token",
                "id-token",
                "auth-token",
                "bearer-token",
                "token",
                "client-secret",
                "authorization",
                "chatgpt-account-id",
                "openai-organization",
                "openai-project",
            ]
            .iter()
            .any(|known| name == known.as_bytes());
        }
        if forbidden {
            return Err((403, "login credential query"));
        }
    }
    const CREDENTIALS: [&str; 11] = [
        "Authorization",
        "X-Api-Key",
        "Api-Key",
        "X-Goog-Api-Key",
        "X-Auth-Token",
        "X-Access-Token",
        "ChatGPT-Account-ID",
        "OpenAI-Organization",
        "OpenAI-Project",
        "Cookie",
        "Cookie2",
    ];
    remove.extend(request.headers.iter().filter_map(|(name, _)| {
        (CREDENTIALS
            .iter()
            .any(|known| name.eq_ignore_ascii_case(known))
            || name
                .get(..6)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("proxy-")))
        .then_some(name.as_str())
    }));
    Ok(remove)
}

/// Decode query bytes once, including strict escapes in parameter values.
fn query_decode(value: &str) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(match byte {
            b'%' => {
                let high = char::from(bytes.next()?).to_digit(16)?;
                let low = char::from(bytes.next()?).to_digit(16)?;
                (high * 16 + low) as u8
            }
            b'+' => b' ',
            _ => byte,
        });
    }
    Some(decoded)
}

/// Forward one parsed request with `host` as its Host header, then its
/// Content-Length body, and stream the response back until the upstream
/// closes; a failed dial (`None`) is answered 502. `report` gets the
/// upstream's status code once its status line has arrived, or `None` when
/// the exchange ends first. One request per
/// connection. Each injected header replaces any the box sent under the
/// same name, and a box-sent header named in `remove` is dropped. There is
/// no upstream half-close: over TLS a close_notify before the response
/// would end the exchange, and the server's close ends the response, with
/// or without its own close_notify.
fn forward(
    client: &mut UnixStream,
    server: Option<impl Read + Write>,
    request: &Plain,
    host: &str,
    inject: &[(String, String)],
    remove: &[&str],
    report: &mut dyn FnMut(Option<u16>) -> io::Result<()>,
) -> io::Result<()> {
    let Some(mut server) = server else {
        report(None)?;
        return respond(client, 502);
    };
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
                .map(|(injected, _)| injected.as_str())
                .chain(remove.iter().copied())
                .any(|dropped| dropped.eq_ignore_ascii_case(name))
        })
        .chain(inject)
    {
        write!(head, "{name}: {value}\r\n")?;
    }
    // Keep-alive is out of scope, so the upstream closes after answering.
    head.extend_from_slice(b"Connection: close\r\n\r\n");
    let mut exchange = || -> io::Result<(Vec<u8>, Option<u16>)> {
        server.write_all(&head)?;
        let mut body = Read::take(&mut *client, request.content_length);
        io::copy(&mut body, &mut server)?;
        server.flush()?;
        read_status_line(&mut server)
    };
    let (line, status) = match exchange() {
        Ok(sent) => sent,
        Err(error) => {
            report(None)?;
            return Err(error);
        }
    };
    report(status)?;
    let mut writer = Deadline(client.as_fd(), IDLE_TIMEOUT);
    writer.write_all(&line)?;
    io::copy(&mut server, &mut writer)?;
    let _ = client.shutdown(Shutdown::Write);
    Ok(())
}

/// Read the upstream's response status line, ending at its LF, and return
/// the bytes read plus the parsed code. The bytes are relayed as sent; a
/// line that does not name an HTTP status, or no line at all, is `None`.
fn read_status_line(server: &mut impl Read) -> io::Result<(Vec<u8>, Option<u16>)> {
    let line = read_until(server, |line| line.ends_with(b"\n"))?;
    let status = line
        .strip_prefix(b"HTTP/")
        .and_then(|rest| rest.split(|byte| *byte == b' ').nth(1))
        .and_then(|code| std::str::from_utf8(code).ok())
        .and_then(|code| code.parse::<u16>().ok());
    Ok((line, status))
}

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
    let end = |head: &[u8]| head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n");
    let head = read_until(client, end)?;
    if !end(&head) {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(head)
}

/// Read one byte at a time until `done`, EOF or MAX_HEAD bytes.
fn read_until(reader: &mut impl Read, done: impl Fn(&[u8]) -> bool) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while bytes.len() < MAX_HEAD && !done(&bytes) {
        if reader.read(&mut byte)? == 0 {
            break;
        }
        bytes.push(byte[0]);
    }
    Ok(bytes)
}

fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// A head with only CRLF line endings. httparse accepts bare LF, so this
/// framing check is done on the bytes; a folded header it rejects itself.
fn head_well_formed(head: &[u8]) -> bool {
    head.iter().enumerate().all(|(index, byte)| match byte {
        b'\n' => index > 0 && head[index - 1] == b'\r',
        b'\r' => head.get(index + 1) == Some(&b'\n'),
        _ => true,
    })
}

/// A system resolver may not return. Its worker holds only the name and
/// lookup parameters; cancellation discards its result without dialing.
fn lookup(
    host: &str,
    port: u16,
    checked: bool,
    connection: &Connection,
) -> Result<SocketAddr, network::ResolveError> {
    if connection.cancelled() {
        return Err(network::ResolveError::Unresolved);
    }
    LOOKUPS
        .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < MAX_CONNECTIONS).then_some(count + 1)
        })
        .map_err(|_| network::ResolveError::Unresolved)?;
    let slot = LookupSlot;
    let host = host.to_string();
    let (send, receive) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("proxy-lookup".into())
        .spawn(move || {
            let _slot = slot;
            let resolved = if checked {
                network::resolve(&host, port)
            } else {
                (host.as_str(), port)
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut addresses| addresses.next())
                    .ok_or(network::ResolveError::Unresolved)
            };
            let _ = send.send(resolved);
        })
        .map_err(|_| network::ResolveError::Unresolved)?;
    let deadline = Instant::now() + LOOKUP_TIMEOUT;
    loop {
        if connection.cancelled() || Instant::now() >= deadline {
            return Err(network::ResolveError::Unresolved);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receive.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(result) => {
                let _ = worker.join();
                return if connection.cancelled() || Instant::now() >= deadline {
                    Err(network::ResolveError::Unresolved)
                } else {
                    result
                };
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                return Err(network::ResolveError::Unresolved);
            }
        }
    }
}

/// Connect to an already resolved address and bound idle reads/writes.
fn dial(address: SocketAddr, connection: &Connection) -> Option<TcpStream> {
    if connection.cancelled() {
        return None;
    }
    let server = connect_upstream(address, connection)?;
    server.set_read_timeout(Some(IDLE_TIMEOUT)).ok()?;
    // Deadline still needs this fallback on Darwin's blocking sockets.
    server.set_write_timeout(Some(IDLE_TIMEOUT)).ok()?;
    Some(server)
}

/// Register the socket before connect so teardown can interrupt a pending dial.
fn connect_upstream(address: SocketAddr, connection: &Connection) -> Option<TcpStream> {
    use nix::libc;
    let domain = if address.is_ipv4() {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };
    #[cfg(target_os = "macos")]
    let creation = crate::core::process::fork_guard();
    #[cfg(target_os = "linux")]
    let kind = libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK;
    #[cfg(target_os = "macos")]
    let kind = libc::SOCK_STREAM;
    // SAFETY: socket has no pointer arguments and returns a new descriptor.
    let fd = unsafe { libc::socket(domain, kind, 0) };
    if fd < 0 {
        return None;
    }
    // SAFETY: this descriptor belongs exclusively to this stream.
    let server = unsafe { TcpStream::from_raw_fd(fd) };
    #[cfg(target_os = "macos")]
    {
        // SAFETY: fd is live; process creation is excluded until CLOEXEC is set.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return None;
        }
        drop(creation);
        server.set_nonblocking(true).ok()?;
    }
    connection.server(&server).ok()?;
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let length = match address {
        SocketAddr::V4(address) => {
            let mut raw: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            raw.sin_family = libc::AF_INET as _;
            raw.sin_port = address.port().to_be();
            raw.sin_addr.s_addr = u32::from_ne_bytes(address.ip().octets());
            #[cfg(target_os = "macos")]
            {
                raw.sin_len = std::mem::size_of_val(&raw) as _;
            }
            // SAFETY: storage is aligned for and larger than sockaddr_in.
            unsafe {
                std::ptr::write((&mut storage as *mut libc::sockaddr_storage).cast(), raw);
            }
            std::mem::size_of::<libc::sockaddr_in>()
        }
        SocketAddr::V6(address) => {
            let mut raw: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
            raw.sin6_family = libc::AF_INET6 as _;
            raw.sin6_port = address.port().to_be();
            raw.sin6_addr.s6_addr = address.ip().octets();
            raw.sin6_flowinfo = address.flowinfo().to_be();
            raw.sin6_scope_id = address.scope_id();
            #[cfg(target_os = "macos")]
            {
                raw.sin6_len = std::mem::size_of_val(&raw) as _;
            }
            // SAFETY: storage is aligned for and larger than sockaddr_in6.
            unsafe {
                std::ptr::write((&mut storage as *mut libc::sockaddr_storage).cast(), raw);
            }
            std::mem::size_of::<libc::sockaddr_in6>()
        }
    };
    // SAFETY: storage contains the matching address structure and length.
    let connected = unsafe {
        libc::connect(
            fd,
            (&storage as *const libc::sockaddr_storage).cast(),
            length as _,
        )
    };
    if connected < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS) {
        return None;
    }
    if connected < 0 {
        loop {
            if connection.cancelled() {
                return None;
            }
            let mut ready = [PollFd::new(server.as_fd(), PollFlags::POLLOUT)];
            match poll(&mut ready, 100u16) {
                Ok(0) | Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => return None,
                Ok(_) => break,
            }
        }
        if server.take_error().ok()?.is_some() {
            return None;
        }
    }
    if connection.cancelled() {
        return None;
    }
    server.set_nonblocking(false).ok()?;
    Some(server)
}

/// How one tunnel direction ended. EOF preserves the opposite direction;
/// a failed or idle direction interrupts both sockets before the join.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyEnd {
    Eof,
    Idle,
    WriteTimeout,
    Failed,
}

/// Reads share their activity clock, so a one-way active tunnel stays open.
fn copy_direction(reader: impl AsFd, writer: impl AsFd, activity: &Mutex<Instant>) -> CopyEnd {
    let mut writer = Deadline(writer.as_fd(), IDLE_TIMEOUT);
    let mut buffer = [0u8; 8 * 1024];
    let fd = reader.as_fd();
    let mut ready = [PollFd::new(fd, PollFlags::POLLIN)];
    loop {
        let elapsed = activity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed();
        let Some(remaining) = IDLE_TIMEOUT.checked_sub(elapsed) else {
            return CopyEnd::Idle;
        };
        // Long socket receive timeouts can be rounded up by Linux's timer
        // wheel. Poll waits on the shared deadline without that rounding.
        let timeout = PollTimeout::try_from(remaining.max(Duration::from_millis(1)))
            .expect("five-minute timeout fits poll");
        match poll(&mut ready, timeout) {
            Ok(0) | Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return CopyEnd::Failed,
            Ok(_) => {}
        }
        // MSG_DONTWAIT handles stale readiness without making writes nonblocking.
        let received = match recv(fd.as_raw_fd(), &mut buffer, MsgFlags::MSG_DONTWAIT) {
            Ok(0) => return CopyEnd::Eof,
            Ok(received) => received,
            Err(nix::errno::Errno::EAGAIN | nix::errno::Errno::EINTR) => continue,
            Err(_) => return CopyEnd::Failed,
        };
        if let Err(error) = writer.write_all(&buffer[..received]) {
            return if timed_out(&error) {
                CopyEnd::WriteTimeout
            } else {
                CopyEnd::Failed
            };
        }
        *activity.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }
}

/// Both sockets are interrupted on failure, including the peer's writer.
/// Ordinary EOF closes only the destination's upload half.
fn tunnel(client: &UnixStream, server: TcpStream, log: &AuditLog, host: &str) {
    let activity = Mutex::new(Instant::now());
    let (upload, download) = thread::scope(|scope| {
        let up = scope.spawn(|| {
            let end = copy_direction(client, &server, &activity);
            let _ = server.shutdown(if end == CopyEnd::Eof {
                Shutdown::Write
            } else {
                let _ = client.shutdown(Shutdown::Both);
                Shutdown::Both
            });
            end
        });
        let end = copy_direction(&server, client, &activity);
        let _ = client.shutdown(if end == CopyEnd::Eof {
            Shutdown::Write
        } else {
            let _ = server.shutdown(Shutdown::Both);
            Shutdown::Both
        });
        (up.join().unwrap_or(CopyEnd::Failed), end)
    });
    if [upload, download].contains(&CopyEnd::Idle) {
        let _ = record(log, host, "closed", "idle timeout", None);
    } else if [upload, download].contains(&CopyEnd::WriteTimeout) {
        let _ = record(log, host, "closed", "write timeout", None);
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
    Deadline(client.as_fd(), IDLE_TIMEOUT)
        .write_all(format!("HTTP/1.1 {code} {reason}\r\n{framing}\r\n").as_bytes())
}

fn refuse(client: &mut UnixStream, log: &AuditLog, host: &str, code: u16, reason: &str) {
    if record(log, host, "refused", reason, None).is_ok() {
        let _ = respond(client, code);
    }
}

/// Append one decision as a JSON line. A route decision also names the
/// method, the request path and the upstream status. No header value is ever
/// written.
fn record(
    log: &AuditLog,
    host: &str,
    decision: &str,
    reason: &str,
    route: Option<serde_json::Value>,
) -> io::Result<()> {
    let mut entry = serde_json::json!({
        "time": rfc3339(now() as i64),
        "host": host,
        "decision": decision,
        "reason": reason,
    });
    if let (Some(entry), Some(serde_json::Value::Object(route))) = (entry.as_object_mut(), route) {
        entry.extend(route);
    }
    let mut line = entry.to_string();
    line.push('\n');
    let mut failure = log.failure.lock().unwrap_or_else(PoisonError::into_inner);
    if failure.is_none() {
        return Err(io::Error::other("egress audit log failed"));
    }
    let written = runs::open_log(&log.path).and_then(|mut file| {
        let bytes = file
            .metadata()?
            .len()
            .checked_add(line.len() as u64)
            .ok_or_else(|| io::Error::other("egress audit log size overflow"))?;
        if bytes > MAX_LOG_BYTES {
            return Err(io::Error::other("egress audit log is full"));
        }
        file.write_all(line.as_bytes())
    });
    if written.is_err() {
        if let Some(failure) = failure.take() {
            let _ = failure.send(());
        }
        return Err(io::Error::other("egress audit log failed"));
    }
    Ok(())
}

/// The append lock also latches the first failure and notifies the owner.
struct AuditLog {
    path: PathBuf,
    failure: Mutex<Option<oneshot::Sender<()>>>,
}

impl AuditLog {
    fn failed(&self) -> bool {
        self.failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
    }
}
