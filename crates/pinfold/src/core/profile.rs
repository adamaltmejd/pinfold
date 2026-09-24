//! Profile lookup: the user's copy under the config dir, else the embedded
//! default.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

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
        "pi/skills/.gitkeep",
        include_bytes!("../../../../profile/share/pi/skills/.gitkeep"),
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
    /// The profile's Containerfile.
    pub containerfile: Vec<u8>,
    /// The profile's `pinfold.toml`. Core never reads it; `profile new`
    /// copies it.
    pub config: Vec<u8>,
    /// Seeds for `$HOME`.
    pub home: Vec<Seed>,
    /// The `share/` directory mounted read-only at `/opt/pinfold/profile`.
    /// The embedded default's files are extracted to the cache so a
    /// directory exists to mount.
    pub share: Option<PathBuf>,
}

impl Profile {
    /// The profile named `name`. A user directory under
    /// `~/.config/pinfold/profiles/<name>/` wins whole; only when no such
    /// directory exists does `default` fall back to the embedded copy.
    pub fn load(name: &str) -> io::Result<Profile> {
        if !valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("profile name {name:?} must start alphanumeric and hold only [a-z0-9._-]"),
            ));
        }
        let root = dirs::config_dir()?.join("profiles").join(name);
        if root.is_dir() {
            return load_dir(name, &root);
        }
        if name == "default" {
            return Ok(Profile {
                name: name.to_string(),
                containerfile: DEFAULT_CONTAINERFILE.to_vec(),
                config: DEFAULT_CONFIG.to_vec(),
                home: embedded_home(),
                share: Some(embedded_share()?),
            });
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("profile {name:?} has no directory {}", root.display()),
        ))
    }

    /// The stable ref of this profile's image.
    pub fn image_ref(&self) -> String {
        format!("pinfold/profile-{}:latest", self.name)
    }
}

/// The image a Containerfile's first `FROM` builds on, when it is a plain
/// reference. A `FROM` that names a variable has none.
pub fn base_image(containerfile: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(containerfile).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("FROM") {
            continue;
        }
        for word in words {
            if word.starts_with("--") {
                continue;
            }
            if word.contains('$') {
                return None;
            }
            return Some(word);
        }
    }
    None
}

/// Read a profile from a user's directory.
fn load_dir(name: &str, root: &Path) -> io::Result<Profile> {
    let containerfile_path = root.join("Containerfile");
    let containerfile = match fs::read(&containerfile_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("profile {name:?} has no {}", containerfile_path.display()),
            ));
        }
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("read {}: {error}", containerfile_path.display()),
            ));
        }
    };
    let config_path = root.join("pinfold.toml");
    let config = match fs::read(&config_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("read {}: {error}", config_path.display()),
            ));
        }
    };
    let home_dir = root.join("home");
    let home = if home_dir.is_dir() {
        read_seeds(&home_dir)?
    } else {
        Vec::new()
    };
    let share_dir = root.join("share");
    let share = if share_dir.is_dir() {
        Some(share_dir)
    } else {
        None
    };
    Ok(Profile {
        name: name.to_string(),
        containerfile,
        config,
        home,
        share,
    })
}

/// The embedded default's `home/` files.
fn embedded_home() -> Vec<Seed> {
    DEFAULT_HOME
        .iter()
        .map(|(path, contents)| Seed {
            path: PathBuf::from(path),
            contents: contents.to_vec(),
        })
        .collect()
}

/// The embedded default's `share/`, written under the cache. The content
/// hash in the path keeps a new binary from mounting an old extraction.
fn embedded_share() -> io::Result<PathBuf> {
    let mut hasher = Sha256::new();
    for (path, contents) in DEFAULT_SHARE {
        hasher.update(path.as_bytes());
        // A separator, so a path and contents cannot run together.
        hasher.update([0]);
        hasher.update(contents);
    }
    let mut id = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(id, "{byte:02x}").expect("writing to a string cannot fail");
    }
    let dir = dirs::cache_dir()?.join("profiles").join(id).join("share");
    if dir.is_dir() {
        return Ok(dir);
    }
    let staging = dir
        .parent()
        .expect("the share directory has a parent")
        .join(format!(".tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    for (path, contents) in DEFAULT_SHARE {
        let target = staging.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, contents)?;
    }
    match fs::rename(&staging, &dir) {
        Ok(()) => Ok(dir),
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            // A concurrent load may have extracted the same files first.
            if dir.is_dir() { Ok(dir) } else { Err(error) }
        }
    }
}

/// Every regular file under `home/`, at its path relative to it.
fn read_seeds(home: &Path) -> io::Result<Vec<Seed>> {
    let mut seeds = Vec::new();
    read_seed_dir(home, home, &mut seeds)?;
    Ok(seeds)
}

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

/// Whether `name` is safe as a profile directory name: one path component
/// with no traversal.
pub fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    bytes.all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || byte == b'-'
            || byte == b'_'
            || byte == b'.'
    })
}
