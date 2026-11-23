//! Pi configuration: the project's file under pinfold's host configuration
//! directory, the selected profile, and the host environment.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::plan::{Env, Mount, MountKind, Route, guest_path};
use crate::core::profile::Profile;
use crate::dirs;
use crate::formatting::terminal_text;
use crate::pi::state::project_id;

/// The built-in defaults (ARCHITECTURE.md, Configuration).
pub(crate) const DEFAULT_PROFILE: &str = "default";
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

/// The project mount's kind. Further `mounts` entries are read-only
/// folders ([`Config::folders`]).
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

    pub(crate) fn default_kind() -> Self {
        if cfg!(target_os = "macos") {
            Self::Share
        } else {
            Self::Disk
        }
    }

    /// A new project file: from first-run setup, the chosen profile, then
    /// this mount, as active lines, then each other key a project file
    /// accepts as a commented example.
    fn template(self, profile: Option<&str>) -> String {
        let kinds = if cfg!(target_os = "macos") {
            "\"share\" lets the box edit this folder directly; \"disk\" gives it its own isolated disk"
        } else {
            "\"disk\" gives the box its own isolated disk"
        };
        let profile_help = "# Profile to use (default, documents, adam, or a saved profile):\n";
        // Profile::load accepts only ASCII names safe in a TOML string.
        let (chosen, example) = match profile {
            Some(name) => (
                format!("{profile_help}profile = \"{name}\"\n"),
                String::new(),
            ),
            None => (
                String::new(),
                format!("{profile_help}# profile = \"{DEFAULT_PROFILE}\"\n"),
            ),
        };
        format!(
            "{chosen}mounts = [{{ host = \".\", kind = \"{}\" }}]
# kind: {kinds}.
# Host folders (absolute or ~/, real paths with no links) the box can only read follow the project entry: {{ host = \"~/notes\", guest = \"/notes\" }}

# Optional settings: uncomment a line to set it.
{example}# Local image to run instead of the profile's (not with a Containerfile beside this file):
# image = \"localhost/pinfold/image-my-image:latest\"
# Hosts the box may reach; replaces the default list:
# allow = [\"api.anthropic.com\", \"github.com\"]
# HTTP services on this machine the box may reach as http://NAME:
# routes = {{ api = \"127.0.0.1:8080\" }}
# CPUs and memory for the box:
# cpus = {DEFAULT_CPUS}
# memory = \"{DEFAULT_MEMORY}\"
# Variables in the box: a value, {{ from = \"VAR\" }} for the host's VAR, or {{ op = \"op://VAULT/ITEM/FIELD\" }}:
# env = {{ LANG = \"C.UTF-8\", API_KEY = {{ from = \"MY_API_KEY\" }} }}
# A Containerfile beside this file builds the project's own image.

# Agent logins kept on the host. These tables stay last, take the place of the
# routes line above, and replace the profile's routes. Pi on the host's Codex login:
# [routes.openai]
# login = \"pi\"
# provider = \"openai-codex\"
# Claude on a `claude setup-token` token in the host variable CLAUDE_SETUP_TOKEN:
# [routes.anthropic]
# login = \"claude\"
# from = \"CLAUDE_SETUP_TOKEN\"
",
            self.as_str()
        )
    }

    /// The project mount, then each further entry as a read-only mount
    /// whose host is as written.
    fn parse(value: Option<&toml::Value>, source: &str) -> io::Result<(Self, Vec<Mount>)> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Attachment {
            host: String,
            kind: ProjectMount,
        }
        let refused = || {
            invalid(
                source,
                "spec: mounts must start with [{ host = \".\", kind = \"share\" }] or [{ host = \".\", kind = \"disk\" }], then only read-only folders { host = \"PATH\", guest = \"/PATH\" }",
            )
        };
        let Some([first, rest @ ..]) = value.and_then(toml::Value::as_array).map(Vec::as_slice)
        else {
            return Err(refused());
        };
        let Ok(mount) = first.clone().try_into::<Attachment>() else {
            return Err(refused());
        };
        if mount.host != "." {
            return Err(refused());
        }
        if mount.kind == Self::Share && cfg!(target_os = "linux") {
            return Err(invalid(
                source,
                "spec: share mounts are unsupported on Linux",
            ));
        }
        let mut folders = Vec::new();
        for entry in rest {
            match entry.clone().try_into::<Mount>() {
                Ok(folder) if folder.kind == MountKind::Readonly => folders.push(folder),
                _ => return Err(refused()),
            }
        }
        Ok((mount.kind, folders))
    }
}

