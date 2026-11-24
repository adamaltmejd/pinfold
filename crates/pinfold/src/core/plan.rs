//! The box spec: the JSON a caller gives `box up`, parsed into a [`Plan`].

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::r#box::{Refusal, RefusalReason};
use crate::core::{login, network, proxy};

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
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    #[serde(default)]
    pub disks: Vec<Disk>,
    /// Exact environment. `{ "from": "NAME" }` takes the caller's value.
    #[serde(default, deserialize_with = "env_entries")]
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
    /// headers from the launch's [`Values`].
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
    /// The agent the login belongs to.
    pub login: String,
    /// pi's provider, one of [`PI_PROVIDERS`]; only pi names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The token's [`Source`]. claude and pi need one; codex takes none:
    /// its token comes from the host helper.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
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

/// One injected header: `prefix` then the value of its [`Source`]. Only
/// the variable's name or the reference is ever written down.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub from: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub prefix: Option<String>,
}

/// Where a secret's value comes from, as a `from` key names it: a
/// 1Password reference when it starts with `op://`, else a host variable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    From(String),
    Op(String),
}

/// A key that is present holds a string: an explicit null is refused,
/// never read as absent.
pub(crate) fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

/// The box spec's env map, each entry's decoding error naming its key.
fn env_entries<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Env>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, Env>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of environment variables")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut env = BTreeMap::new();
            while let Some(name) = map.next_key::<String>()? {
                let value = map
                    .next_value::<Env>()
                    .map_err(|error| serde::de::Error::custom(format!("env {name}: {error}")))?;
                env.insert(name, value);
            }
            Ok(env)
        }
    }
    deserializer.deserialize_map(Visitor)
}

impl Source {
    /// A `from` key's source, or why it cannot be read.
    pub(crate) fn declared(from: &str) -> Result<Source, &'static str> {
        if from.starts_with("op://") {
            if valid_op(from) {
                Ok(Source::Op(from.to_string()))
            } else {
                Err("its 1Password reference holds a control character or exceeds 1024 bytes")
            }
        } else if valid_from(from) {
            Ok(Source::From(from.to_string()))
        } else {
            Err("its from variable is empty or holds = or NUL")
        }
    }

    /// The variable's name or the reference: never a value.
    pub fn name(&self) -> &str {
        match self {
            Source::From(name) | Source::Op(name) => name,
        }
    }
}

impl Header {
    pub fn source(&self) -> Result<Source, &'static str> {
        Source::declared(&self.from)
    }
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
    /// The parsed target, after checking each header's declaration without
    /// reading its value.
    fn check(&self) -> Result<Target, String> {
        // Never echo `to`: a pasted URL may hold a password or token.
        let target = parse_target(&self.to).ok_or_else(|| {
            invalid("a route's `to` must be an http:// or https:// origin".to_string())
        })?;
        for (name, header) in &self.headers {
            if !valid_header_name(name) {
                return Err(invalid(format!("route header {name:?} cannot be injected")));
            }
            header
                .source()
                .map_err(|error| invalid(format!("route header {name:?}: {error}")))?;
            if header
                .prefix
                .as_ref()
                .is_some_and(|prefix| prefix.contains(['\r', '\n', '\0']))
            {
                return Err(invalid(format!(
                    "route header {name:?}: its prefix holds CR, LF or NUL"
                )));
            }
        }
        Ok(target)
    }

    /// The parsed target and each header with its value from `values`.
    /// Errors name the header and the source, never a value.
    pub fn resolve(
        &self,
        route: &str,
        values: &Values,
    ) -> Result<(Target, Vec<(String, String)>), String> {
        let target = self.check()?;
        let mut headers = Vec::with_capacity(self.headers.len());
        for (name, header) in &self.headers {
            let source = header.source().map_err(invalid)?;
            if values.pending(&source) {
                continue;
            }
            let value = match values.get(&source).map(OsStr::to_str) {
                Some(Some(value)) => value,
                None => return Err(unset("route", route, values, &source)),
                Some(None) => {
                    return Err(invalid(format!(
                        "route header {name:?}: {} is not UTF-8",
                        source.name()
                    )));
                }
            };
            // A line break or NUL would let the value frame its own headers.
            if value.contains(['\r', '\n', '\0']) {
                return Err(invalid(format!(
                    "route header {name:?}: {} holds CR, LF or NUL",
                    source.name()
                )));
            }
            let value = format!("{}{value}", header.prefix.as_deref().unwrap_or_default());
            headers.push((name.clone(), value));
        }
        Ok((target, headers))
    }
}

