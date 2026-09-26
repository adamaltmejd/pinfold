//! The box spec: the JSON a caller gives `box up`, parsed into a [`Plan`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::r#box::{Refusal, RefusalReason};
use crate::core::{artifacts, network, proxy};

/// A parsed box spec.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub name: String,
    /// The image to run. When absent, the profile's image is used.
    #[serde(default)]
    pub image: Option<String>,
    /// The profile to apply: its image when `image` is absent, its `home/`
    /// seeds and its `share/`.
    #[serde(default)]
    pub profile: Option<String>,
    /// The pinned harness to install in the box, by its name in
    /// `harnesses.toml`.
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

/// An absolute URL's origin, parsed: an injecting route's `to`, or a plain
/// HTTP request's target.
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
    pub fn resolve(&self) -> Result<(Target, Vec<(String, String)>), String> {
        let target = parse_target(&self.to).ok_or_else(|| {
            invalid(format!(
                "route target {:?} must be an http:// or https:// origin",
                self.to
            ))
        })?;
        let mut headers = Vec::with_capacity(self.headers.len());
        for (name, header) in &self.headers {
            if !valid_header_name(name) {
                return Err(invalid(format!("route header {name:?} cannot be injected")));
            }
            if header.from.is_empty() {
                return Err(invalid(format!(
                    "route header {name:?} needs a from variable"
                )));
            }
            let value = match std::env::var(&header.from) {
                Ok(value) => value,
                Err(std::env::VarError::NotPresent) => {
                    return Err(format!("environment variable {} is not set", header.from));
                }
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err(invalid(format!(
                        "route header {name:?}: {} is not UTF-8",
                        header.from
                    )));
                }
            };
            let value = format!("{}{value}", header.prefix);
            // A line break or NUL would let the value frame its own headers.
            if value.contains(['\r', '\n', '\0']) {
                return Err(invalid(format!(
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
    let (target, path) = network::absolute_url(to)?;
    if !path.is_empty() && path != "/" {
        return None;
    }
    // The host is the TLS server name, so it must be one.
    if target.https && rustls::pki_types::ServerName::try_from(target.host.as_str()).is_err() {
        return None;
    }
    Some(target)
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
    /// Parse `box up`'s spec from a reader, leaving any data after the JSON
    /// value unread so `box up` can watch the same stdin for EOF. Parsing to
    /// a value first keeps a readable `name` even when serde refuses, so a
    /// refusal names the box. A caller's labels may not use pinfold's
    /// namespace; `pinfold pi`'s own plan carries `dev.pinfold.project`.
    pub fn from_reader(reader: impl Read) -> Result<Plan, Refusal> {
        let refusal = |box_name, detail| Refusal {
            box_name,
            reason: RefusalReason::Spec,
            detail,
        };
        let value = serde_json::Deserializer::from_reader(reader)
            .into_iter::<serde_json::Value>()
            .next()
            .unwrap_or_else(|| Err(serde::de::Error::custom("no box spec on stdin")))
            .map_err(|error| refusal(None, invalid(error)))?;
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let plan = serde_json::from_value::<Plan>(value)
            .map_err(|error| refusal(name.clone(), invalid(error)))?;
        check_reserved_labels(&plan.labels).map_err(|detail| refusal(name, detail))?;
        Ok(plan)
    }

    /// Check a parsed spec.
    pub fn validate(&self) -> Result<(), String> {
        if !valid_name(&self.name) {
            return Err(invalid(format!(
                "name {:?} must start alphanumeric and hold only [A-Za-z0-9._-]",
                self.name
            )));
        }
        if self.image.as_deref() == Some("") {
            return Err(invalid("image must not be empty"));
        }
        if self.image.is_none() && self.profile.is_none() {
            return Err(invalid("a box spec needs an image or a profile"));
        }
        if let Some(harness) = &self.harness
            && artifacts::harness(harness).is_none()
        {
            let names: Vec<&str> = artifacts::HARNESSES
                .iter()
                .map(|harness| harness.name.as_str())
                .collect();
            return Err(invalid(format!(
                "harness {harness:?} is not supported; the harnesses are {names:?}"
            )));
        }
        if let Some(memory) = &self.memory
            && !valid_memory(memory)
        {
            return Err(invalid(format!(
                "memory {memory:?} must be a whole number of M or G, at least 256M"
            )));
        }
        for mount in &self.mounts {
            if !mount.host.is_absolute() || !mount.guest.is_absolute() {
                return Err(invalid(format!(
                    "mount {}:{} must be absolute",
                    mount.host.display(),
                    mount.guest.display()
                )));
            }
            for path in [&mount.host, &mount.guest] {
                if !valid_mount_path(path) {
                    return Err(invalid(format!(
                        "mount path {:?} must not hold ',' or an ASCII control character",
                        path
                    )));
                }
            }
            // A missing host is left to the runtime; `metadata` follows
            // symlinks, so a symlink to a directory passes.
            if std::fs::metadata(&mount.host).is_ok_and(|metadata| !metadata.is_dir()) {
                return Err(invalid(format!(
                    "mount host {} is not a directory",
                    mount.host.display()
                )));
            }
        }
        self.validate_guests(None)?;
        for (name, value) in &self.env {
            if !valid_env_name(name) {
                return Err(invalid(format!(
                    "env name {name:?} must match [A-Za-z_][A-Za-z0-9_]*"
                )));
            }
            if let Env::From { from } = value
                && std::env::var_os(from).is_none()
            {
                return Err(format!("environment variable {from} is not set"));
            }
        }
        if let Some(egress) = &self.egress {
            for (name, route) in &egress.routes {
                match route {
                    Route::Address(address) => {
                        if !valid_route_address(address) {
                            let mut detail =
                                format!("route {name:?} target {address:?} must be host:port");
                            if address.contains("://") {
                                detail.push_str(
                                    "; the object form { \"to\": … } is what takes an origin",
                                );
                            }
                            return Err(invalid(detail));
                        }
                    }
                    Route::Inject(inject) => {
                        inject.resolve()?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Refuse two mounts at one guest path: the runtime applies both, and
    /// whichever comes last shadows the other. `extra` is a guest path
    /// pinfold mounts after `validate` has run, so `Box::up` checks the
    /// final list too.
    pub fn validate_guests(&self, extra: Option<&Path>) -> Result<(), String> {
        let mut guests = BTreeSet::new();
        for guest in self.mounts.iter().map(|m| m.guest.as_path()).chain(extra) {
            if !guests.insert(guest) {
                return Err(invalid(format!(
                    "two mounts name the same guest path {}",
                    guest.display()
                )));
            }
        }
        Ok(())
    }
}

/// Refuse a caller label in pinfold's `dev.pinfold.` namespace:
/// `dev.pinfold.owner` drives `list`'s owner and `prune`, and the identity
/// labels name the image.
pub fn check_reserved_labels(labels: &BTreeMap<String, String>) -> Result<(), String> {
    match labels.keys().find(|key| key.starts_with("dev.pinfold.")) {
        Some(key) => Err(format!(
            "label {key:?} is pinfold's: a caller label may not start with dev.pinfold."
        )),
        None => Ok(()),
    }
}

/// A POSIX environment name: `[A-Za-z_][A-Za-z0-9_]*`. Podman expands a
/// trailing `*` in `--env NAME` against its own environment, which is the
/// caller's, so anything else would leak or fail unpredictably.
fn valid_env_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|byte| !byte.is_ascii_digit())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
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

/// A plain route's value: a host and an explicit port in 1–65535, nothing
/// else. A URL-shaped value is a caller reaching for the injecting object
/// form, whose `to` takes the origin, so the refusal points there.
fn valid_route_address(address: &str) -> bool {
    if address.contains("://") {
        return false;
    }
    // `authority_host` fills in `default` when the authority names no port;
    // 0 is not a valid port, so a missing port and `:0` are refused alike.
    let Some((host, port)) = network::authority_host(address, 0) else {
        return false;
    };
    // A path, query, fragment or userinfo is not a host either.
    port != 0 && !host.contains(['/', '?', '#', '@'])
}

/// A memory limit is a whole number of mebibytes or gibibytes, at least
/// 256M. The value reaches the runtime's `--memory` unparsed, and a
/// unitless number is bytes on podman and something else on Apple.
fn valid_memory(memory: &str) -> bool {
    let multiplier: u64 = match memory.bytes().last() {
        Some(b'M') => 1,
        Some(b'G') => 1024,
        _ => return false,
    };
    // `parse` alone would take a leading `+`; `all` alone the empty string.
    let number = &memory[..memory.len() - 1];
    number.bytes().all(|byte| byte.is_ascii_digit())
        && number
            .parse::<u64>()
            .ok()
            .and_then(|number| number.checked_mul(multiplier))
            .is_some_and(|mebibytes| mebibytes >= 256)
}

fn valid_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn invalid(message: impl fmt::Display) -> String {
    format!("invalid box spec: {message}")
}