/// `~/.config/pinfold/projects/<id>/`: the project's configuration file
/// and its optional recipe. A moved project has another id, so no file.
pub fn project_dir(root: &Path) -> io::Result<PathBuf> {
    Ok(dirs::config_dir()?.join("projects").join(project_id(root)))
}

/// First-run setup's summary: the file it saves and that file's active
/// lines, filtered from the template it writes.
pub(crate) fn setup_summary(
    root: &Path,
    profile: &str,
    mount: ProjectMount,
) -> io::Result<(PathBuf, Vec<String>)> {
    let lines = mount
        .template(Some(profile))
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    Ok((project_dir(root)?.join("pinfold.toml"), lines))
}

/// The project's configuration file, written when it is missing: by
/// `pinfold config edit` with the platform's default mount, or by first-run
/// setup with the chosen profile and mount. The project is checked first,
/// with the profile a run would select, so nothing is created inside it.
pub fn create(root: &Path, setup: Option<(&str, ProjectMount)>) -> io::Result<(PathBuf, bool)> {
    let selected = setup.map(|(name, _)| name);
    let dir = project_dir(root)?;
    let path = dir.join("pinfold.toml");
    // A file that cannot be read or parsed selects no profile, and a
    // profile that cannot be loaded has no paths to check: the editor
    // must still open to fix either.
    let profile = match selected {
        Some(name) => Some(Profile::load(name)?),
        None => {
            let file = read_optional(&path)
                .ok()
                .flatten()
                .and_then(|bytes| Layer::parse(&bytes, "").ok())
                .and_then(|layer| layer.profile);
            Profile::load(&profile_name(file)?).ok()
        }
    };
    check_paths_outside(
        root,
        root,
        profile.as_ref(),
        setup.is_some_and(|(_, mount)| mount == ProjectMount::Share),
    )?;
    if read_optional(&path)?.is_some() {
        return Ok((path, false));
    }
    fs::create_dir_all(&dir)?;
    // Linking a complete file into place creates it in one step, and never
    // replaces one a concurrent edit created. `create_new` refuses a
    // planted staging symlink.
    let staging = dir.join(format!(".pinfold.toml.tmp-{}", std::process::id()));
    let contents = setup
        .map_or(ProjectMount::default_kind(), |(_, mount)| mount)
        .template(selected);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?
        .write_all(contents.as_bytes())?;
    let linked = fs::hard_link(&staging, &path);
    // A staging file left behind is harmless; the result is the link's.
    let _ = fs::remove_file(&staging);
    match linked {
        Ok(()) => Ok((path, true)),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok((path, false)),
        Err(error) => Err(error),
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
    ProjectMount::parse(layer.mounts.as_ref(), &source).map(|(mount, _)| Some(mount))
}

#[derive(Debug)]
pub(crate) struct NoConfig(PathBuf);

impl std::fmt::Display for NoConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no-config: {} has no configuration. Run `pinfold -- pi` or `pinfold -- sh` in a terminal to set it up",
            terminal_text(&self.0.to_string_lossy())
        )
    }
}

impl std::error::Error for NoConfig {}

