//! The box spec: the JSON a caller gives `box up`, parsed into a [`Plan`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::r#box::{Refusal, RefusalReason};
use crate::core::{artifacts, login, network, proxy};

/// A parsed box spec.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub name: String,
    /// The image to run. When absent, the profile's image is used.
    pub image: Option<String>,
    /// The profile to apply: its image when `image` is absent, its `home/`
    /// seeds and its `share/`.
    pub profile: Option<String>,
    /// The pinned harness to install in the box, by its name in
    /// `harnesses.toml`.
    pub harness: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    #[serde(default)]
    pub disks: Vec<Disk>,
    /// Exact environment. `{ "from": "NAME" }` takes the caller's value.
    #[serde(default)]
    pub env: BTreeMap<String, Env>,
    /// Present means the box gets a proxy; absent means no way out at all.
    pub egress: Option<Egress>,
    pub cpus: Option<f64>,
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
    /// `{ "login": HARNESS, "provider"?: P, "from"?: VAR, "to"?: ORIGIN }`:
    /// the proxy dials the harness's origin with the harness's own headers.
    Login(Login),
}

/// A login route: the token stays in the proxy's process and pinfold
/// supplies the origin, the headers and what the box's harness needs to use
/// the route.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    /// The harness the login belongs to; it must be the spec's `harness`.
    pub login: String,
    /// pi's provider, one of [`PI_PROVIDERS`]; only pi names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The variable the token is read from, by name only. claude and pi
    /// need one; codex takes none: its token comes from the host helper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The origin to dial instead of the harness's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
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

/// A login route's target, its headers with values, and the box-sent
/// header names it removes.
pub type Resolved = (Target, Vec<(String, String)>, &'static [&'static str]);

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

/// The box's fixed placeholder for a login token; the proxy injects the
/// real header, so the box never needs the value.
const LOGIN_PLACEHOLDER: &str = "pinfold-placeholder";

/// A pi login route's provider: its origin, the base path of its built-in
/// URL in the pinned pi, its key variable, the header the key goes in, and
/// the box-sent headers removed besides.
pub struct PiProvider {
    pub name: &'static str,
    origin: &'static str,
    pub base_path: &'static str,
    key: &'static str,
    header: &'static str,
    prefix: &'static str,
    strip: &'static [&'static str],
}

pub const PI_PROVIDERS: [PiProvider; 3] = [
    PiProvider {
        name: "openrouter",
        origin: "https://openrouter.ai",
        base_path: "/api/v1",
        key: "OPENROUTER_API_KEY",
        header: "Authorization",
        prefix: "Bearer ",
        strip: &[],
    },
    PiProvider {
        name: "openai",
        origin: "https://api.openai.com",
        base_path: "/v1",
        key: "OPENAI_API_KEY",
        header: "Authorization",
        prefix: "Bearer ",
        strip: &[],
    },
    PiProvider {
        name: "anthropic",
        origin: "https://api.anthropic.com",
        base_path: "",
        key: "ANTHROPIC_API_KEY",
        header: "x-api-key",
        prefix: "",
        // Anthropic also reads a bearer token, which must not travel
        // beside the key.
        strip: &["Authorization"],
    },
];

impl Login {
    /// The origin this login dials: the harness's own, or the spec's `to`.
    pub fn origin(&self) -> Result<&str, String> {
        let default = match self.login.as_str() {
            "claude" => "https://api.anthropic.com",
            "codex" => "https://chatgpt.com",
            "pi" => self.pi_provider()?.origin,
            login => return Err(invalid(format!("login {login:?} is not supported"))),
        };
        Ok(self.to.as_deref().unwrap_or(default))
    }

    /// Whether this is a codex login, whose token comes from the host
    /// helper and whose box gets a config file rather than env.
    pub fn is_codex(&self) -> bool {
        self.login == "codex"
    }

    /// Whether this is a pi login, whose box gets pinfold's pi extension.
    pub fn is_pi(&self) -> bool {
        self.login == "pi"
    }

