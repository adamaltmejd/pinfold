//! Guarantee 26: credentials stay on the host. A route's credential is
//! added at the proxy and never enters the box; a `from` or `PINFOLD_ENV_`
//! value reaches the box's environment and no host argv, runtime record or
//! file pinfold writes.

use std::collections::BTreeSet;
use std::collections::hash_map::RandomState;
use std::ffi::{c_int, c_void};
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use e2e::{Client, Fixture, Project, READER, TestEnv, default_image, json_lines, run};

const COMMAND: Duration = Duration::from_secs(120);

/// A throwaway random word.
fn random() -> String {
    format!("{:016x}", RandomState::new().build_hasher().finish())
}

fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

/// Which of `values`, plain or hex, `text` holds.
fn holds(text: &str, values: &[&str]) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| [value.to_string(), hex(value)])
        .filter(|form| text.contains(form.as_str()))
        .collect()
}

/// Every needle starts with this, plain or hex, so a file is searched for
/// two short strings, not for each needle.
const PREFIX: &str = "pfsecret";

unsafe extern "C" {
    fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    fn memmem(haystack: *const c_void, len: usize, needle: *const c_void, n: usize) -> *mut c_void;
}
#[cfg(target_os = "linux")]
const SEEK_DATA: c_int = 3;
#[cfg(target_os = "linux")]
const SEEK_HOLE: c_int = 4;
#[cfg(target_os = "macos")]
const SEEK_DATA: c_int = 4;
#[cfg(target_os = "macos")]
const SEEK_HOLE: c_int = 3;

/// Add `(file, needle)` for every needle in every regular file under
/// `dir`, symlinks not followed. Only data extents are read, so a sparse
/// disk image costs what it holds, not its size.
fn sweep(dir: &Path, needles: &[String], found: &mut BTreeSet<(PathBuf, String)>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return,
        Err(error) => panic!("read {}: {error}", dir.display()),
    };
    let prefixes = [PREFIX.to_string(), hex(PREFIX)];
    let longest = needles.iter().map(String::len).max().unwrap_or(0);
    for entry in entries {
        let path = entry.unwrap().path();
        // A live box's socket or lock may go between listing and reading.
        let kind = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata.file_type(),
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => panic!("stat {}: {error}", path.display()),
        };
        if kind.is_dir() {
            sweep(&path, needles, found);
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => panic!("open {}: {error}", path.display()),
        };
        let mut offset = 0;
        loop {
            // SAFETY: the descriptor is open for the call; lseek reads no
            // memory of ours.
            let data = unsafe { lseek(file.as_raw_fd(), offset, SEEK_DATA) };
            if data < 0 {
                // ENXIO: no data at or after `offset`.
                let error = std::io::Error::last_os_error();
                assert_eq!(error.raw_os_error(), Some(6), "{}: {error}", path.display());
                break;
            }
            // SAFETY: as above.
            let hole = unsafe { lseek(file.as_raw_fd(), data, SEEK_HOLE) };
            assert!(hole > data, "{}: no hole after {data}", path.display());
            // 8 MiB at a time, carrying the longest needle's length less one
            // byte into the next window.
            let mut window: Vec<u8> = Vec::new();
            let mut at = data as u64;
            while at < hole as u64 {
                let start = window.len();
                let take = (hole as u64 - at).min(8 << 20) as usize;
                window.resize(start + take, 0);
                file.read_exact_at(&mut window[start..], at).unwrap();
                at += take as u64;
                for prefix in &prefixes {
                    let mut from = 0;
                    while from < window.len() {
                        let rest = &window[from..];
                        // SAFETY: both slices are live for the call and their
                        // lengths are their own; memmem only reads them.
                        let hit = unsafe {
                            memmem(
                                rest.as_ptr().cast(),
                                rest.len(),
                                prefix.as_ptr().cast(),
                                prefix.len(),
                            )
                        };
                        if hit.is_null() {
                            break;
                        }
                        let position = from + (hit as usize - rest.as_ptr() as usize);
                        for needle in needles {
                            if window[position..].starts_with(needle.as_bytes()) {
                                found.insert((path.clone(), needle.clone()));
                            }
                        }
                        from = position + 1;
                    }
                }
                window.drain(..window.len().saturating_sub(longest - 1));
            }
            offset = hole;
        }
    }
}