/// The merged configuration for one project run.
pub struct Config {
    pub mount: ProjectMount,
    /// Read-only host folders after the project mount, each host absolute,
    /// `~/` expanded to the user's canonical home. A box start admits each
    /// host only as its own real path ([`admit_folder`]).
    pub folders: Vec<Mount>,
    /// Whether the project is a Git checkout to cross.
    pub repository: bool,
    /// The project's configuration file.
    pub path: PathBuf,
    pub project_profile: Option<String>,
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
    /// The box's variables: the configured `env` tables, then each host
    /// `PINFOLD_ENV_<NAME>` as `<NAME>`. A `from` value stays on the host;
    /// a box spec passes it by name only.
    pub env: BTreeMap<String, Env>,
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
        let dir = project_dir(root)?;
        let path = dir.join("pinfold.toml");
        let source = terminal_text(&path.to_string_lossy());
        let Some(bytes) = read_optional(&path)? else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                NoConfig(root.to_owned()),
            ));
        };
        let project = Layer::parse(&bytes, &source)?;
        let (mount, mut folders) = ProjectMount::parse(project.mounts.as_ref(), &source)?;
        // A relative host would be walked through the project, where a
        // box can plant a link that chooses the next start's folder.
        for folder in &mut folders {
            let refused = |detail: &str| {
                invalid(
                    &source,
                    &format!(
                        "spec: read-only folder {} {detail}",
                        terminal_text(&folder.host.to_string_lossy())
                    ),
                )
            };
            // TOML strings are UTF-8. Only a canonical spelling can equal
            // the real path a start requires.
            let written = match folder
                .host
                .to_str()
                .and_then(|host| host.strip_prefix("~/"))
            {
                Some(rest) if !Path::new(rest).is_absolute() => Path::new(rest),
                Some(_) => return Err(refused("must not start with ~//")),
                None if folder.host.is_absolute() => folder.host.as_path(),
                None => return Err(refused("must be absolute or start with ~/")),
            };
            let canonical = |path: &Path| guest_path(path).as_os_str() == path.as_os_str();
            if !canonical(written) || written.as_os_str().is_empty() {
                return Err(refused(
                    "needs a host with no `.` or `..`, repeated or trailing slash",
                ));
            }
            if !canonical(&folder.guest) {
                return Err(refused(
                    "needs a guest with no `.` or `..`, repeated or trailing slash",
                ));
            }
            if written != folder.host {
                // HOME canonicalized as the project's startup roots hold it.
                let home = env::home_dir().ok_or_else(|| {
                    io::Error::other("HOME is not set and the uid has no passwd entry")
                })?;
                folder.host = dirs::resolve_host_path(&home)?.join(written);
            }
        }
        let mut config = Self::from_project(root, dir, project, mount)?;
        config.folders = folders;
        Ok(config)
    }

    pub(crate) fn first_run_preview(root: &Path) -> io::Result<Self> {
        Self::from_project(
            root,
            project_dir(root)?,
            Layer::default(),
            ProjectMount::default_kind(),
        )
    }

    fn from_project(
        root: &Path,
        dir: PathBuf,
        mut project: Layer,
        mount: ProjectMount,
    ) -> io::Result<Self> {
        let repository = match fs::symlink_metadata(root.join(".git")) {
            Ok(metadata) => metadata.is_dir(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        let project_profile = project.profile.take();
        let profile_name = profile_name(project_profile.clone())?;
        let profile = Profile::load(&profile_name)?;
        if project.base.is_some() {
            return Err(invalid(
                &terminal_text(&dir.join("pinfold.toml").to_string_lossy()),
                "`base` is a profile key",
            ));
        }
        let top = project.over(Layer::from_env()?);
        // The chain's root first: each profile's keys replace its base's.
        let mut merged = Layer::default();
        for link in profile.links() {
            let source = format!("profile {:?} pinfold.toml", link.name);
            let layer = Layer::parse(&link.config, &source)?;
            if layer.mounts.is_some() {
                return Err(invalid(&source, "`mounts` is a project key"));
            }
            merged = merged.over(layer);
        }
        let containerfile = read_optional(&dir.join("Containerfile"))?
            .map(|bytes| (dir.join("Containerfile"), bytes));
        let merged = merged.over(top);
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
        } else if containerfile.is_none() && profile.recipe().is_none() {
            return Err(invalid(
                "configuration",
                "image-source-missing: select an image or supply a Containerfile",
            ));
        }
        Ok(Config {
            mount,
            folders: Vec::new(),
            repository,
            path: dir.join("pinfold.toml"),
            project_profile,
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
            env: merged
                .env
                .unwrap_or_default()
                .into_iter()
                .chain(env_overrides()?)
                .collect(),
        })
    }

    /// Refuse a project that holds what configures its boxes (see
    /// [`check_paths_outside`]).
    pub fn check_host_paths(&self, root: &Path) -> io::Result<()> {
        let share = self.mount == ProjectMount::Share;
        check_paths_outside(root, root, Some(&self.profile), share)
    }
}