/// The box's fixed placeholder for a login token; the proxy injects the
/// real header, so the box never needs the value.
pub(crate) const LOGIN_PLACEHOLDER: &str = "pinfold-placeholder";

/// A pi login route's provider `P`: pi's own provider id, its origin, the
/// base path of its built-in URL in the pinned pi, and its credential.
pub struct PiProvider {
    pub name: &'static str,
    /// pi's provider id; `anthropic` and `anthropic-subscription` share one.
    pub pi: &'static str,
    origin: &'static str,
    pub base_path: &'static str,
    pub credential: PiCredential,
}

pub enum PiCredential {
    /// The proxy sets `header` to `prefix` and `$VAR`; the extension uses
    /// only `placeholder`.
    Key {
        placeholder: &'static str,
        header: &'static str,
        prefix: &'static str,
    },
    /// The host's Codex login sets the headers, as for a codex login route.
    /// pinfold's extension gives pi `placeholder` as the provider's `apiKey`.
    HostCodex { placeholder: &'static str },
}

pub const PI_PROVIDERS: [PiProvider; 5] = [
    PiProvider {
        name: "openrouter",
        pi: "openrouter",
        origin: "https://openrouter.ai",
        base_path: "/api/v1",
        credential: PiCredential::Key {
            placeholder: LOGIN_PLACEHOLDER,
            header: "Authorization",
            prefix: "Bearer ",
        },
    },
    PiProvider {
        name: "openai",
        pi: "openai",
        origin: "https://api.openai.com",
        base_path: "/v1",
        credential: PiCredential::Key {
            placeholder: LOGIN_PLACEHOLDER,
            header: "Authorization",
            prefix: "Bearer ",
        },
    },
    PiProvider {
        name: "anthropic",
        pi: "anthropic",
        origin: "https://api.anthropic.com",
        base_path: "",
        credential: PiCredential::Key {
            placeholder: LOGIN_PLACEHOLDER,
            header: "x-api-key",
            prefix: "",
        },
    },
    PiProvider {
        name: "anthropic-subscription",
        pi: "anthropic",
        origin: "https://api.anthropic.com",
        base_path: "",
        credential: PiCredential::Key {
            // pi sends a subscription's headers only for a key holding
            // `sk-ant-oat` in its placeholder.
            placeholder: "sk-ant-oat-pinfold-placeholder",
            header: "Authorization",
            prefix: "Bearer ",
        },
    },
    PiProvider {
        name: "openai-codex",
        pi: "openai-codex",
        origin: "https://chatgpt.com",
        base_path: "/backend-api",
        credential: PiCredential::HostCodex {
            // pi reads the account id from this JWT's payload with `atob`,
            // so it is plain base64 without `-` or `_`, and unsigned. The
            // payload is {"https://api.openai.com/auth":{"chatgpt_account_id":
            // "pinfold-placeholder"}} padded with spaces. pi takes it
            // literally: no `$`, no leading `!`.
            placeholder: "eyJhbGciOiJub25lIiB9.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoicGluZm9sZC1wbGFjZWhvbGRlciJ9ICB9.cGluZm9sZA",
        },
    },
];

impl Login {
    /// The origin this login dials: the harness's own, or the spec's `to`.
    pub fn origin(&self) -> Result<&str, String> {
        let default = match self.login.as_str() {
            "claude" => "https://api.anthropic.com",
            "codex" => "https://chatgpt.com",
            "github" => "https://github.com",
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

    /// Whether this is a github login, served at [`GITHUB_NAMES`] rather
    /// than its own name.
    pub fn is_github(&self) -> bool {
        self.login == "github"
    }

    /// The proxy names this route serves, each with the origin it dials
    /// and the path put before each request's. Holds no credential.
    pub fn served<'a>(
        &'a self,
        route: &'a str,
    ) -> Result<Vec<(&'a str, &'a str, &'static str)>, String> {
        let origin = self.origin()?;
        if !self.is_github() {
            return Ok(vec![(route, origin, "")]);
        }
        let [git_name, api_name] = GITHUB_NAMES;
        let api = match self.to {
            Some(_) => (api_name, origin, "/api/v3"),
            None => (api_name, "https://api.github.com", ""),
        };
        Ok(vec![(git_name, origin, ""), api])
    }

    /// Whether the proxy takes this login's headers from the host's Codex
    /// login: codex's, and pi's `openai-codex`.
    pub fn uses_host_codex(&self) -> bool {
        self.is_codex()
            || (self.is_pi()
                && self.pi_provider().is_ok_and(|provider| {
                    matches!(provider.credential, PiCredential::HostCodex { .. })
                }))
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

    /// The token's source; `None` for a login on the host's Codex login.
    pub fn source(&self) -> Option<Result<Source, &'static str>> {
        (!self.uses_host_codex()).then(|| {
            self.from
                .as_deref()
                .map_or(Err("needs a from source"), Source::declared)
        })
    }

    /// Check a login route without reading the environment: this build
    /// serves the login, only pi names a provider, `from` is absent for a
    /// login on the host's Codex login and present otherwise, and `to`
    /// parses like an injecting route's.
    fn check(&self, route: &str) -> Result<(), String> {
        self.target()?;
        if !self.is_pi() && self.provider.is_some() {
            return Err(invalid(format!(
                "login route {route:?} takes no provider: only pi's does"
            )));
        }
        match self.source() {
            None if self.from.is_some() => Err(invalid(format!(
                "login route {route:?} takes no from source: its token comes from the host's Codex login"
            ))),
            None => Ok(()),
            Some(source) => source
                .map(|_| ())
                .map_err(|error| invalid(format!("login route {route:?}: {error}"))),
        }
    }

    /// The dialed target.
    pub fn target(&self) -> Result<Target, String> {
        let origin = self.origin()?;
        // Never echo `to`, as for an injecting route.
        parse_target(origin).ok_or_else(|| {
            invalid("a login route's `to` must be an http:// or https:// origin".to_string())
        })
    }

    /// claude's, github's or pi's served names, each with its dialed target
    /// and credential header, from `values`. The target and the value rules
    /// are [`Inject::resolve`]'s.
    pub fn resolve(&self, route: &str, values: &Values) -> Result<Vec<Served>, String> {
        let (header, prefix) = if self.is_pi() {
            match self.pi_provider()?.credential {
                PiCredential::Key { header, prefix, .. } => (header, prefix),
                PiCredential::HostCodex { .. } => {
                    return Err("its token comes from the host's Codex login".to_string());
                }
            }
        } else if self.is_github() {
            ("Authorization", "")
        } else {
            ("Authorization", "Bearer ")
        };
        // Every caller prefixes the agent and route, so these name neither.
        let source = match self.source() {
            Some(source) => source.map_err(invalid)?,
            None => return Err("its token comes from the host's Codex login".to_string()),
        };
        match values.get(&source) {
            None => return Err(values.missing(&source)),
            // An `op` value is never empty: its read refuses one.
            Some(value) if value.is_empty() => {
                return Err(format!("the host variable {} is empty", source.name()));
            }
            Some(_) => {}
        }
        let inject = Inject {
            to: self.origin()?.to_string(),
            headers: BTreeMap::from([(
                header.to_string(),
                Header {
                    from: source.name().to_string(),
                    prefix: (!prefix.is_empty()).then(|| prefix.to_string()),
                },
            )]),
        };
        let (_, headers) = inject.resolve(route, values)?;
        if self.is_github() && headers.is_empty() {
            return Err(values.missing(&source));
        }
        let mut served = Vec::new();
        for (name, origin, base_path) in self.served(route)? {
            let headers = match headers.first() {
                Some((_, token)) if self.is_github() && name == GITHUB_NAMES[0] => {
                    let basic = base64(format!("x-access-token:{token}").as_bytes());
                    vec![(header.to_string(), format!("Basic {basic}"))]
                }
                Some((_, token)) if self.is_github() => {
                    vec![(header.to_string(), format!("Bearer {token}"))]
                }
                _ => headers.clone(),
            };
            served.push(Served {
                name: name.to_string(),
                // `inject.resolve` checked `to`; GitHub's API origin parses.
                target: parse_target(origin).expect("a checked login origin"),
                base_path,
                headers,
            });
        }
        Ok(served)
    }
}

/// A github login's proxy names, git's then the API's. Fixed: `gh` speaks
/// plain HTTP only to `github.localhost`, an undocumented `gh` behaviour.
pub const GITHUB_NAMES: [&str; 2] = ["github.localhost", "api.github.localhost"];

/// One proxy name a login route serves: where it dials, the path put
/// before each request's, and the credential headers. No `Debug`: the
/// headers hold a credential.
pub struct Served {
    pub name: String,
    pub target: Target,
    pub base_path: &'static str,
    pub headers: Vec<(String, String)>,
}

/// The host values a plan's sources name, keyed by source. The process
/// that starts a launch reads them once; every later reader takes this
/// table. An unset variable, and a reference not yet read, has no entry; a
/// failed read keeps its cause. No `Debug`: it holds secrets.
#[derive(Clone, Default)]
pub struct Values {
    values: BTreeMap<Source, OsString>,
    /// Each reference whose read failed, with the cause, which names no
    /// value and none of op's output.
    failed: BTreeMap<String, String>,
    /// Whether the 1Password references were read. Until then validation
    /// checks host variables alone.
    op_read: bool,
}

impl Values {
    /// A table of values another process read and sent.
    pub(crate) fn received(values: BTreeMap<Source, OsString>) -> Values {
        Values {
            values,
            failed: BTreeMap::new(),
            op_read: true,
        }
    }

