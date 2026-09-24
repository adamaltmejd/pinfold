//! The box spec: the JSON a caller gives `box up`, parsed into a [`Plan`].

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::{network, proxy};

/// The only harness a box spec may select.
pub const HARNESS_PI: &str = "pi";

/// A parsed box spec.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// The box's name; also its state directory name.
    pub name: String,
    /// The image to run. When absent, the profile's image is used.
    #[serde(default)]
    pub image: Option<String>,
    /// The profile to apply: its image when `image` is absent, its `home/`
    /// seeds and its `share/`.
    #[serde(default)]
    pub profile: Option<String>,
    /// The pinned harness to install in the box. Only `pi` exists.
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    /// The uid:gid for PID 1 and all work. The host uid:gid when absent.
    #[serde(default)]
    pub user: Option<User>,
    /// Exact environment. `{ "from": "NAME" }` takes the caller's value.
    #[serde(default)]
    pub env: BTreeMap<String, Env>,
    /// Present means the box gets a proxy; absent means no way out at all.
    #[serde(default)]
    pub egress: Option<Egress>,
    #[serde(default)]
    pub cpus: Option<f64>,
    #[serde(default)]
    pub memory: Option<String>,
}

/// The box's egress allowlist and routes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Egress {
    /// Exact host names, or `.suffix` for a name and its subdomains.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Route names served over plain HTTP, each mapped to a host service or
    /// an injecting route.
    #[serde(default)]
    pub routes: BTreeMap<String, Route>,
}

/// Where a route name leads.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Route {
    /// `host:port`: a host service.
    Address(String),
    /// `{ "to": ORIGIN, "headers": {…} }`: the proxy dials `to` and adds the
    /// headers from its own environment.
    Inject(Inject),
}

/// An injecting route: the credential stays in the proxy's process.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Inject {
    /// An `http://` or `https://` origin.
    pub to: String,
    #[serde(default)]
    pub headers: BTreeMap<String, Header>,
}

/// One injected header: `prefix` then the value of `$from`. Only the
/// variable's name is ever written down.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub from: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix: String,
}

/// An injecting route's `to`, parsed.
#[derive(Debug, Clone)]
pub struct Target {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// `to`'s authority, for the upstream Host header.
    pub authority: String,
}

impl Inject {
    /// The parsed target and each header with its value from this process's
    /// environment. Errors name the header and the variable, never a value.
    pub fn resolve(&self) -> Result<(Target, Vec<(String, String)>), PlanError> {
        let target = parse_target(&self.to).ok_or_else(|| {
            PlanError::Invalid(format!(
                "route target {:?} must be an http:// or https:// origin",
                self.to
            ))
        })?;
        let mut headers = Vec::with_capacity(self.headers.len());
        for (name, header) in &self.headers {
            if !valid_header_name(name) {
                return Err(PlanError::Invalid(format!(
                    "route header {name:?} cannot be injected"
                )));
            }
            if header.from.is_empty() {
                return Err(PlanError::Invalid(format!(
                    "route header {name:?} needs a from variable"
                )));
            }
            let value = match std::env::var(&header.from) {
                Ok(value) => value,
                Err(std::env::VarError::NotPresent) => {
                    return Err(PlanError::MissingEnv(header.from.clone()));
                }
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err(PlanError::Invalid(format!(
                        "route header {name:?}: {} is not UTF-8",
                        header.from
                    )));
                }
            };
            let value = format!("{}{value}", header.prefix);
            // A line break or NUL would let the value frame its own headers.
            if value.contains(['\r', '\n', '\0']) {
                return Err(PlanError::Invalid(format!(
                    "route header {name:?}: its prefix or {} holds CR, LF or NUL",
                    header.from
                )));
            }
            headers.push((name.clone(), value));
        }
        Ok((target, headers))
    }
}

/// Parse `http(s)://host[:port]`, with at most a trailing `/`. No userinfo,
/// path, query or fragment.
fn parse_target(to: &str) -> Option<Target> {
    let (scheme, rest) = to.split_once("://")?;
    let https = if scheme.eq_ignore_ascii_case("https") {
        true
    } else if scheme.eq_ignore_ascii_case("http") {
        false
    } else {
        return None;
    };
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '?', '#', '@']) {
        return None;
    }
    let (host, port) = network::authority_host(authority, if https { 443 } else { 80 })?;
    // The host is the TLS server name, so it must be one.
    if https && rustls::pki_types::ServerName::try_from(host).is_err() {
        return None;
    }
    Some(Target {
        https,
        host: host.to_string(),
        port,
        authority: authority.to_string(),
    })
}

