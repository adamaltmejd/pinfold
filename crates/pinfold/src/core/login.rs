//! A codex login route's token: asked of the pinned host helper, checked,
//! and held in the proxy's memory only. Also the 1Password reads behind
//! `op://` `from` values, and the files a login route's agents read in the box.
//! Renderers never receive resolved credentials.

use crate::core::process::GroupChild;

use std::fs::{File, TryLockError};
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, PoisonError};

use nix::poll::PollFlags;
use std::time::{Duration, Instant};

use crate::core::pipe::{self, nonblocking, remaining, wait_ready};
use crate::core::plan::{LOGIN_PLACEHOLDER, PiCredential, PiProvider};
use crate::core::{artifacts, now, ownership};

/// Within this much of `exp`, the proxy asks the helper again. Codex's own
/// `AuthManager` refreshes inside the same window, so the helper then
/// answers with a new token.
const REFRESH_WINDOW: u64 = 5 * 60;

/// The least time between two asks for one box.
const ASK_INTERVAL: Duration = Duration::from_secs(60);

const HELPER_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LINE: usize = 64 * 1024;
const MAX_OUTPUT: usize = 256 * 1024;

/// The guest directory that holds codex's system config layer.
pub const CODEX_CONFIG_DIR: &str = "/etc/codex";

/// A checked ChatGPT access token. No `Debug`: it is a credential.
pub struct Token {
    value: String,
    /// The JWT's `chatgpt_account_id` claim.
    account: String,
    /// The JWT's `exp`, in Unix seconds.
    exp: u64,
}

/// A codex login route's live token. Each request takes its headers from
/// here, so a refreshed token replaces the old one for the box's life.
pub struct Codex {
    state: Mutex<(Token, Instant)>,
}

impl Codex {
    /// Hold `token`, asked of the helper just now.
    pub fn new(token: Token) -> Codex {
        Codex {
            state: Mutex::new((token, Instant::now())),
        }
    }

    /// The `Authorization` and `ChatGPT-Account-ID` headers for one request,
    /// or `None` when the token has lapsed. Within [`REFRESH_WINDOW`] of
    /// `exp` the helper is asked again, at most once per [`ASK_INTERVAL`];
    /// a failed ask keeps the old token until it lapses.
    pub fn headers(&self, cancel: &AtomicBool) -> Option<Vec<(String, String)>> {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return None;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return None;
        }
        let (token, asked) = &mut *state;
        if token.exp <= now() + REFRESH_WINDOW && asked.elapsed() >= ASK_INTERVAL {
            *asked = Instant::now();
            if let Ok(fresh) = token_from_helper(Some(cancel)) {
                *token = fresh;
            }
        }
        (token.exp > now() && !cancel.load(std::sync::atomic::Ordering::Acquire)).then(|| {
            vec![
                (
                    "Authorization".to_string(),
                    format!("Bearer {}", token.value),
                ),
                ("ChatGPT-Account-ID".to_string(), token.account.clone()),
            ]
        })
    }
}

/// Ask the pinned host helper for the host's Codex access token, installing
/// it first if the cache lacks it, and check the token. One host-wide lock
/// serializes every ask, so two boxes never spend one refresh token at once.
/// Errors never hold the token.
pub fn token_from_helper(cancel: Option<&AtomicBool>) -> Result<Token, String> {
    let helper = artifacts::codex_helper(cancel)
        .map_err(|error| format!("install the codex host helper: {error}"))?;
    let deadline = Instant::now() + HELPER_TIMEOUT;
    let _lock = lock(deadline, cancel).map_err(|error| format!("lock the codex login: {error}"))?;
    let status = ask(&helper, deadline, cancel)
        .map_err(|error| format!("the codex host helper: {error}"))?;
    match (status["authToken"].as_str(), status["authMethod"].as_str()) {
        (Some(token), _) => check(token.to_string()),
        (None, None) => Err("the host has no Codex login".to_string()),
        // A ChatGPT login whose refresh failed for good, or an API key.
        (None, Some(_)) => Err("the host's Codex login gave no token".to_string()),
    }
}

/// The ownership domain is stable across HOME and XDG overrides. Never
/// unlink this lock: waiters must keep referring to the same inode.
fn lock(deadline: Instant, cancel: Option<&AtomicBool>) -> io::Result<File> {
    let file = ownership::open_lock("codex-login")?;
    let mut reported = false;
    loop {
        remaining(deadline, cancel, "helper")?;
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                if !reported {
                    eprintln!("pinfold: login-busy: waiting for the host's Codex login");
                    reported = true;
                }
                wait_ready(None, PollFlags::empty(), deadline, cancel, "helper")?;
            }
            Err(TryLockError::Error(_)) => {
                return Err(io::Error::other("login lock failed"));
            }
        }
    }
}

