//! Guarantees 1 and 2: the box's only way out is its proxy, and the proxy
//! lets out only allowlisted names and routes.

use e2e::{Exec, Fixture, TestEnv, box_name, default_image};
use serde_json::{Value, json};

/// `curl -sS --max-time 20 ARGS…` in the box. The bound turns a hang into
/// a failure that names it; no expected outcome is a timeout.
fn curl(env: &TestEnv, name: &str, args: &[&str]) -> Exec {
    let mut argv = vec!["curl", "-sS", "--max-time", "20"];
    argv.extend_from_slice(args);
    env.exec(name, &argv)
}

/// From inside the box, connect a vsock stream to the host (CID 2) on
/// `port` and print `connected` or the kernel's error. perl-base opens the
/// socket by number: AF_VSOCK is 40, and a sockaddr_vm is family, reserved,
/// port and CID. A dropped connection would hit the alarm and print
/// `timeout`, which no assertion accepts.
#[cfg(target_os = "linux")]
fn vsock_to_host(env: &TestEnv, name: &str, port: u32) -> String {
    let out = env.exec(
        name,
        &[
            "perl",
            "-e",
            "$SIG{ALRM} = sub { print \"timeout\\n\"; exit }; alarm 5; \
             socket(my $s, 40, 1, 0) or die \"socket: $!\\n\"; \
             print connect($s, pack('SSLLx4', 40, 0, $ARGV[0], 2)) ? \"connected\\n\" : \"$!\\n\"",
            &port.to_string(),
        ],
    );
    assert_eq!(out.code, 0, "vsock probe: {out:?}");
    out.stdout.trim().to_string()
}

#[test]
fn box_has_no_network_but_loopback() {
    // Sabotage (run): on Apple, drop `--network none` from `container run`
    // in runtime/apple.rs. The box gains an interface beside `lo` and
    // reaches 1.1.1.1. Sabotage (Linux, not yet run): listen on the host
    // proxy port without `egress`; the no-egress box's vsock connect to
    // 3128 is accepted. The route through the proxy is the positive control.
    let env = TestEnv::new("network");
    let fixture = Fixture::start(418);

    // Without `egress` the box has no proxy: its port refuses.
    let name = box_name("no-egress");
    let up = env.up(&json!({ "name": name, "image": default_image() }));
    let proxied = curl(
        &env,
        &name,
        &[
            "-v",
            "-x",
            "http://127.0.0.1:3128",
            "http://fixture.internal/",
        ],
    );
    assert_eq!(
        proxied.code, 7,
        "the no-egress box reached a proxy: {proxied:?}"
    );
    assert!(
        proxied.stderr.contains("Connection refused"),
        "the proxy port failed for another reason: {proxied:?}"
    );
    // The host refuses at once on every vsock port, the proxy's included.
    #[cfg(target_os = "linux")]
    for port in [1024, 4000, 3128] {
        assert_eq!(
            vsock_to_host(&env, &name, port),
            "Connection reset by peer",
            "vsock CID 2 port {port} without egress"
        );
    }
    drop(up);

    let name = box_name("route");
    let _up = env.up(&json!({
        "name": name,
        "image": default_image(),
        "egress": { "routes": { "fixture.internal": fixture.route() } },
    }));
    let dev = env.exec(&name, &["cat", "/proc/net/dev"]);
    assert_eq!(dev.code, 0, "{dev:?}");
    let interfaces: Vec<&str> = dev
        .stdout
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, _)| name.trim())
        .collect();
    assert_eq!(interfaces, ["lo"], "the box's interfaces: {}", dev.stdout);
    let public = curl(&env, &name, &["-v", "--noproxy", "*", "http://1.1.1.1/"]);
    assert_eq!(public.code, 7, "1.1.1.1 answered: {public:?}");
    assert!(
        public.stderr.contains("Network is unreachable"),
        "1.1.1.1 failed for another reason: {public:?}"
    );

    // Control: the same image reaches the host fixture through the proxy.
    let route = curl(
        &env,
        &name,
        &[
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "http://fixture.internal/",
        ],
    );
    assert_eq!((route.code, route.stdout.as_str()), (0, "418"), "{route:?}");
    assert_eq!(fixture.requests().len(), 1, "the fixture's requests");
    #[cfg(target_os = "linux")]
    assert_eq!(vsock_to_host(&env, &name, 3128), "connected");
}

