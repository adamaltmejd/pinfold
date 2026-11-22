//! In-memory comparison inputs. These are never retained in run records.

use std::collections::BTreeMap;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

use crate::config::{Config, ProjectMount};
use crate::core::image::service;
use crate::core::plan::{Env, MountKind, Plan, Route, memory_mib};
use crate::core::profile::Share;
use crate::dirs;
use crate::pi::{context::Context, launch, state};

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Bytes(String);

impl Bytes {
    fn new(bytes: &[u8]) -> Self {
        Self(crate::core::hex(bytes))
    }

    fn path(path: &Path) -> Self {
        Self::new(path.as_os_str().as_bytes())
    }

    fn canonical(path: &Path) -> io::Result<Self> {
        Ok(Self::path(&dirs::resolve_host_path(path)?))
    }

    fn variable(name: &str) -> Option<Self> {
        std::env::var_os(name).map(|value| Self::new(value.as_bytes()))
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.len() % 2 != 0
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(serde::de::Error::custom("invalid encoded bytes"));
        }
        Ok(Self(value))
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Roots {
    config: Bytes,
    state: Bytes,
    cache: Bytes,
    home: Bytes,
    codex_home: Option<Bytes>,
}

impl Roots {
    pub(crate) fn resolve() -> io::Result<Self> {
        let home = std::env::home_dir()
            .ok_or_else(|| io::Error::other("HOME is not set and the uid has no passwd entry"))?;
        Ok(Self {
            config: Bytes::canonical(&dirs::config_dir()?)?,
            state: Bytes::canonical(&dirs::state_dir()?)?,
            cache: Bytes::canonical(&dirs::cache_dir()?)?,
            home: Bytes::canonical(&home)?,
            // The helper inherits this selection. No helper or auth file is opened.
            codex_home: Bytes::variable("CODEX_HOME"),
        })
    }

