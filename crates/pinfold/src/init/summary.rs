//! Bounded guest observations. The host treats this output as untrusted.

use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::process::{Command, Stdio};

pub const BYTES: usize = 64 << 10;
pub const ENTRIES: usize = 100;

pub struct Entry {
    pub kind: String,
    pub path: String,
    pub branch: Option<String>,
    pub bytes: Option<u64>,
    pub commits: Option<u64>,
    pub added: Option<String>,
    pub deleted: Option<String>,
}

#[derive(Default)]
pub struct Summary {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

fn escaped(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| std::ascii::escape_default(*byte))
        .map(char::from)
        .collect()
}

impl Summary {
    fn encode(&self) -> String {
        let mut output = format!("pinfold-summary-v1\t{}\n", u8::from(self.truncated));
        for entry in &self.entries {
            output.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                entry.kind,
                entry.path,
                entry.bytes.map_or_else(|| "-".into(), |n| n.to_string()),
                entry.commits.map_or_else(|| "-".into(), |n| n.to_string()),
                entry.branch.as_deref().unwrap_or(""),
                entry.added.as_deref().unwrap_or(""),
                entry.deleted.as_deref().unwrap_or("")
            ));
        }
        output
    }

    fn push(&mut self, entry: Entry) {
        if self.entries.len() == ENTRIES || entry.path.len() > 4096 {
            self.truncated = true;
        } else {
            self.entries.push(entry);
        }
    }
}

pub fn run(args: &[String]) -> io::Result<()> {
    let [mode, root] = args else {
        return Err(io::Error::other("summary: expected mode and root"));
    };
    let root = Path::new(root);
    let mut summary = Summary::default();
    match mode.as_str() {
        "scratch" => {
            let dir = fs::OpenOptions::new()
                .read(true)
                .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
                .open(root)?;
            walk(&dir, Path::new(""), &mut summary)?;
        }
        "repository" => repository(root, &mut summary)?,
        _ => return Err(io::Error::other("summary: unknown mode")),
    }
    let mut bytes = summary.encode();
    while bytes.len() > BYTES {
        summary
            .entries
            .pop()
            .ok_or_else(|| io::Error::other("summary: limit"))?;
        summary.truncated = true;
        bytes = summary.encode();
    }
    io::stdout().lock().write_all(bytes.as_bytes())
}

fn walk(dir: &fs::File, relative: &Path, summary: &mut Summary) -> io::Result<()> {
    // A held directory descriptor pins each level. Background guest writes
    // cannot redirect traversal through a replacement symlink into home.
    for entry in fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
        if summary.entries.len() == ENTRIES {
            summary.truncated = true;
            break;
        }
        let entry = entry?;
        let name = entry.file_name();
        let path = relative.join(&name);
        let metadata = fs::symlink_metadata(entry.path())?;
        let child = if metadata.is_dir() {
            let name = std::ffi::CString::new(name.as_bytes()).map_err(io::Error::other)?;
            let fd = unsafe {
                nix::libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    nix::libc::O_RDONLY
                        | nix::libc::O_DIRECTORY
                        | nix::libc::O_NOFOLLOW
                        | nix::libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                let error = io::Error::last_os_error();
                if relative.as_os_str().is_empty()
                    && entry.file_name() == "lost+found"
                    && metadata.uid() == 0
                    && error.kind() == io::ErrorKind::PermissionDenied
                {
                    // It may contain recovered files. An advisory omission
                    // must never authorize automatic disk deletion.
                    summary.truncated = true;
                    continue;
                }
                return Err(error);
            }
            Some(unsafe { fs::File::from_raw_fd(fd) })
        } else {
            None
        };
        let kind = if metadata.is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else {
            "special"
        };
        summary.push(Entry {
            kind: kind.to_string(),
            path: escaped(path.as_os_str().as_bytes()),
            branch: None,
            bytes: metadata.is_file().then_some(metadata.len()),
            commits: None,
            added: None,
            deleted: None,
        });
        if let Some(child) = child {
            if path.components().count() < 32 {
                walk(&child, &path, summary)?;
            } else {
                summary.truncated = true;
            }
        }
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    git_input(root, args, &[])
}

