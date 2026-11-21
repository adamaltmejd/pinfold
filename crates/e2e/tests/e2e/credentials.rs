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
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use e2e::{Client, Fixture, Project, READER, Request, TestEnv, default_image, json_lines, run};

const COMMAND: Duration = Duration::from_secs(120);

/// A throwaway random word.
fn random() -> String {
    format!("{:016x}", RandomState::new().build_hasher().finish())
}

fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

/// The labels of the `(value, label)` pairs whose value `text` holds,
/// plain or hex in either case: a failure names the value, never prints
/// it or the text around it.
fn exposed(text: &str, values: &[(&str, &str)]) -> Vec<String> {
    let text = text.to_ascii_lowercase();
    let mut labels = Vec::new();
    for (value, label) in values {
        if text.contains(value) {
            labels.push(label.to_string());
        }
        if text.contains(&hex(value)) {
            labels.push(format!("{label} hex"));
        }
    }
    labels
}

/// The names of a request's credential fields (the login rules' fixed list
/// and every `Proxy-` field), lowercased.
fn credential_fields(request: &Request) -> Vec<String> {
    const FIXED: [&str; 11] = [
        "authorization",
        "x-api-key",
        "api-key",
        "x-goog-api-key",
        "x-auth-token",
        "x-access-token",
        "chatgpt-account-id",
        "openai-organization",
        "openai-project",
        "cookie",
        "cookie2",
    ];
    request
        .headers
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .filter(|name| FIXED.contains(&name.as_str()) || name.starts_with("proxy-"))
        .collect()
}

/// Whether the request carries `Authorization: Bearer <login>` and no
/// other credential field.
fn only_bearer(request: &Request, login: &str) -> bool {
    credential_fields(request) == ["authorization"]
        && request.header("authorization") == [format!("Bearer {login}")]
}

/// Every needle starts with this, plain or hex, so a file is searched for
/// two short strings, not for each needle. Its hex form has no letters.
const PREFIX: &str = "pfsecret";

/// A value to find in host files, plain or hex, and the label a failure
/// prints instead of it.
struct Needle {
    bytes: Vec<u8>,
    label: String,
}

unsafe extern "C" {
    fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    fn memmem(haystack: *const c_void, len: usize, needle: *const c_void, n: usize) -> *mut c_void;
}
#[cfg(target_os = "linux")]
const SEEK_DATA: c_int = 3;
#[cfg(target_os = "linux")]
const SEEK_HOLE: c_int = 4;
#[cfg(target_os = "linux")]
const O_NOFOLLOW: c_int = 0o400000;
#[cfg(target_os = "macos")]
const SEEK_DATA: c_int = 4;
#[cfg(target_os = "macos")]
const SEEK_HOLE: c_int = 3;
#[cfg(target_os = "macos")]
const O_NOFOLLOW: c_int = 0x100;
const ENXIO: i32 = 6;

/// Add `(file, label)` for every needle in every regular file at or under
/// `path`. Symlinks are not followed and a file that goes mid-walk (a live
/// box's socket or lock) is skipped; any other failure fails the test.
fn sweep(path: &Path, needles: &[Needle], found: &mut BTreeSet<(PathBuf, String)>) {
    let kind = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type(),
        Err(error) if error.kind() == ErrorKind::NotFound => return,
        Err(error) => panic!("stat {}: {error}", path.display()),
    };
    if kind.is_file() {
        scan(path, needles, found);
    }
    if !kind.is_dir() {
        return;
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return,
        Err(error) => panic!("read {}: {error}", path.display()),
    };
    for entry in entries {
        match entry {
            Ok(entry) => sweep(&entry.path(), needles, found),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => panic!("read {}: {error}", path.display()),
        }
    }
}

/// The byte ranges of `file` that hold data: its data extents, so a sparse
/// disk image costs what it holds, not its size; or the whole file where
/// the file system cannot say.
fn extents(file: &fs::File, path: &Path) -> Vec<(u64, u64)> {
    let mut extents = Vec::new();
    let mut offset = 0;
    loop {
        // SAFETY: the descriptor is open for the call; lseek reads no memory
        // of ours.
        let data = unsafe { lseek(file.as_raw_fd(), offset, SEEK_DATA) };
        if data < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ENXIO) {
                return extents;
            }
            assert_eq!(offset, 0, "{}: {error}", path.display());
            return vec![(0, file.metadata().unwrap().len())];
        }
        // SAFETY: as above.
        let hole = unsafe { lseek(file.as_raw_fd(), data, SEEK_HOLE) };
        assert!(hole > data, "{}: no hole after {data}", path.display());
        extents.push((data as u64, hole as u64));
        offset = hole;
    }
}

