//! The pi layer's configuration: the project's `.pinfold.toml`, the selected
//! profile's `pinfold.toml`, and the host environment, merged.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

use crate::core::plan::Route;
use crate::core::profile::Profile;

/// The built-in defaults (ARCHITECTURE.md, Configuration).
const DEFAULT_PROFILE: &str = "default";
const DEFAULT_CPUS: f64 = 4.0;
const DEFAULT_MEMORY: &str = "8G";
const DEFAULT_ALLOW: [&str; 9] = [
    "api.anthropic.com",
    "platform.claude.com",
    "api.openai.com",
    "auth.openai.com",
    "chatgpt.com",
    "openrouter.ai",
    "opencode.ai",
    "registry.npmjs.org",
    "pi.dev",
];

/// The merged configuration for one project run.
pub struct Config {
    pub profile: Profile,
    /// The project's Containerfile: its path from `.pinfold.toml`, relative
    /// to the project root, and its bytes, read once at load time. `None`
    /// means the profile's image. Trust hashes these bytes and the build
    /// uses them, so a live box cannot swap the file in between.
    pub containerfile: Option<(String, Vec<u8>)>,
    /// The raw bytes of the project's `.pinfold.toml` that were parsed,
    /// `None` when the file is absent. Trust hashes these bytes.
    pub project_toml: Option<Vec<u8>>,
    pub allow: Vec<String>,
    pub routes: BTreeMap<String, Route>,
    pub protect: Vec<String>,
    pub cpus: f64,
    pub memory: String,
    /// The `<NAME>`s of the host's `PINFOLD_ENV_<NAME>` variables. Their
    /// values stay on the host; a box spec passes each by name only.
    pub env: BTreeSet<String>,
}

impl Config {
    /// Load and merge the layers for the project rooted at `root`.
    pub fn load(root: &Path) -> io::Result<Config> {
        let (project, project_toml) = Layer::read(&root.join(".pinfold.toml"))?;
        let top = project.over(Layer::from_env()?);
        let profile_name = top.profile.as_deref().unwrap_or(DEFAULT_PROFILE);
        let profile = Profile::load(profile_name)?;
        let source = format!("profile {profile_name:?} pinfold.toml");
        let profile_layer = Layer::parse(&profile.config, &source)?;
        if profile_layer.containerfile.is_some() {
            return Err(invalid(
                &source,
                "`containerfile` is a project key; a profile's image is its own Containerfile",
            ));
        }
        let merged = profile_layer.over(top);
        let containerfile = match merged.containerfile {
            None => None,
            Some(path) => {
                if Path::new(&path).is_absolute() {
                    return Err(invalid(
                        ".pinfold.toml",
                        &format!(
                            "`containerfile` must be a path relative to the project root: {path:?}"
                        ),
                    ));
                }
                let joined = root.join(&path);
                let read = |error: io::Error| {
                    io::Error::new(error.kind(), format!("read {}: {error}", joined.display()))
                };
                // Resolve symlinks, so a path that stays under the root but
                // leaves the project is refused too.
                let canonical = fs::canonicalize(&joined).map_err(read)?;
                if !canonical.starts_with(root) {
                    return Err(invalid(
                        ".pinfold.toml",
                        &format!("`containerfile` must name a file inside the project: {path:?}"),
                    ));
                }
                // Read the canonical path, the same one the check saw.
                let bytes = fs::read(&canonical).map_err(read)?;
                Some((path, bytes))
            }
        };
        Ok(Config {
            profile,
            containerfile,
            project_toml,
            allow: merged
                .allow
                .unwrap_or_else(|| DEFAULT_ALLOW.iter().map(|host| host.to_string()).collect()),
            routes: merged.routes.unwrap_or_default(),
            protect: merged.protect.unwrap_or_default(),
            cpus: merged.cpus.unwrap_or(DEFAULT_CPUS),
            memory: merged.memory.unwrap_or_else(|| DEFAULT_MEMORY.to_string()),
            env: env::vars_os()
                .filter_map(|(name, _)| {
                    name.to_str()?
                        .strip_prefix("PINFOLD_ENV_")
                        .map(str::to_string)
                })
                .collect(),
        })
    }
}

/// One configuration layer. Every key is optional; an absent key does not
/// override a lower layer.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layer {
    /// Only `.pinfold.toml` and `PINFOLD_PROFILE` select the profile; the
    /// profile's own file is read after the selection, so its value is
    /// ignored.
    profile: Option<String>,
    containerfile: Option<String>,
    allow: Option<Vec<String>>,
    routes: Option<BTreeMap<String, Route>>,
    protect: Option<Vec<String>>,
    cpus: Option<f64>,
    memory: Option<String>,
}

impl Layer {
    /// Read a `.pinfold.toml`; an absent file is an empty layer and `None`.
    fn read(path: &Path) -> io::Result<(Layer, Option<Vec<u8>>)> {
        match fs::read(path) {
            Ok(bytes) => {
                let layer = Layer::parse(&bytes, &path.display().to_string())?;
                Ok((layer, Some(bytes)))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((Layer::default(), None)),
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!("read {}: {error}", path.display()),
            )),
        }
    }

    /// Parse one layer. `source` names the file in errors.
    fn parse(bytes: &[u8], source: &str) -> io::Result<Layer> {
        toml::from_slice(bytes).map_err(|error| invalid(source, &error.to_string()))
    }

    /// The environment layer. An absent variable is not a layer; a present
    /// variable, even empty, sets its key.
    fn from_env() -> io::Result<Layer> {
        Ok(Layer {
            profile: env::var("PINFOLD_PROFILE").ok(),
            containerfile: None,
            allow: env::var("PINFOLD_ALLOW").ok().as_deref().map(split_list),
            routes: env::var("PINFOLD_ROUTES")
                .ok()
                .as_deref()
                .map(parse_routes)
                .transpose()?,
            protect: env::var("PINFOLD_PROTECT").ok().as_deref().map(split_list),
            cpus: env::var("PINFOLD_CPUS")
                .ok()
                .map(|value| {
                    value
                        .parse::<f64>()
                        .map_err(|error| invalid("PINFOLD_CPUS", &format!("{value:?}: {error}")))
                })
                .transpose()?,
            memory: env::var("PINFOLD_MEMORY").ok(),
        })
    }

    /// `self` with `higher` applied over it.
    fn over(mut self, higher: Layer) -> Layer {
        self.profile = higher.profile.or(self.profile);
        self.containerfile = higher.containerfile.or(self.containerfile);
        self.cpus = higher.cpus.or(self.cpus);
        self.memory = higher.memory.or(self.memory);
        self.allow = higher.allow.or(self.allow);
        self.routes = higher.routes.or(self.routes);
        self.protect = higher.protect.or(self.protect);
        self
    }
}

/// A comma-separated list, trimmed; blank entries are ignored.
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// `name=host:port` pairs, comma-separated.
fn parse_routes(value: &str) -> io::Result<BTreeMap<String, Route>> {
    let mut routes = BTreeMap::new();
    for entry in split_list(value) {
        let Some((name, target)) = entry
            .split_once('=')
            .filter(|(name, target)| !name.is_empty() && !target.is_empty())
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("PINFOLD_ROUTES entry {entry:?} is not name=host:port"),
            ));
        };
        routes.insert(name.to_string(), Route::Address(target.to_string()));
    }
    Ok(routes)
}

fn invalid(source: &str, message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{source}: {message}"))
}