    pub(crate) fn differences(&self, candidate: &Self) -> Vec<String> {
        let mut fields = Vec::new();
        for (different, field) in [
            (self.config != candidate.config, "roots.config"),
            (self.state != candidate.state, "roots.state"),
            (self.cache != candidate.cache, "roots.cache"),
            (self.home != candidate.home, "roots.home"),
            (self.codex_home != candidate.codex_home, "roots.codex_home"),
        ] {
            if different {
                fields.push(field.to_string());
            }
        }
        fields.sort();
        fields
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileSelection {
    name: String,
    built_in: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    source: String,
    reference: String,
    id: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resources {
    cpus: f64,
    memory_mib: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum MountSource {
    Host { path: Bytes },
    Embedded { profile: String },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mount {
    source: MountSource,
    guest: Bytes,
    kind: MountKind,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Disk {
    name: String,
    guest: Bytes,
    size: Option<String>,
    from: Option<Bytes>,
    readonly: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum Declaration {
    Address(String),
    Inject(Inject),
    Login(Login),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inject {
    to: String,
    headers: Option<BTreeMap<String, Header>>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    from: String,
    prefix: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    login: String,
    provider: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum EnvSource {
    Exact {},
    From { name: String },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub(crate) roots: Roots,
    repository: bool,
    project_mount: ProjectMount,
    profile: ProfileSelection,
    image: Image,
    resources: Resources,
    mounts: Vec<Mount>,
    disks: Vec<Disk>,
    allow: Vec<String>,
    routes: BTreeMap<String, Declaration>,
    route_values: BTreeMap<String, Option<Bytes>>,
    env_names: BTreeMap<String, EnvSource>,
    env_values: BTreeMap<String, Option<Bytes>>,
}

pub(crate) struct Resolved {
    pub(crate) snapshot: Snapshot,
    pub(crate) config: Config,
    pub(crate) plan: Plan,
}

impl Snapshot {
    pub(crate) fn owner_environment(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        let mut names = std::collections::BTreeSet::from_iter(
            [
                "PATH",
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_STATE_HOME",
                "XDG_CACHE_HOME",
                "CODEX_HOME",
                "PINFOLD_PROFILE",
                "PINFOLD_CPUS",
                "PINFOLD_MEMORY",
                "PINFOLD_ALLOW",
                "PINFOLD_ROUTES",
            ]
            .map(String::from),
        );
        names.extend(self.route_values.keys().cloned());
        for source in self.env_names.values() {
            if let EnvSource::From { name } = source {
                names.insert(name.clone());
            }
        }
        names
            .into_iter()
            .filter_map(|name| std::env::var_os(&name).map(|value| (name.into(), value)))
            .collect()
    }
    pub(crate) fn resolve(
        root: &Path,
        expected_mount: Option<ProjectMount>,
    ) -> io::Result<Resolved> {
        let roots = Roots::resolve()?;
        let context = Context::discover(root)?;
        if context.root != root {
            return Err(io::Error::other("git-discovery: project root changed"));
        }
        let mut config = Config::load(root)?;
        if expected_mount.is_some_and(|mount| mount != config.mount) {
            return Err(io::Error::other(
                "config-changed: the project mount kind changed (project.mount). Restore the previous mount kind or finish active sessions and run `pinfold stop` before retrying.",
            ));
        }
        if config.repository != context.repository {
            return Err(io::Error::other(
                "git-discovery: repository changed during configuration",
            ));
        }
        if let Some(Share::Directory(path)) = config.profile.share.as_mut() {
            *path = dirs::resolve_host_path(path)?;
        }
        config.check_host_paths(root)?;
        context.verify(config.mount)?;
        let id = state::project_id(root);
        let image = launch::resolve_image(&config, &id);
        let mut plan = launch::build_plan(&config, &id, root, &image)?;
        for mount in &mut plan.mounts {
            mount.host = dirs::resolve_host_path(&mount.host)?;
        }
        for disk in &mut plan.disks {
            if let Some(from) = disk.from.as_mut() {
                *from = dirs::resolve_host_path(from)?;
            }
        }
        crate::core::r#box::validate_project(&plan, &config.profile).map_err(io::Error::other)?;
        if !config.cpus.is_finite() {
            return Err(io::Error::other("spec-invalid: CPUs must be finite"));
        }
        let memory_mib = memory_mib(&config.memory)
            .ok_or_else(|| io::Error::other("spec-invalid: invalid memory"))?;
        let identity = service().resolve_image(&image)?.ok().map(|image| image.id);
        let mut mounts = plan
            .mounts
            .iter()
            .map(|mount| Mount {
                source: MountSource::Host {
                    path: Bytes::path(&mount.host),
                },
                guest: Bytes::path(&mount.guest),
                kind: mount.kind,
            })
            .collect::<Vec<_>>();
        if let Some(share) = &config.profile.share {
            mounts.push(Mount {
                source: match share {
                    Share::Directory(path) => MountSource::Host {
                        path: Bytes::path(path),
                    },
                    Share::Files(_) => MountSource::Embedded {
                        profile: config.profile.name.clone(),
                    },
                },
                guest: Bytes::path(Path::new("/opt/pinfold/profile")),
                kind: MountKind::Readonly,
            });
        }
        let disks = plan
            .disks
            .iter()
            .map(|disk| Disk {
                name: disk.name.clone(),
                guest: Bytes::path(&disk.guest),
                size: disk.size.clone(),
                from: disk.from.as_deref().map(Bytes::path),
                readonly: disk.readonly,
            })
            .collect();
        let mut routes = BTreeMap::new();
        for (name, declaration) in &config.route_declarations {
            let declaration = Declaration::deserialize(declaration.clone())
                .map_err(|_| io::Error::other("spec-invalid: invalid route declaration"))?;
            routes.insert(name.clone(), declaration);
        }
        let mut route_values = BTreeMap::new();
        for route in config.routes.values() {
            match route {
                Route::Inject(inject) => {
                    for header in inject.headers.values() {
                        route_values.insert(header.from.clone(), Bytes::variable(&header.from));
                    }
                }
                Route::Login(login) => {
                    if let Some(from) = &login.from {
                        route_values.insert(from.clone(), Bytes::variable(from));
                    }
                }
                Route::Address(_) => {}
            }
        }
        let mut env_names = BTreeMap::new();
        let mut env_values = BTreeMap::new();
        for (name, value) in &plan.env {
            let (source, value) = match value {
                Env::Exact(value) => (EnvSource::Exact {}, Some(Bytes::new(value.as_bytes()))),
                Env::From { from } => (
                    EnvSource::From { name: from.clone() },
                    Bytes::variable(from),
                ),
            };
            env_names.insert(name.clone(), source);
            env_values.insert(name.clone(), value);
        }
        let snapshot = Self {
            roots,
            repository: config.repository,
            project_mount: config.mount,
            profile: ProfileSelection {
                name: config.profile.name.clone(),
                built_in: config.profile.built_in,
            },
            image: Image {
                source: config.image_source().to_string(),
                reference: image,
                id: identity,
            },
            resources: Resources {
                cpus: config.cpus,
                memory_mib,
            },
            mounts,
            disks,
            allow: config.allow.clone(),
            routes,
            route_values,
            env_names,
            env_values,
        };
        Ok(Resolved {
            snapshot,
            config,
            plan,
        })
    }

    pub(crate) fn image_id(&self) -> Option<&str> {
        self.image.id.as_deref()
    }

    pub(crate) fn project_mount(&self) -> ProjectMount {
        self.project_mount
    }

    pub(crate) fn is_share(&self) -> bool {
        self.project_mount == ProjectMount::Share
    }

    /// The owner rechecks a share's root Git marker without following it,
    /// since both snapshots can predate a `.git` that appears later.
    pub(crate) fn repository_differences(&self, root: &Path) -> Vec<String> {
        if !self.is_share()
            || crate::pi::context::share_repository(root)
                .is_ok_and(|present| present == self.repository)
        {
            Vec::new()
        } else {
            vec!["repository".into()]
        }
    }

    pub(crate) fn differences(&self, candidate: &Self) -> Vec<String> {
        let mut fields = self.roots.differences(&candidate.roots);
        for (different, field) in [
            (self.repository != candidate.repository, "repository"),
            (
                self.project_mount != candidate.project_mount,
                "project.mount",
            ),
            (self.profile != candidate.profile, "profile"),
            (self.image.source != candidate.image.source, "image.source"),
            (
                self.image.reference != candidate.image.reference,
                "image.ref",
            ),
            (self.image.id != candidate.image.id, "image.id"),
            (self.resources != candidate.resources, "resources"),
            (self.mounts != candidate.mounts, "mounts"),
            (self.disks != candidate.disks, "disks"),
            (self.allow != candidate.allow, "allow"),
            (self.routes != candidate.routes, "routes.declarations"),
            (self.route_values != candidate.route_values, "routes.values"),
            (self.env_names != candidate.env_names, "env.names"),
            (self.env_values != candidate.env_values, "env.values"),
        ] {
            if different {
                fields.push(field.to_string());
            }
        }
        fields.sort();
        fields
    }
}
