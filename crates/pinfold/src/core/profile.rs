//! Profile lookup: the user's copy under the config dir, else the embedded
//! profiles. A profile may name a `base`, applied beneath it.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::dirs;

const DEFAULT: &[u8] = include_bytes!("../../../../profile/image/default.Containerfile");
const DOCUMENTS: &[u8] = include_bytes!("../../../../profile/image/documents.Containerfile");
const ADAM: &[u8] = include_bytes!("../../../../profile/image/adam.Containerfile");
const CONFIG: &[u8] = include_bytes!("../../../../profile/pinfold.toml");
const DOCUMENTS_CONFIG: &[u8] = include_bytes!("../../../../profile/documents/pinfold.toml");
const ADAM_CONFIG: &[u8] = include_bytes!("../../../../profile/adam/pinfold.toml");
const PROJECT_GUIDANCE: &[u8] = include_bytes!("../../../../profile/project-guidance.md");
const HOME: &[(&str, &[u8])] = &[
    (".pi/agent/APPEND_SYSTEM.md", PROJECT_GUIDANCE),
    (".claude/CLAUDE.md", PROJECT_GUIDANCE),
    (".codex/AGENTS.md", PROJECT_GUIDANCE),
    (
        ".pi/agent/settings.json",
        include_bytes!("../../../../profile/home/.pi/agent/settings.json"),
    ),
    (
        ".claude/settings.json",
        include_bytes!("../../../../profile/home/.claude/settings.json"),
    ),
    (
        ".codex/config.toml",
        include_bytes!("../../../../profile/home/.codex/config.toml"),
    ),
    (
        ".codex/app-server-daemon/settings.json",
        include_bytes!("../../../../profile/home/.codex/app-server-daemon/settings.json"),
    ),
];
const DEFAULT_SHARE: &[(&str, &[u8])] = &[
    (
        "pi/package.json",
        include_bytes!("../../../../profile/share/pi/package.json"),
    ),
    (
        "pi/extensions/operating-context.ts",
        include_bytes!("../../../../profile/share/pi/extensions/operating-context.ts"),
    ),
];
const DOCUMENTS_SHARE: &[(&str, &[u8])] = &[(
    "pi/skills/read-documents/SKILL.md",
    include_bytes!("../../../../profile/share/pi/skills/read-documents/SKILL.md"),
)];
const ADAM_SHARE: &[(&str, &[u8])] = &[(
    "pi/package.json",
    include_bytes!("../../../../profile/adam/package.json"),
)];

/// One shared file held in memory: its path under `share/`, its bytes and
/// whether it is executable.
pub type ShareFile = (String, Vec<u8>, bool);

/// A profile's read-only shared files.
#[derive(Clone)]
pub enum Share {
    /// A user directory, mounted live.
    Directory(PathBuf),
    /// A built-in's files, or the files of a chain with more than one
    /// share, merged by path. Each box start writes a fresh copy into its
    /// own state dir, which goes with that generation.
    Files(Vec<ShareFile>),
}

/// The copy's directory in a box's state dir.
const SHARE_COPY: &str = "profile-share";

impl Share {
    /// The host mount path for box `name`, without creating it.
    pub fn path(&self, name: &str) -> io::Result<PathBuf> {
        match self {
            Self::Directory(path) => Ok(path.clone()),
            Self::Files(_) => {
                let path = dirs::box_state_dir(name)?.join(SHARE_COPY);
                // The copy does not exist when the spec's mount checks run,
                // but its path still goes into a mount option.
                if !super::plan::valid_mount_path(&path) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "mount path {path:?} must not hold ',' or an ASCII control character"
                        ),
                    ));
                }
                Ok(path)
            }
        }
    }

    /// Write held files into the claimed box's state dir.
    pub fn materialize(&self, state_dir: &Path) -> io::Result<()> {
        if let Self::Files(files) = self {
            let dir = state_dir.join(SHARE_COPY);
            fs::create_dir(&dir)?;
            write_share(files, &dir)?;
        }
        Ok(())
    }

    /// Every file, read whole.
    fn files(&self) -> io::Result<Vec<ShareFile>> {
        let root = match self {
            Self::Files(files) => return Ok(files.clone()),
            Self::Directory(root) => root,
        };
        let mut found = Vec::new();
        files(root, root, &mut found)?;
        found
            .into_iter()
            .map(|path| {
                let full = root.join(&path);
                let name = path.to_str().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "path-not-utf8: share file {}",
                            crate::formatting::terminal_text(&full.to_string_lossy())
                        ),
                    )
                })?;
                let executable = fs::metadata(&full)?.permissions().mode() & 0o111 != 0;
                Ok((name.to_string(), fs::read(&full)?, executable))
            })
            .collect()
    }
}

fn embedded(files: &[(&str, &[u8])]) -> Vec<ShareFile> {
    files
        .iter()
        .map(|(path, contents)| (path.to_string(), contents.to_vec(), false))
        .collect()
}