fn git_input(root: &Path, args: &[&str], input: &[u8]) -> io::Result<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("--no-pager")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "status.renames=false",
            "-c",
            "color.ui=false",
        ])
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_DIFF_OPTS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| {
        child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("summary: missing stdin"))?
            .write_all(input)?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("summary: missing pipe"))?
            .take((BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > BYTES {
            return Err(io::Error::other("summary: limit"));
        }
        if !child.wait()?.success() {
            return Err(io::Error::other("summary: git failed"));
        }
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn numstat(record: &[u8]) -> io::Result<(&[u8], &[u8], &[u8])> {
    let mut fields = record.splitn(3, |byte| *byte == b'\t');
    let mut field = || {
        fields
            .next()
            .ok_or_else(|| io::Error::other("summary: invalid stat"))
    };
    Ok((field()?, field()?, field()?))
}

fn repository(root: &Path, summary: &mut Summary) -> io::Result<()> {
    let mut baseline = Vec::new();
    io::stdin()
        .lock()
        .take((4096 * 65 + 1) as u64)
        .read_to_end(&mut baseline)?;
    if baseline.len() > 4096 * 65
        || baseline
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .any(|line| !matches!(line.len(), 40 | 64) || !line.iter().all(u8::is_ascii_hexdigit))
    {
        return Err(io::Error::other("summary: invalid host baseline"));
    }
    // Git's command-line --not does not apply to revisions read from stdin.
    let baseline: Vec<u8> = baseline
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .flat_map(|line| {
            std::iter::once(b'^')
                .chain(line.iter().copied())
                .chain(std::iter::once(b'\n'))
        })
        .collect();
    let branches = git(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/heads/"],
    )?;
    for branch in branches
        .split(|byte| *byte == b'\n')
        .filter(|branch| !branch.is_empty())
    {
        let branch = std::str::from_utf8(branch).map_err(io::Error::other)?;
        let count = git_input(root, &["rev-list", "--count", branch, "--stdin"], &baseline)?;
        let count: u64 = std::str::from_utf8(&count)
            .map_err(io::Error::other)?
            .trim()
            .parse()
            .map_err(io::Error::other)?;
        if count > 0 {
            let short = escaped(
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch)
                    .as_bytes(),
            );
            summary.push(Entry {
                kind: "committed".into(),
                path: escaped(branch.as_bytes()),
                branch: Some(short.clone()),
                bytes: None,
                commits: Some(count),
                added: None,
                deleted: None,
            });
            let stats = git_input(
                root,
                &[
                    "log",
                    "--format=",
                    "--numstat",
                    "-z",
                    "--no-renames",
                    "--no-ext-diff",
                    "--no-textconv",
                    branch,
                    "--stdin",
                ],
                &baseline,
            )?;
            for record in stats
                .split(|byte| *byte == 0)
                .filter(|record| !record.is_empty())
            {
                let record = record.strip_prefix(b"\n").unwrap_or(record);
                let (added, deleted, path) = numstat(record)?;
                summary.push(Entry {
                    kind: "committed".into(),
                    path: escaped(path),
                    branch: Some(short.clone()),
                    bytes: None,
                    commits: None,
                    added: Some(String::from_utf8(added.to_vec()).map_err(io::Error::other)?),
                    deleted: Some(String::from_utf8(deleted.to_vec()).map_err(io::Error::other)?),
                });
            }
        }
        if summary.entries.len() == ENTRIES {
            summary.truncated = true;
            break;
        }
    }
    let status = git(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=all",
        ],
    )?;
    for record in status
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if record.len() < 4 || record[2] != b' ' {
            return Err(io::Error::other("summary: invalid status"));
        }
        summary.push(Entry {
            kind: if &record[..2] == b"??" {
                "untracked"
            } else {
                "tracked"
            }
            .into(),
            path: escaped(&record[3..]),
            branch: None,
            bytes: None,
            commits: None,
            added: None,
            deleted: None,
        });
    }
    // Numstat contains counts and names only. Neither textconv nor external
    // diff helpers may turn a report into execution of repository programs.
    if git(root, &["rev-parse", "--verify", "HEAD"]).is_ok() {
        let stats = git(
            root,
            &[
                "diff",
                "--numstat",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "HEAD",
                "--",
            ],
        )?;
        for record in stats
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            let (added, deleted, path) = numstat(record)?;
            if let Some(entry) = summary
                .entries
                .iter_mut()
                .find(|entry| entry.kind == "tracked" && entry.path == escaped(path))
            {
                entry.added = Some(String::from_utf8(added.to_vec()).map_err(io::Error::other)?);
                entry.deleted =
                    Some(String::from_utf8(deleted.to_vec()).map_err(io::Error::other)?);
            }
        }
    }
    Ok(())
}
