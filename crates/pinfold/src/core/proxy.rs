//! The per-box egress proxy.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::fd::{AsFd, AsRawFd};
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
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};

use crate::core::login;
use crate::core::plan::{Egress, Route, Target, Values};
use crate::core::{ca, network, now, rfc3339, runs, tls};
use crate::init::Deadline;

/// The loopback URL clients reach through `pinfold init`'s relay.
pub const PROXY_URL: &str = "http://127.0.0.1:3128";

/// The box's `NO_PROXY`: its own loopback servers are not egress (#497).
pub const NO_PROXY: &str = "localhost,127.0.0.1,::1";

/// Whether clients honouring [`NO_PROXY`] skip the proxy for `name`, as
/// curl does: an entry matches itself and, as a domain, its subdomains,
/// ignoring one trailing dot.
pub fn bypassed(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase();
    NO_PROXY
        .split(',')
        .any(|entry| name == entry || name.ends_with(&format!(".{entry}")))
}

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
/// `issued` is the leaf routed hosts are served with.
pub fn start(
    listener: UnixListener,
    egress: &Egress,
    values: &Values,
    codex: Option<login::Token>,
    issued: Option<ca::Issued>,
    log: PathBuf,
) -> io::Result<Server> {
    let rules = Arc::new(Rules::new(egress, values, codex, issued)?);
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
    /// Lowercased names of plain routes, served over plain HTTP, mapped to
    /// where they lead.
    routes: BTreeMap<String, Upstream>,
    /// Lowercased routed hosts, served over TLS the proxy terminates.
    routed: BTreeMap<String, Upstream>,
    /// The box-side TLS server for routed hosts; `None` without one.
    server: Option<Arc<ServerConfig>>,
}

