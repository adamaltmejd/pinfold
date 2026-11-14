//! The project: its root folder, and whether Git crosses it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::crossing;
use crate::pi::state::canonical;

pub struct Context {
    pub root: PathBuf,
    pub repository: bool,
}

impl Context {
    /// Discover by host metadata so read-only reports do not require Git.
    /// A repository marker never falls back to scratch, even if damaged.
    pub fn discover(cwd: &Path) -> io::Result<Self> {
        let cwd = canonical(cwd)?;
        for ancestor in cwd.ancestors() {
            if ancestor.file_name().is_some_and(|name| name == ".git")
                || (ancestor.join("HEAD").is_file()
                    && ancestor.join("objects").is_dir()
                    && ancestor.join("refs").is_dir())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "project-in-git: refusing to run inside a git directory",
                ));
            }
            match fs::symlink_metadata(ancestor.join(".git")) {
                Ok(metadata) if metadata.is_dir() => {
                    return Ok(Self {
                        root: ancestor.to_path_buf(),
                        repository: true,
                    });
                }
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "worktree: .git is not a directory; run pinfold from the main checkout",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if cfg!(target_os = "linux") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "not-git: {} is not a git repository; a Linux project is a Git checkout",
                    cwd.display()
                ),
            ));
        }
        Ok(Self {
            root: cwd,
            repository: false,
        })
    }

    /// Launches must verify a discovered repository before creating state.
    pub fn verify(&self) -> io::Result<()> {
        if !self.repository {
            return Ok(());
        }
        let output = crossing::git_command(&self.root)
            .args(["rev-parse", "--show-toplevel"])
            .output()?;
        if output.status.success()
            && let Ok(top) = std::str::from_utf8(&output.stdout)
        {
            let top = Path::new(top.strip_suffix('\n').unwrap_or(top));
            if canonical(top)? == self.root {
                return Ok(());
            }
        }
        Err(io::Error::other(
            "git-discovery: Git could not verify the project root",
        ))
    }
}
