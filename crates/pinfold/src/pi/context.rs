//! The project: its root folder, and whether Git crosses it.

use crate::core::process::CommandGuard;

use std::fs;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::config::{self, ProjectMount};
use crate::core::{crossing, plan::guest_path};
use crate::formatting::terminal_text;
use crate::pi::state::canonical;

pub struct Context {
    pub root: PathBuf,
    pub repository: bool,
}

impl Context {
    /// Discover by host metadata so read-only reports do not require Git.
    /// A repository marker never falls back to a folder project, even if damaged.
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
                Ok(metadata) if metadata.is_file() => return Self::linked(ancestor),
                Ok(_) => return Err(worktree_error()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if cfg!(target_os = "linux") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "not-git: {} is not a git repository; a Linux project is a Git checkout",
                    terminal_text(&cwd.to_string_lossy())
                ),
            ));
        }
        Ok(Self {
            root: cwd,
            repository: false,
        })
    }

    fn linked(worktree: &Path) -> io::Result<Self> {
        if !cfg!(target_os = "macos") {
            return Err(worktree_error());
        }
        // Select the main root by ancestry, never by a writable Git pointer.
        let root = worktree
            .ancestors()
            .skip(1)
            .find(|ancestor| {
                fs::symlink_metadata(ancestor.join(".git")).is_ok_and(|metadata| metadata.is_dir())
            })
            .ok_or_else(worktree_error)?;
        if config::project_mount(root)?.unwrap_or(ProjectMount::Share) != ProjectMount::Share {
            return Err(worktree_error());
        }
        let relative_worktree = worktree.strip_prefix(root).map_err(|_| worktree_error())?;
        let marker = read_pointer(root, &relative_worktree.join(".git"))?;
        let directory = marker
            .strip_prefix(b"gitdir: ")
            .ok_or_else(worktree_error)?;
        let directory = pointer_path(worktree, directory);
        let git = root.join(".git");
        let relative = directory.strip_prefix(&git).map_err(|_| worktree_error())?;
        let mut components = relative.components();
        if components.next() != Some(std::path::Component::Normal("worktrees".as_ref()))
            || !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(worktree_error());
        }
        let common = read_pointer(root, &Path::new(".git").join(relative).join("commondir"))?;
        if pointer_path(&directory, &common) != git {
            return Err(worktree_error());
        }
        let backlink = read_pointer(root, &Path::new(".git").join(relative).join("gitdir"))?;
        if pointer_path(&directory, &backlink) != worktree.join(".git") {
            return Err(worktree_error());
        }
        Ok(Self {
            root: root.to_path_buf(),
            repository: true,
        })
    }

    /// Launches must verify a discovered repository before creating state.
    pub fn verify(&self) -> io::Result<()> {
        if !self.repository {
            return Ok(());
        }
        let output = crossing::git_command(&self.root)
            .args(["rev-parse", "--show-toplevel"])
            .stdin(std::process::Stdio::null())
            .output_guarded()?;
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

fn worktree_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "worktree: only linked worktrees beneath a shared main checkout are supported",
    )
}

fn pointer_path(base: &Path, bytes: &[u8]) -> PathBuf {
    guest_path(&base.join(Path::new(std::ffi::OsStr::from_bytes(bytes))))
}

/// Open every pointer component without following symlinks. Refuse special files
/// and bound data before interpreting paths; no pointer-selected Git is executed.
fn read_pointer(root: &Path, relative: &Path) -> io::Result<Vec<u8>> {
    use nix::fcntl::{OFlag, open, openat};
    use nix::sys::stat::Mode;
    let directory_flags =
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let mut directory = open(root, directory_flags, Mode::empty()).map_err(|_| worktree_error())?;
    let components: Vec<_> = relative.components().collect();
    let (file, parents) = components.split_last().ok_or_else(worktree_error)?;
    for component in parents {
        let std::path::Component::Normal(name) = component else {
            return Err(worktree_error());
        };
        directory = openat(&directory, *name, directory_flags, Mode::empty())
            .map_err(|_| worktree_error())?;
    }
    let std::path::Component::Normal(name) = file else {
        return Err(worktree_error());
    };
    let fd = openat(
        &directory,
        *name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| worktree_error())?;
    let file = fs::File::from(fd);
    if !file.metadata()?.is_file() {
        return Err(worktree_error());
    }
    let mut data = Vec::new();
    file.take(4097).read_to_end(&mut data)?;
    if data.len() > 4096 {
        return Err(worktree_error());
    }
    if data.last() == Some(&b'\n') {
        data.pop();
    }
    if data.is_empty() || data.contains(&0) || data.contains(&b'\n') {
        return Err(worktree_error());
    }
    Ok(data)
}
