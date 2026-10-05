//! Profile lookup: the user's copy under the config dir, else the embedded
//! default.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::dirs;

/// The built-in default profile's image, from `profile/` in this repo.
const DEFAULT_CONTAINERFILE: &[u8] = include_bytes!("../../../../profile/Containerfile");

/// The built-in default profile's config, from `profile/pinfold.toml`.
const DEFAULT_CONFIG: &[u8] = include_bytes!("../../../../profile/pinfold.toml");

/// The built-in default profile's `home/` seeds.
const DEFAULT_HOME: &[(&str, &[u8])] = &[(
    ".pi/agent/settings.json",
    include_bytes!("../../../../profile/home/.pi/agent/settings.json"),
)];

/// The built-in default profile's `share/`, extracted to the cache so a
/// directory exists to mount.
const DEFAULT_SHARE: &[(&str, &[u8])] = &[
    (
        "pi/package.json",
        include_bytes!("../../../../profile/share/pi/package.json"),
    ),
    (
        "pi/extensions/operating-context.ts",
        include_bytes!("../../../../profile/share/pi/extensions/operating-context.ts"),
    ),
    (
        "pi/skills/read-documents/SKILL.md",
        include_bytes!("../../../../profile/share/pi/skills/read-documents/SKILL.md"),
    ),
];

/// One file in a profile's `home/`: a seed for `$HOME`, copied only when
/// missing.
pub struct Seed {
    /// The path under `$HOME`.
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

/// One profile, as much of it as pinfold uses.
pub struct Profile {
    pub name: String,
    pub containerfile: Vec<u8>,
    /// The profile's `pinfold.toml`. Core never reads it; `profile new`
    /// copies it.
    pub config: Vec<u8>,
    pub home: Vec<Seed>,
    /// The `share/` directory mounted read-only at `/opt/pinfold/profile`.
    pub share: Option<PathBuf>,
}

impl Profile {
    /// The profile named `name`. A user directory under
    /// `~/.config/pinfold/profiles/<name>/` wins whole; only when no such
    /// directory exists does `default` fall back to the embedded copy.
    pub fn load(name: &str) -> io::Result<Profile> {
        check_name("profile", name)?;
        let root = dirs::config_dir()?.join("profiles").join(name);
        if root.is_dir() {
            return load_dir(name, &root);
        }
        if name == "default" {
            return Self::builtin();
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("profile {name:?} has no directory {}", root.display()),
        ))
    }

    /// The bundled default, even when a user's default profile overrides it.
    pub fn builtin() -> io::Result<Profile> {
        Ok(Profile {
            name: "default".to_string(),
            containerfile: DEFAULT_CONTAINERFILE.to_vec(),
            config: DEFAULT_CONFIG.to_vec(),
            home: DEFAULT_HOME
                .iter()
                .map(|(path, contents)| Seed {
                    path: PathBuf::from(path),
                    contents: contents.to_vec(),
                })
                .collect(),
            share: Some(embedded_share()?),
        })
    }

    /// The stable ref of this profile's image.
    pub fn image_ref(&self) -> String {
        format!("pinfold/profile-{}:latest", self.name)
    }
}

/// Tuple fields distinguish image, config, home and share; JSON preserves
/// each path/content boundary. User profile overrides do not enter this hash.
pub fn builtin_hash() -> io::Result<String> {
    Ok(super::sha256_hex(serde_json::to_vec(&(
        DEFAULT_CONTAINERFILE,
        DEFAULT_CONFIG,
        DEFAULT_HOME,
        DEFAULT_SHARE,
    ))?))
}

/// Read a profile from a user's directory.
fn load_dir(name: &str, root: &Path) -> io::Result<Profile> {
    let read = |file: &str| {
        let path = root.join(file);
        fs::read(&path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("profile {name:?}: read {}: {error}", path.display()),
            )
        })
    };
    let containerfile = read("Containerfile")?;
    let config = match read("pinfold.toml") {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        config => config?,
    };
    let home_dir = root.join("home");
    let mut home = Vec::new();
    if home_dir.is_dir() {
        read_seed_dir(&home_dir, &home_dir, &mut home)?;
    }
    let share = root.join("share");
    Ok(Profile {
        name: name.to_string(),
        containerfile,
        config,
        home,
        share: share.is_dir().then_some(share),
    })
}

/// The embedded default's `share/`, written under the cache. The content
/// hash in the path keeps a new binary from mounting an old extraction.
fn embedded_share() -> io::Result<PathBuf> {
    let dir = dirs::cache_dir()?
        .join("profiles")
        .join(super::sha256_hex(serde_json::to_vec(DEFAULT_SHARE)?))
        .join("share");
    dirs::install_dir(&dir, |staging| {
        for (path, contents) in DEFAULT_SHARE {
            let target = staging.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, contents)?;
        }
        Ok(())
    })?;
    Ok(dir)
}

/// Every regular file under `home/`, at its path relative to it.
fn read_seed_dir(root: &Path, dir: &Path, seeds: &mut Vec<Seed>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            read_seed_dir(root, &path, seeds)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("a seed path stays under home/")
                .to_path_buf();
            seeds.push(Seed {
                path: relative,
                contents: fs::read(&path)?,
            });
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