/// An HTTP token that pinfold's own framing does not own.
fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        && !name.eq_ignore_ascii_case("host")
        && !name.eq_ignore_ascii_case("content-length")
        && !proxy::hop_by_hop(name)
}

/// One host directory and where it appears in the box.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub host: PathBuf,
    pub guest: PathBuf,
    #[serde(default)]
    pub readonly: bool,
}

/// The box user.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
}

/// One environment entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Env {
    /// A literal value from the spec.
    Exact(String),
    /// `{ "from": "NAME" }`: the caller's value, passed by name only.
    From { from: String },
}

impl Plan {
    /// Parse and validate a box spec.
    pub fn parse(json: &str) -> Result<Plan, PlanError> {
        let plan: Plan = serde_json::from_str(json)?;
        plan.validate()?;
        Ok(plan)
    }

    /// Parse one box spec from a reader, leaving any data after the JSON
    /// value unread so `box up` can watch the same stdin for EOF. The caller
    /// validates, so a refusal can name the box when the JSON parsed.
    pub fn from_reader(reader: impl Read) -> Result<Plan, PlanError> {
        serde_json::Deserializer::from_reader(reader)
            .into_iter::<Plan>()
            .next()
            .ok_or_else(|| PlanError::Invalid("no box spec on stdin".into()))?
            .map_err(PlanError::from)
    }

    /// Check a parsed spec.
    pub fn validate(&self) -> Result<(), PlanError> {
        if !valid_name(&self.name) {
            return Err(PlanError::Invalid(format!(
                "name {:?} must start alphanumeric and hold only [A-Za-z0-9._-]",
                self.name
            )));
        }
        if self.image.as_deref() == Some("") {
            return Err(PlanError::Invalid("image must not be empty".into()));
        }
        if self.image.is_none() && self.profile.is_none() {
            return Err(PlanError::Invalid(
                "a box spec needs an image or a profile".into(),
            ));
        }
        if let Some(harness) = &self.harness
            && harness != HARNESS_PI
        {
            return Err(PlanError::Invalid(format!(
                "harness {harness:?} is not supported; the only harness is {HARNESS_PI:?}"
            )));
        }
        for mount in &self.mounts {
            if !mount.host.is_absolute() || !mount.guest.is_absolute() {
                return Err(PlanError::Invalid(format!(
                    "mount {}:{} must be absolute",
                    mount.host.display(),
                    mount.guest.display()
                )));
            }
            for path in [&mount.host, &mount.guest] {
                if !valid_mount_path(path) {
                    return Err(PlanError::Invalid(format!(
                        "mount path {:?} must not hold ',' or an ASCII control character",
                        path
                    )));
                }
            }
        }
        for (name, value) in &self.env {
            if !valid_env_name(name) {
                return Err(PlanError::Invalid(format!(
                    "env name {name:?} must match [A-Za-z_][A-Za-z0-9_]*"
                )));
            }
            if let Env::From { from } = value
                && std::env::var_os(from).is_none()
            {
                return Err(PlanError::MissingEnv(from.clone()));
            }
        }
        if let Some(egress) = &self.egress {
            for route in egress.routes.values() {
                if let Route::Inject(inject) = route {
                    inject.resolve()?;
                }
            }
        }
        Ok(())
    }
}

/// A POSIX environment name: `[A-Za-z_][A-Za-z0-9_]*`. Podman expands a
/// trailing `*` in `--env NAME` against its own environment, which is the
/// caller's, so anything else would leak or fail unpredictably.
fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// A mount path that survives `type=bind,source=HOST,target=GUEST`: the
/// value is concatenated, not CSV-quoted, and the runtime's option parser
/// reads a comma as a separator and a control character as noise.
fn valid_mount_path(path: &Path) -> bool {
    !path
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .any(|byte| *byte == b',' || *byte < 0x20 || *byte == 0x7f)
}

fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.')
}

/// Why a box spec was refused.
#[derive(Debug)]
pub enum PlanError {
    Json(serde_json::Error),
    Invalid(String),
    MissingEnv(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::Json(error) => write!(formatter, "invalid box spec: {error}"),
            PlanError::Invalid(message) => write!(formatter, "invalid box spec: {message}"),
            PlanError::MissingEnv(name) => {
                write!(formatter, "environment variable {name} is not set")
            }
        }
    }
}

impl Error for PlanError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PlanError::Json(error) => Some(error),
            PlanError::Invalid(_) | PlanError::MissingEnv(_) => None,
        }
    }
}

impl From<serde_json::Error> for PlanError {
    fn from(error: serde_json::Error) -> PlanError {
        PlanError::Json(error)
    }
}
