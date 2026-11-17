//! Pi configuration: the project's file under pinfold's host configuration
//! directory, the selected profile, and the host environment.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::plan::Route;
use crate::core::profile::Profile;
use crate::dirs;
use crate::formatting::terminal_text;
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

/// The project itself is the only configurable mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectMount {
    Disk,
    Share,
}

impl ProjectMount {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disk => "disk",
            Self::Share => "share",
        }
    }

    fn default_kind() -> Self {
        if cfg!(target_os = "macos") {
            Self::Share
        } else {
            Self::Disk
        }
    }

    fn declaration(self) -> String {
        format!(
            "mounts = [{{ host = \".\", kind = \"{}\" }}]\n",
            self.as_str()
        )
    }

    fn parse(value: Option<&toml::Value>, source: &str) -> io::Result<Self> {
        for kind in [Self::Disk, Self::Share] {
            let expected = toml::from_str::<toml::Table>(&kind.declaration())
                .expect("the project mount is TOML")
                .remove("mounts");
            if value == expected.as_ref() {
                if kind == Self::Share && cfg!(target_os = "linux") {
                    return Err(invalid(
                        source,
                        "spec: share mounts are unsupported on Linux",
                    ));
                }
                return Ok(kind);
            }
        }
        Err(invalid(
            source,
            "spec: mounts must be exactly [{ host = \".\", kind = \"share\" }] or [{ host = \".\", kind = \"disk\" }]",
        ))
    }
}

/// `~/.config/pinfold/projects/<id>/`: the project's configuration file
/// and its optional recipe. A moved project has another id, so no file.
pub fn project_dir(root: &Path) -> io::Result<PathBuf> {
    Ok(dirs::config_dir()?.join("projects").join(project_id(root)))
}

/// `pinfold config edit`'s first step: the project's configuration file,
/// written with the project mount when it is missing. The project is
/// checked first, with the profile a run would select, so nothing is
/// created inside it.
pub fn create(root: &Path) -> io::Result<PathBuf> {
    let dir = project_dir(root)?;
    let path = dir.join("pinfold.toml");
    // A file that cannot be read or parsed selects no profile, and a
    // profile that cannot be loaded has no paths to check: the editor
    // must still open to fix either.
    let file = read_optional(&path)
        .ok()
        .flatten()
        .and_then(|bytes| Layer::parse(&bytes, "").ok())
        .and_then(|layer| layer.profile);
    let profile = Profile::load(&profile_name(file)?).ok();
    check_paths_outside(root, profile.as_ref())?;
    if read_optional(&path)?.is_some() {
        return Ok(path);
    }
    fs::create_dir_all(&dir)?;
    // Linking a complete file into place creates it in one step, and never
    // replaces one a concurrent edit created. `create_new` refuses a
    // planted staging symlink.
    let staging = dir.join(format!(".pinfold.toml.tmp-{}", std::process::id()));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?
        .write_all(ProjectMount::default_kind().declaration().as_bytes())?;
    let linked = fs::hard_link(&staging, &path);
    // A staging file left behind is harmless; the result is the link's.
    let _ = fs::remove_file(&staging);
    match linked {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => Err(error),
        _ => Ok(path),
    }
}

/// Discovery needs only the existing mount selection, before any state exists.
pub(crate) fn project_mount(root: &Path) -> io::Result<Option<ProjectMount>> {
    let path = project_dir(root)?.join("pinfold.toml");
    let Some(bytes) = read_optional(&path)? else {
        return Ok(None);
    };
    let source = terminal_text(&path.to_string_lossy());
    let layer = Layer::parse(&bytes, &source)?;
    ProjectMount::parse(layer.mounts.as_ref(), &source).map(Some)
}