#[test]
fn credentials_stay_on_the_host() {
    // Sabotage (run): in proxy.rs, forward a login route's request without
    // its credential filter. The box's own Authorization and X-Api-Key then
    // reach the Pi fixture beside the host's, and the `?api_key=` query is
    // forwarded.
    // The same login request without the query, both agents' exchanges and
    // the env values arriving in the box are the positive controls.
    default_image();
    let env = TestEnv::new("credentials");
    let project = Project::new(&env, "credentials");
    let inject = Fixture::start(418);
    let (claude_reply, pi_reply) = (format!("claude-{}", random()), format!("pi-{}", random()));
    let claude = Fixture::model(&claude_reply);
    let pi = Fixture::model(&pi_reply);

    // Every value starts with PREFIX, which the host sweep looks for; every
    // route credential with `pfsecretroute`, so the box can be searched for
    // that without being told a credential.
    let injected = format!("pfsecretrouteinject{}", random());
    let claude_login = format!("pfsecretrouteclaude{}", random());
    let pi_login = format!("pfsecretroutepi{}", random());
    let routes = [injected.as_str(), claude_login.as_str(), pi_login.as_str()];
    let from = format!("pfsecretfrom{}", random());
    let penv = format!("pfsecretpenv{}", random());
    let secrets = [&routes[..], &[from.as_str(), penv.as_str()]].concat();
    // A literal is configuration: it is the file sweep's canary.
    let literal = format!("pfsecretliteral{}", random());
    let file = project.config_dir().join("pinfold.toml");
    let saved = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        format!(
            "{saved}allow = []\n\
             [env]\n\
             E2E_FROM = {{ from = \"E2E_FROM_SOURCE\" }}\n\
             E2E_LITERAL = \"{literal}\"\n\
             [routes]\n\
             \"inject.internal\" = {{ to = \"http://{}\", headers = {{ \"X-E2E-Key\" = {{ from = \"E2E_INJECTED\", prefix = \"Key \" }} }} }}\n\
             \"claude.login\" = {{ login = \"claude\", from = \"E2E_CLAUDE_LOGIN\", to = \"http://{}\" }}\n\
             \"openrouter.login\" = {{ login = \"pi\", provider = \"openrouter\", from = \"E2E_PI_LOGIN\", to = \"http://{}\" }}\n",
            inject.route(),
            claude.route(),
            pi.route(),
        ),
    )
    .unwrap();

    // One session holds the box. Its argv carries the process sweep's
    // canary.
    let canary = format!("pfargv{}", random());
    let mut command = project.command(&["--", "sh", "-c", READER, &canary]);
    command
        .env("E2E_FROM_SOURCE", &from)
        .env("E2E_INJECTED", &injected)
        .env("E2E_CLAUDE_LOGIN", &claude_login)
        .env("E2E_PI_LOGIN", &pi_login)
        .env("PINFOLD_ENV_E2E_PENV", &penv);
    let mut session = Client::spawn(command);
    assert_eq!(session.line(), "started");
    let mut ask = |line: &str| session.ask(line);

    // Control: the `from` and `PINFOLD_ENV_` values arrive in the box's
    // environment, which holds no route credential.
    let environment = ask("env").join("\n");
    for line in [format!("E2E_FROM={from}"), format!("E2E_PENV={penv}")] {
        assert!(
            environment.lines().any(|l| l == line),
            "{line} is not in the box's env"
        );
    }
    assert_eq!(
        holds(&environment, &routes),
        Vec::<String>::new(),
        "the box's env"
    );

    // The injecting route replaces the guest's own value, sent in another
    // letter case.
    let curl =
        |args: &str| format!("curl -sS --max-time 20 -o /dev/null -w '%{{http_code}}\\n' {args}");
    assert_eq!(
        ask(&curl(
            "-H 'x-e2e-key: pfbox-inject' http://inject.internal/"
        )),
        ["418"]
    );
    let requests = inject.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].header("x-e2e-key"), [format!("Key {injected}")]);

    // Through the Pi login route the guest's mixed-case duplicates, and a
    // Connection field nominating them, are dropped. A credential query is
    // refused before any upstream request.
    let forged = "-H 'authorization: Bearer pfbox-a' -H 'AUTHORIZATION: Bearer pfbox-b' \
                  -H 'x-api-key: pfbox-c' -H 'X-API-KEY: pfbox-d' \
                  -H 'Connection: Authorization, X-Api-Key'";
    let log = env.state_home().join("pinfold/runs");
    let decisions = || -> Vec<serde_json::Value> {
        let mut lines = Vec::new();
        for run in fs::read_dir(&log).unwrap() {
            let path = run.unwrap().path().join("egress.jsonl");
            if let Ok(text) = fs::read_to_string(path) {
                lines.extend(json_lines(&text));
            }
        }
        lines
    };
    let before = decisions().len();
    assert_eq!(
        ask(&curl(&format!(
            "{forged} 'http://openrouter.login/api/v1/models?api_key=pfbox-e'"
        ))),
        ["403"]
    );
    let lines = decisions()[before..].to_vec();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(
        (
            &lines[0]["host"],
            &lines[0]["decision"],
            &lines[0]["reason"]
        ),
        (
            &serde_json::json!("openrouter.login"),
            &serde_json::json!("refused"),
            &serde_json::json!("login credential query")
        ),
        "{}",
        lines[0]
    );
    assert_eq!(
        pi.requests().len(),
        0,
        "the refused query reached the fixture"
    );
    // Control: the same request without the query.
    assert_eq!(
        ask(&curl(&format!(
            "{forged} http://openrouter.login/api/v1/models"
        ))),
        ["200"]
    );
    let requests = pi.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        (
            requests[0].header("authorization"),
            requests[0].header("x-api-key")
        ),
        (vec![format!("Bearer {pi_login}").as_str()], vec![]),
        "the Pi fixture's credential fields"
    );

    // Control: Claude and Pi each complete one exchange with their fixture
    // through the generated configuration, which sends the host credential
    // only.
    for (agent, fixture, reply, login, path) in [
        (
            "claude -p 'Reply.'",
            &claude,
            &claude_reply,
            &claude_login,
            "/v1/messages",
        ),
        (
            "pi -p --provider openrouter --model openai/gpt-4o-mini 'Reply.'",
            &pi,
            &pi_reply,
            &pi_login,
            "/api/v1/chat/completions",
        ),
    ] {
        let seen = fixture.requests().len();
        assert_eq!(
            ask(&format!("timeout 100 {agent}")),
            [reply.as_str()],
            "{agent}"
        );
        let requests = fixture.requests()[seen..].to_vec();
        assert!(
            requests
                .iter()
                .any(|request| request.target.split('?').next() == Some(path)),
            "{agent}: {requests:?}"
        );
        for request in &requests {
            assert_eq!(
                request.header("authorization"),
                [format!("Bearer {login}")],
                "{agent} at {}",
                request.target
            );
        }
    }

    // No file the box can read, its home disk included, holds a route
    // credential: the box prints every `pfsecretroute` word it finds, plain
    // or hex, and the test compares them. The canary on the home disk shows
    // the search reads it.
    let found = ask(&format!(
        "printf pfsecretroutecanary > \"$HOME/e2e-canary\"; \
         for d in /*; do case $d in /proc|/sys|/dev) continue ;; esac; \
         [ -L \"$d\" ] || set -- \"$@\" \"$d\"; done; \
         grep -rhoa -D skip -s -e 'pfsecretroute[0-9a-z]*' -e '{}[0-9a-f]*' \"$@\" | sort -u",
        hex("pfsecretroute")
    ));
    assert!(
        found.iter().any(|word| word == "pfsecretroutecanary"),
        "{found:?}"
    );
    assert_eq!(
        holds(&found.join("\n"), &routes),
        Vec::<String>::new(),
        "box files"
    );

    // On the host, no secret is in any process's argv while the session
    // runs; the session's own canary is.
    let ps = run(
        Command::new("ps").args(if cfg!(target_os = "macos") {
            ["-axww", "-o", "args"]
        } else {
            ["-eww", "-o", "args"]
        }),
        COMMAND,
    );
    assert_eq!(ps.code, 0, "{ps:?}");
    assert!(ps.stdout.contains(&canary), "ps shows no client argv");
    assert_eq!(
        holds(&ps.stdout, &secrets),
        Vec::<String>::new(),
        "host argv"
    );

    // Nor in any file under the state, config or cache roots, the egress
    // log included, or on Apple in the container's directory or `inspect`.
    // The configuration's literal is found, in its file only.
    let id = project
        .config_dir()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let label = format!("dev.pinfold.project={id}");
    let listed = run(
        env.command().args(["box", "list", "--label", &label]),
        COMMAND,
    );
    let listed = json_lines(&listed.stdout);
    assert_eq!(listed.len(), 1, "{listed:?}");
    let name = listed[0]["name"].as_str().expect("a box name").to_string();
    let mut roots: Vec<PathBuf> = vec![
        env.state_home().into(),
        env.config_home().into(),
        env.cache_home().into(),
    ];
    if cfg!(target_os = "macos") {
        let inspect = run(Command::new("container").args(["inspect", &name]), COMMAND);
        assert_eq!(inspect.code, 0, "container inspect: {inspect:?}");
        assert!(inspect.stdout.contains(&name), "{inspect:?}");
        assert_eq!(
            holds(&inspect.stdout, &secrets),
            Vec::<String>::new(),
            "container inspect"
        );
        let home = std::env::var_os("HOME").expect("HOME");
        let dir = PathBuf::from(home)
            .join("Library/Application Support/com.apple.container/containers")
            .join(&name);
        assert!(dir.is_dir(), "{} is missing", dir.display());
        roots.push(dir);
    }
    let mut needles: Vec<String> = secrets
        .iter()
        .flat_map(|value| [value.to_string(), hex(value)])
        .collect();
    needles.push(literal.clone());
    let mut found = BTreeSet::new();
    for root in &roots {
        sweep(root, &needles, &mut found);
    }
    assert_eq!(
        found,
        BTreeSet::from([(file.clone(), literal.clone())]),
        "files holding a secret or the literal"
    );

    assert_eq!(session.finish(), 0, "the session after its stdin closed");
}