/// Run the pinned helper with `up`'s environment and the built-in provider.
/// Its process group always dies and the helper is reaped before the login
/// lock is released. Revisit when the pin removes getAuthStatus, which is
/// deprecated upstream.
fn ask(
    helper: &std::path::Path,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<serde_json::Value> {
    remaining(deadline, cancel, "helper")?;
    let mut child = GroupChild::spawn(
        Command::new(helper)
            .args(["-c", "model_provider=\"openai\""])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|_| io::Error::other("helper launch failed"))?;
    let mut stdin = child.take_stdin().expect("piped stdin");
    let mut stdout = child.take_stdout().expect("piped stdout");
    nonblocking(&stdin, "helper")?;
    nonblocking(&stdout, "helper")?;
    let requests = [
        serde_json::json!({
            "method": "initialize",
            "id": 0,
            "params": {
                "clientInfo": {
                    "name": "pinfold",
                    "title": "pinfold",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            },
        }),
        serde_json::json!({ "method": "initialized" }),
        serde_json::json!({
            "method": "getAuthStatus",
            "id": 1,
            "params": { "includeToken": true },
        }),
    ];
    let mut input = Vec::new();
    for request in requests {
        writeln!(input, "{request}")?;
    }
    pipe::write_all(&mut stdin, &input, deadline, cancel, "helper")?;
    // stdin remains open until the answer arrives; some helper versions
    // exit when the caller closes it before the auth lookup finishes.
    let mut line = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0u8; 8192];
    loop {
        let n = pipe::read(&mut stdout, &mut buffer, deadline, cancel, "helper")?;
        if n == 0 {
            return Err(io::Error::other("helper exited without an answer"));
        }
        total += n;
        if total > MAX_OUTPUT {
            return Err(io::Error::other("helper output exceeds 256 KiB"));
        }
        for byte in &buffer[..n] {
            if *byte != b'\n' {
                if line.len() >= MAX_LINE {
                    return Err(io::Error::other("helper line exceeds 64 KiB"));
                }
                line.push(*byte);
                continue;
            }
            let message = serde_json::from_slice::<serde_json::Value>(&line);
            line.clear();
            let Ok(mut message) = message else {
                continue;
            };
            if message["id"] != 1 {
                continue;
            }
            if message.get("error").is_some() {
                return Err(io::Error::other("getAuthStatus failed"));
            }
            return Ok(message["result"].take());
        }
    }
}

const OP_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MAX_OP_VALUE: usize = 64 * 1024;

/// Read one 1Password secret with the first `op` on PATH, as `op read
/// --no-newline REFERENCE` with this process's environment, stdin null and
/// stderr discarded, in a process group of its own. Past the deadline, or
/// on this thread's cancellation, the group is killed and `op` reaped. An
/// error is the cause alone: never the value or any of op's output.
pub fn op_read(reference: &str) -> Result<String, String> {
    // A cancelled launch starts no further `op`.
    if crate::core::process::is_cancelled() {
        return Err("op read was cancelled".to_string());
    }
    let deadline = Instant::now() + OP_TIMEOUT;
    let mut child = GroupChild::spawn(
        Command::new("op")
            .args(["read", "--no-newline", reference])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => "op is not on PATH".to_string(),
        _ => "op did not start".to_string(),
    })?;
    let failed = |error: io::Error| match error.kind() {
        io::ErrorKind::TimedOut => "op read timed out after 30 s".to_string(),
        io::ErrorKind::Interrupted => "op read was cancelled".to_string(),
        _ => "op read failed".to_string(),
    };
    let mut stdout = child.take_stdout().expect("piped stdout");
    nonblocking(&stdout, "op").map_err(failed)?;
    let mut value = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = pipe::read(&mut stdout, &mut buffer, deadline, None, "op").map_err(failed)?;
        if n == 0 {
            break;
        }
        if value.len() + n > MAX_OP_VALUE {
            return Err("op read gave more than 64 KiB".to_string());
        }
        value.extend_from_slice(&buffer[..n]);
    }
    while !child.exited().map_err(failed)? {
        wait_ready(None, PollFlags::empty(), deadline, None, "op").map_err(failed)?;
    }
    let status = child.finish().map_err(failed)?;
    if !status.success() {
        use std::os::unix::process::ExitStatusExt;
        return Err(match (status.code(), status.signal()) {
            (Some(code), _) => format!("op read exited with status {code}"),
            (None, Some(signal)) => format!("op read was killed by signal {signal}"),
            (None, None) => "op read failed".to_string(),
        });
    }
    if value.is_empty() {
        return Err("op read gave an empty value".to_string());
    }
    if value.contains(&0) {
        return Err("op read gave a value holding NUL".to_string());
    }
    String::from_utf8(value).map_err(|_| "op read gave a value that is not UTF-8".to_string())
}

