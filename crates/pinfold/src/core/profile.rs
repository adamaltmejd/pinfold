//! Profile lookup: the user's copy under the config dir, else the embedded
//! profiles.

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
const FULL_CONFIG: &[u8] = include_bytes!("../../../../profile/full/pinfold.toml");
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
const PI_PACKAGE: (&str, &[u8]) = (
    "pi/package.json",
    include_bytes!("../../../../profile/share/pi/package.json"),
);
const DEFAULT_SHARE: &[(&str, &[u8])] = &[PI_PACKAGE, OPERATING_CONTEXT];
const DOCUMENTS_SHARE: &[(&str, &[u8])] = &[PI_PACKAGE, OPERATING_CONTEXT, READ_DOCUMENTS];
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
            dirs::install_dir(&self.path()?, |staging| {
                write_files(files.iter().copied(), staging)
            })?;
        }
        Ok(())
    }
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
        Self::builtin(name).map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("profile {name:?} has no directory {}", root.display()),
            )
        })
    }

    /// A named built-in, even when a user's profile overrides it.
    pub fn builtin(name: &str) -> io::Result<Profile> {
        let (fragments, config, share): (&[&[u8]], &[u8], _) = match name {
            "default" => (&[BASE, HARDEN], CONFIG, DEFAULT_SHARE),
            "documents" => (&[BASE, BUN, DOCUMENTS, HARDEN], CONFIG, DOCUMENTS_SHARE),
            "full" => (
                &[BASE, BUN, DOCUMENTS, FULL, HARDEN],
                FULL_CONFIG,
                FULL_SHARE,
            ),
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
            config: config.to_vec(),
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
        format!("localhost/pinfold/profile-{}:latest", self.name)
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
        FULL_CONFIG,
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
        containerfile,
        config,
        home,
        share: share.is_dir().then_some(Share::Directory(share)),
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

pub fn write_profile(source: &Profile, target: &Path) -> io::Result<()> {
    fs::write(target.join("Containerfile"), &source.containerfile)?;
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
        Some(Share::Embedded(files)) => write_files(files.iter().copied(), &share),
        None => Ok(()),
    }
}
