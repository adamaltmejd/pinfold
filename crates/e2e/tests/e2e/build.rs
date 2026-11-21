//! Guarantee 44: Linux image work stays in its own VM. Linux only.

use std::fs;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use e2e::{Exec, TestEnv, box_name, json_lines, run};
use serde_json::{Value, json};

const BUILD: Duration = Duration::from_secs(600);

/// A RUN step that records, in /e2e, a direct connection to `address`
/// and a plain HTTP request through the build's proxy to each host. bash's
/// /dev/tcp needs no tool the base lacks.
fn recipe(address: SocketAddr) -> String {
    format!(
        r#"FROM debian:trixie-slim
RUN <<EOF
#!/bin/bash
mkdir /e2e
timeout 20 bash -c 'exec 3<>/dev/tcp/{ip}/{port}' 2> /e2e/direct-error
echo "$?" > /e2e/direct
proxy=${{http_proxy#http://}}
for host in api.github.com example.com; do
  timeout 30 bash -c 'exec 3<>"/dev/tcp/${{1%:*}}/${{1##*:}}" &&
    printf "GET http://%s/ HTTP/1.1\r\nHost: %s\r\nConnection: close\r\n\r\n" "$2" "$2" >&3 &&
    head -n 1 <&3 | tr -d "\r"' sh "$proxy" "$host" > /e2e/$host
done
EOF
"#,
        ip = address.ip(),
        port = address.port(),
    )
}

#[test]
fn linux_image_work_stays_in_its_own_vm() {
    // Sabotage (run): in core/image/linux/job.rs's `run_transaction`, push
    // `example.com` onto the build's `hosts`. The first build's example.com
    // then answers 200, not the proxy's 403.
    // The direct connection from the host, api.github.com through the
    // proxy and the `--allow example.com` build are the positive controls.
    let env = TestEnv::own_cache("build");

    // example.com's address, reachable from the host.
    let address = ("example.com", 80)
        .to_socket_addrs()
        .expect("resolve example.com")
        .find(SocketAddr::is_ipv4)
        .expect("an IPv4 address for example.com");
    TcpStream::connect_timeout(&address, Duration::from_secs(20))
        .expect("the host connects to example.com directly");

    let dir = env.root().join("recipe");
    fs::create_dir(&dir).unwrap();
    let containerfile = dir.join("Containerfile");
    fs::write(&containerfile, recipe(address)).unwrap();
    let name = format!("e2e-{}-build", std::process::id());
    let build = |extra: &[&str]| -> (Vec<Value>, [String; 4]) {
        let built = run(
            env.command()
                .args(["image", "build", &name, "--containerfile"])
                .arg(&containerfile)
                .arg("--context")
                .arg(&dir)
                .args(extra),
            BUILD,
        );
        assert_eq!(built.code, 0, "image build {extra:?}: {built:?}");
        (egress_log(&built), results(&env, &name))
    };

    // The RUN step has no route of its own: bash exits 1, not timeout's
    // 124, with the kernel's ENETUNREACH text. Through the proxy GitHub answers its plain HTTP redirect, and
    // example.com gets the proxy's 403, recorded with its reason.
    let (log, [direct, error, github, example]) = build(&[]);
    assert_eq!(direct, "1", "the direct connection's exit");
    assert!(error.contains("Network is unreachable"), "{error}");
    assert_eq!(github, "HTTP/1.1 301 Moved Permanently");
    assert_eq!(example, "HTTP/1.1 403 Forbidden");
    assert_eq!(
        decisions(&log, "example.com"),
        [("refused".to_string(), "not allowlisted".to_string())],
        "{log:?}"
    );
    assert_eq!(
        decisions(&log, "api.github.com")
            .iter()
            .map(|(decision, _)| decision.as_str())
            .collect::<Vec<_>>(),
        ["allowed"],
        "{log:?}"
    );

    // Control: the same build with `--allow example.com` reaches it.
    let (log, [direct, error, _, example]) = build(&["--allow", "example.com", "--no-cache"]);
    assert_eq!(direct, "1", "the direct connection's exit");
    assert!(error.contains("Network is unreachable"), "{error}");
    assert_eq!(example, "HTTP/1.1 200 OK");
    assert_eq!(
        decisions(&log, "example.com")
            .iter()
            .map(|(decision, _)| decision.as_str())
            .collect::<Vec<_>>(),
        ["allowed"],
        "{log:?}"
    );
}

/// The decisions the log records for `host`, as `(decision, reason)`.
fn decisions(log: &[Value], host: &str) -> Vec<(String, String)> {
    log.iter()
        .filter(|line| line["host"] == host)
        .map(|line| {
            let field = |key: &str| line[key].as_str().unwrap_or_default().to_string();
            (field("decision"), field("reason"))
        })
        .collect()
}

/// The egress log the build names on stderr (Linux image preparation).
fn egress_log(built: &Exec) -> Vec<Value> {
    let named: Vec<PathBuf> = built
        .stderr
        .split_whitespace()
        .skip_while(|word| *word != "log")
        .nth(1)
        .map(PathBuf::from)
        .into_iter()
        .collect();
    assert_eq!(named.len(), 1, "no egress log on stderr: {}", built.stderr);
    json_lines(&fs::read_to_string(&named[0]).unwrap())
}

/// The RUN step's three records, read in a box of the built image.
fn results(env: &TestEnv, name: &str) -> [String; 4] {
    let boxed = box_name("build");
    let up = env.up(&json!({
        "name": boxed,
        "image": format!("localhost/pinfold/image-{name}:latest"),
    }));
    let read = |file: &str| {
        let out = env.exec(&boxed, &["cat", &format!("/e2e/{file}")]);
        assert_eq!(out.code, 0, "cat {file}: {out:?}");
        out.stdout.trim().to_string()
    };
    let read = [
        read("direct"),
        read("direct-error"),
        read("api.github.com"),
        read("example.com"),
    ];
    drop(up);
    read
}
