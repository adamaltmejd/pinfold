//! Profile lookup: the user's copy under the config dir, else the embedded
//! default.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::dirs;

/// The built-in default profile's image, from `profile/` in this repo.
const DEFAULT_CONTAINERFILE: &[u8] = include_bytes!("../../../../profile/Containerfile");

/// One profile, as much as a build needs of it.
pub struct Profile {
    pub name: String,
    /// The profile's Containerfile.
    pub containerfile: Vec<u8>,
}

impl Profile {
    /// The profile named `name`. The user's
    /// `~/.config/pinfold/profiles/<name>/Containerfile` wins; `default`
    /// falls back to the embedded copy when the user has none.
    pub fn load(name: &str) -> io::Result<Profile> {
        if !valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("profile name {name:?} must start alphanumeric and hold only [a-z0-9._-]"),
            ));
        }
        let path = config_path(name)?;
        match fs::read(&path) {
            Ok(containerfile) => Ok(Profile {
                name: name.to_string(),
                containerfile,
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound && name == "default" => {
                Ok(Profile {
                    name: name.to_string(),
                    containerfile: DEFAULT_CONTAINERFILE.to_vec(),
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("profile {name:?} has no {}", path.display()),
            )),
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!("read {}: {error}", path.display()),
            )),
        }
    }

    /// The image the Containerfile's first `FROM` builds on, when it is a
    /// plain reference. A `FROM` that names a variable has none.
    pub fn base_image(&self) -> Option<&str> {
        let text = std::str::from_utf8(&self.containerfile).ok()?;
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
}

fn config_path(name: &str) -> io::Result<PathBuf> {
    Ok(dirs::config_dir()?
        .join("profiles")
        .join(name)
        .join("Containerfile"))
}

fn valid_name(name: &str) -> bool {
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