    /// A pi login's provider.
    pub fn pi_provider(&self) -> Result<&'static PiProvider, String> {
        let provider = self.provider.as_deref().unwrap_or_default();
        PI_PROVIDERS
            .iter()
            .find(|known| known.name == provider)
            .ok_or_else(|| {
                let names = PI_PROVIDERS.each_ref().map(|known| known.name);
                invalid(format!(
                    "pi login provider {provider:?} is not supported; the providers are {names:?}"
                ))
            })
    }

    /// Check a login route without reading the environment: this build
    /// serves the login, only pi names a provider, `from` is present for
    /// claude and pi and absent for codex, and `to` parses like an
    /// injecting route's.
    fn check(&self, route: &str) -> Result<(), String> {
        self.target()?;
        if !self.is_pi() && self.provider.is_some() {
            return Err(invalid(format!(
                "login route {route:?} takes no provider: only pi's does"
            )));
        }
        if self.is_codex() {
            if self.from.is_some() {
                return Err(invalid(format!(
                    "login route {route:?} takes no from variable: codex's token comes from the host's Codex login"
                )));
            }
        } else if self.from.as_deref().is_none_or(str::is_empty) {
            return Err(invalid(format!(
                "login route {route:?} needs a from variable"
            )));
        }
        Ok(())
    }

    /// The dialed target.
    pub fn target(&self) -> Result<Target, String> {
        let origin = self.origin()?;
        parse_target(origin).ok_or_else(|| {
            invalid(format!(
                "login target {origin:?} must be an http:// or https:// origin"
            ))
        })
    }

    /// claude's or pi's dialed target and credential header, from this
    /// process's environment once at start, and the box-sent headers to
    /// remove besides. The target and the value rules are
    /// [`Inject::resolve`]'s.
    pub fn resolve(&self) -> Result<Resolved, String> {
        let (header, prefix, strip) = if self.is_pi() {
            let provider = self.pi_provider()?;
            (provider.header, provider.prefix, provider.strip)
        } else {
            ("Authorization", "Bearer ", &[][..])
        };
        let from = self.from.clone().unwrap_or_default();
        if std::env::var_os(&from).is_some_and(|value| value.is_empty()) {
            return Err(format!("environment variable {from} is empty"));
        }
        let inject = Inject {
            to: self.origin()?.to_string(),
            headers: BTreeMap::from([(
                header.to_string(),
                Header {
                    from,
                    prefix: prefix.to_string(),
                },
            )]),
        };
        let (target, headers) = inject.resolve()?;
        Ok((target, headers, strip))
    }

    /// The box environment a login route adds beside the harness defaults;
    /// the spec's own `env` still wins. `route` is the route's name. codex
    /// adds none: its box gets [`login::codex_config`]. pi gets only its
    /// key's placeholder: [`login::pi_extension`] points pi at the route.
    pub fn env(&self, route: &str) -> Vec<(&'static str, String)> {
        if self.is_codex() {
            return Vec::new();
        }
        if self.is_pi() {
            return self
                .pi_provider()
                .map(|provider| vec![(provider.key, LOGIN_PLACEHOLDER.to_string())])
                .unwrap_or_default();
        }
        vec![
            ("ANTHROPIC_BASE_URL", format!("http://{route}")),
            ("CLAUDE_CODE_OAUTH_TOKEN", LOGIN_PLACEHOLDER.to_string()),
        ]
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
}

/// One named disk and where it appears in the box.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disk {
    pub name: String,
    pub guest: PathBuf,
    /// `<n>G`; caps a disk pinfold creates.
    pub size: Option<String>,
    /// A host directory copied in when pinfold creates the disk.
    pub from: Option<PathBuf>,
    #[serde(default)]
    pub readonly: bool,
}

impl Disk {
    /// The size in bytes; `validate` has checked the spelling.
    pub fn bytes(&self) -> u64 {
        self.size
            .as_deref()
            .and_then(disk_gibibytes)
            .unwrap_or(DEFAULT_DISK_GIB)
            << 30
    }
}

const DEFAULT_DISK_GIB: u64 = 64;

/// A disk size: a whole number of gibibytes, at least 1, then `G`.
fn disk_gibibytes(size: &str) -> Option<u64> {
    let number = size.strip_suffix('G')?;
    // `parse` alone would take a leading `+`.
    if !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number
        .parse::<u64>()
        .ok()
        .filter(|gib| (1..=u64::MAX >> 30).contains(gib))
}

/// One environment entry.
#[derive(Debug, Clone)]
pub enum Env {
    /// A literal value from the spec.
    Exact(String),
    /// `{ "from": "NAME" }`: the caller's value, passed by name only.
    From { from: String },
}