fn scan(path: &Path, needles: &[Needle], found: &mut BTreeSet<(PathBuf, String)>) {
    let file = match fs::File::options()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return,
        Err(error) => panic!("open {}: {error}", path.display()),
    };
    let prefixes = [PREFIX.to_string(), hex(PREFIX)];
    let longest = needles
        .iter()
        .map(|needle| needle.bytes.len())
        .max()
        .unwrap_or(1);
    for (start, end) in extents(&file, path) {
        // 8 MiB at a time, carrying the longest needle's length less one
        // byte into the next window.
        let mut window: Vec<u8> = Vec::new();
        let mut at = start;
        while at < end {
            let kept = window.len();
            let take = (end - at).min(8 << 20) as usize;
            window.resize(kept + take, 0);
            match file.read_exact_at(&mut window[kept..], at) {
                Ok(()) => {}
                // Truncated while read: what was there has been searched.
                Err(error) if error.kind() == ErrorKind::UnexpectedEof => return,
                Err(error) => panic!("read {}: {error}", path.display()),
            }
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
                        if window[position..]
                            .get(..needle.bytes.len())
                            .is_some_and(|bytes| bytes.eq_ignore_ascii_case(&needle.bytes))
                        {
                            found.insert((path.to_path_buf(), needle.label.clone()));
                        }
                    }
                    from = position + 1;
                }
            }
            window.drain(..window.len().saturating_sub(longest - 1));
        }
    }
}

/// A box disk: Linux's `*.ext4` root, init and mount images, Apple's
/// `rootfs.ext4` and a volume's `volume.img`.
fn is_box_disk(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "ext4" || extension == "img")
}

/// Apple's volumes, by name, with their image file.
#[cfg(target_os = "macos")]
fn volumes() -> std::collections::BTreeMap<String, PathBuf> {
    let out = run(
        Command::new("container").args(["volume", "list", "--format", "json"]),
        COMMAND,
    );
    assert_eq!(out.code, 0, "container volume list: {out:?}");
    let volumes: serde_json::Value = serde_json::from_str(&out.stdout).expect("volume list JSON");
    volumes
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|volume| {
            Some((
                volume["id"].as_str()?.to_string(),
                PathBuf::from(volume["configuration"]["source"].as_str()?),
            ))
        })
        .collect()
}