/// The host's own non-loopback address: the source of its default route.
/// A UDP connect picks it and sends nothing.
fn host_address() -> String {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    socket
        .connect("192.0.2.1:9")
        .expect("the host has a default route");
    let address = socket.local_addr().unwrap().ip();
    assert!(!address.is_loopback() && !address.is_unspecified());
    address.to_string()
}

/// The egress log's lines for `host` with `decision`.
fn decisions<'a>(log: &'a [Value], host: &str, decision: &str) -> Vec<&'a Value> {
    log.iter()
        .filter(|line| line["host"] == host && line["decision"] == decision)
        .collect()
}

#[test]
fn only_allowlisted_hosts_get_through() {
    // Sabotage (run): skip the refused-address loop in network::resolve.
    // localhost then passes, so its CONNECT gets 200, not 403. Deleting
    // only the loopback checks is not enough: `::1` is still refused as the
    // IPv4-compatible `0.0.0.1`. Sabotage: skip the SNI comparison in
    // tls.rs; the mismatched hello reaches api.github.com, whose
    // certificate curl then rejects (60, not 35).
    // api.github.com and the route are the positive controls.
    let env = TestEnv::new("allowlist");
    let fixture = Fixture::start(418);
    let host = host_address();
    let name = box_name("allowlist");
    let up = env.up(&json!({
        "name": name,
        "image": default_image(),
        "egress": {
            "allow": ["api.github.com", "localhost", host],
            "routes": { "fixture.internal": fixture.route() },
        },
    }));
    let connect = |url: &str| {
        curl(
            &env,
            &name,
            &["-o", "/dev/null", "-w", "%{http_connect}", url],
        )
    };

    // Control: the tunnel opens and TLS completes. GitHub's own status is
    // not asserted: it may rate-limit a shared CI address.
    let github = connect("https://api.github.com/");
    assert_eq!(
        (github.code, github.stdout.as_str()),
        (0, "200"),
        "{github:?}"
    );

    for target in ["localhost", host.as_str(), "example.com"] {
        let refused = connect(&format!("https://{target}/"));
        assert_ne!(refused.code, 0, "{target} got through: {refused:?}");
        assert_eq!(refused.stdout, "403", "{target}'s CONNECT: {refused:?}");
    }

    // The proxy accepts CONNECT to api.github.com, then reads a ClientHello
    // naming example.com. curl's handshake fails (35) when the proxy drops it.
    let sni = curl(
        &env,
        &name,
        &[
            "-o",
            "/dev/null",
            "--connect-to",
            "example.com:443:api.github.com:443",
            "https://example.com/",
        ],
    );
    assert_eq!(sni.code, 35, "the mismatched SNI: {sni:?}");

    // Control: a route reaches the fixture with the route's name as Host,
    // whatever Host the box sent.
    let sentinel = format!("pf-sentinel-{}", std::process::id());
    let route = curl(
        &env,
        &name,
        &[
            "-X",
            "DELETE",
            "-H",
            "Host: evil.example",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            &format!("http://fixture.internal/a/b?{sentinel}"),
        ],
    );
    assert_eq!((route.code, route.stdout.as_str()), (0, "418"), "{route:?}");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1, "the fixture's requests: {requests:?}");
    assert_eq!(requests[0].method, "DELETE");
    assert_eq!(requests[0].header("host"), ["fixture.internal"]);

    let log = up.egress_log();
    let reason = |host: &str| -> String {
        let lines = decisions(&log, host, "refused");
        assert_eq!(lines.len(), 1, "{host}'s refusals in {log:?}");
        lines[0]["reason"].as_str().unwrap().to_string()
    };
    assert_eq!(reason("example.com"), "not allowlisted");
    // Allowlisted, so each refusal names its own reason.
    let (loopback, own) = (reason("localhost"), reason(&host));
    assert!(
        loopback != own && ![&loopback, &own].contains(&&"not allowlisted".to_string()),
        "localhost refused as {loopback:?}, {host} as {own:?}"
    );
    // The SNI refusal is logged under the CONNECT host.
    reason("api.github.com");
    assert_eq!(decisions(&log, "api.github.com", "allowed").len(), 1);

    let routed = decisions(&log, "fixture.internal", "allowed");
    assert_eq!(routed.len(), 1, "the route's lines in {log:?}");
    assert_eq!(
        (
            &routed[0]["method"],
            &routed[0]["path"],
            &routed[0]["status"]
        ),
        (&json!("DELETE"), &json!("/a/b"), &json!(418)),
        "the route's line: {}",
        routed[0]
    );
    let text = serde_json::to_string(&log).unwrap();
    assert!(!text.contains(&sentinel), "the log holds the query");
}
