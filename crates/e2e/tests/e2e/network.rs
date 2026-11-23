//! Guarantees 1 and 2: the box's only way out is its proxy, and the proxy
//! lets out only allowlisted names and routes.

use e2e::{Exec, Fixture, TestEnv, box_name, default_image};
use serde_json::{Value, json};

// Linux errno values, from the guest kernel on both runtimes.
const ENETUNREACH: &str = "101";
#[cfg(target_os = "linux")]
const ECONNRESET: &str = "104";
const ECONNREFUSED: &str = "111";

/// `curl -sS --max-time 20 ARGS…` in the box. The bound turns a hang into
/// a failure that names it; no expected outcome is a timeout.
fn curl(env: &TestEnv, name: &str, args: &[&str]) -> Exec {
    let mut argv = vec!["curl", "-sS", "--max-time", "20"];
    argv.extend_from_slice(args);
    env.exec(name, &argv)
}

/// Run a perl probe in the box and return its stdout. perl-base has no
/// socket modules, so addresses are packed by hand. A hang hits the alarm
/// and prints `timeout`, which no assertion accepts.
fn perl(env: &TestEnv, name: &str, script: &str, args: &[&str]) -> String {
    let script = format!("$SIG{{ALRM}} = sub {{ print \"timeout\\n\"; exit }}; alarm 30; {script}");
    let mut argv = vec!["perl", "-e", &script];
    argv.extend_from_slice(args);
    let out = env.exec(name, &argv);
    assert_eq!(out.code, 0, "perl probe: {out:?}");
    out.stdout.trim().to_string()
}

/// From inside the box, a TCP connect to IPv4 `address:port`: `0` or the
/// kernel's errno. A sockaddr_in is family, port (network order),
/// address and padding.
fn tcp_connect(env: &TestEnv, name: &str, address: &str, port: u16) -> String {
    perl(
        env,
        name,
        "socket(my $s, 2, 1, 6) or die \"socket: $!\\n\"; \
         my $to = pack('SnC4x8', 2, $ARGV[1], split(/\\./, $ARGV[0])); \
         print connect($s, $to) ? 0 : $! + 0, \"\\n\"",
        &[address, &port.to_string()],
    )
}

/// From inside the box, a vsock connect to the host (CID 2) on every port
/// in 1..=4096. Prints `PORT ERRNO` for each port whose connect did not
/// fail with ECONNRESET (`0` for a connect that succeeded), then `done`.
/// AF_VSOCK is 40; a sockaddr_vm is family, reserved, port and CID.
#[cfg(target_os = "linux")]
fn vsock_sweep(env: &TestEnv, name: &str) -> String {
    perl(
        env,
        name,
        &format!(
            "for my $port (1 .. 4096) {{ \
               socket(my $s, 40, 1, 0) or die \"socket: $!\\n\"; \
               my $errno = connect($s, pack('SSLLx4', 40, 0, $port, 2)) ? 0 : $! + 0; \
               print \"$port $errno\\n\" if $errno != {ECONNRESET}; \
             }} print \"done\\n\""
        ),
        &[],
    )
}

/// The box has only `lo`, and a direct connection to a documentation
/// address, so a failing check contacts nothing real, has no route.
fn assert_loopback_only(env: &TestEnv, name: &str) {
    let dev = env.exec(name, &["cat", "/proc/net/dev"]);
    assert_eq!(dev.code, 0, "{dev:?}");
    let interfaces: Vec<&str> = dev
        .stdout
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, _)| name.trim())
        .collect();
    assert_eq!(interfaces, ["lo"], "{name}'s interfaces: {}", dev.stdout);
    assert_eq!(tcp_connect(env, name, "192.0.2.1", 80), ENETUNREACH);
}