    /// The entries of `sources` alone, failed reads included.
    pub(crate) fn only(&self, sources: impl Iterator<Item = Source>) -> Values {
        let mut only = Values {
            op_read: self.op_read,
            ..Values::default()
        };
        for source in sources {
            if let Source::Op(reference) = &source
                && let Some(cause) = self.failed.get(reference)
            {
                only.failed.insert(reference.clone(), cause.clone());
            }
            if let Some(value) = self.values.get(&source) {
                only.values.insert(source, value.clone());
            }
        }
        only
    }

    /// The `op` values, by reference.
    pub(crate) fn op_values(&self) -> impl Iterator<Item = (&str, &OsStr)> {
        self.values
            .iter()
            .filter_map(|(source, value)| match source {
                Source::Op(reference) => Some((reference.as_str(), value.as_os_str())),
                Source::From(_) => None,
            })
    }

    /// Each variable `plan` names, from this process's environment. A name
    /// validation refuses is never read; nor is any `op` reference.
    pub fn read(plan: &Plan) -> Values {
        let variables: BTreeSet<String> = plan
            .sources()
            .filter_map(|(_, _, source)| match source {
                Ok(Source::From(variable)) => Some(variable),
                _ => None,
            })
            .collect();
        let mut values = Values::default();
        for variable in variables {
            if let Some(value) = std::env::var_os(&variable) {
                values.values.insert(Source::From(variable), value);
            }
        }
        values
    }