impl<'de> Deserialize<'de> for Env {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Env;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a string or an object with only a from key")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Env, E> {
                Ok(Env::Exact(value.to_string()))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Env, E> {
                Ok(Env::Exact(value))
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Env, A::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct From {
                    from: String,
                }
                let From { from } =
                    From::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(Env::From { from })
            }
        }
        deserializer.deserialize_any(Visitor)
    }
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
            && !memory_mib(memory).is_some_and(|mebibytes| mebibytes >= 256)
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
            host_directory("mount host", &mount.host)?;
            // A Linux box copies the whole tree at each start.
            #[cfg(target_os = "linux")]
            unreadable(&mount.host).map_err(|(path, error)| {
                invalid(format!("mount host {}: {error}", path.display()))
            })?;
        }
        let mut names = std::collections::BTreeSet::new();
        for disk in &self.disks {
            if !names.insert(&disk.name) {
                return Err(invalid(format!("disk {:?} is named twice", disk.name)));
            }
            if !valid_name(&disk.name) {
                return Err(invalid(format!(
                    "disk name {:?} must start alphanumeric and hold only [A-Za-z0-9._-]",
                    disk.name
                )));
            }
            if let Some(size) = &disk.size
                && disk_gibibytes(size).is_none()
            {
                return Err(invalid(format!(
                    "disk {:?} size {size:?} must be a whole number of G, at least 1G",
                    disk.name
                )));
            }
            if !disk.guest.is_absolute() {
                return Err(invalid(format!(
                    "disk {:?} guest {} must be absolute",
                    disk.name,
                    disk.guest.display()
                )));
            }
            for path in std::iter::once(&disk.guest).chain(&disk.from) {
                if !valid_mount_path(path) {
                    return Err(invalid(format!(
                        "disk path {:?} must not hold ',' or an ASCII control character",
                        path
                    )));
                }
            }
            if let Some(from) = &disk.from {
                // A read-only attachment cannot receive the copy.
                if disk.readonly {
                    return Err(invalid(format!(
                        "disk {:?} cannot be readonly with from",
                        disk.name
                    )));
                }
                if !from.is_absolute() {
                    return Err(invalid(format!(
                        "disk {:?} from {} must be absolute",
                        disk.name,
                        from.display()
                    )));
                }
                host_directory("disk from", from)?;
            }
        }
        self.validate_guests(&[])?;
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
            for entry in &egress.allow {
                if let Some(suffix) = entry.strip_prefix('.')
                    && on_or_above_public_suffix(suffix)
                {
                    return Err(invalid(format!(
                        "egress allow entry {entry:?} is on or above a public suffix"
                    )));
                }
            }
            let mut seen_login = false;
            let mut pi_providers = BTreeSet::new();
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
                    Route::Login(login) => {
                        if self.harness.as_deref() != Some(login.login.as_str()) {
                            let detail = match &self.harness {
                                Some(harness) => format!(
                                    "login route {name:?} names {:?}, but the spec's harness is {harness:?}",
                                    login.login
                                ),
                                None => format!(
                                    "login route {name:?} names {:?}, but the spec has no harness",
                                    login.login
                                ),
                            };
                            return Err(invalid(detail));
                        }
                        login.check(name)?;
                        // pi takes one login route per provider, the other
                        // harnesses one per spec.
                        if login.is_pi() {
                            let provider = login.pi_provider()?.name;
                            if !pi_providers.insert(provider) {
                                return Err(invalid(format!(
                                    "login route {name:?} is a second pi login route for {provider:?}"
                                )));
                            }
                        } else if seen_login {
                            return Err(invalid(format!(
                                "login route {name:?} is a second login route"
                            )));
                        }
                        seen_login = true;
                    }
                }
            }
        }
        Ok(())
    }

    /// The spec's login routes, with their names.
    pub fn logins(&self) -> impl Iterator<Item = (&str, &Login)> {
        self.egress
            .iter()
            .flat_map(|egress| &egress.routes)
            .filter_map(|(name, route)| match route {
                Route::Login(login) => Some((name.as_str(), login)),
                _ => None,
            })
    }

    /// Resolve each login route's credential before the claim, so a login
    /// that cannot be used refuses as `login` rather than `spec`: claude's
    /// and pi's from this process's environment, codex's from the host
    /// helper. The value never enters the plan; codex's token is returned
    /// for the proxy.
    pub fn resolve_login(
        &self,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Option<login::Token>, String> {
        let mut token = None;
        for (route, login) in self.logins() {
            let resolved = if login.is_codex() {
                login::token_from_helper(cancel).map(|codex| token = Some(codex))
            } else {
                login.resolve().map(|_| ())
            };
            resolved.map_err(|error| format!("{} login route {route:?}: {error}", login.login))?;
        }
        Ok(token)
    }

    /// Refuse two mounts or disks at one lexical guest path. `extra` holds the paths
    /// pinfold mounts after `validate` has run, so `Box::up` checks the
    /// final list too.
    pub fn validate_guests(&self, extra: &[&Path]) -> Result<(), String> {
        let mut guests = BTreeSet::new();
        for guest in self
            .mounts
            .iter()
            .map(|m| m.guest.as_path())
            .chain(self.disks.iter().map(|disk| disk.guest.as_path()))
            .chain(extra.iter().copied())
        {
            let normalized = guest_path(guest);
            // The guest kernel's filesystems carry the box's controls (the
            // userns sysctl, the work cgroup); a disk or mount there would
            // replace them.
            if KERNEL_PATHS
                .iter()
                .any(|kernel| normalized.starts_with(kernel))
            {
                return Err(invalid(format!(
                    "guest path {} is at or below /proc, /sys or /dev",
                    guest.display()
                )));
            }
            if HIDING_PATHS
                .iter()
                .any(|path| normalized == Path::new(path))
            {
                return Err(invalid(format!(
                    "guest path {} is /, /tmp, /etc or /etc/resolv.conf, which would hide the box's tmpfs /tmp or empty /etc/resolv.conf",
                    guest.display()
                )));
            }
            if !guests.insert(normalized) {
                return Err(invalid(format!(
                    "two mounts or disks name the same guest path {}",
                    guest.display()
                )));
            }
        }
        // A Firecracker VM's drives are vda to vdz; the init and root disks
        // take the first two.
        if cfg!(target_os = "linux") && guests.len() > MAX_ATTACHMENTS {
            return Err(invalid(format!(
                "a box attaches at most {MAX_ATTACHMENTS} disks and mounts, pinfold's own mounts included; this one has {}",
                guests.len()
            )));
        }
        Ok(())
    }
}