#[test]
fn box_has_no_network_but_loopback() {
    // Sabotage (run): on Apple, drop `--network none` from `container run`
    // in runtime/apple.rs. The box gains an interface beside `lo`.
    // Sabotage (Linux, run): start the proxy's vsock listener even without
    // `egress`; the no-egress sweep then shows port 3128 connected.
    // The route through the proxy is the positive control.
    let env = TestEnv::new("network");
    let fixture = Fixture::start(418);

    // Without `egress` the box has no proxy: its port refuses, and on
    // Linux the host resets a vsock connect on every port, 3128 included.
    let name = box_name("no-egress");
    let up = env.up(&json!({ "name": name, "image": default_image() }));
    assert_eq!(tcp_connect(&env, &name, "127.0.0.1", 3128), ECONNREFUSED);
    #[cfg(target_os = "linux")]
    assert_eq!(vsock_sweep(&env, &name), "done");
    drop(up);

    let name = box_name("route");
    let _up = env.up(&json!({
        "name": name,
        "image": default_image(),
        "egress": { "routes": { "fixture.internal": fixture.route() } },
    }));
    assert_loopback_only(&env, &name);
    // Only the proxy's port reaches the host.
    #[cfg(target_os = "linux")]
    assert_eq!(vsock_sweep(&env, &name), "3128 0\ndone");

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

/// Split curl's `-D - -w CODE` stdout into the `details` string of the
/// `pinfold` member of `Proxy-Status` (RFC 9209; an RFC 8941 string) and
/// the code.
fn proxy_status(stdout: &str) -> (Option<String>, &str) {
    let (head, code) = stdout.rsplit_once('\n').unwrap_or(("", stdout));
    let details = head.lines().find_map(|line| {
        let (field, value) = line.split_once(':')?;
        if !field.eq_ignore_ascii_case("proxy-status") {
            return None;
        }
        // The member's parameters: `; key=token` or `; key="string"`.
        let mut rest = value.trim().strip_prefix("pinfold")?;
        loop {
            rest = rest.trim_start().strip_prefix(';')?.trim_start();
            let (key, after) = rest.split_once('=')?;
            let (parameter, after) = match after.strip_prefix('"') {
                Some(string) => sf_string(string)?,
                None => {
                    let (token, after) = after.split_at(after.find(';').unwrap_or(after.len()));
                    (token.trim_end().to_string(), after)
                }
            };
            if key == "details" {
                return Some(parameter);
            }
            rest = after;
        }
    });
    (details, code)
}

/// An RFC 8941 string after its opening quote, unescaped, and the text
/// after its closing quote.
fn sf_string(text: &str) -> Option<(String, &str)> {
    let mut string = String::new();
    let mut chars = text.char_indices();
    while let Some((index, char)) = chars.next() {
        match char {
            '"' => return Some((string, &text[index + 1..])),
            '\\' => string.push(chars.next()?.1),
            char => string.push(char),
        }
    }
    None
}

#[test]
fn only_allowlisted_hosts_get_through() {
    // Sabotage (run): delete the CONNECT IP-literal check in proxy.rs. The
    // host address is then resolved and refused as `private`, not
    // `ip literal`. Sabotage (run): delete the loopback checks in
    // network.rs; localhost's `::1` is then refused as `unspecified`.
    // Sabotage (run): skip the SNI comparison in tls.rs; curl then rejects
    // api.github.com's certificate (60, not 35). Sabotage (run): drop the
    // plain HTTP allowlist check; example.com over port 80 answers 200.
    // Sabotage (run): skip the proxy's `100 Continue` in `forward`; the
    // 100-continue upload then waits past curl's bound (28); or forward a
    // body other than the box's, and the fixture answers 400. Sabotage
    // (run): drop the 417 in `plain`; the other expectation then reaches
    // the fixture (418). Sabotage (not run): give `refuse`'s
    // `Proxy-Status` a fixed `details`, rename the parameter, drop the
    // field, or send it only with a 403; a refusal's response (the 417
    // included) then no longer names its logged reason.
    // api.github.com and the route are the positive controls.
    let env = TestEnv::new("allowlist");
    let fixture = Fixture::start(418);
    // Answers 200 only to a POST whose body holds this, else 400.
    let upload = format!("pf-upload-{}", std::process::id());
    let model = Fixture::model("", &upload);
    let host = host_address();
    let name = box_name("allowlist");
    let up = env.up(&json!({
        "name": name,
        "image": default_image(),
        "egress": {
            "allow": ["api.github.com", "localhost", host],
            "routes": {
                "fixture.internal": fixture.route(),
                "model.internal": model.route(),
            },
        },
    }));
    let connect = |args: &[&str]| {
        let mut argv = vec!["-o", "/dev/null", "-w", "%{http_connect}"];
        argv.extend_from_slice(args);
        curl(&env, &name, &argv)
    };

    // Each request's log lines are those it added, so every entry is tied
    // to its request. A refusal names the spec's token (Egress proxy, Log).
    let logged = |request: &dyn Fn() -> Exec| -> (Exec, Vec<(String, String, String)>) {
        let before = up.egress_log().len();
        let out = request();
        let lines = up.egress_log()[before..]
            .iter()
            .map(|line| {
                let field = |key: &str| line[key].as_str().unwrap_or_default().to_string();
                (field("host"), field("decision"), field("reason"))
            })
            .collect();
        (out, lines)
    };
    let line = |host: &str, decision: &str, reason: &str| {
        vec![(host.to_string(), decision.to_string(), reason.to_string())]
    };
    let plain = |url: &str| curl(&env, &name, &["-o", "/dev/null", "-w", "%{http_code}", url]);

    // Controls: the tunnel opens and TLS completes, and plain HTTP is
    // forwarded. GitHub's own status is not asserted: it may rate-limit a
    // shared CI address.
    let (github, lines) = logged(&|| connect(&["https://api.github.com/"]));
    assert_eq!(
        (github.code, github.stdout.as_str()),
        (0, "200"),
        "{github:?}"
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1, "allowed", "{lines:?}");
    let (github, lines) = logged(&|| plain("http://api.github.com/"));
    assert_eq!(github.code, 0, "{github:?}");
    assert_ne!(
        github.stdout, "403",
        "the proxy refused plain HTTP: {lines:?}"
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1, "allowed", "{lines:?}");

    // curl exits 56 when the proxy answers CONNECT with an error status.
    // Each refusal's response names the logged reason: curl dumps the
    // CONNECT response's head too.
    for (target, reason) in [
        ("localhost", "loopback"),
        (host.as_str(), "ip literal"),
        ("example.com", "not allowlisted"),
    ] {
        let (refused, lines) = logged(&|| connect(&["-D", "-", &format!("https://{target}/")]));
        let (details, code) = proxy_status(&refused.stdout);
        assert_eq!(
            (refused.code, code),
            (56, "403"),
            "{target}'s CONNECT: {refused:?}"
        );
        assert_eq!(lines, line(target, "refused", reason));
        assert_eq!(details, Some(lines[0].2.clone()), "{refused:?}");
    }
    let (refused, lines) = logged(&|| {
        curl(
            &env,
            &name,
            &[
                "-o",
                "/dev/null",
                "-D",
                "-",
                "-w",
                "%{http_code}",
                "http://example.com/",
            ],
        )
    });
    let (details, code) = proxy_status(&refused.stdout);
    assert_eq!((refused.code, code), (0, "403"), "{refused:?}");
    assert_eq!(lines, line("example.com", "refused", "not allowlisted"));
    assert_eq!(details, Some(lines[0].2.clone()), "{refused:?}");

    // The proxy accepts CONNECT to api.github.com, then reads a ClientHello
    // naming example.com and drops it: curl's handshake fails (35). The
    // refusal is logged under the CONNECT host.
    let (sni, lines) = logged(&|| {
        connect(&[
            "--connect-to",
            "example.com:443:api.github.com:443",
            "https://example.com/",
        ])
    });
    assert_eq!(
        (sni.code, sni.stdout.as_str()),
        (35, "200"),
        "the mismatched SNI: {sni:?}"
    );
    assert_eq!(lines, line("api.github.com", "refused", "sni mismatch"));

    // Control: a route reaches the fixture with the route's name as Host,
    // whatever Host the box sent, and with the query the log leaves out.
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
    assert_eq!(requests[0].target, format!("/a/b?{sentinel}"));
    assert_eq!(requests[0].header("host"), ["fixture.internal"]);

    let log = up.egress_log();
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

    // A client that waits for `100 Continue` before its body gets it from
    // the proxy, inside curl's 20-second bound; any other expectation is
    // refused before the fixture is dialed.
    let expect = |value: &str, host: &str| {
        curl(
            &env,
            &name,
            &[
                "--expect100-timeout",
                "600",
                "-H",
                &format!("Expect: {value}"),
                "--data-binary",
                &upload,
                "-o",
                "/dev/null",
                "-D",
                "-",
                "-w",
                "%{http_code}",
                &format!("http://{host}/upload"),
            ],
        )
    };
    let continued = expect("100-continue", "model.internal");
    assert_eq!(
        (continued.code, proxy_status(&continued.stdout).1),
        (0, "200"),
        "{continued:?}"
    );
    let requests = model.requests();
    assert_eq!(
        requests.len(),
        1,
        "the model fixture's requests: {requests:?}"
    );
    assert!(requests[0].header("expect").is_empty(), "{requests:?}");
    let (refused, lines) = logged(&|| expect("x-pinfold-test", "fixture.internal"));
    let (details, code) = proxy_status(&refused.stdout);
    assert_eq!((refused.code, code), (0, "417"), "{refused:?}");
    assert_eq!(
        lines,
        line("fixture.internal", "refused", "expectation unsupported")
    );
    assert_eq!(details, Some(lines[0].2.clone()), "{refused:?}");
    assert_eq!(fixture.requests().len(), 1, "the refused request was sent");
}