    /// Read each distinct `op` reference `plan` names, once, with the `op`
    /// on PATH. Only the process that starts a box calls this.
    pub fn read_op(&mut self, plan: &Plan) {
        let references: BTreeSet<String> = plan
            .sources()
            .filter_map(|(_, _, source)| match source {
                Ok(Source::Op(reference)) => Some(reference),
                _ => None,
            })
            .collect();
        self.op_read = true;
        for reference in references {
            match login::op_read(&reference) {
                Ok(value) => {
                    self.values.insert(Source::Op(reference), value.into());
                }
                Err(cause) => {
                    self.failed.insert(reference, cause);
                }
            }
        }
    }

    /// Whether `source` is an `op` reference not read yet.
    fn pending(&self, source: &Source) -> bool {
        matches!(source, Source::Op(_)) && !self.op_read
    }

    pub fn get(&self, source: &Source) -> Option<&OsStr> {
        self.values.get(source).map(OsString::as_os_str)
    }

    /// Why `source` has no value, after the key that names it.
    pub fn missing(&self, source: &Source) -> String {
        match source {
            Source::From(variable) => {
                format!("needs the host variable {variable}, which is not set")
            }
            Source::Op(reference) => format!(
                "needs the 1Password reference {reference}: {}",
                self.failed
                    .get(reference)
                    .map_or("op read did not run", String::as_str)
            ),
        }
    }

