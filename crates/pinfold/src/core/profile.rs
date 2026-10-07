//! Profile lookup: the user's copy under the config dir, else the embedded
//! profiles.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::dirs;

const BASE: &[u8] = include_bytes!("../../../../profile/image/base.Containerfile");
const BUN: &[u8] = include_bytes!("../../../../profile/image/bun.Containerfile");
const DOCUMENTS: &[u8] = include_bytes!("../../../../profile/image/documents.Containerfile");
const FULL: &[u8] = include_bytes!("../../../../profile/image/full.Containerfile");
const HARDEN: &[u8] = include_bytes!("../../../../profile/image/harden.Containerfile");
const CONFIG: &[u8] = include_bytes!("../../../../profile/pinfold.toml");
const HOME: &[(&str, &[u8])] = &[(
    ".pi/agent/settings.json",
    include_bytes!("../../../../profile/home/.pi/agent/settings.json"),
)];
const OPERATING_CONTEXT: (&str, &[u8]) = (
    "pi/extensions/operating-context.ts",
    include_bytes!("../../../../profile/share/pi/extensions/operating-context.ts"),
);
const READ_DOCUMENTS: (&str, &[u8]) = (
    "pi/skills/read-documents/SKILL.md",
    include_bytes!("../../../../profile/share/pi/skills/read-documents/SKILL.md"),
);
const DEFAULT_SHARE: &[(&str, &[u8])] = &[
    (
        "pi/package.json",
        include_bytes!("../../../../profile/share/pi/package.json"),
    ),
    OPERATING_CONTEXT,
];
const DOCUMENTS_SHARE: &[(&str, &[u8])] = &[
    (
        "pi/package.json",
        include_bytes!("../../../../profile/documents/package.json"),
    ),
    OPERATING_CONTEXT,
    READ_DOCUMENTS,
];
const FULL_SHARE: &[(&str, &[u8])] = &[
    (
        "pi/package.json",
        include_bytes!("../../../../profile/full/package.json"),
    ),
    OPERATING_CONTEXT,
    READ_DOCUMENTS,
];

/// A profile's read-only shared files. Embedded files are materialized only
/// when a box starts; metadata lookup and copying do not populate the cache.
pub enum Share {
    Directory(PathBuf),
    Embedded(&'static [(&'static str, &'static [u8])]),
}

impl Share {
    /// The host mount path, without creating it.
    pub fn path(&self) -> io::Result<PathBuf> {
        match self {
            Self::Directory(path) => Ok(path.clone()),
            Self::Embedded(files) => Ok(dirs::cache_dir()?
                .join("profiles")
                .join(super::sha256_hex(serde_json::to_vec(files)?))
                .join("share")),
        }
    }

    /// Install embedded files after the box has claimed its name.
    pub fn materialize(&self) -> io::Result<()> {
        if let Self::Embedded(files) = self {
            dirs::install_dir(&self.path()?, |staging| write_files(files, staging))?;
        }
        Ok(())
    }
}

/// Write embedded files directly, also for `profile new`.
pub fn write_files(files: &[(&str, &[u8])], root: &Path) -> io::Result<()> {
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
    pub share: Option<Share>,
}

impl Profile {
    /// The profile named `name`. A user directory under
    /// `~/.config/pinfold/profiles/<name>/` wins whole; only when no such
    /// directory exists does a built-in name fall back to its embedded copy.
    pub fn load(name: &str) -> io::Result<Profile> {
        check_name("profile", name)?;
        let root = dirs::config_dir()?.join("profiles").join(name);
        if root.is_dir() {
            return load_dir(name, &root);
        }
        if matches!(name, "default" | "documents" | "full") {
            return Self::builtin(name);
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("profile {name:?} has no directory {}", root.display()),
        ))
    }

    /// A named built-in, even when a user's profile overrides it.
    pub fn builtin(name: &str) -> io::Result<Profile> {
        check_name("profile", name)?;
        let (fragments, share): (&[&[u8]], _) = match name {
            "default" => (&[BASE, HARDEN], DEFAULT_SHARE),
            "documents" => (&[BASE, BUN, DOCUMENTS, HARDEN], DOCUMENTS_SHARE),
            "full" => (&[BASE, BUN, DOCUMENTS, FULL, HARDEN], FULL_SHARE),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no built-in profile {name:?}; expected default, documents or full"),
                ));
            }
        };
        Ok(Profile {
            name: name.to_string(),
            containerfile: fragments.concat(),
            config: CONFIG.to_vec(),
            home: HOME
                .iter()
                .map(|(path, contents)| Seed {
                    path: PathBuf::from(path),
                    contents: contents.to_vec(),
                })
                .collect(),
            share: Some(Share::Embedded(share)),
        })
    }

    /// The stable ref of this profile's image.
    pub fn image_ref(&self) -> String {
        format!("pinfold/profile-{}:latest", self.name)
    }
}

/// Hash all bundled inputs with their path/content boundaries preserved.
/// User overrides and extracted files do not enter this fingerprint.
pub fn builtin_hash() -> io::Result<String> {
    Ok(super::sha256_hex(serde_json::to_vec(&(
        BASE,
        BUN,
        DOCUMENTS,
        FULL,
        HARDEN,
        CONFIG,
        HOME,
        DEFAULT_SHARE,
        DOCUMENTS_SHARE,
        FULL_SHARE,
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
        share: share.is_dir().then_some(Share::Directory(share)),
    })
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

pub fn write_profile(source: &Profile, target: &Path) -> io::Result<()> {
    fs::write(target.join("Containerfile"), &source.containerfile)?;
    fs::write(target.join("pinfold.toml"), &source.config)?;
    for seed in &source.home {
        let path = target.join("home").join(&seed.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &seed.contents)?;
    }
    if let Some(share) = &source.share {
        match share {
            Share::Directory(path) => copy_tree(path, &target.join("share"), |_| false)?,
            Share::Embedded(files) => write_files(files, &target.join("share"))?,
        }
    }
    Ok(())
}

pub fn copy_tree(from: &Path, to: &Path, skip: fn(&OsStr) -> bool) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if skip(&entry.file_name()) {
            continue;
        }
        let target = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target, skip)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
