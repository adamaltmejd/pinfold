//! The box's read-only `.git` and protected editor config.
//!
//! Host git runs code named in `.git` (hooks, `core.fsmonitor`,
//! `commondir`) and editors run `.vscode/`, `.claude/` and `.idea/` config
//! when the project is opened, so the box gets those paths read-only. A
//! worktree's `.git` is a file, which the runtimes cannot mount read-only,
//! so a worktree is refused. A protected path that is a symlink, or is
//! reached through one, is refused too: the runtime resolves a bind-mount
//! source on the host, and the box can write the project.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::core::plan::Mount;

/// Editor config the host runs on open, always protected.
const ALWAYS_PROTECT: [&str; 3] = [".vscode", ".claude", ".idea"];

/// One run's read-only mounts and the absent protected directories it
/// created for them.
pub struct Git {
    /// The project root the mounts are relative to.
    root: PathBuf,
    /// Read-only mounts at their own absolute paths.
    readonly: Vec<Mount>,
    /// Protected directories that did not exist and were created empty.
    created: Vec<PathBuf>,
}

impl Git {
    /// Prepare the read-only mounts for the project rooted at `root`.
    ///
    /// Refuses a worktree: its `.git` is a file that host git follows, and
    /// the runtimes mount directories only. An absent protected directory is
    /// created empty, so the box cannot create it; [`cleanup`](Self::cleanup)
    /// removes it after the run when it is still empty.
    pub fn prepare(root: &Path, protect: &[String]) -> io::Result<Git> {
        let dot_git = root.join(".git");
        let mut paths = Vec::new();
        match path_kind(&dot_git)? {
            PathKind::Absent => {}
            PathKind::Directory => paths.push(dot_git),
            PathKind::File => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "refusing to run in a git worktree: {} is a file, not a directory",
                        dot_git.display()
                    ),
                ));
            }
            PathKind::Other => return Err(not_real_dir(&dot_git)),
        }
        for name in ALWAYS_PROTECT {
            paths.push(root.join(name));
        }
        for entry in protect {
            paths.push(protected_path(root, entry)?);
        }

        let mut readonly = Vec::new();
        let mut created = Vec::new();
        for path in paths {
            if readonly.iter().any(|mount: &Mount| mount.guest == path) {
                continue;
            }
            match path_kind(&path)? {
                PathKind::Directory => {}
                PathKind::Absent => {
                    // create_dir follows a symlinked parent, so the parent
                    // must be a real directory before anything is created.
                    let parent = path.parent().expect("protected paths have a parent");
                    if !matches!(path_kind(parent)?, PathKind::Directory) {
                        return Err(not_real_dir(&path));
                    }
                    fs::create_dir(&path).map_err(|error| {
                        io::Error::new(
                            error.kind(),
                            format!("create protected directory {}: {error}", path.display()),
                        )
                    })?;
                    created.push(path.clone());
                }
                PathKind::File | PathKind::Other => return Err(not_real_dir(&path)),
            }
            readonly.push(Mount {
                host: path.clone(),
                guest: path,
                readonly: true,
            });
        }
        Ok(Git {
            root: root.to_path_buf(),
            readonly,
            created,
        })
    }

    /// The project root the mounts belong to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The read-only mounts, at their own absolute paths.
    pub fn mounts(&self) -> &[Mount] {
        &self.readonly
    }

    /// Remove the protected directories this run created, if the host left
    /// them empty.
    pub fn cleanup(&self) {
        for path in &self.created {
            let _ = fs::remove_dir(path);
        }
    }
}

/// The absolute path of one project-relative `protect` entry.
fn protected_path(root: &Path, entry: &str) -> io::Result<PathBuf> {
    let path = Path::new(entry);
    let escapes = path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir));
    let path = root.join(path);
    if entry.is_empty() || escapes || path == root {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("protect entry {entry:?} is not a project-relative directory"),
        ));
    }
    Ok(path)
}

/// What is at a protected path.
enum PathKind {
    /// Nothing.
    Absent,
    /// A real directory with no symlink in any component.
    Directory,
    /// A regular file, the worktree `.git` case.
    File,
    /// A symlink, or anything else that is not a real directory.
    Other,
}

/// Classify `path` without following a symlink at its final component. The
/// project root is canonical, so an existing directory whose canonical path
/// differs from `path` has a symlink in some component.
fn path_kind(path: &Path) -> io::Result<PathKind> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(PathKind::File),
        Ok(metadata) if metadata.is_dir() => {
            let real = fs::canonicalize(path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("canonicalize {}: {error}", path.display()),
                )
            })?;
            Ok(if real.as_path() == path {
                PathKind::Directory
            } else {
                PathKind::Other
            })
        }
        Ok(_) => Ok(PathKind::Other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PathKind::Absent),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("stat {}: {error}", path.display()),
        )),
    }
}

/// The refusal for a protected path or `.git` that is not a real directory
/// under the project root. The runtime resolves a bind-mount source on the
/// host, so following a symlink the box planted would mount its target.
fn not_real_dir(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "refusing to run: {} is not a real directory under the project root",
            path.display()
        ),
    )
}