fn write_share(files: &[ShareFile], root: &Path) -> io::Result<()> {
    write_files(files.iter().map(|(path, bytes, _)| (path, bytes)), root)?;
    for (path, _, executable) in files {
        if *executable {
            fs::set_permissions(root.join(path), fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// Write embedded files directly, also for `profile new`.
pub fn write_files(
    files: impl IntoIterator<Item = (impl AsRef<Path>, impl AsRef<[u8]>)>,
    root: &Path,
) -> io::Result<()> {
    for (path, contents) in files {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, contents)?;
    }
    Ok(())
}

/// One file in a profile's `home/`: a seed for `$HOME`, copied only when
/// missing.
#[derive(Clone)]
pub struct Seed {
    /// The path under `$HOME`.
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

/// One profile, as much of it as pinfold uses. `containerfile` and
/// `config` are this profile's own; `home` and `share` include its base's
/// beneath its own.
pub struct Profile {
    pub name: String,
    pub(crate) built_in: bool,
    pub containerfile: Option<Vec<u8>>,
    /// The profile's `pinfold.toml`. Core reads only its `base`.
    pub config: Vec<u8>,
    pub home: Vec<Seed>,
    /// The `share/` directory mounted read-only at `/opt/pinfold/profile`.
    pub share: Option<Share>,
    pub base: Option<Box<Profile>>,
}

impl Profile {
    /// The profile named `name` with its base chain applied beneath it.
    pub fn load(name: &str) -> io::Result<Profile> {
        Self::load_chain(name, &mut Vec::new())
    }

    fn load_chain(name: &str, above: &mut Vec<String>) -> io::Result<Profile> {
        if above.iter().any(|link| link == name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("base cycle back to profile {name:?}"),
            ));
        }
        let mut profile = Self::link(name)?;
        let Some(base) = base_name(&profile)? else {
            return Ok(profile);
        };
        above.push(name.to_string());
        let base = Self::load_chain(&base, above)
            .map_err(|error| io::Error::new(error.kind(), format!("profile {name:?}: {error}")))?;
        // A recipe FROMs its base's own image, which only a base with a
        // recipe has; a stale image under its name would go unrecorded.
        if profile.containerfile.is_some() && base.containerfile.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "profile {name:?} has a Containerfile, so its base {:?} needs one too",
                    base.name
                ),
            ));
        }
        profile.home = overlay(
            base.home.clone(),
            std::mem::take(&mut profile.home),
            |seed| &seed.path,
        );
        // One contributing share keeps its own form: a user directory stays
        // a live mount. Two or more are merged by path into a copy.
        profile.share = match (base.share.as_ref(), profile.share.take()) {
            (Some(lower), Some(upper)) => Some(Share::Files(overlay(
                lower.files()?,
                upper.files()?,
                |(path, _, _)| Path::new(path),
            ))),
            (lower, upper) => upper.or_else(|| lower.cloned()),
        };
        profile.base = Some(Box::new(base));
        Ok(profile)
    }

    /// The profile named `name` alone, without its base. A user directory
    /// under `~/.config/pinfold/profiles/<name>/` wins whole; only when no
    /// such directory exists does a built-in name fall back to its embedded
    /// copy.
    pub fn link(name: &str) -> io::Result<Profile> {
        check_name("profile", name)?;
        let root = dirs::config_dir()?.join("profiles").join(name);
        if root.is_dir() {
            return load_dir(name, &root);
        }
        Self::builtin(name).map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "profile {name:?} has no directory {}",
                    crate::formatting::terminal_text(&root.to_string_lossy())
                ),
            )
        })
    }

    /// A named built-in alone, even when a user's profile overrides it.
    fn builtin(name: &str) -> io::Result<Profile> {
        let (containerfile, config, home, share): (_, _, &[(&str, &[u8])], _) = match name {
            "default" => (DEFAULT, CONFIG, HOME, DEFAULT_SHARE),
            "documents" => (DOCUMENTS, DOCUMENTS_CONFIG, &[], DOCUMENTS_SHARE),
            "adam" => (ADAM, ADAM_CONFIG, &[], ADAM_SHARE),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no built-in profile {name:?}; expected default, documents or adam"),
                ));
            }
        };
        Ok(Profile {
            name: name.to_string(),
            built_in: true,
            containerfile: Some(containerfile.to_vec()),
            config: config.to_vec(),
            home: home
                .iter()
                .map(|(path, contents)| Seed {
                    path: PathBuf::from(path),
                    contents: contents.to_vec(),
                })
                .collect(),
            share: Some(Share::Files(embedded(share))),
            base: None,
        })
    }

    /// The chain from its root up to this profile.
    pub fn links(&self) -> Vec<&Profile> {
        let mut links = vec![self];
        while let Some(base) = &links.last().expect("one link").base {
            links.push(base);
        }
        links.reverse();
        links
    }

    /// The nearest profile in the chain with a Containerfile: this one, or
    /// the base whose image a recipe-less profile uses.
    pub fn recipe(&self) -> Option<&Profile> {
        if self.containerfile.is_some() {
            return Some(self);
        }
        self.base.as_deref().and_then(Profile::recipe)
    }

    /// The stable ref of this profile's own image.
    pub fn image_ref(&self) -> String {
        format!("localhost/pinfold/profile-{}:latest", self.name)
    }
}

