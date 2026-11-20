//! The project: its root folder, and whether Git crosses it.

use crate::core::process::CommandGuard;

use std::fs;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::config::ProjectMount;
use crate::core::{crossing, plan::guest_path};
use crate::formatting::terminal_text;
use crate::pi::state::canonical;

pub struct Context {
    pub root: PathBuf,
    pub repository: bool,
    worktree: Option<PathBuf>,
}

impl Context {
    /// Select a root without interpreting pointers or configuration. Admission
    /// checks the owner's roots before validating a candidate's worktree metadata.
    pub(crate) fn candidate(cwd: &Path) -> io::Result<Self> {
        let cwd = canonical(cwd)?;
        for ancestor in cwd.ancestors() {
            let directory = open_directory(ancestor, Path::new(""))?;
            if ancestor.file_name().is_some_and(|name| name == ".git")
                || (matches!(
                    marker(&directory, "HEAD")?,
                    Some(nix::libc::S_IFREG | nix::libc::S_IFLNK)
                ) && matches!(
                    marker(&directory, "objects")?,
                    Some(nix::libc::S_IFDIR | nix::libc::S_IFLNK)
                ) && matches!(
                    marker(&directory, "refs")?,
                    Some(nix::libc::S_IFDIR | nix::libc::S_IFLNK)
                ))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "project-in-git: refusing to run inside a git directory",
                ));
            }
            match marker(&directory, ".git")? {
                Some(nix::libc::S_IFDIR) => {
                    return Ok(Self {
                        root: ancestor.to_path_buf(),
                        repository: true,
                        worktree: None,
                    });
                }
                Some(nix::libc::S_IFREG) if cfg!(target_os = "macos") => {
                    let mut root = None;
                    for parent in ancestor.ancestors().skip(1) {
                        let directory = open_directory(parent, Path::new(""))?;
                        if marker(&directory, ".git")? == Some(nix::libc::S_IFDIR) {
                            root = Some(parent);
                            break;
                        }
                    }
                    let root = root.ok_or_else(worktree_error)?;
                    return Ok(Self {
                        root: root.to_path_buf(),
                        repository: true,
                        worktree: Some(ancestor.to_path_buf()),
                    });
                }
                Some(_) => return Err(worktree_error()),
                None => {}
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
            worktree: None,
        })
    }

    /// Discovery reads metadata only. Disk selection is checked after loading
    /// configuration; it cannot influence which root a pointer selects.
    pub fn discover(cwd: &Path) -> io::Result<Self> {
        let context = Self::candidate(cwd)?;
        context.check_worktree(ProjectMount::Share)?;
        Ok(context)
    }

    pub(crate) fn is_worktree(&self) -> bool {
        self.worktree.is_some()
    }

    pub(crate) fn check_worktree(&self, mount: ProjectMount) -> io::Result<()> {
        let Some(worktree) = self.worktree.as_deref() else {
            return Ok(());
        };
        if mount != ProjectMount::Share {
            return Err(worktree_error());
        }
        let root = &self.root;
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
        Ok(())
    }

    /// Launches must verify a discovered repository before creating state.
    pub fn verify(&self, mount: ProjectMount) -> io::Result<()> {
        self.check_metadata(mount)?;
        if mount == ProjectMount::Share || !self.repository {
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

    /// `verify` without host Git: the worktree, and a share's Git metadata.
    pub(crate) fn check_metadata(&self, mount: ProjectMount) -> io::Result<()> {
        self.check_worktree(mount)?;
        if mount == ProjectMount::Share {
            return self.verify_share();
        }
        Ok(())
    }

    fn verify_share(&self) -> io::Result<()> {
        if share_repository(&self.root)? != self.repository {
            return Err(worktree_error());
        }
        if !self.repository {
            return Ok(());
        }
        let git = self.root.join(".git");
        // A plain .git directory can redirect its common storage too. Never
        // let Git interpret it; open every target beneath the admitted root.
        let mut relative = PathBuf::from(".git");
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..16 {
            if !seen.insert(relative.clone()) {
                return Err(worktree_error());
            }
            open_directory(&self.root, &relative)?;
            let Some(common) = read_optional_pointer(&self.root, &relative.join("commondir"))?
            else {
                return Ok(());
            };
            let target = pointer_path(&self.root.join(&relative), &common);
            if !target.starts_with(&git) {
                return Err(worktree_error());
            }
            relative = target
                .strip_prefix(&self.root)
                .map_err(|_| worktree_error())?
                .to_path_buf();
        }
        Err(worktree_error())
    }
}

/// A share's root Git marker is a plain directory, or absent.
pub(crate) fn share_repository(root: &Path) -> io::Result<bool> {
    let directory = open_directory(root, Path::new(""))?;
    match marker(&directory, ".git")? {
        Some(nix::libc::S_IFDIR) => Ok(true),
        None => Ok(false),
        _ => Err(worktree_error()),
    }
}

fn marker(directory: &std::os::fd::OwnedFd, name: &str) -> io::Result<Option<nix::libc::mode_t>> {
    match nix::sys::stat::fstatat(directory, name, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(metadata) => Ok(Some(metadata.st_mode & nix::libc::S_IFMT)),
        Err(nix::errno::Errno::ENOENT) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn worktree_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "worktree: .git is not a directory; run pinfold from the main checkout",
    )
}

fn pointer_path(base: &Path, bytes: &[u8]) -> PathBuf {
    guest_path(&base.join(Path::new(std::ffi::OsStr::from_bytes(bytes))))
}

fn open_directory(root: &Path, relative: &Path) -> io::Result<std::os::fd::OwnedFd> {
    use nix::fcntl::{OFlag, open, openat};
    use nix::sys::stat::Mode;
    #[cfg(target_os = "linux")]
    let search = OFlag::O_PATH;
    #[cfg(target_os = "macos")]
    let search = OFlag::O_SEARCH;
    let flags = search | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let mut directory = open(root, flags, Mode::empty()).map_err(|_| worktree_error())?;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(worktree_error());
        };
        directory = openat(&directory, name, flags, Mode::empty()).map_err(|_| worktree_error())?;
    }
    Ok(directory)
}

fn read_pointer(root: &Path, relative: &Path) -> io::Result<Vec<u8>> {
    read_optional_pointer(root, relative)?.ok_or_else(worktree_error)
}

/// Refuse special files and bound data before interpreting paths. Only absence
/// is optional: symlinks, malformed pointers and inaccessible files all refuse.
fn read_optional_pointer(root: &Path, relative: &Path) -> io::Result<Option<Vec<u8>>> {
    use nix::fcntl::{OFlag, openat};
    use nix::sys::stat::Mode;
    let parent = relative.parent().ok_or_else(worktree_error)?;
    let name = relative.file_name().ok_or_else(worktree_error)?;
    let directory = open_directory(root, parent)?;
    let fd = match openat(
        &directory,
        name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Err(_) => return Err(worktree_error()),
    };
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
    Ok(Some(data))
}
