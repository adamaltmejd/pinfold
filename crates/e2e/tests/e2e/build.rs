//! Guarantee 44: Linux image work stays in its own VM. Linux only.

use std::fs;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use e2e::{TestEnv, box_name, json_lines, run};
use serde_json::{Value, json};

const BUILD: Duration = Duration::from_secs(600);

/// A RUN step that records, in /e2e, a direct connection to each of
/// `v4` and `v6`, and a plain HTTP request through the build's proxy to
/// each host. bash's /dev/tcp needs no tool the base lacks.
fn recipe(v4: SocketAddr, v6: SocketAddr) -> String {
    format!(
        r#"FROM debian:trixie-slim
RUN <<EOF
#!/bin/bash
mkdir /e2e
timeout 20 bash -c 'exec 3<>/dev/tcp/{v4ip}/{v4port}' 2> /e2e/direct-error
echo "$?" > /e2e/direct
timeout 20 bash -c 'exec 3<>/dev/tcp/{v6ip}/{v6port}' 2> /e2e/direct6-error
echo "$?" > /e2e/direct6
proxy=${{http_proxy#http://}}
for host in api.github.com example.com; do
  timeout 30 bash -c 'exec 3<>"/dev/tcp/${{1%:*}}/${{1##*:}}" &&
    printf "GET http://%s/ HTTP/1.1\r\nHost: %s\r\nConnection: close\r\n\r\n" "$2" "$2" >&3 &&
    head -n 1 <&3 | tr -d "\r"' sh "$proxy" "$host" > /e2e/$host
done
EOF
"#,
        v4ip = v4.ip(),
        v4port = v4.port(),
        v6ip = v6.ip(),
        v6port = v6.port(),
    )
}

#[test]
fn linux_image_work_stays_in_its_own_vm() {
    // Sabotage (run): in core/image/linux/job.rs's `run_transaction`, push
    // `example.com` onto the build's `hosts`. The first build's example.com
    // then answers 200, not the proxy's 403.
    // The direct IPv4 connection from the host, api.github.com through
    // the proxy and the `--allow example.com` build are the positive
    // controls. The IPv6 attempt has a control only where the host itself
    // has an IPv6 route; the dev VM has none, so the host's own IPv6
    // attempt is not asserted.
    let env = TestEnv::own_cache("build");

    // example.com's addresses as the host resolves them; IPv4 reachable.
    let addresses: Vec<SocketAddr> = ("example.com", 80)
        .to_socket_addrs()
        .expect("resolve example.com")
        .collect();
    let v4 = *addresses
        .iter()
        .find(|address| address.is_ipv4())
        .expect("an IPv4 address for example.com");
    let v6 = *addresses
        .iter()
        .find(|address| address.is_ipv6())
        .expect("an IPv6 address for example.com");
    TcpStream::connect_timeout(&v4, Duration::from_secs(20))
        .expect("the host connects to example.com directly");

    let dir = env.root().join("recipe");
    fs::create_dir(&dir).unwrap();
    let containerfile = dir.join("Containerfile");
    fs::write(&containerfile, recipe(v4, v6)).unwrap();
    let name = format!("e2e-{}-build", std::process::id());
    // Both builds run every step: only `--allow` differs between them.
    let runs = env.state_home().join("pinfold/runs");
    let build = |extra: &[&str]| -> (Vec<Value>, Results) {
        let before = run_dirs(&runs);
        let built = run(
            env.command()
                .args(["image", "build", &name, "--no-cache", "--containerfile"])
                .arg(&containerfile)
                .arg("--context")
                .arg(&dir)
                .args(extra),
            BUILD,
        );
        assert_eq!(built.code, 0, "image build {extra:?}: {built:?}");
        // Build records use normal run records (Linux image preparation):
        // the build's is the one new run.
        let new: Vec<PathBuf> = run_dirs(&runs)
            .into_iter()
            .filter(|dir| !before.contains(dir))
            .collect();
        assert_eq!(new.len(), 1, "new runs: {new:?}");
        let log = json_lines(&fs::read_to_string(new[0].join("egress.jsonl")).unwrap());
        (log, results(&env, &name))
    };

    // The RUN step has no route of its own, IPv4 or IPv6: bash exits 1,
    // not timeout's 124, with the kernel's ENETUNREACH text. Through the
    // proxy GitHub answers its plain HTTP redirect, and example.com gets
    // the proxy's 403, recorded with its reason.
    let (log, first) = build(&[]);
    first.no_route();
    assert_eq!(first.github, "301");
    assert_eq!(first.example, "403");
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
    let (log, control) = build(&["--allow", "example.com"]);
    control.no_route();
    assert_eq!(control.example, "200");
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

/// The run directories under `runs`.
fn run_dirs(runs: &Path) -> Vec<PathBuf> {
    match fs::read_dir(runs) {
        Ok(entries) => entries.map(|entry| entry.unwrap().path()).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("{}: {error}", runs.display()),
    }
}

/// What the RUN step recorded: each direct attempt's exit and stderr, and
/// each proxied request's status code.
struct Results {
    direct: [(String, String); 2],
    github: String,
    example: String,
}

impl Results {
    fn no_route(&self) {
        for (family, (code, error)) in ["IPv4", "IPv6"].iter().zip(&self.direct) {
            assert_eq!(code, "1", "the direct {family} connection's exit");
            assert!(
                error.contains("Network is unreachable"),
                "{family}: {error}"
            );
        }
    }
}

/// The RUN step's records, read in a box of the built image.
fn results(env: &TestEnv, name: &str) -> Results {
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
    // `HTTP/1.1 CODE REASON`: the code alone.
    let status = |file: &str| {
        let line = read(file);
        line.split(' ').nth(1).unwrap_or_default().to_string()
    };
    let results = Results {
        direct: [
            (read("direct"), read("direct-error")),
            (read("direct6"), read("direct6-error")),
        ],
        github: status("api.github.com"),
        example: status("example.com"),
    };
    drop(up);
    results
}