#[test]
fn credentials_stay_on_the_host() {
    // Sabotage (run): in proxy.rs's `route`, set `login_policy` false. The
    // `?api_key=` query is then forwarded: curl prints 200, not 403.
    // Sabotage (run): in proxy.rs's `login_removals`, drop the fixed list's
    // `remove.extend`. The box's own Authorization and X-Api-Key fields then
    // reach the Pi fixture.
    // Sabotage (run): in `login_removals`, skip `remove.push(option)`. The
    // nominated X-E2E-Nominated field then reaches the Pi fixture.
    // Sabotage (run): in proxy.rs's `forward`, drop an injected field that
    // the request's Connection field names. The nominating request then
    // reaches the Pi fixture with no Authorization.
    // Sabotage (run): in `forward`, send no body and no Content-Length. The
    // model fixture then answers the control request 400, not 200.
    // The same login request without the query, the unnominated field
    // arriving, both agents' exchanges and the env values arriving in the
    // box are the positive controls.
    default_image();
    let env = TestEnv::new("credentials");
    let project = Project::new(&env, "credentials");
    let inject = Fixture::start(418);
    let (claude_reply, pi_reply) = (format!("claude-{}", random()), format!("pi-{}", random()));
    // A model fixture answers only a POST whose body holds its prompt.
    let (claude_prompt, pi_prompt) = (
        format!("pfprompt{}", random()),
        format!("pfprompt{}", random()),
    );
    let claude = Fixture::model(&claude_reply, &claude_prompt);
    let pi = Fixture::model(&pi_reply, &pi_prompt);

    // Every value starts with PREFIX, which the host sweep looks for; every
    // route credential with `pfsecretroute`, so the box can be searched for
    // that without being told a credential.
    let injected = format!("pfsecretrouteinject{}", random());
    let claude_login = format!("pfsecretrouteclaude{}", random());
    let pi_login = format!("pfsecretroutepi{}", random());
    let from = format!("pfsecretfrom{}", random());
    let penv = format!("pfsecretpenv{}", random());
    let secrets = [
        (injected.as_str(), "injected"),
        (claude_login.as_str(), "claude login"),
        (pi_login.as_str(), "pi login"),
        (from.as_str(), "from"),
        (penv.as_str(), "PINFOLD_ENV_"),
    ];
    let routes = &secrets[..3];
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
    #[cfg(target_os = "macos")]
    let volumes_before = volumes();
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
    for (name, value) in [("E2E_FROM", &from), ("E2E_PENV", &penv)] {
        assert!(
            environment
                .lines()
                .any(|line| line == format!("{name}={value}")),
            "{name} does not hold its value in the box's env"
        );
    }
    assert_eq!(
        exposed(&environment, routes),
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
    assert_eq!(requests.len(), 1, "requests at the injecting fixture");
    assert!(
        requests[0].header("x-e2e-key") == [format!("Key {injected}")],
        "x-e2e-key is not the host's value alone"
    );

    // Through the Pi login route the guest's mixed-case duplicates of
    // fixed-list fields are dropped, with no Connection field naming them.
    // A credential query is refused before any upstream request. Each
    // request posts the Pi fixture's prompt, so the fixture's 200 also
    // shows the body arrived.
    let completions = "http://openrouter.login/api/v1/chat/completions";
    let forged = format!(
        "-d '{pi_prompt}' \
         -H 'authorization: Bearer pfbox-a' -H 'AUTHORIZATION: Bearer pfbox-b' \
         -H 'x-api-key: pfbox-c' -H 'X-API-KEY: pfbox-d' \
         -H 'X-E2E-Nominated: pfbox-n'"
    );
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
        ask(&curl(&format!("{forged} '{completions}?api_key=pfbox-e'"))),
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
    // Control: the same request without the query. A field off the fixed
    // list that no Connection field names passes unchanged.
    assert_eq!(ask(&curl(&format!("{forged} {completions}"))), ["200"]);
    let requests = pi.requests();
    assert_eq!(requests.len(), 1, "requests at the Pi fixture");
    assert!(
        only_bearer(&requests[0], &pi_login),
        "credential fields at the Pi fixture: {:?}",
        credential_fields(&requests[0])
    );
    assert_eq!(requests[0].header("x-e2e-nominated"), ["pfbox-n"]);

    // A Connection field nominating that field, off the fixed list, in
    // another letter case, removes it. Nominating Authorization too does
    // not suppress the host's.
    assert_eq!(
        ask(&curl(&format!(
            "-d '{pi_prompt}' -H 'Authorization: Bearer pfbox-f' \
             -H 'X-E2E-Nominated: pfbox-n' -H 'Connection: x-e2e-NOMINATED, Authorization' \
             {completions}"
        ))),
        ["200"]
    );
    let requests = pi.requests();
    assert_eq!(requests.len(), 2, "requests at the Pi fixture");
    assert_eq!(
        requests[1].header("x-e2e-nominated"),
        Vec::<&str>::new(),
        "the nominated field"
    );
    assert!(
        only_bearer(&requests[1], &pi_login),
        "credential fields at the Pi fixture: {:?}",
        credential_fields(&requests[1])
    );

    // Control: Claude and Pi each complete one exchange with their fixture
    // through the generated configuration, which sends the host credential
    // only.
    for (agent, fixture, prompt, reply, login, path) in [
        (
            "claude -p",
            &claude,
            &claude_prompt,
            &claude_reply,
            &claude_login,
            "/v1/messages",
        ),
        (
            "pi -p --provider openrouter --model openai/gpt-4o-mini",
            &pi,
            &pi_prompt,
            &pi_reply,
            &pi_login,
            "/api/v1/chat/completions",
        ),
    ] {
        let seen = fixture.requests().len();
        assert_eq!(
            ask(&format!(
                "timeout 100 {agent} 'Reply to {prompt}.'; echo \"exit $?\""
            )),
            [reply.as_str(), "exit 0"],
            "{agent}"
        );
        let requests = fixture.requests()[seen..].to_vec();
        // Paths only: a query is the guest's and could carry anything.
        let paths: Vec<&str> = requests
            .iter()
            .map(|request| request.target.split('?').next().unwrap_or_default())
            .collect();
        assert!(paths.contains(&path), "{agent}: {paths:?}");
        for (request, path) in requests.iter().zip(&paths) {
            assert!(
                only_bearer(request, login),
                "{agent} at {path}: credential fields {:?}",
                credential_fields(request)
            );
        }
    }

    // No file the box can read, its home disk included, holds a route
    // credential: the box prints every `pfsecretroute` word it finds, plain
    // or hex in either case, and the test compares them. The canary on the
    // home disk shows the search reads it. Searched: every top-level entry,
    // hidden ones included, and /dev/shm. Skipped: /proc, /sys and the rest
    // of /dev, which are the kernel's views, not files; top-level symlinks,
    // whose targets are searched; files box work may not read (No
    // privilege: the host uid, no capabilities, no setuid files); and
    // sockets, which hold no bytes. Any other grep error fails the test.
    let found = ask(&format!(
        "printf pfsecretroutecanary > \"$HOME/e2e-canary\"; set -- /dev/shm; \
         for d in /* /.[!.]* /..?*; do case $d in /proc|/sys|/dev) continue ;; esac; \
         [ -e \"$d\" ] && [ ! -L \"$d\" ] && set -- \"$@\" \"$d\"; done; \
         words=$(grep -rhoai -D skip -e 'pfsecretroute[0-9a-z]*' -e '{}[0-9a-f]*' \"$@\" \
         2>/tmp/e2e-grep-errors); status=$?; printf '%s\\n' \"$words\" | sort -u; \
         grep -v -e ': Permission denied$' -e ': No such device or address$' \
         /tmp/e2e-grep-errors | sed 's/^/error /'; echo \"grep $status\"",
        hex("pfsecretroute")
    ));
    let errors: Vec<&String> = found
        .iter()
        .filter(|line| line.starts_with("error "))
        .collect();
    assert!(
        errors.is_empty() && matches!(found.last().map(String::as_str), Some("grep 0" | "grep 2")),
        "the box search: {errors:?} {:?}",
        found.last()
    );
    assert!(
        found.iter().any(|word| word == "pfsecretroutecanary"),
        "the canary is not found"
    );
    assert_eq!(
        exposed(&found.join("\n"), routes),
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
    assert_eq!(ps.code, 0, "ps exit {}", ps.code);
    assert!(ps.stdout.contains(&canary), "ps shows no client argv");
    assert_eq!(
        exposed(&ps.stdout, &secrets),
        Vec::<String>::new(),
        "host argv"
    );

    // Nor in any file under the state, config or cache roots, the egress
    // log included, or on Apple in `inspect`, the container's directory or
    // the box's volumes. A box disk may hold the `from` and `PINFOLD_ENV_`
    // values, which the box has; it is searched for route credentials. The
    // configuration's literal is found, in its file only.
    #[cfg(not(target_os = "macos"))]
    let apple: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    let apple = {
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
        let inspect = run(Command::new("container").args(["inspect", &name]), COMMAND);
        assert_eq!(inspect.code, 0, "container inspect exit {}", inspect.code);
        assert!(
            inspect.stdout.contains(&name),
            "inspect does not name {name}"
        );
        assert_eq!(
            exposed(&inspect.stdout, &secrets),
            Vec::<String>::new(),
            "container inspect"
        );
        let home = std::env::var_os("HOME").expect("HOME");
        let dir = PathBuf::from(home)
            .join("Library/Application Support/com.apple.container/containers")
            .join(&name);
        assert!(dir.is_dir(), "{} is missing", dir.display());
        let mut roots = vec![dir];
        let own: Vec<(String, PathBuf)> = volumes()
            .into_iter()
            .filter(|(volume, _)| {
                !volumes_before.contains_key(volume) && volume.contains(&format!("-pi-{id}-"))
            })
            .collect();
        assert!(
            own.iter().any(|(volume, _)| volume.ends_with("-home")),
            "the box's new volumes: {own:?}"
        );
        roots.extend(own.into_iter().map(|(_, source)| source));
        roots
    };
    let roots = [
        env.state_home().to_path_buf(),
        env.config_home().to_path_buf(),
        env.cache_home().to_path_buf(),
    ]
    .into_iter()
    .chain(apple);
    let mut needles = Vec::new();
    for (value, label) in secrets {
        needles.push(Needle {
            bytes: value.as_bytes().to_vec(),
            label: label.to_string(),
        });
        needles.push(Needle {
            bytes: hex(value).into_bytes(),
            label: format!("{label} hex"),
        });
    }
    needles.push(Needle {
        bytes: literal.clone().into_bytes(),
        label: "literal".to_string(),
    });
    let mut found = BTreeSet::new();
    for root in roots {
        sweep(&root, &needles, &mut found);
    }
    found.retain(|(path, label)| {
        !(is_box_disk(path) && (label.starts_with("from") || label.starts_with("PINFOLD_ENV_")))
    });
    assert_eq!(
        found,
        BTreeSet::from([(file.clone(), "literal".to_string())]),
        "files holding a secret or the literal"
    );

    assert_eq!(session.finish(), 0, "the session after its stdin closed");
}
