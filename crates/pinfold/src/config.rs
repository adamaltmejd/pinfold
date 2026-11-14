//! Pi configuration: the project's file under pinfold's host configuration
//! directory, the selected profile, and the host environment.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::core::plan::Route;
use crate::core::profile::Profile;
use crate::dirs;
use crate::pi::state::project_id;

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

/// The only mount this step accepts: the project itself, as a disk. Inline,
/// so keys appended to the file stay top-level.
const PROJECT_MOUNT: &str = "mounts = [{ host = \".\", kind = \"disk\" }]\n";

/// `~/.config/pinfold/projects/<id>/`: the project's configuration file
/// and its optional recipe. A moved project has another id, so no file.
pub fn project_dir(root: &Path) -> io::Result<PathBuf> {
    Ok(dirs::config_dir()?.join("projects").join(project_id(root)))
}

/// The bytes `pinfold config edit` writes into a new configuration file.
pub fn template(root: &Path) -> String {
    format!(
        "# pinfold configuration for {}\n{PROJECT_MOUNT}",
        root.display()
    )
}

/// The merged configuration for one project run.
pub struct Config {
    /// Whether the project is a Git checkout to cross.
    pub repository: bool,
    /// The project's configuration file.
    pub path: PathBuf,
    pub profile: Profile,
    pub image: Option<String>,
    /// The project's `Containerfile` beside its configuration file: its path
    /// and its bytes, read once at load time so a build uses what was read.
    pub containerfile: Option<(PathBuf, Vec<u8>)>,
    pub allow: Vec<String>,
    pub routes: BTreeMap<String, Route>,
    pub cpus: f64,
    pub memory: String,
    /// The `<NAME>`s of the host's `PINFOLD_ENV_<NAME>` variables. Their
    /// values stay on the host; a box spec passes each by name only.
    pub env: BTreeSet<String>,
}

impl Config {
    pub fn image_source(&self) -> &'static str {
        if self.image.is_some() {
            "explicit"
        } else if self.containerfile.is_some() {
            "project"
        } else {
            "profile"
        }
    }

    pub fn load(root: &Path) -> io::Result<Config> {
        let repository = match fs::symlink_metadata(root.join(".git")) {
            Ok(metadata) => metadata.is_dir(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        let dir = project_dir(root)?;
        let path = dir.join("pinfold.toml");
        let source = path.display().to_string();
        let Some(bytes) = read_optional(&path)? else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no-config: {} has no configuration; run `pinfold config edit`",
                    root.display()
                ),
            ));
        };
        let project = Layer::parse(&bytes, &source)?;
        match project.mounts.as_deref() {
            Some([mount]) if mount.host == "." && mount.kind == "disk" => {}
            _ => {
                return Err(invalid(
                    &source,
                    &format!(
                        "spec: `mounts` must be exactly {}",
                        PROJECT_MOUNT.trim_end()
                    ),
                ));
            }
        }
        let top = project.over(Layer::from_env()?);
        let profile_name = top.profile.as_deref().unwrap_or(DEFAULT_PROFILE);
        let profile = Profile::load(profile_name)?;
        let profile_source = format!("profile {profile_name:?} pinfold.toml");
        let profile_layer = Layer::parse(&profile.config, &profile_source)?;
        if profile_layer.mounts.is_some() {
            return Err(invalid(&profile_source, "`mounts` is a project key"));
        }
        let containerfile = read_optional(&dir.join("Containerfile"))?
            .map(|bytes| (dir.join("Containerfile"), bytes));
        let merged = profile_layer.over(top);
        if let Some(image) = &merged.image {
            if image.is_empty() {
                return Err(invalid(
                    "configuration",
                    "image-invalid: image must not be empty",
                ));
            }
            if containerfile.is_some() {
                return Err(invalid(
                    "configuration",
                    "image-source-conflict: image and project Containerfile cannot be combined",
                ));
            }
        } else if containerfile.is_none() && profile.containerfile.is_none() {
            return Err(invalid(
                "configuration",
                "image-source-missing: select an image or supply a Containerfile",
            ));
        }
        Ok(Config {
            repository,
            path,
            profile,
            image: merged.image,
            containerfile,
            allow: merged
                .allow
                .unwrap_or_else(|| DEFAULT_ALLOW.iter().map(|host| host.to_string()).collect()),
            routes: merged.routes.unwrap_or_default(),
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

    /// Refuse a project mount that holds pinfold's own configuration, state
    /// or cache, or the selected profile, before any state is created: box
    /// content crosses into the project, so it must not reach them.
    pub fn check_mounts(&self, root: &Path) -> io::Result<()> {
        if !self.repository {
            return Ok(());
        }
        let root = fs::canonicalize(root)?;
        let config_dir = dirs::config_dir()?;
        let mut paths = vec![
            config_dir.join("profiles").join(&self.profile.name),
            config_dir,
            dirs::state_dir()?,
            dirs::artifacts_dir()?,
            dirs::cache_dir()?,
            crate::core::ownership::host_dir()?,
        ];
        if let Some(share) = &self.profile.share {
            paths.push(share.path()?);
        }
        // The project's own directory and files, each of which may be a
        // symlink the config root's resolution does not follow.
        paths.push(self.path.clone());
        paths.extend(self.path.parent().map(Path::to_path_buf));
        paths.extend(self.containerfile.as_ref().map(|(path, _)| path.clone()));
        for path in paths {
            if dirs::resolve_host_path(&path)?.starts_with(&root) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "host-path-in-project: {} resolves inside {}",
                        path.display(),
                        root.display()
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// A file's bytes, `None` when it is absent. A dangling symlink or an
/// unreadable file is an error, never an absence.
fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Ok(_) => Err(invalid(&path.display().to_string(), "file is not readable")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        },
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("read {}: {error}", path.display()),
        )),
    }
}

