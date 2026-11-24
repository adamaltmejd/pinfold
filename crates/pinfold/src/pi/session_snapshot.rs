//! In-memory comparison inputs. These are never retained in run records.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

use crate::config::{Config, ProjectMount};
use crate::core::image::service;
use crate::core::login::MAX_OP_VALUE;
use crate::core::plan::{self, Env, MountKind, Plan, Source, Values, memory_mib};
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

    fn value(values: &Values, source: &Source) -> Option<Self> {
        values.get(source).map(|value| Self::new(value.as_bytes()))
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
        let home = dirs::home()?;
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
    #[serde(
        default,
        deserialize_with = "plan::present",
        skip_serializing_if = "Option::is_none"
    )]
    basic: Option<String>,
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
    From {
        name: String,
    },
    /// Compared by its reference: a joining client reads no `op` value.
    Op {
        reference: String,
    },
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

/// The `op` values a starting client read, by reference. They travel beside
/// its snapshot in the bootstrap request only, so no admission compares
/// them and no joining client needs them. A list, not a map, so the owner
/// sees a repeated reference rather than a decoder's choice of one.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct OpValues(Vec<(String, Bytes)>);

impl OpValues {
    pub(crate) fn new(values: &Values) -> OpValues {
        OpValues(
            values
                .op_values()
                .map(|(reference, value)| (reference.to_string(), Bytes::new(value.as_bytes())))
                .collect(),
        )
    }
}

pub(crate) struct Resolved {
    pub(crate) snapshot: Snapshot,
    pub(crate) config: Config,
    pub(crate) plan: Plan,
    pub(crate) values: Values,
}

impl Snapshot {
    /// The owner's environment: the fixed names from this process's, and each
    /// `from` variable from `values`, this launch's one read.
    pub(crate) fn owner_environment(&self, values: &Values) -> Vec<(OsString, OsString)> {
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
        let mut sources = std::collections::BTreeSet::from_iter(self.route_values.keys().cloned());
        for source in self.env_names.values() {
            if let EnvSource::From { name } = source {
                sources.insert(name.clone());
            }
        }
        names.retain(|name| !sources.contains(name));
        let fixed = names
            .into_iter()
            .filter_map(|name| Some((name.clone().into(), std::env::var_os(&name)?)));
        let sources = sources.into_iter().filter_map(|name| {
            let value = values.get(&Source::From(name.clone()))?;
            Some((name.into(), value.to_os_string()))
        });
        fixed.chain(sources).collect()
    }

    /// The values a starting client sent, decoded: its snapshot's `from`
    /// values and the `op` values beside it, the table the owner starts the
    /// box with. That client refused any unset variable, failed read and
    /// malformed name or reference, so a declared source without a value,
    /// such a name or reference, a NUL value, or an empty or non-UTF-8 `op`
    /// value fails.
    pub(crate) fn values(&self, op: OpValues) -> io::Result<Values> {
        let invalid = || io::Error::other("invalid encoded environment");
        let decode = |source: Source, value: Option<&Bytes>| {
            let value = crate::init::decode_hex(value.ok_or_else(invalid)?.0.as_bytes())?;
            let valid = match &source {
                Source::From(name) => {
                    matches!(Source::declared(name), Ok(Source::From(_)))
                }
                Source::Op(reference) => {
                    matches!(Source::declared(reference), Ok(Source::Op(_)))
                        && !value.is_empty()
                        && value.len() <= MAX_OP_VALUE
                        && std::str::from_utf8(&value).is_ok()
                }
            };
            if !valid || value.contains(&0) {
                return Err(invalid());
            }
            Ok((source, OsString::from_vec(value)))
        };
        // Each entry is decoded once; an env entry is one per box variable.
        let routes = self
            .route_values
            .iter()
            .map(|(from, value)| decode(Source::From(from.clone()), value.as_ref()));
        let env = self
            .env_names
            .iter()
            .filter_map(|(name, source)| match source {
                EnvSource::From { name: from } => Some(decode(
                    Source::From(from.clone()),
                    self.env_values.get(name).and_then(Option::as_ref),
                )),
                EnvSource::Exact {} | EnvSource::Op { .. } => None,
            });
        // Exactly the declared references, each once.
        let references: std::collections::BTreeSet<&str> =
            op.0.iter()
                .map(|(reference, _)| reference.as_str())
                .collect();
        let declared_op: std::collections::BTreeSet<&str> = self
            .routes
            .values()
            .flat_map(|declaration| match declaration {
                Declaration::Inject(inject) => inject
                    .headers
                    .iter()
                    .flatten()
                    .map(|(_, header)| header.from.as_str())
                    .collect(),
                Declaration::Login(login) => login.from.as_deref().into_iter().collect(),
                Declaration::Address(_) => Vec::new(),
            })
            .filter(|from| matches!(Source::declared(from), Ok(Source::Op(_))))
            .chain(self.env_names.values().filter_map(|source| match source {
                EnvSource::Op { reference } => Some(reference.as_str()),
                EnvSource::Exact {} | EnvSource::From { .. } => None,
            }))
            .collect();
        if references.len() != op.0.len() || references != declared_op {
            return Err(invalid());
        }
        let op =
            op.0.iter()
                .map(|(reference, value)| decode(Source::Op(reference.clone()), Some(value)));
        let values: BTreeMap<_, _> = routes.chain(env).chain(op).collect::<io::Result<_>>()?;
        let has =
            |from: &str| Source::declared(from).is_ok_and(|source| values.contains_key(&source));
        let declared = self.routes.values().all(|declaration| match declaration {
            Declaration::Inject(inject) => inject
                .headers
                .iter()
                .flatten()
                .all(|(_, header)| has(&header.from)),
            Declaration::Login(login) => {
                let host_codex = plan::Login {
                    login: login.login.clone(),
                    provider: login.provider.clone(),
                    from: None,
                    to: None,
                }
                .uses_host_codex();
                match &login.from {
                    None => true,
                    Some(from) => !host_codex && has(from),
                }
            }
            Declaration::Address(_) => true,
        }) && self.env_names.iter().all(|(name, source)| match source {
            // An `op` value travels only in `op`, never in an env slot.
            EnvSource::Op { reference } => {
                values.contains_key(&Source::Op(reference.clone()))
                    && self.env_values.get(name).is_none_or(Option::is_none)
            }
            EnvSource::Exact {} | EnvSource::From { .. } => true,
        });
        if !declared {
            return Err(invalid());
        }
        Ok(Values::received(values))
    }