/// Check that `value` is a JWT with a future `exp` and an account id. The
/// signature is the backend's to check; pinfold reads the claims only.
fn check(value: String) -> Result<Token, String> {
    // Base64url and dots only, so the value cannot frame a header.
    let jwt = value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte));
    let claims = match value.split('.').collect::<Vec<_>>().as_slice() {
        [_, payload, _] if jwt => base64url(payload)
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok()),
        _ => None,
    }
    .ok_or("the Codex login's token is not a JWT")?;
    let exp = claims["exp"]
        .as_f64()
        .ok_or("the Codex login's token has no exp")? as u64;
    if exp <= now() {
        return Err("the Codex login's token has expired".to_string());
    }
    let account = claims["https://api.openai.com/auth"]["chatgpt_account_id"]
        .as_str()
        .filter(|account| !account.is_empty() && account.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or("the Codex login's token has no account id")?
        .to_string();
    Ok(Token {
        value,
        account,
        exp,
    })
}

/// Decode unpadded base64url; `None` on any other byte.
fn base64url(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
        }
    }
    Some(bytes)
}

/// The box's `/etc/codex/config.toml`: a custom provider at ChatGPT's own
/// host, which needs no login of its own. It holds no secret; the proxy
/// adds the headers.
pub const CODEX_CONFIG: &str = "model_provider = \"pinfold\"\n\
     \n\
     [model_providers.pinfold]\n\
     name = \"pinfold\"\n\
     base_url = \"https://chatgpt.com/backend-api/codex\"\n\
     wire_api = \"responses\"\n\
     requires_openai_auth = false\n\
     supports_websockets = false\n";

/// Claude's system settings carry only placeholders.
pub const CLAUDE_CONFIG_DIR: &str = "/etc/claude-code";

pub fn claude_config() -> String {
    serde_json::json!({"env": {
        "CLAUDE_CODE_OAUTH_TOKEN": LOGIN_PLACEHOLDER,
        "ANTHROPIC_API_KEY": "",
    }})
    .to_string()
}

/// The box environment a github login route sets: a placeholder token for
/// `gh`, SSH remotes rewritten to HTTPS, and a push buffer large enough
/// that Git does not chunk a body, which the proxy refuses.
pub const GITHUB_ENV: [(&str, &str); 6] = [
    ("GH_TOKEN", LOGIN_PLACEHOLDER),
    ("GIT_CONFIG_COUNT", "2"),
    ("GIT_CONFIG_KEY_0", "url.https://github.com/.insteadOf"),
    ("GIT_CONFIG_VALUE_0", "git@github.com:"),
    ("GIT_CONFIG_KEY_1", "http.https://github.com/.postBuffer"),
    ("GIT_CONFIG_VALUE_1", "524288000"),
];

/// The single autoload entry under the writable home; sibling extensions remain.
pub const PI_LOGIN_DIR: &str = ".pi/agent/extensions/pinfold-login";

/// Give each routed provider its nonsecret placeholder. Its `baseUrl` is
/// the provider's own, set so pi keeps its built-in models.
pub fn pi_extension(providers: &[&PiProvider]) -> String {
    let mut calls = String::new();
    for provider in providers {
        // A JSON value is a JavaScript literal.
        let name = serde_json::Value::from(provider.pi);
        let placeholder = match provider.credential {
            PiCredential::Key { placeholder, .. } | PiCredential::HostCodex { placeholder } => {
                placeholder
            }
        };
        let config = serde_json::json!({
            "baseUrl": format!("{}{}", provider.origin, provider.base_path),
            "apiKey": placeholder,
        });
        calls.push_str(&format!("  pi.registerProvider({name}, {config});\n"));
    }
    format!("export default function (pi) {{\n{calls}}}\n")
}
