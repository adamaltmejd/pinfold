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
use std::path::Path;

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
    /// The raw bytes of the project's `.pinfold.toml` that were parsed,
    /// `None` when the file is absent. Trust hashes these bytes.
    pub project_toml: Option<Vec<u8>>,
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
    /// The layer each effective value came from, for `doctor`.
    pub origins: Origins,
}

/// The layer an effective configuration value came from, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The built-in defaults.
    Default,
    /// The selected profile's `pinfold.toml`.
    Profile,
    /// The project's `.pinfold.toml`.
    Project,
    /// The host environment.
    Environment,
}

impl Origin {
    /// The layer's name, for `doctor`.
    pub fn name(self) -> &'static str {
        match self {
            Origin::Default => "built-in default",
            Origin::Profile => "profile",
            Origin::Project => ".pinfold.toml",
            Origin::Environment => "environment",
        }
    }
}

/// The layer each effective configuration value came from. `allow`, `routes`
/// and `protect` carry one entry per effective item.
pub struct Origins {
    /// The selected profile.
    pub profile: Origin,
    /// The effective `image` value.
    pub image: Origin,
    /// The effective `cpus` value.
    pub cpus: Origin,
    /// The effective `memory` value.
    pub memory: Origin,
    /// Every effective allow entry, in effective order, with its origin.
    pub allow: Vec<(String, Origin)>,
    /// Every effective route, sorted by name, with its origin.
    pub routes: Vec<(String, Origin)>,
    /// Every effective protect entry, in effective order, with its origin.
    pub protect: Vec<(String, Origin)>,
}

/// The Containerfile whose bytes decide the effective image.
pub enum Containerfile {
    /// The selected profile's Containerfile, held in memory.
    Profile(Vec<u8>),
    /// The project's Containerfile, read once at load time. Trust hashes
    /// these bytes and the build uses them, so a live box cannot swap the
    /// file in between.
    Project(Vec<u8>),
}

impl Config {
    /// Load and merge the layers for the project rooted at `root`.
    pub fn load(root: &Path) -> io::Result<Config> {
        let (project, project_toml) = Layer::read(&root.join(".pinfold.toml"))?;
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
        // Provenance is read before `over` consumes the layers.
        let origins = Origins {
            profile: scalar_origin(
                environment.profile.is_some(),
                project.profile.is_some(),
                false,
            ),
            image: scalar_origin(
                environment.image.is_some(),
                project.image.is_some(),
                profile_layer.image.is_some(),
            ),
            cpus: scalar_origin(
                environment.cpus.is_some(),
                project.cpus.is_some(),
                profile_layer.cpus.is_some(),
            ),
            memory: scalar_origin(
                environment.memory.is_some(),
                project.memory.is_some(),
                profile_layer.memory.is_some(),
            ),
            allow: union_origins([
                (&profile_layer.allow, Origin::Profile),
                (&project.allow, Origin::Project),
                (&environment.allow, Origin::Environment),
            ]),
            protect: union_origins([
                (&profile_layer.protect, Origin::Profile),
                (&project.protect, Origin::Project),
                (&environment.protect, Origin::Environment),
            ]),
            routes: route_origins(&profile_layer.routes, &project.routes, &environment.routes),
        };
        let merged = profile_layer.over(project).over(environment);
        let image = merged.image.clone();
        // The config cannot tell a bare image name from a bare file name, so
        // a relative path naming an existing project file is the
        // Containerfile; anything else is an image ref. The bytes are read
        // here so trust and the build see the same ones.
        let containerfile = match image.as_deref() {
            Some(image) if Path::new(image).is_relative() && root.join(image).is_file() => {
                let path = root.join(image);
                let bytes = fs::read(&path).map_err(|error| {
                    io::Error::new(error.kind(), format!("read {}: {error}", path.display()))
                })?;
                Containerfile::Project(bytes)
            }
            _ => Containerfile::Profile(profile.containerfile.clone()),
        };
        Ok(Config {
            profile,
            image,
            containerfile,
            project_toml,
            allow: merged.allow,
            routes: merged.routes,
            protect: merged.protect,
            cpus: merged.cpus.map(Cpus::value).unwrap_or(DEFAULT_CPUS),
            memory: merged.memory.unwrap_or_else(|| DEFAULT_MEMORY.to_string()),
            env: env_names(),
            origins,
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
    /// Read a `.pinfold.toml`; an absent file is an empty layer and `None`.
    /// The bytes are returned so trust hashes exactly what was parsed.
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

/// The origin of a scalar: the highest layer that set it, else the default.
fn scalar_origin(environment: bool, project: bool, profile: bool) -> Origin {
    if environment {
        Origin::Environment
    } else if project {
        Origin::Project
    } else if profile {
        Origin::Profile
    } else {
        Origin::Default
    }
}

/// The origin of each union entry: the lowest layer that added it wins,
/// matching [`union`]'s order.
fn union_origins(layers: [(&[String], Origin); 3]) -> Vec<(String, Origin)> {
    let mut entries: Vec<(String, Origin)> = Vec::new();
    for (items, origin) in layers {
        for item in items {
            if !entries.iter().any(|(existing, _)| existing == item) {
                entries.push((item.clone(), origin));
            }
        }
    }
    entries
}

/// The origin of each route: the highest layer that set it wins, as
/// [`Layer::over`] does.
fn route_origins(
    profile: &BTreeMap<String, String>,
    project: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
) -> Vec<(String, Origin)> {
    let mut routes: BTreeMap<String, Origin> = BTreeMap::new();
    for name in profile.keys() {
        routes.insert(name.clone(), Origin::Profile);
    }
    for name in project.keys() {
        routes.insert(name.clone(), Origin::Project);
    }
    for name in environment.keys() {
        routes.insert(name.clone(), Origin::Environment);
    }
    routes.into_iter().collect()
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