/// `upper`'s files over `lower`'s, sorted by path. An upper path replaces
/// the lower file at that path and, where one is a directory in the other
/// link, everything lower beneath it or above it on that path.
fn overlay<T>(lower: Vec<T>, upper: Vec<T>, path: impl Fn(&T) -> &Path) -> Vec<T> {
    let mut files: Vec<T> = lower
        .into_iter()
        .filter(|low| {
            !upper
                .iter()
                .any(|high| path(low).starts_with(path(high)) || path(high).starts_with(path(low)))
        })
        .collect();
    files.extend(upper);
    files.sort_by(|a, b| path(a).cmp(path(b)));
    files
}

/// The `base` a profile's `pinfold.toml` names. Configuration checks the
/// rest of the file.
fn base_name(profile: &Profile) -> io::Result<Option<String>> {
    let invalid = |message: String| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("profile {:?} pinfold.toml: {message}", profile.name),
        )
    };
    let table: toml::Table =
        toml::from_slice(&profile.config).map_err(|error| invalid(error.to_string()))?;
    match table.get("base") {
        None => Ok(None),
        Some(toml::Value::String(base)) => Ok(Some(base.clone())),
        Some(_) => Err(invalid("`base` must be a profile name".into())),
    }
}

/// Hash all bundled inputs with their path/content boundaries preserved.
/// User overrides and extracted files do not enter this fingerprint.
pub fn builtin_hash() -> io::Result<String> {
    Ok(super::sha256_hex(serde_json::to_vec(&(
        DEFAULT,
        DOCUMENTS,
        ADAM,
        CONFIG,
        DOCUMENTS_CONFIG,
        ADAM_CONFIG,
        HOME,
        DEFAULT_SHARE,
        DOCUMENTS_SHARE,
        ADAM_SHARE,
    ))?))
}

/// Read a profile from a user's directory.
fn load_dir(name: &str, root: &Path) -> io::Result<Profile> {
    let read = |file: &str| {
        let path = root.join(file);
        fs::read(&path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "profile {name:?}: read {}: {error}",
                    crate::formatting::terminal_text(&path.to_string_lossy())
                ),
            )
        })
    };
    let containerfile = match read("Containerfile") {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let config = match read("pinfold.toml") {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        config => config?,
    };
    let home_dir = root.join("home");
    let mut home = Vec::new();
    if home_dir.is_dir() {
        files(&home_dir, &home_dir, &mut home)?;
    }
    let home = home
        .into_iter()
        .map(|path| {
            let contents = fs::read(home_dir.join(&path))?;
            Ok(Seed { path, contents })
        })
        .collect::<io::Result<_>>()?;
    let share = root.join("share");
    Ok(Profile {
        name: name.to_string(),
        built_in: false,
        containerfile,
        config,
        home,
        share: share.is_dir().then_some(Share::Directory(share)),
        base: None,
    })
}

/// Every regular file under `dir`, at its path relative to `root`.
fn files(root: &Path, dir: &Path, found: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            files(root, &path, found)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("a walked path stays under its root");
            found.push(relative.to_path_buf());
        }
    }
    Ok(())
}

/// Refuse a name that is not safe as a directory name: one path component
/// with no traversal. `what` names the kind of name, a profile or an image.
pub fn check_name(what: &str, name: &str) -> io::Result<()> {
    let mut bytes = name.bytes();
    let valid = bytes
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'-'
                || byte == b'_'
                || byte == b'.'
        });
    if valid {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{what} name {name:?} must start alphanumeric and hold only [a-z0-9._-]"),
    ))
}

/// Write one profile's own files, without its base, for `profile new`.
pub fn write_profile(source: &Profile, target: &Path) -> io::Result<()> {
    if let Some(bytes) = &source.containerfile {
        fs::write(target.join("Containerfile"), bytes)?;
    }
    fs::write(target.join("pinfold.toml"), &source.config)?;
    let seeds = source.home.iter().map(|seed| (&seed.path, &seed.contents));
    write_files(seeds, &target.join("home"))?;
    let share = target.join("share");
    match &source.share {
        Some(Share::Directory(from)) => {
            let mut found = Vec::new();
            files(from, from, &mut found)?;
            fs::create_dir_all(&share)?;
            for path in found {
                let to = share.join(&path);
                fs::create_dir_all(to.parent().expect("a file path has a directory"))?;
                // `copy` keeps the file's mode, so an executable stays one.
                fs::copy(from.join(&path), to)?;
            }
            Ok(())
        }
        Some(Share::Files(files)) => write_share(files, &share),
        None => Ok(()),
    }
}