/// One configuration layer. Every key is optional; an absent key does not
/// override a lower layer.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layer {
    /// Only the project's file and `PINFOLD_PROFILE` select the profile; the
    /// profile's own file is read after the selection, so its value is
    /// ignored.
    profile: Option<String>,
    image: Option<String>,
    mounts: Option<Vec<Mount>>,
    allow: Option<Vec<String>>,
    routes: Option<BTreeMap<String, Route>>,
    cpus: Option<f64>,
    memory: Option<String>,
}

/// One `[[mounts]]` entry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mount {
    host: String,
    kind: String,
}

impl Layer {
    /// Parse one layer. `source` names the file in errors.
    fn parse(bytes: &[u8], source: &str) -> io::Result<Layer> {
        toml::from_slice(bytes).map_err(|error| invalid(source, &error.to_string()))
    }

    /// The environment layer. An absent variable is not a layer; a present
    /// variable, even empty, sets its key.
    fn from_env() -> io::Result<Layer> {
        Ok(Layer {
            profile: override_value("PINFOLD_PROFILE")?,
            image: None,
            mounts: None,
            allow: override_value("PINFOLD_ALLOW")?.as_deref().map(split_list),
            routes: override_value("PINFOLD_ROUTES")?
                .as_deref()
                .map(parse_routes)
                .transpose()?,
            cpus: override_value("PINFOLD_CPUS")?
                .map(|value| {
                    value
                        .parse::<f64>()
                        .map_err(|error| invalid("PINFOLD_CPUS", &error.to_string()))
                })
                .transpose()?,
            memory: override_value("PINFOLD_MEMORY")?,
        })
    }

    /// `self` with `higher` applied over it.
    fn over(mut self, higher: Layer) -> Layer {
        self.profile = higher.profile.or(self.profile);
        self.image = higher.image.or(self.image);
        self.mounts = higher.mounts.or(self.mounts);
        self.cpus = higher.cpus.or(self.cpus);
        self.memory = higher.memory.or(self.memory);
        self.allow = higher.allow.or(self.allow);
        self.routes = higher.routes.or(self.routes);
        self
    }
}

fn override_value(name: &str) -> io::Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(invalid(name, "value is not valid UTF-8")),
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
