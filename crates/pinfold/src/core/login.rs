//! A codex login route's token: asked of the pinned host helper, checked,
//! and held in the proxy's memory only.

use std::fs::{File, TryLockError};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, poll};
use std::time::{Duration, Instant};

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
const CANCEL_INTERVAL: Duration = Duration::from_millis(100);

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
    pub fn headers(&self) -> Option<Vec<(String, String)>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (token, asked) = &mut *state;
        if token.exp <= now() + REFRESH_WINDOW && asked.elapsed() >= ASK_INTERVAL {
            *asked = Instant::now();
            if let Ok(fresh) = token_from_helper(None) {
                *token = fresh;
            }
        }
        (token.exp > now()).then(|| {
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
    let codex = artifacts::harness("codex").expect("codex is pinned");
    let helper = codex
        .install_host(cancel)
        .map_err(|error| format!("install the codex host helper: {error}"))?
        .join("codex-app-server");
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
    loop {
        remaining(deadline, cancel)?;
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                wait_ready(None, PollFlags::empty(), deadline, cancel)?;
            }
            Err(TryLockError::Error(_)) => {
                return Err(io::Error::other("login lock failed"));
            }
        }
    }
}

/// A helper always dies and is reaped before the login lock is released.
struct Helper(std::process::Child);

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Run the pinned helper with `up`'s environment and the built-in provider.
/// Revisit when the pin removes getAuthStatus, which is deprecated upstream.
fn ask(
    helper: &std::path::Path,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<serde_json::Value> {
    remaining(deadline, cancel)?;
    let mut child = Helper(
        Command::new(helper)
            .args(["-c", "model_provider=\"openai\""])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| io::Error::other("helper launch failed"))?,
    );
    exchange(&mut child.0, deadline, cancel)
}

/// Nonblocking pipes keep the absolute deadline and cancellation effective
/// while writing requests, reading output or waiting on an unfinished line.
fn exchange(
    child: &mut std::process::Child,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<serde_json::Value> {
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    nonblocking(&stdin)?;
    nonblocking(&stdout)?;
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
    let mut pending = input.as_slice();
    while !pending.is_empty() {
        remaining(deadline, cancel)?;
        match stdin.write(pending) {
            Ok(0) => return Err(io::Error::other("helper input closed")),
            Ok(n) => pending = &pending[n..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_ready(Some(stdin.as_fd()), PollFlags::POLLOUT, deadline, cancel)?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(io::Error::other("helper input failed")),
        }
    }
    // stdin remains open until the answer arrives; some helper versions
    // exit when the caller closes it before the auth lookup finishes.
    let mut line = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0u8; 8192];
    loop {
        remaining(deadline, cancel)?;
        let n = match stdout.read(&mut buffer) {
            Ok(0) => return Err(io::Error::other("helper exited without an answer")),
            Ok(n) => n,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_ready(Some(stdout.as_fd()), PollFlags::POLLIN, deadline, cancel)?;
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(io::Error::other("helper output failed")),
        };
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

fn nonblocking(pipe: &impl AsFd) -> io::Result<()> {
    let flags = fcntl(pipe, FcntlArg::F_GETFL)
        .map_err(|_| io::Error::other("helper pipe configuration failed"))?;
    fcntl(
        pipe,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map_err(|_| io::Error::other("helper pipe configuration failed"))?;
    Ok(())
}

fn remaining(deadline: Instant, cancel: Option<&AtomicBool>) -> io::Result<Duration> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "helper cancelled",
        ));
    }
    deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "helper deadline exceeded"))
}

/// With no fd, the same bounded poll waits for a held login lock.
fn wait_ready(
    fd: Option<std::os::fd::BorrowedFd<'_>>,
    events: PollFlags,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<()> {
    loop {
        let timeout = remaining(deadline, cancel)?.min(CANCEL_INTERVAL);
        let mut fds: Vec<_> = fd.into_iter().map(|fd| PollFd::new(fd, events)).collect();
        let millis = timeout.as_millis().max(1) as u16;
        match poll(&mut fds, millis) {
            Ok(n) if n > 0 || fd.is_none() => return Ok(()),
            Ok(_) | Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return Err(io::Error::other("helper pipe wait failed")),
        }
    }
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

/// The box's `/etc/codex/config.toml`: a custom provider at the login route.
/// It holds no secret; the proxy adds the headers.
pub fn codex_config(route: &str) -> String {
    // A JSON string is a TOML basic string, escapes included.
    let base_url = serde_json::Value::from(format!("http://{route}/backend-api/codex"));
    format!(
        "model_provider = \"pinfold\"\n\
         \n\
         [model_providers.pinfold]\n\
         name = \"pinfold\"\n\
         base_url = {base_url}\n\
         wire_api = \"responses\"\n\
         requires_openai_auth = false\n"
    )
}