/// The merged configuration for one project run.
pub struct Config {
    pub mount: ProjectMount,
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
    pub(crate) route_declarations: BTreeMap<String, toml::Value>,
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
        let source = terminal_text(&path.to_string_lossy());
        let Some(bytes) = read_optional(&path)? else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no-config: {} has no configuration; run `pinfold config edit`",
                    terminal_text(&root.to_string_lossy())
                ),
            ));
        };
        let mut project = Layer::parse(&bytes, &source)?;
        let mount = ProjectMount::parse(project.mounts.as_ref(), &source)?;
        let profile_name = profile_name(project.profile.take())?;
        let profile = Profile::load(&profile_name)?;
        let top = project.over(Layer::from_env()?);
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
            mount,
            repository,
            path,
            profile,
            image: merged.image,
            containerfile,
            allow: merged
                .allow
                .unwrap_or_else(|| DEFAULT_ALLOW.iter().map(|host| host.to_string()).collect()),
            routes: merged.routes.unwrap_or_default(),
            route_declarations: merged.route_declarations.unwrap_or_default(),
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

    /// Refuse a project that holds what configures its boxes (see
    /// [`check_paths_outside`]).
    pub fn check_host_paths(&self, root: &Path) -> io::Result<()> {
        check_paths_outside(root, Some(&self.profile))
    }

    /// Core share admission checks common paths; add only project-specific paths.
    pub(crate) fn check_project_paths(&self, root: &Path) -> io::Result<()> {
        if self.mount == ProjectMount::Disk {
            self.check_host_paths(root)
        } else {
            let project = project_dir(root)?;
            let paths = [
                project.join("pinfold.toml"),
                project.join("Containerfile"),
                project,
            ]
            .into_iter()
            .map(|path| dirs::resolve_host_path(&path))
            .collect::<io::Result<Vec<_>>>()?;
            crate::core::mounts::check_extra_paths_outside(root, paths, true)
        }
    }
}

/// Refuse a project root that holds pinfold's own configuration, state or
/// cache, the project's configuration, or what `profile` reads, before any
/// project state is created: box content crosses into the project, so it
/// must not reach them. Each file is named too, since resolving its
/// directory does not follow a symlink below it.
fn check_paths_outside(root: &Path, profile: Option<&Profile>) -> io::Result<()> {
    let project = project_dir(&fs::canonicalize(root)?)?;
    crate::core::mounts::check_paths_outside(
        root,
        profile,
        [
            project.join("pinfold.toml"),
            project.join("Containerfile"),
            project,
        ],
    )
}

/// A file's bytes, `None` when it is absent. A dangling symlink or an
/// unreadable file is an error, never an absence.
fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Ok(_) => Err(invalid(
                &terminal_text(&path.to_string_lossy()),
                "file is not readable",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        },
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("read {}: {error}", terminal_text(&path.to_string_lossy())),
        )),
    }
}

/// One configuration layer. Every key is optional; an absent key does not
/// override a lower layer.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layer {
    /// Read from the project's file only, by [`profile_name`]; a profile's
    /// own value is ignored.
    profile: Option<String>,
    image: Option<String>,
    /// Checked whole against the project mount, so any other value is
    /// refused as `spec`.
    mounts: Option<toml::Value>,
    allow: Option<Vec<String>>,
    routes: Option<BTreeMap<String, Route>>,
    #[serde(skip)]
    route_declarations: Option<BTreeMap<String, toml::Value>>,
    cpus: Option<f64>,
    memory: Option<String>,
}

impl Layer {
    /// Parse one layer. `source` names the file in errors.
    fn parse(bytes: &[u8], source: &str) -> io::Result<Layer> {
        let mut layer: Layer =
            toml::from_slice(bytes).map_err(|error| invalid(source, &error.to_string()))?;
        let input: toml::Table =
            toml::from_slice(bytes).map_err(|error| invalid(source, &error.to_string()))?;
        layer.route_declarations =
            input
                .get("routes")
                .and_then(toml::Value::as_table)
                .map(|routes| {
                    routes
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect()
                });
        Ok(layer)
    }

    /// The environment layer. An absent variable is not a layer; a present
    /// variable, even empty, sets its key.
    fn from_env() -> io::Result<Layer> {
        let routes = override_value("PINFOLD_ROUTES")?
            .as_deref()
            .map(parse_routes)
            .transpose()?;
        let route_declarations = routes.as_ref().map(|routes| {
            routes
                .iter()
                .map(|(name, route)| {
                    let Route::Address(address) = route else {
                        unreachable!("environment routes are addresses")
                    };
                    (name.clone(), toml::Value::String(address.clone()))
                })
                .collect()
        });
        Ok(Layer {
            profile: None,
            image: None,
            mounts: None,
            allow: override_value("PINFOLD_ALLOW")?.as_deref().map(split_list),
            routes,
            route_declarations,
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
        self.image = higher.image.or(self.image);
        self.cpus = higher.cpus.or(self.cpus);
        self.memory = higher.memory.or(self.memory);
        self.allow = higher.allow.or(self.allow);
        self.routes = higher.routes.or(self.routes);
        self.route_declarations = higher.route_declarations.or(self.route_declarations);
        self
    }
}

/// The selected profile: `PINFOLD_PROFILE`, else the project file's
/// `profile`, else the default.
fn profile_name(file: Option<String>) -> io::Result<String> {
    Ok(override_value("PINFOLD_PROFILE")?
        .or(file)
        .unwrap_or_else(|| DEFAULT_PROFILE.to_string()))
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