/// The disks and mounts one Linux box attaches.
const MAX_ATTACHMENTS: usize = 24;

/// Refuse the first entry under `dir` the user cannot read: a directory
/// it cannot list or a file it cannot open. Symlinks are copied as links.
#[cfg(target_os = "linux")]
fn unreadable(dir: &Path) -> Result<(), (PathBuf, std::io::Error)> {
    use nix::unistd::{AccessFlags, access};
    let entries = std::fs::read_dir(dir).map_err(|error| (dir.to_path_buf(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| (dir.to_path_buf(), error))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| (path.clone(), error))?;
        if kind.is_dir() {
            access(&path, AccessFlags::R_OK | AccessFlags::X_OK)
                .map_err(|error| (path.clone(), error.into()))?;
            unreadable(&path)?;
        } else if kind.is_file() {
            access(&path, AccessFlags::R_OK).map_err(|error| (path.clone(), error.into()))?;
        }
    }
    Ok(())
}

/// Refuse `path` unless it is a host directory.
fn host_directory(what: &str, path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(invalid(format!(
            "{what} {} is not a directory",
            path.display()
        ))),
        Err(error) => Err(invalid(format!("{what} {}: {error}", path.display()))),
    }
}

/// Guest paths no mount or disk may name, at or below.
const KERNEL_PATHS: [&str; 3] = ["/proc", "/sys", "/dev"];

/// Guest paths no mount or disk may name exactly: one there would cover
/// an Always applied tmpfs `/tmp` or empty `/etc/resolv.conf`.
const HIDING_PATHS: [&str; 4] = ["/", "/tmp", "/etc", "/etc/resolv.conf"];

/// Resolve guest components lexically, without consulting the host filesystem.
pub(crate) fn guest_path(path: &Path) -> PathBuf {
    let mut guest = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                guest.pop();
            }
            Component::CurDir => {}
            component => guest.push(component.as_os_str()),
        }
    }
    guest
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

/// A POSIX environment name: `[A-Za-z_][A-Za-z0-9_]*`. Anything else
/// would leak or fail unpredictably in a runtime CLI's `--env`.
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
    // `authority_host` fills in `default` when the authority names no port;
    // 0 is not a valid port, so a missing port and `:0` are refused alike.
    let Some((host, port)) = network::authority_host(address, 0) else {
        return false;
    };
    // A path, query, fragment or userinfo is not a host either.
    port != 0 && !host.contains(['/', '?', '#', '@'])
}

/// The Public Suffix List, pinned. The proxy admits a `.name` entry's
/// subdomains, so `.name` on or above a public suffix admits names an
/// attacker can register.
const PUBLIC_SUFFIX_LIST: &str = include_str!("../../public_suffix_list.dat");

/// Whether `suffix` is a public suffix or has one below it: the list holds
/// a rule that is the suffix or ends with `."suffix"`. Comment and blank
/// lines are skipped, a rule's `*.` or `!` marker is stripped, and the
/// comparison ignores case.
fn on_or_above_public_suffix(suffix: &str) -> bool {
    let name = suffix.to_lowercase();
    let below = format!(".{name}");
    PUBLIC_SUFFIX_LIST.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            return false;
        }
        let rule = line
            .strip_prefix("*.")
            .or_else(|| line.strip_prefix('!'))
            .unwrap_or(line)
            .to_lowercase();
        rule == name || rule.ends_with(&below)
    })
}

/// A memory value's mebibytes: a whole number, then `M` or `G`.
pub fn memory_mib(memory: &str) -> Option<u64> {
    let multiplier: u64 = match memory.bytes().last() {
        Some(b'M') => 1,
        Some(b'G') => 1024,
        _ => return None,
    };
    // `parse` alone would take a leading `+`; `all` alone the empty string.
    let number = &memory[..memory.len() - 1];
    if !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse::<u64>().ok()?.checked_mul(multiplier)
}

pub(crate) fn valid_name(name: &str) -> bool {
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