/// At a box start, before anything reads the folder's contents: its host
/// must be its own real path, with no link a box could have planted to
/// choose another folder. That path must be UTF-8, lie outside the project
/// and clear every protected path, since the box could read pinfold's own state, the egress log included.
pub(crate) fn admit_folder(root: &Path, host: &Path, profile: &Profile) -> io::Result<()> {
    let refused = |detail: &str| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "spec: read-only folder {} {detail}",
                terminal_text(&host.to_string_lossy())
            ),
        )
    };
    let real = fs::canonicalize(host).map_err(|error| refused(&error.to_string()))?;
    if real != host {
        return Err(refused(&format!(
            "is not its real path {}; name that path",
            terminal_text(&real.to_string_lossy())
        )));
    }
    // The box layer checks the rest of the path's form, and the directory.
    crate::pi::launch::utf8(host, "read-only folder")?;
    if crate::core::mounts::source_beneath(host, &fs::canonicalize(root)?)? {
        return Err(refused("lies inside the project"));
    }
    check_paths_outside(root, host, Some(profile), true)
}

/// The protected paths no configuration selects, checked before admission
/// opens a lock: a project holding the lock directory would get lock files.
/// The configured check still runs under the locks.
pub(crate) fn check_roots_outside(root: &Path) -> io::Result<()> {
    check_paths_outside(root, root, None, false)
}

/// Refuse `checked`, the root of the project at `root` or one of its
/// read-only folders, when it holds pinfold's own configuration, state or
/// cache, the project's configuration, or what `profile` reads, before any
/// project state is created: box content crosses into the project, so it
/// must not reach them. Each file is named too, since resolving its
/// directory does not follow a symlink below it.
fn check_paths_outside(
    root: &Path,
    checked: &Path,
    profile: Option<&Profile>,
    bidirectional: bool,
) -> io::Result<()> {
    let project = project_dir(&fs::canonicalize(root)?)?;
    crate::core::mounts::check_paths_outside(
        checked,
        profile,
        [
            project.join("pinfold.toml"),
            project.join("Containerfile"),
            project,
        ],
        bidirectional,
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
    /// A profile's file only; core resolves the chain it names.
    base: Option<String>,
    image: Option<String>,
    /// Checked whole against the project mount and read-only folders, so
    /// any other value is refused as `spec`.
    mounts: Option<toml::Value>,
    allow: Option<Vec<String>>,
    routes: Option<BTreeMap<String, Route>>,
    #[serde(skip)]
    route_declarations: Option<BTreeMap<String, toml::Value>>,
    cpus: Option<f64>,
    memory: Option<String>,
    env: Option<BTreeMap<String, Env>>,
}

impl Layer {
    /// Parse one layer. `source` names the file in errors.
    fn parse(bytes: &[u8], source: &str) -> io::Result<Layer> {
        let mut layer: Layer =
            toml::from_slice(bytes).map_err(|error| invalid(source, &error.to_string()))?;
        let input: toml::Table =
            toml::from_slice(bytes).map_err(|error| invalid(source, &error.to_string()))?;
        crate::core::plan::check_declared(
            layer.env.as_ref().unwrap_or(&BTreeMap::new()),
            layer.routes.as_ref().unwrap_or(&BTreeMap::new()),
        )
        .map_err(|error| invalid(source, &error))?;
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
            ..Default::default()
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
        // Unlike the other keys, `env` is replaced name by name.
        self.env = match (self.env, higher.env) {
            (Some(mut lower), Some(higher)) => {
                lower.extend(higher);
                Some(lower)
            }
            (lower, higher) => higher.or(lower),
        };
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

/// Each host `PINFOLD_ENV_<NAME>` as `<NAME>`, read by name. One whose
/// name is not UTF-8 is refused, naming neither its value nor its bytes.
fn env_overrides() -> io::Result<Vec<(String, Env)>> {
    let mut found = Vec::new();
    for (name, _) in env::vars_os() {
        if !name.as_encoded_bytes().starts_with(b"PINFOLD_ENV_") {
            continue;
        }
        let Some(name) = name.to_str() else {
            return Err(invalid(
                "PINFOLD_ENV_*",
                "a variable's name is not valid UTF-8",
            ));
        };
        let from = Some(name.to_string());
        let name = name["PINFOLD_ENV_".len()..].to_string();
        found.push((name, Env::Secret { from, op: None }));
    }
    Ok(found)
}

pub(crate) fn override_value(name: &str) -> io::Result<Option<String>> {
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
                // Not echoed: a pasted URL may hold a credential.
                "a PINFOLD_ROUTES entry is not name=host:port",
            ));
        };
        routes.insert(name.to_string(), Route::Address(target.to_string()));
    }
    Ok(routes)
}

fn invalid(source: &str, message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{source}: {message}"))
}