    /// Whether `source` was read and has no value: an unset variable, or a
    /// failed `op` read. A reference never read is not missing here.
    fn lacks(&self, source: &Source) -> bool {
        match source {
            Source::From(_) => self.get(source).is_none(),
            Source::Op(reference) => self.failed.contains_key(reference),
        }
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
        && !name.eq_ignore_ascii_case("expect")
        && !proxy::hop_by_hop(name)
}

/// One host directory and where it appears in the box.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub host: PathBuf,
    pub guest: PathBuf,
    #[serde(default)]
    pub kind: MountKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MountKind {
    #[default]
    Readonly,
    Share,
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
    digits(size.strip_suffix('G')?).filter(|gib| (1..=u64::MAX >> 30).contains(gib))
}

/// A whole number spelled in ASCII digits only: `parse` alone would take a
/// leading `+`.
fn digits(number: &str) -> Option<u64> {
    number
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| number.parse().ok())?
}

/// One environment entry. It serializes as declared, never as a value read
/// from the caller.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Env {
    /// A literal value from the spec.
    Exact(String),
    /// `{ "from": SOURCE }`: the caller's value, passed by its source only.
    Secret { from: String },
}

impl Env {
    /// The source of a [`Env::Secret`] entry.
    pub fn source(&self) -> Option<Result<Source, &'static str>> {
        match self {
            Env::Exact(_) => None,
            Env::Secret { from } => Some(Source::declared(from)),
        }
    }
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

            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Env, A::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Secret {
                    from: String,
                }
                let Secret { from } =
                    Secret::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(Env::Secret { from })
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
    /// namespace; `pinfold -- CMD`'s own plan carries `dev.pinfold.project`.
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

    /// Refuse, naming the first one, when a source the spec reads is an
    /// unset variable or a failed `op` read. Only presence is checked, and
    /// a reference not read is not checked. Declarations are checked first.
    pub(crate) fn check_variables(&self, values: &Values) -> Result<(), String> {
        for (kind, name, source) in self.sources() {
            if let Ok(source) = source
                && values.lacks(&source)
            {
                return Err(unset(kind, name, values, &source));
            }
        }
        Ok(())
    }

    /// Each secret source the spec declares, as (`env` or `route`, the
    /// entry's name, the source or why it is malformed): env entries, then
    /// injecting headers and login routes.
    pub(crate) fn sources(
        &self,
    ) -> impl Iterator<Item = (&'static str, &str, Result<Source, &'static str>)> {
        let env = self
            .env
            .iter()
            .filter_map(|(name, value)| Some(("env", name.as_str(), value.source()?)));
        let routes = self
            .egress
            .iter()
            .flat_map(|egress| &egress.routes)
            .flat_map(|(name, route)| match route {
                Route::Inject(inject) => inject
                    .headers
                    .values()
                    .map(|header| ("route", name.as_str(), header.source()))
                    .collect(),
                Route::Login(login) => login
                    .source()
                    .map(|source| ("route", name.as_str(), source))
                    .into_iter()
                    .collect(),
                Route::Address(_) => Vec::new(),
            });
        env.chain(routes)
    }

    /// Whether a source is a well-formed `op` reference, which only the
    /// process that starts a box reads.
    pub fn reads_op(&self) -> bool {
        self.sources()
            .any(|(_, _, source)| matches!(source, Ok(Source::Op(_))))
    }

