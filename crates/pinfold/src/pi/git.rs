//! The box's read-only `.git` and protected editor config.
//!
//! Host git runs code named in `.git` (hooks, `core.hooksPath`,
//! `core.fsmonitor`, `commondir`) and editors run `.vscode/`, `.claude/`
//! and `.idea/` config when the project is opened, so the box gets those
//! paths read-only. A worktree's `.git` is a file, which the runtimes
//! cannot mount read-only, so a worktree is refused. A protected path that
//! is a symlink, or is reached through one, is refused too: the runtime
//! resolves a bind-mount source on the host, and the box can write the
//! project.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::core::plan::Mount;
use crate::pi::state::canonical;

/// Editor config the host runs on open, always protected.
const ALWAYS_PROTECT: [&str; 3] = [".vscode", ".claude", ".idea"];

/// One run's read-only mounts and the absent protected directories it
/// created for them.
pub struct Git {
    /// Read-only mounts at their own absolute paths.
    pub(crate) readonly: Vec<Mount>,
    /// Protected directories that did not exist and were created empty.
    created: Vec<PathBuf>,
}

impl Git {
    /// Prepare the read-only mounts for the project rooted at `root`.
    ///
    /// An absent protected directory is created empty, so the box cannot
    /// create it; [`cleanup`](Self::cleanup) removes it after the run when it
    /// is still empty.
    pub fn prepare(root: &Path, protect: &[String]) -> io::Result<Git> {
        let dot_git = root.join(".git");
        let mut paths = BTreeSet::new();
        match path_kind(&dot_git)? {
            PathKind::Absent => {}
            PathKind::Directory => {
                paths.insert(dot_git);
                if let Some(hooks) = hooks_path(root)? {
                    paths.insert(hooks);
                }
            }
            PathKind::Other => return Err(not_real_dir(&dot_git)),
        }
        for name in ALWAYS_PROTECT {
            paths.insert(root.join(name));
        }
        for entry in protect {
            let what = format!("protect entry {entry:?}");
            paths.insert(protected_path(root, Path::new(entry), &what)?);
        }

        let mut readonly = Vec::new();
        let mut created = Vec::new();
        for path in paths {
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
                PathKind::Other => return Err(not_real_dir(&path)),
            }
            readonly.push(Mount {
                host: path.clone(),
                guest: path,
                readonly: true,
            });
        }
        Ok(Git { readonly, created })
    }

    /// Remove the protected directories this run created, if the host left
    /// them empty.
    pub fn cleanup(&self) {
        for path in &self.created {
            let _ = fs::remove_dir(path);
        }
    }
}

/// The protected path of host git's hooks directory, when `core.hooksPath`
/// puts it inside the project and outside `.git`.
///
/// Host git runs the hooks there on the next commit. git resolves the value
/// itself (`rev-parse --git-path hooks`): the config file has quoting,
/// escapes, includes and case-insensitive keys a hand parser would get
/// wrong, and a global config that points inside the project counts too. A
/// relative value is relative to the project root, as git takes it for a
/// non-bare repository; an absolute value outside the project cannot be
/// reached by the box, so it adds nothing.
fn hooks_path(root: &Path) -> io::Result<Option<PathBuf>> {
    let value = git(root, &["rev-parse", "--git-path", "hooks"])?;
    let spelled = Path::new(&value);
    let relative = match spelled.strip_prefix(root) {
        Ok(relative) => relative,
        Err(_) if spelled.is_absolute() => return Ok(None),
        Err(_) => spelled,
    };
    let path = protected_path(root, relative, &format!("core.hooksPath {value:?}"))?;
    // `.git` is already mounted read-only.
    Ok((!path.starts_with(root.join(".git"))).then_some(path))
}

/// The absolute path of `relative` under `root`, refused when it is the root
/// or leaves it; `what` names it in the refusal.
fn protected_path(root: &Path, relative: &Path, what: &str) -> io::Result<PathBuf> {
    let escapes = relative
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir));
    let path = root.join(relative);
    if escapes || path == root {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} is not a project-relative directory"),
        ));
    }
    Ok(path)
}

/// Run git in `dir` and return its stdout without the final newline.
pub(crate) fn git(dir: &Path, args: &[&str]) -> io::Result<String> {
    let command = format!("git -C {} {}", dir.display(), args.join(" "));
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("run {command}: {error}")))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut stdout = String::from_utf8(output.stdout).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{command} printed non-UTF-8 output: {error}"),
        )
    })?;
    if stdout.ends_with('\n') {
        stdout.pop();
    }
    Ok(stdout)
}

/// What is at a protected path.
enum PathKind {
    /// Nothing.
    Absent,
    /// A real directory with no symlink in any component.
    Directory,
    /// A symlink, or anything else that is not a real directory.
    Other,
}

/// Classify `path` without following a symlink at its final component. The
/// project root is canonical, so an existing directory whose canonical path
/// differs from `path` has a symlink in some component.
fn path_kind(path: &Path) -> io::Result<PathKind> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            let real = canonical(path)?;
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
/// under the project root.
fn not_real_dir(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "protected-path-invalid: {} is not a real directory under the project root",
            path.display()
        ),
    )
}
