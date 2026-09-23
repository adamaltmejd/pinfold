//! The pi layer's configuration: the project's `.pinfold.toml`, the selected
//! profile's `pinfold.toml`, and the host environment, merged.
//!
//! Layers, highest first: environment, `.pinfold.toml`, the profile's
//! `pinfold.toml`, built-in defaults. Scalars take the highest layer;
//! `allow`, `routes` and `protect` are unions, so no layer removes what
//! another adds.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::core::profile::Profile;

/// The built-in defaults (ARCHITECTURE.md, Configuration).
const DEFAULT_PROFILE: &str = "default";
const DEFAULT_CPUS: f64 = 4.0;
const DEFAULT_MEMORY: &str = "8G";

/// The merged configuration for one project run.
pub struct Config {
    /// The selected profile, loaded.
    pub profile: Profile,
    /// The merged `image` value. It is an image ref unless `containerfile`
    /// is `Project`, in which case it is that Containerfile's
    /// project-relative path. `None` means the profile's image.
    pub image: Option<String>,
    /// The Containerfile whose bytes decide the effective image: the
    /// project's when `image` names one, else the profile's.
    pub containerfile: Containerfile,
    /// The effective allowlist: the union of every layer.
    pub allow: Vec<String>,
    /// The effective routes: the union of every layer. A higher layer wins
    /// a name it shares with a lower one.
    pub routes: BTreeMap<String, String>,
    /// The effective extra read-only directories: the union of every layer.
    pub protect: Vec<String>,
    /// The box's vCPUs.
    pub cpus: f64,
    /// The box's memory limit, e.g. `8G`.
    pub memory: String,
    /// The `<NAME>`s of the host's `PINFOLD_ENV_<NAME>` variables. Their
    /// values stay on the host; a box spec passes each by name only.
    pub env: BTreeSet<String>,
}

/// The Containerfile whose bytes decide the effective image.
pub enum Containerfile {
    /// The selected profile's Containerfile, held in memory.
    Profile(Vec<u8>),
    /// A Containerfile path relative to the project root.
    Project(PathBuf),
}

impl Config {
    /// Load and merge the layers for the project rooted at `root`.
    pub fn load(root: &Path) -> io::Result<Config> {
        let project = Layer::read(&root.join(".pinfold.toml"))?;
        let environment = Layer::from_env()?;
        let profile_name = environment
            .profile
            .clone()
            .or_else(|| project.profile.clone())
            .unwrap_or_else(|| DEFAULT_PROFILE.to_string());
        let profile = Profile::load(&profile_name)?;
        let profile_layer = Layer::parse(
            &profile.config,
            &format!("profile {profile_name:?} pinfold.toml"),
        )?;
        let merged = profile_layer.over(project).over(environment);
        let image = merged.image.clone();
        // The config cannot tell a bare image name from a bare file name, so
        // a value that names an existing project file is the Containerfile;
        // anything else is an image ref. Y-9 builds a project Containerfile.
        let containerfile = match image.as_deref() {
            Some(image) if Path::new(image).is_relative() && root.join(image).is_file() => {
                Containerfile::Project(PathBuf::from(image))
            }
            _ => Containerfile::Profile(profile.containerfile.clone()),
        };
        Ok(Config {
            profile,
            image,
            containerfile,
            allow: merged.allow,
            routes: merged.routes,
            protect: merged.protect,
            cpus: merged.cpus.map(Cpus::value).unwrap_or(DEFAULT_CPUS),
            memory: merged.memory.unwrap_or_else(|| DEFAULT_MEMORY.to_string()),
            env: env_names(),
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
    image: Option<String>,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    routes: BTreeMap<String, String>,
    #[serde(default)]
    protect: Vec<String>,
    cpus: Option<Cpus>,
    memory: Option<String>,
}

impl Layer {
    /// Read a `.pinfold.toml`; an absent file is an empty layer.
    fn read(path: &Path) -> io::Result<Layer> {
        match fs::read(path) {
            Ok(bytes) => Layer::parse(&bytes, &path.display().to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Layer::default()),
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!("read {}: {error}", path.display()),
            )),
        }
    }

    /// Parse one layer. `source` names the file in errors.
    fn parse(bytes: &[u8], source: &str) -> io::Result<Layer> {
        let text =
            std::str::from_utf8(bytes).map_err(|error| invalid(source, &error.to_string()))?;
        toml::from_str(text).map_err(|error| invalid(source, &error.to_string()))
    }

    /// The environment layer. An absent variable is not a layer.
    fn from_env() -> io::Result<Layer> {
        Ok(Layer {
            profile: var("PINFOLD_PROFILE"),
            image: var("PINFOLD_IMAGE"),
            allow: var("PINFOLD_ALLOW")
                .map(|value| split_list(&value))
                .unwrap_or_default(),
            routes: match var("PINFOLD_ROUTES") {
                Some(value) => parse_routes(&value)?,
                None => BTreeMap::new(),
            },
            protect: var("PINFOLD_PROTECT")
                .map(|value| split_list(&value))
                .unwrap_or_default(),
            cpus: match var("PINFOLD_CPUS") {
                Some(value) => Some(Cpus::Float(value.parse::<f64>().map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("PINFOLD_CPUS={value:?}: {error}"),
                    )
                })?)),
                None => None,
            },
            memory: var("PINFOLD_MEMORY"),
        })
    }

    /// `self` with `higher` applied over it.
    fn over(mut self, higher: Layer) -> Layer {
        self.profile = higher.profile.or(self.profile);
        self.image = higher.image.or(self.image);
        self.cpus = higher.cpus.or(self.cpus);
        self.memory = higher.memory.or(self.memory);
        self.allow = union(self.allow, higher.allow);
        self.routes.extend(higher.routes);
        self.protect = union(self.protect, higher.protect);
        self
    }
}

/// A TOML number: `cpus = 4` and `cpus = 4.5` are both valid.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
enum Cpus {
    Integer(i64),
    Float(f64),
}

impl Cpus {
    fn value(self) -> f64 {
        match self {
            Cpus::Integer(value) => value as f64,
            Cpus::Float(value) => value,
        }
    }
}

/// The value of `name`, or `None` when it is absent.
fn var(name: &str) -> Option<String> {
    env::var(name).ok()
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
fn parse_routes(value: &str) -> io::Result<BTreeMap<String, String>> {
    let mut routes = BTreeMap::new();
    for entry in split_list(value) {
        let Some((name, target)) = entry.split_once('=') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("PINFOLD_ROUTES entry {entry:?} is not name=host:port"),
            ));
        };
        if name.is_empty() || target.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("PINFOLD_ROUTES entry {entry:?} is not name=host:port"),
            ));
        }
        routes.insert(name.to_string(), target.to_string());
    }
    Ok(routes)
}

/// `lower` plus the items of `higher` it does not already hold.
fn union(mut lower: Vec<String>, higher: Vec<String>) -> Vec<String> {
    for item in higher {
        if !lower.contains(&item) {
            lower.push(item);
        }
    }
    lower
}

/// The `<NAME>`s of the host's `PINFOLD_ENV_<NAME>` variables, sorted.
fn env_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (name, _) in env::vars_os() {
        if let Some(name) = name.to_str()
            && let Some(name) = name.strip_prefix("PINFOLD_ENV_")
            && !name.is_empty()
        {
            names.insert(name.to_string());
        }
    }
    names
}

fn invalid(source: &str, message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{source}: {message}"))
}