    /// Resolve the project's configuration. A client reads each `from`
    /// value here, once, and a starting client its `op` values afterwards;
    /// the owner passes the table its client sent. Only a `start` touches
    /// folders on the host: a joining client compares them as
    /// configured.
    pub(crate) fn resolve(
        root: &Path,
        expected_mount: Option<ProjectMount>,
        received: Option<Values>,
        start: bool,
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
        // A joining client is compared first: a changed folder list is
        // `config-changed`, never this refusal.
        if start {
            launch::check_folder_guests(&config, root)?;
        }
        let mut plan = launch::build_plan(&config, &id, root, &image)?;
        // The folders are the plan's last mounts, held as written.
        let project_mounts = plan.mounts.len() - config.folders.len();
        for mount in &mut plan.mounts[..project_mounts] {
            if mount.kind == MountKind::Share {
                mount.host = dirs::resolve_host_path(&mount.host)?;
            }
        }
        for disk in &mut plan.disks {
            if let Some(from) = disk.from.as_mut() {
                *from = dirs::resolve_host_path(from)?;
            }
        }
        // The owner compares snapshots before anything checks its table: a
        // source edited since its client resolved is `config-changed`, and an
        // equal snapshot names only sources its client checked.
        let check = received.is_none();
        let values = received.unwrap_or_else(|| Values::read(&plan));
        // Only a start touches a folder on the host, and admits it before
        // the box layer reads its contents. The snapshot holds each host as
        // written, which a start requires to be its real path.
        let mut checked = plan.clone();
        if start {
            for folder in &config.folders {
                crate::config::admit_folder(root, folder, &config.profile)?;
            }
        } else {
            checked.mounts.truncate(project_mounts);
        }
        crate::core::r#box::validate_project(&checked, check.then_some(&values), &config.profile)
            .map_err(io::Error::other)?;
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
        // An `op` source's value is never compared, so it is in neither
        // table: a joining client reads none.
        let route_values = plan
            .sources()
            .filter_map(|(kind, _, source)| match source {
                Ok(Source::From(from)) if kind == "route" => {
                    let value = Bytes::value(&values, &Source::From(from.clone()));
                    Some((from, value))
                }
                _ => None,
            })
            .collect();
        let mut env_names = BTreeMap::new();
        let mut env_values = BTreeMap::new();
        for (name, value) in &plan.env {
            let (source, value) = match (value, value.source()) {
                (Env::Exact(value), _) => (EnvSource::Exact {}, Some(Bytes::new(value.as_bytes()))),
                (_, Some(Ok(Source::Op(reference)))) => (EnvSource::Op { reference }, None),
                (_, Some(Ok(source))) => (
                    EnvSource::From {
                        name: source.name().to_string(),
                    },
                    Bytes::value(&values, &source),
                ),
                (_, _) => return Err(io::Error::other("spec-invalid: invalid env source")),
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
            values,
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