    /// Check a parsed spec, and against the values its `from` entries name
    /// when given.
    pub fn validate(&self, values: Option<&Values>) -> Result<(), String> {
        if cfg!(target_os = "linux") && self.mounts.iter().any(|m| m.kind == MountKind::Share) {
            return Err(invalid("share mounts are unsupported on Linux"));
        }
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
        for (name, value) in &self.env {
            if !valid_env_name(name) {
                return Err(invalid(format!(
                    "env name {name:?} must match [A-Za-z_][A-Za-z0-9_]*"
                )));
            }
            // No execve accepts it; the message names only the variable.
            if let Env::Exact(value) = value
                && value.contains('\0')
            {
                return Err(invalid(format!("env {name} must not hold a NUL byte")));
            }
            let Some(source) = value.source() else {
                continue;
            };
            let source = source.map_err(|error| invalid(format!("env {name}: {error}")))?;
            if let Some(values) = values
                && values.get(&source).is_none()
                && !values.pending(&source)
            {
                return Err(unset("env", name, values, &source));
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
            let mut logins = BTreeSet::new();
            let mut pi_providers = BTreeSet::new();
            let mut route_names = BTreeSet::new();
            for (name, route) in &egress.routes {
                let served = match route {
                    Route::Login(login) => login.served(name)?,
                    _ => vec![(name.as_str(), "", "")],
                };
                for (name, _, _) in served {
                    if !route_names.insert(name.to_ascii_lowercase()) {
                        return Err(invalid(format!(
                            "route {name:?} duplicates a name ignoring ASCII case"
                        )));
                    }
                }
                match route {
                    Route::Address(address) => {
                        if !valid_route_address(address) {
                            // Never echo the address, as for an injecting route.
                            let mut detail = format!("route {name:?} target must be host:port");
                            if address.contains("://") {
                                detail.push_str(
                                    "; the object form { \"to\": … } is what takes an origin",
                                );
                            }
                            return Err(invalid(detail));
                        }
                    }
                    Route::Inject(inject) => match values {
                        Some(values) => _ = inject.resolve(name, values)?,
                        None => _ = inject.check()?,
                    },
                    Route::Login(login) => {
                        login.check(name)?;
                        // pi takes one login route per pi provider, the
                        // other agents one each.
                        if login.is_pi() {
                            let provider = login.pi_provider()?.pi;
                            if !pi_providers.insert(provider) {
                                return Err(invalid(format!(
                                    "login route {name:?} is a second pi login route for {provider:?}"
                                )));
                            }
                        } else if !logins.insert(&login.login) {
                            return Err(invalid(format!(
                                "login route {name:?} is a second {} login route",
                                login.login
                            )));
                        }
                        if login.is_github()
                            && let Some((variable, _)) = login::GITHUB_ENV
                                .iter()
                                .find(|(variable, _)| self.env.contains_key(*variable))
                        {
                            return Err(invalid(format!(
                                "env {variable} is set by the github login route {name:?}"
                            )));
                        }
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
    /// that cannot be used refuses as `login` rather than `spec`: from
    /// `values`, or from the host helper for a login on the host's Codex
    /// login. The value never enters the plan; the Codex token
    /// is returned for the proxy. A codex route and pi's `openai-codex`
    /// route share that one token, so the helper is asked once.
    pub fn resolve_login(
        &self,
        values: &Values,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Option<login::Token>, String> {
        let mut token = None;
        for (route, login) in self.logins() {
            let resolved = if login.uses_host_codex() {
                match token {
                    Some(_) => Ok(()),
                    None => login::token_from_helper(cancel).map(|codex| token = Some(codex)),
                }
            } else {
                login.resolve(route, values).map(|_| ())
            };
            resolved.map_err(|error| format!("{} login route {route:?}: {error}", login.login))?;
        }
        Ok(token)
    }

    /// Refuse two mounts or disks at one lexical guest path, or one on the
    /// kernel's or a hiding path. `Box::up` calls it after `validate`, with
    /// `extra` the paths pinfold adds itself, and again once a profile adds
    /// its `share/`, so it checks the final list.
    pub fn validate_guests(&self, extra: &[&Path]) -> Result<(), String> {
        let mut guests = BTreeSet::new();
        for guest in self
            .mounts
            .iter()
            .map(|m| m.guest.as_path())
            .chain(self.disks.iter().map(|disk| disk.guest.as_path()))
            .chain(extra.iter().copied())
        {
            if !guest.is_absolute() || !valid_mount_path(guest) {
                return Err(invalid(format!(
                    "guest path {:?} must be absolute and hold no comma or ASCII control character",
                    guest
                )));
            }
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
        // The pinned x86_64 VMM supports 17 VirtIO devices. Init, root and
        // vsock take three.
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
const MAX_ATTACHMENTS: usize = 14;

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

/// A host variable name: no environment holds one that is empty or holds
/// `=` or NUL, and macOS `getenv` reads `A=B` as `A`.
fn valid_from(name: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0'])
}

/// A 1Password secret reference `op read` takes as one argument.
fn valid_op(reference: &str) -> bool {
    reference.starts_with("op://")
        && !reference.chars().any(|c| c.is_ascii_control())
        && reference.len() <= 1024
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
pub(crate) fn valid_mount_path(path: &Path) -> bool {
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
    digits(&memory[..memory.len() - 1])?.checked_mul(multiplier)
}

pub(crate) fn valid_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &byte)| n | u32::from(byte) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn invalid(message: impl fmt::Display) -> String {
    format!("invalid box spec: {message}")
}

/// `kind` is `env` or `route`, `name` its key; never a value.
fn unset(kind: &str, name: &str, values: &Values, source: &Source) -> String {
    format!("{kind} {name:?} {}", values.missing(source))
}