/// A route's upstream. No `Debug`: an injected header holds a credential.
enum Upstream {
    /// A host service at `host:port`, dialed unchecked. The box's own
    /// authority stays the Host header.
    Address(Target),
    /// An injecting route's target and its headers, values already read.
    /// Static login routes apply the login credential policy. `base_path`
    /// goes before each request's path.
    Inject {
        target: Target,
        base_path: &'static str,
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
    /// The rules for one box. Injected header values come from `values`; a
    /// codex login route takes `codex`. Two routes that share a routed host
    /// serve it alike (`validate`), so either's entry stands.
    fn new(
        egress: &Egress,
        values: &Values,
        codex: Option<login::Token>,
        issued: Option<ca::Issued>,
    ) -> io::Result<Rules> {
        let codex = codex.map(|token| Arc::new(login::Codex::new(token)));
        let mut routes = BTreeMap::new();
        let mut routed = BTreeMap::new();
        for (name, route) in &egress.routes {
            match route {
                Route::Address(address) => {
                    let (host, port) = network::authority_host(address, 0)
                        .expect("validate refuses a route target that is not host:port");
                    let target = Target {
                        https: false,
                        host: host.to_string(),
                        port,
                        authority: address.clone(),
                    };
                    routes.insert(name.to_ascii_lowercase(), Upstream::Address(target));
                }
                Route::Inject(inject) => {
                    let (target, headers) =
                        inject.resolve(name, values).map_err(io::Error::other)?;
                    let (https, host) = (target.https, target.host.to_ascii_lowercase());
                    let upstream = Upstream::Inject {
                        target,
                        base_path: "",
                        headers,
                        login: false,
                    };
                    if https {
                        routed.insert(host, upstream);
                    } else {
                        routes.insert(name.to_ascii_lowercase(), upstream);
                    }
                }
                Route::Login(login) if login.uses_host_codex() => {
                    for serving in login.served().map_err(io::Error::other)? {
                        let upstream = Upstream::Codex {
                            target: login.target().map_err(io::Error::other)?,
                            login: codex.clone().expect("up resolves a Codex login's token"),
                        };
                        routed.insert(serving.name, upstream);
                    }
                }
                Route::Login(login) => {
                    let served = login.resolve(name, values).map_err(|error| {
                        io::Error::other(format!("{} login route {name:?}: {error}", login.login))
                    })?;
                    for served in served {
                        routed.insert(
                            served.name,
                            Upstream::Inject {
                                target: served.target,
                                base_path: served.base_path,
                                headers: served.headers,
                                login: true,
                            },
                        );
                    }
                }
            }
        }
        let server = match issued {
            Some(issued) => {
                let mut config = ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(vec![issued.leaf], issued.key)
                    .map_err(|error| io::Error::other(format!("box TLS server: {error}")))?;
                config.alpn_protocols = vec![b"http/1.1".to_vec()];
                Some(Arc::new(config))
            }
            None => None,
        };
        Ok(Rules {
            allow: egress
                .allowed()
                .iter()
                .map(|entry| entry.to_ascii_lowercase())
                .collect(),
            routes,
            routed,
            server,
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
            for worker in workers.extract_if(.., |worker| worker.is_finished()) {
                worker
                    .join()
                    .map_err(|_| io::Error::other("proxy request worker panicked"))?;
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
                refuse(
                    &mut Client::Plain(&mut client),
                    &log,
                    "",
                    None,
                    503,
                    "connection cap",
                );
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

fn handle(mut socket: UnixStream, rules: &Rules, log: &AuditLog, connection: &Connection) {
    if connection.cancelled()
        || log.failed()
        || socket.set_read_timeout(Some(HEADER_TIMEOUT)).is_err()
    {
        return;
    }
    let client = &mut Client::Plain(&mut socket);
    let Some((method, version, target, headers)) = parse_head(client, log, "") else {
        return;
    };
    if method == "CONNECT" {
        return connect(&mut socket, rules, log, &target, connection);
    }
    match parse_plain(&method, version, &target, &headers) {
        Ok(request) => plain(client, rules, log, &request, connection),
        Err(reason) => refuse(client, log, "", None, 400, reason),
    }
}

/// Read one head and return its method, minor version, target and headers,
/// or `None` once it is refused or the read fails. One parse for every
/// head; httparse rejects a folded header, a fourth token on the request
/// line and more than MAX_HEADERS headers.
#[allow(clippy::type_complexity)]
fn parse_head(
    client: &mut Client,
    log: &AuditLog,
    host: &str,
) -> Option<(String, u8, String, Vec<(String, Vec<u8>)>)> {
    let head = match read_head(client) {
        Ok(head) => head,
        Err(error) if timed_out(&error) => {
            let _ = record(log, host, "refused", "header timeout", None);
            return None;
        }
        Err(_) => return None,
    };
    let head = head.as_slice();
    if !head_well_formed(head) {
        refuse(client, log, host, None, 400, "ambiguous framing");
        return None;
    }
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    let (Ok(httparse::Status::Complete(_)), Some(method), Some(version), Some(target)) = (
        request.parse(head),
        request.method,
        request.version,
        request.path,
    ) else {
        refuse(client, log, host, None, 400, "malformed request");
        return None;
    };
    let headers = request
        .headers
        .iter()
        .map(|header| (header.name.to_string(), header.value.to_vec()))
        .collect();
    Some((method.to_string(), version, target.to_string(), headers))
}

/// Handle one CONNECT on port 443 only: to a routed host, terminated
/// whether or not the allowlist also names it; else to an allowlisted
/// host, with the ClientHello's SNI checked before the server is dialed.
fn connect(
    socket: &mut UnixStream,
    rules: &Rules,
    log: &AuditLog,
    target: &str,
    connection: &Connection,
) {
    let client = &mut Client::Plain(&mut *socket);
    let Some((host, port)) = target.rsplit_once(':').filter(|(host, _)| !host.is_empty()) else {
        return refuse(client, log, "", None, 400, "malformed request");
    };
    if network::literal(host).is_some() {
        return refuse(client, log, host, Some("CONNECT"), 403, "ip literal");
    }
    let lower = host.to_ascii_lowercase();
    if rules.routes.contains_key(&lower) {
        return refuse(client, log, host, Some("CONNECT"), 403, "route");
    }
    // The routed set comes first: an allowlist entry naming a routed host
    // must not open a tunnel the proxy cannot read.
    if let Some(upstream) = rules.routed.get(&lower) {
        if port.parse::<u16>() != Ok(443) {
            return refuse(client, log, host, Some("CONNECT"), 403, "port not allowed");
        }
        return intercept(socket, rules, log, host, upstream, connection);
    }
    if !rules.allows(host) {
        return refuse(client, log, host, Some("CONNECT"), 403, "not allowlisted");
    }
    if port.parse::<u16>() != Ok(443) {
        return refuse(client, log, host, Some("CONNECT"), 403, "port not allowed");
    }
    let Some(address) = resolve_checked(
        client,
        log,
        host,
        Some("CONNECT"),
        None,
        host,
        443,
        connection,
    ) else {
        return;
    };
    if respond(client, 200).is_err() {
        return;
    }
    // The client sends its ClientHello only after the 200, so it is read
    // here and forwarded unchanged once the SNI checks out.
    let hello = match tls::read_client_hello(socket, host) {
        Ok((_, hello)) => hello,
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
        tunnel(socket, server, log, host);
    }
}

/// End the box's TLS to a routed host with the box's leaf, then serve one
/// origin-form request under the route rules. A handshake the box aborts,
/// as one that does not trust its CA does, is logged.
fn intercept(
    socket: &mut UnixStream,
    rules: &Rules,
    log: &AuditLog,
    host: &str,
    upstream: &Upstream,
    connection: &Connection,
) {
    let config = rules
        .server
        .clone()
        .expect("a box with a routed host has a TLS server");
    if respond(&mut Client::Plain(&mut *socket), 200).is_err() {
        return;
    }
    let accepted = match tls::read_client_hello(socket, host) {
        Ok((accepted, _)) => accepted,
        Err(reason) => {
            let _ = record(log, host, "refused", reason, None);
            return;
        }
    };
    let mut side = BoxSide(socket);
    let mut server = match accepted.into_connection(config) {
        Ok(server) => server,
        Err((_, mut alert)) => {
            let _ = alert.write_all(&mut side);
            let _ = record(log, host, "closed", "tls handshake", None);
            return;
        }
    };
    while server.is_handshaking() {
        if server.complete_io(&mut side).is_err() {
            let _ = record(log, host, "closed", "tls handshake", None);
            return;
        }
    }
    let client = &mut Client::Tls(Box::new(StreamOwned::new(server, side)));
    let Some((method, version, target, headers)) = parse_head(client, log, host) else {
        return;
    };
    let request = match parse_origin(&method, version, &target, host, &headers) {
        Ok(request) => request,
        // Unparsed, so empty, as plain HTTP's.
        Err(reason) => return refuse(client, log, host, None, 400, reason),
    };
    route(client, log, &request, upstream, connection);
}

/// Handle one plain HTTP request: an allowlisted host or a route, port 80
/// only, framed by Content-Length, one request per connection.
fn plain(
    client: &mut Client,
    rules: &Rules,
    log: &AuditLog,
    request: &Plain,
    connection: &Connection,
) {
    let deny = |client: &mut Client, code: u16, reason: &'static str| {
        refuse(
            client,
            log,
            &request.host,
            Some(&request.method),
            code,
            reason,
        )
    };
    let host = request.host.to_ascii_lowercase();
    // Before any other check; a route's or routed host's refusal names the
    // request as its other lines do.
    if request.expect == Some(false) {
        let routed = rules.routes.contains_key(&host) || rules.routed.contains_key(&host);
        return refuse_with(
            client,
            log,
            &request.host,
            Some(&request.method),
            routed.then(|| route_fields(request, None)),
            417,
            "expectation unsupported",
        );
    }
    if request.port != 80 {
        return deny(client, 403, "port not allowed");
    }
    if network::literal(&request.host).is_some() {
        return deny(client, 403, "ip literal");
    }
    // A routed host takes TLS only, so a credential is never sent in clear.
    if rules.routed.contains_key(&host) {
        return deny(client, 403, "route");
    }
    if let Some(upstream) = rules.routes.get(&host) {
        route(client, log, request, upstream, connection);
        return;
    }
    if !rules.allows(&host) {
        return deny(client, 403, "not allowlisted");
    }
    let Some(address) = resolve_checked(
        client,
        log,
        &request.host,
        Some(&request.method),
        None,
        &host,
        80,
        connection,
    ) else {
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
        "",
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
    client: &mut Client,
    log: &AuditLog,
    request: &Plain,
    upstream: &Upstream,
    connection: &Connection,
) {
    let mut report = |status: Option<u16>| {
        record(
            log,
            &request.host,
            "allowed",
            "route",
            Some(route_fields(request, status)),
        )
    };
    let deny = |client: &mut Client, code: u16, reason: &'static str| {
        refuse_with(
            client,
            log,
            &request.host,
            Some(&request.method),
            Some(route_fields(request, None)),
            code,
            reason,
        )
    };
    if request.expect == Some(false) {
        return deny(client, 417, "expectation unsupported");
    }
    let login_policy = matches!(
        upstream,
        Upstream::Inject { login: true, .. } | Upstream::Codex { .. }
    );
    let remove = if login_policy {
        match login_removals(request) {
            Ok(remove) => remove,
            Err((status, reason)) => {
                return deny(client, status, reason);
            }
        }
    } else {
        Vec::new()
    };
    let (target, host, base_path, headers) = match upstream {
        Upstream::Address(target) => (target, &request.authority, "", Cow::Borrowed(&[][..])),
        Upstream::Inject {
            target,
            base_path,
            headers,
            ..
        } => (
            target,
            &target.authority,
            *base_path,
            Cow::Borrowed(headers.as_slice()),
        ),
        Upstream::Codex { target, login } => match login.headers(&connection.control.cancelled) {
            Some(headers) => (target, &target.authority, "", Cow::Owned(headers)),
            None => {
                return deny(client, 403, "login lapsed");
            }
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
            base_path,
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
        Some(&request.method),
        Some(request),
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
        base_path,
        &headers,
        &remove,
        &mut report,
    );
}

/// Resolve once and check every address before anything is dialed. A
/// forbidden address is refused 403 with its reason, an unresolved name 502;
/// a route request's refusal names it as its other lines do.
#[allow(clippy::too_many_arguments)]
fn resolve_checked(
    client: &mut Client,
    log: &AuditLog,
    log_host: &str,
    method: Option<&str>,
    route: Option<&Plain>,
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
        Err(error) => {
            let (code, reason) = match error {
                network::ResolveError::Forbidden(reason) => (403, reason),
                network::ResolveError::Unresolved => (502, "resolve failed"),
            };
            let fields = route.map(|request| route_fields(request, None));
            refuse_with(client, log, log_host, method, fields, code, reason);
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
    /// An HTTP/1.1 request's `Expect`: `Some(true)` when every member is
    /// `100-continue`, which the proxy answers itself; `Some(false)` is
    /// refused 417.
    expect: Option<bool>,
}

/// Parse and check a plain HTTP request head. The error is the 400's log
/// reason: not absolute-form, `https`, userinfo, ambiguous framing or an
/// invalid header.
fn parse_plain(
    method: &str,
    version: u8,
    raw_target: &str,
    headers: &[(String, Vec<u8>)],
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
    parse_request(
        method,
        version,
        target,
        url.authority,
        url.host,
        url.port,
        headers,
    )
}

/// Parse and check a request inside TLS to the routed `host`: origin form
/// only. The box's own `Host` is dropped like any other.
fn parse_origin(
    method: &str,
    version: u8,
    target: &str,
    host: &str,
    headers: &[(String, Vec<u8>)],
) -> Result<Plain, &'static str> {
    if !target.starts_with('/') || target.contains('#') {
        return Err("malformed request");
    }
    let host = host.to_ascii_lowercase();
    parse_request(
        method,
        version,
        target.to_string(),
        host.clone(),
        host,
        443,
        headers,
    )
}

fn parse_request(
    method: &str,
    version: u8,
    target: String,
    authority: String,
    host: String,
    port: u16,
    headers: &[(String, Vec<u8>)],
) -> Result<Plain, &'static str> {
    let mut content_length = None;
    let mut forwarded = Vec::new();
    let mut connection = Vec::new();
    // Whether the `Expect` list has a member, and whether each is
    // `100-continue`.
    let mut expect: Option<(bool, bool)> = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("connection") {
            connection.push(value.clone());
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("ambiguous framing");
        }
        let length = name.eq_ignore_ascii_case("content-length");
        if length && content_length.is_some() {
            return Err("ambiguous framing");
        }
        // Never forwarded: the proxy answers it. RFC 9110 has an HTTP/1.0
        // server ignore it. All its fields form one comma list, judged
        // once every field is in.
        if name.eq_ignore_ascii_case("expect") {
            if version == 1 {
                let members = value
                    .split(|byte| *byte == b',')
                    .map(|mut member| {
                        // SP and HTAB only: other bytes, U+00A0 included,
                        // make another expectation.
                        while let [b' ' | b'\t', rest @ ..] = member {
                            member = rest;
                        }
                        while let [rest @ .., b' ' | b'\t'] = member {
                            member = rest;
                        }
                        member
                    })
                    .filter(|member| !member.is_empty());
                let (mut any, mut all) = expect.unwrap_or((false, true));
                for member in members {
                    any = true;
                    all &= member.eq_ignore_ascii_case(b"100-continue");
                }
                expect = Some((any, all));
            }
            continue;
        }
        if name.eq_ignore_ascii_case("host") || hop_by_hop(name) {
            continue;
        }
        let value = std::str::from_utf8(value).map_err(|_| "malformed request")?;
        if length {
            // RFC 9110's 1*DIGIT: `parse` alone would also take a leading `+`.
            let digits = value.bytes().all(|byte| byte.is_ascii_digit());
            let parsed = value.parse::<u64>().ok().filter(|_| digits);
            content_length = Some(parsed.ok_or("ambiguous framing")?);
        }
        forwarded.push((name.clone(), value.to_string()));
    }
    Ok(Plain {
        method: method.to_string(),
        version,
        target,
        authority,
        host,
        port,
        headers: forwarded,
        connection,
        content_length: content_length.unwrap_or(0),
        expect: expect.map(|(any, all)| any && all),
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

/// Forward one parsed request with `host` as its Host header and
/// `base_path` before its path, then its Content-Length body, and stream
/// the response back until the upstream closes; a failed dial (`None`) is
/// answered 502. `report` gets the upstream's status code once its status
/// line has arrived, or `None` when the exchange ends first. One request
/// per connection. Each injected header replaces any the box sent under
/// the same name, and a box-sent header named in `remove` is dropped.
/// There is no upstream half-close: over TLS a close_notify before the
/// response would end the exchange, and the server's close ends the
/// response, with or without its own close_notify.
#[allow(clippy::too_many_arguments)]
fn forward(
    client: &mut Client,
    server: Option<impl Read + Write>,
    request: &Plain,
    host: &str,
    base_path: &str,
    inject: &[(String, String)],
    remove: &[&str],
    report: &mut dyn FnMut(Option<u16>) -> io::Result<()>,
) -> io::Result<()> {
    let Some(mut server) = server else {
        report(None)?;
        return respond(client, 502);
    };
    let _ = client.socket().set_read_timeout(Some(IDLE_TIMEOUT));
    let mut head = Vec::with_capacity(256);
    write!(
        head,
        "{} {base_path}{} HTTP/1.{}\r\n",
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
        if request.expect == Some(true) {
            client.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
            client.flush()?;
        }
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
    client.write_all(&line)?;
    io::copy(&mut server, client)?;
    client.finish();
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
fn read_head(client: &mut impl Read) -> io::Result<Vec<u8>> {
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
    let server = connect_upstream(address, connection)?;
    server.set_read_timeout(Some(IDLE_TIMEOUT)).ok()?;
    // Deadline still needs this fallback on Darwin's blocking sockets.
    server.set_write_timeout(Some(IDLE_TIMEOUT)).ok()?;
    Some(server)
}

/// Register the socket before connect so teardown can interrupt a pending dial.
fn connect_upstream(address: SocketAddr, connection: &Connection) -> Option<TcpStream> {
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, SockaddrStorage, connect, socket};
    let domain = if address.is_ipv4() {
        AddressFamily::Inet
    } else {
        AddressFamily::Inet6
    };
    #[cfg(target_os = "macos")]
    let creation = crate::core::process::fork_guard();
    #[cfg(target_os = "linux")]
    let flags = SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK;
    #[cfg(target_os = "macos")]
    let flags = SockFlag::empty();
    let server = TcpStream::from(socket(domain, SockType::Stream, flags, None).ok()?);
    #[cfg(target_os = "macos")]
    {
        use nix::fcntl::{FcntlArg, FdFlag, fcntl};
        // Process creation is excluded until CLOEXEC is set.
        fcntl(&server, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).ok()?;
        drop(creation);
        server.set_nonblocking(true).ok()?;
    }
    connection.server(&server).ok()?;
    let pending = match connect(server.as_raw_fd(), &SockaddrStorage::from(address)) {
        Ok(()) => false,
        Err(nix::errno::Errno::EINPROGRESS) => true,
        Err(_) => return None,
    };
    if pending {
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

fn respond(client: &mut Client, code: u16) -> io::Result<()> {
    let framing = if code == 200 {
        ""
    } else {
        "Content-Length: 0\r\nConnection: close\r\n"
    };
    client.write_all(format!("HTTP/1.1 {code} {}\r\n{framing}\r\n", phrase(code)).as_bytes())?;
    client.flush()
}

fn phrase(code: u16) -> &'static str {
    match code {
        200 => "Connection Established",
        400 => "Bad Request",
        403 => "Forbidden",
        417 => "Expectation Failed",
        502 => "Bad Gateway",
        _ => "Service Unavailable",
    }
}

/// Log a refusal and answer it with its reason (Egress proxy, Refusal
/// responses). The reason is one of the spec's fixed tokens, so nothing the
/// box sent, not even the host, reaches the header or the body.
/// `method` is the parsed request's, `None` when the refusal comes before
/// a request parsed. Only a parsed request other than `HEAD` gets the text
/// body: a HEAD response ends at its head (RFC 9110, 9.3.2), and an
/// unparsed request may have been a HEAD.
fn refuse(
    client: &mut Client,
    log: &AuditLog,
    host: &str,
    method: Option<&str>,
    code: u16,
    reason: &'static str,
) {
    refuse_with(client, log, host, method, None, code, reason)
}

/// A route request's log fields: its method, its path without the query,
/// and the upstream's status, null when none arrived.
fn route_fields(request: &Plain, status: Option<u16>) -> serde_json::Value {
    let path = request
        .target
        .split_once('?')
        .map_or(request.target.as_str(), |(path, _)| path);
    serde_json::json!({ "method": request.method, "path": path, "status": status })
}

/// [`refuse`], its log line carrying a route request's `fields`.
fn refuse_with(
    client: &mut Client,
    log: &AuditLog,
    host: &str,
    method: Option<&str>,
    fields: Option<serde_json::Value>,
    code: u16,
    reason: &'static str,
) {
    if record(log, host, "refused", reason, fields).is_err() {
        return;
    }
    let error = match reason {
        "not allowlisted"
        | "route"
        | "port not allowed"
        | "login lapsed"
        | "login credential query" => "http_request_denied",
        "malformed request" | "ambiguous framing" | "expectation unsupported" => {
            "http_request_error"
        }
        "resolve failed" => "dns_error",
        "connection cap" => "proxy_internal_response",
        // `ip literal` and a refused address's class.
        _ => "destination_ip_prohibited",
    };
    // A HEAD's Content-Length must be its GET's (RFC 9110, 8.6), so a
    // refused HEAD sends none; `Connection: close` ends it.
    let (framing, body) = match method {
        None => ("Content-Length: 0\r\n".to_string(), String::new()),
        Some("HEAD") => (String::new(), String::new()),
        Some(_) => {
            let body = format!(
                "The pinfold sandbox refused this request: {reason}.\n\
                 Allowed hosts and routes are set on the host, in pinfold's \
                 configuration; nothing inside the box can change them.\n"
            );
            let framing = format!(
                "Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n",
                body.len()
            );
            (framing, body)
        }
    };
    let response = format!(
        "HTTP/1.1 {code} {}\r\n\
         Proxy-Status: pinfold; error={error}; details=\"{reason}\"\r\n\
         {framing}Connection: close\r\n\r\n{body}",
        phrase(code),
    );
    if client.write_all(response.as_bytes()).is_ok() {
        client.finish();
    }
}

/// The box's side of one exchange: its socket, or TLS the proxy ends on it.
enum Client<'a> {
    Plain(&'a mut UnixStream),
    Tls(Box<StreamOwned<ServerConnection, BoxSide<'a>>>),
}

/// The box's socket under TLS, its writes on the idle deadline as a plain
/// client's are, including rustls's writes while reading or flushing.
struct BoxSide<'a>(&'a mut UnixStream);

impl Read for BoxSide<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for BoxSide<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let result = Deadline(self.0.as_fd(), IDLE_TIMEOUT).write(buffer);
        // rustls can defer a write error and write again; a shut socket
        // fails that at once, as DeadlineStream's dropped handle does.
        if result.is_err() {
            let _ = self.0.shutdown(std::net::Shutdown::Both);
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Client<'_> {
    fn socket(&self) -> &UnixStream {
        match self {
            Client::Plain(socket) => socket,
            Client::Tls(stream) => stream.sock.0,
        }
    }

    /// End the box's side for writing: TLS's close_notify, then the
    /// socket's write half.
    fn finish(&mut self) {
        if let Client::Tls(stream) = self {
            stream.conn.send_close_notify();
            let _ = stream.flush();
        }
        let _ = self.socket().shutdown(Shutdown::Write);
    }
}

impl Read for Client<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Client::Plain(socket) => socket.read(buffer),
            Client::Tls(stream) => stream.read(buffer),
        }
    }
}

/// Every write to the box takes the idle deadline.
impl Write for Client<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Client::Plain(socket) => Deadline(socket.as_fd(), IDLE_TIMEOUT).write(buffer),
            Client::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Client::Plain(_) => Ok(()),
            Client::Tls(stream) => stream.flush(),
        }
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
