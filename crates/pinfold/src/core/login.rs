//! A codex login route's token: asked of the pinned host helper, checked,
//! and held in the proxy's memory only.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nix::fcntl::{Flock, FlockArg};

use crate::core::artifacts;
use crate::dirs;

/// Within this much of `exp`, the proxy asks the helper again. Codex's own
/// `AuthManager` refreshes inside the same window, so the helper then
/// answers with a new token.
const REFRESH_WINDOW: u64 = 5 * 60;

/// The least time between two asks for one box.
const ASK_INTERVAL: Duration = Duration::from_secs(60);

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
            if let Ok(fresh) = token_from_helper() {
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
pub fn token_from_helper() -> Result<Token, String> {
    let codex = artifacts::harness("codex").expect("codex is pinned");
    let helper = codex
        .install_host()
        .map_err(|error| format!("install the codex host helper: {error}"))?
        .join("codex-app-server");
    let _lock = lock().map_err(|error| format!("lock the codex login: {error}"))?;
    let token = ask(&helper).map_err(|error| format!("the codex host helper: {error}"))?;
    let token = token.ok_or("the host has no usable Codex login")?;
    check(token)
}

/// Take the host-wide lock file under pinfold's state dir, waiting for a
/// holder to finish.
fn lock() -> io::Result<Flock<File>> {
    let dir = dirs::state_dir()?;
    fs::create_dir_all(&dir)?;
    let file = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("codex-login.lock"))?;
    Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, errno)| io::Error::from(errno))
}

/// Run the helper with `up`'s environment and the built-in `openai` provider
/// forced (with a custom provider it returns no token), send `initialize`,
/// `initialized` and `getAuthStatus { includeToken: true }` on stdin, and
/// return the token it answers with, if any. The helper is stopped after.
fn ask(helper: &std::path::Path) -> io::Result<Option<String>> {
    let mut child = Command::new(helper)
        .args(["-c", "model_provider=\"openai\""])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let answer = exchange(&mut child);
    let _ = child.kill();
    let _ = child.wait();
    answer
}

/// The stdio half of [`ask`]. stdin stays open until the answer arrives, so
/// the helper does not exit first.
fn exchange(child: &mut std::process::Child) -> io::Result<Option<String>> {
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
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
    for request in requests {
        writeln!(stdin, "{request}")?;
    }
    stdin.flush()?;
    for line in BufReader::new(stdout).lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line?) else {
            continue;
        };
        if message["id"] != 1 {
            continue;
        }
        if let Some(error) = message.get("error") {
            let text = error["message"].as_str().unwrap_or("no message");
            return Err(io::Error::other(format!("getAuthStatus failed: {text}")));
        }
        return Ok(message["result"]["authToken"].as_str().map(str::to_owned));
    }
    Err(io::Error::other(
        "it exited without answering getAuthStatus",
    ))
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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
