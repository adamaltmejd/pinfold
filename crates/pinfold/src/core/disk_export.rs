//! Explicit extraction from a read-only guest disk into a new host directory.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::r#box::{Box, Shutdown, Signals, UpError};
use super::plan::{Disk, Plan, valid_name};
use super::{artifacts, runtime, sha256_hex};
use crate::init::export::{ABORT, CONTENT_BYTES, ENTRIES, METADATA_BYTES, PATH_BYTES};
use crate::init::{STREAM_DIR, STREAM_END, STREAM_EXECUTABLE, STREAM_FILE, STREAM_SYMLINK};

/// The caller chooses files to cross; this operation never reads Pi policy.
pub fn run(name: &str, destination: &Path, image: Option<&str>) -> io::Result<i32> {
    if !valid_name(name) {
        return Err(io::Error::other("spec: invalid disk name"));
    }
    destination
        .to_str()
        .ok_or_else(|| io::Error::other("path-not-utf8: export destination"))?;
    let staging = Staging::new(destination)?;
    let nonce = nonce()?;
    let plan = Plan {
        name: format!("export-{}", &nonce[..24]),
        labels: std::collections::BTreeMap::from([(
            super::clean::PROJECT_LABEL.to_string(),
            String::new(),
        )]),
        image: Some(
            image
                .unwrap_or("localhost/pinfold/profile-default:latest")
                .to_string(),
        ),
        cpus: Some(1.0),
        memory: Some("1G".to_string()),
        disks: vec![Disk {
            name: name.to_string(),
            guest: PathBuf::from("/export"),
            size: None,
            from: None,
            readonly: true,
        }],
        ..Plan::default()
    };
    let init = artifacts::init()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let mut signals = Signals::with_hangup()?;
        let mut box_ = match Box::up_existing(&plan, &init, &mut signals).await {
            Ok(box_) => box_,
            Err(UpError::Signal) => return Ok(128 + signals.last().unwrap_or(nix::sys::signal::Signal::SIGTERM) as i32),
            Err(error) => return Err(error.into()),
        };
        let args = ["/proc/self/exe".to_string(), "init".to_string(), "send".to_string(), "/export".to_string()];
        let mut command = runtime::runtime().exec_command(&plan.name, &init, false, None, &args);
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut active_file = None;
        let copied = match tokio::process::Command::from(command).kill_on_drop(true).spawn() {
            Err(error) => Err(error),
            Ok(mut child) => {
                let mut output = child.stdout.take().expect("piped export stdout");
                let transfer = async {
                    receive(&staging.root, &mut output, &mut active_file).await?;
                    if !child.wait().await?.success() {
                        return Err(io::Error::other("failed: disk export helper failed"));
                    }
                    Ok(())
                };
                let result = tokio::select! {
                    result = tokio::time::timeout(Duration::from_secs(300), transfer) =>
                        result.unwrap_or_else(|_| Err(io::Error::other("limit: disk export transfer deadline"))),
                    _ = signals.recv() => Err(io::Error::new(io::ErrorKind::Interrupted, "disk export interrupted")),
                    _ = box_.ended() => Err(io::Error::other("failed: disk export box stopped")),
                };
                if result.is_err() {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
                result
            }
        };
        let down = box_.down(Some(Shutdown::Signal.terminal())).await;
        // A cancelled async file operation can leave a blocking-pool write in
        // flight. Reclaim and close it before removing staging or publishing.
        if let Some(file) = active_file.take() { drop(file.into_std().await); }
        if copied.is_err() && let Err(error) = &down {
            eprintln!("pinfold disk export: teardown failed: {error}");
        }
        if copied.as_ref().is_err_and(|error| error.kind() == io::ErrorKind::Interrupted) {
            down?;
            return Ok(128 + signals.last().unwrap_or(nix::sys::signal::Signal::SIGTERM) as i32);
        }
        copied.map_err(|error| if error.kind() == io::ErrorKind::UnexpectedEof {
            io::Error::other("failed: truncated disk export stream")
        } else { error })?;
        down?;
        let pending = tokio::select! {
            biased;
            signal = signals.recv() => Some(signal),
            _ = std::future::ready(()) => None,
        };
        if let Some(signal) = pending { return Ok(128 + signal as i32); }
        Ok(0)
    });
    runtime.shutdown_background();
    if result.as_ref().is_ok_and(|code| *code == 0) {
        staging.publish()?;
    }
    result
}

fn nonce() -> io::Result<String> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(sha256_hex(bytes))
}

struct Staging {
    parent: File,
    root: File,
    path: PathBuf,
    published: bool,
    name: CString,
    destination: CString,
}

impl Staging {
    fn new(destination: &Path) -> io::Result<Self> {
        let filename = destination
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| io::Error::other("spec: export needs a new destination directory"))?;
        let destination_name = cstring(filename.as_bytes())?;
        let parent_path = fs::canonicalize(
            destination
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let parent = File::open(&parent_path)?;
        let mut metadata = std::mem::MaybeUninit::<nix::libc::stat>::uninit();
        // A symlink is an existing destination too. The dirfd pins its parent.
        if unsafe {
            nix::libc::fstatat(
                parent.as_raw_fd(),
                destination_name.as_ptr(),
                metadata.as_mut_ptr(),
                nix::libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            return Err(io::Error::other(
                "destination-exists: export destination already exists",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error);
        }
        let name = format!(".pinfold-export-{}", nonce()?);
        let staging_name = cstring(name.as_bytes())?;
        checked(unsafe { nix::libc::mkdirat(parent.as_raw_fd(), staging_name.as_ptr(), 0o700) })?;
        let path = parent_path.join(&name);
        let root = match open_dir(&parent, &staging_name) {
            Ok(root) => root,
            Err(error) => {
                let _ = fs::remove_dir(&path);
                return Err(error);
            }
        };
        Ok(Self {
            parent,
            root,
            path,
            published: false,
            name: staging_name,
            destination: destination_name,
        })
    }

    fn publish(mut self) -> io::Result<()> {
        // Plain rename may replace an empty directory. Both platforms offer
        // a no-replace operation, which also closes destination creation races.
        // This promises atomic visibility, not durability through a host crash.
        #[cfg(target_os = "linux")]
        let result = unsafe {
            nix::libc::renameat2(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                self.parent.as_raw_fd(),
                self.destination.as_ptr(),
                nix::libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            nix::libc::renameatx_np(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                self.parent.as_raw_fd(),
                self.destination.as_ptr(),
                nix::libc::RENAME_EXCL,
            )
        };
        checked(result).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                io::Error::other("destination-exists: export destination already exists")
            } else {
                error
            }
        })?;
        self.published = true;
        Ok(())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.published
            && let Err(error) = fs::remove_dir_all(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!("pinfold disk export: private staging cleanup failed");
        }
    }
}

fn cstring(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| io::Error::other("spec: export path contains NUL"))
}

fn checked(result: i32) -> io::Result<i32> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn open_dir(parent: &File, name: &CString) -> io::Result<File> {
    let fd = checked(unsafe {
        nix::libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            nix::libc::O_RDONLY
                | nix::libc::O_DIRECTORY
                | nix::libc::O_NOFOLLOW
                | nix::libc::O_CLOEXEC,
        )
    })?;
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn entry_parent(
    root: &File,
    path: &[u8],
    directories: &BTreeSet<Vec<u8>>,
) -> io::Result<(File, CString)> {
    let parts: Vec<_> = path.split(|byte| *byte == b'/').collect();
    if parts.is_empty()
        || parts.len() > 256
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == b"." || *part == b"..")
    {
        return Err(io::Error::other("spec: export path is not plain"));
    }
    // Exact declared ancestors prevent filesystem case folding or Unicode
    // normalization from merging byte-distinct guest paths.
    for (index, byte) in path.iter().enumerate() {
        if *byte == b'/' && !directories.contains(&path[..index]) {
            return Err(io::Error::other("spec: undeclared export parent"));
        }
    }
    let mut parent = root.try_clone()?;
    for part in &parts[..parts.len() - 1] {
        parent = open_dir(&parent, &cstring(part)?)?;
    }
    Ok((parent, cstring(parts[parts.len() - 1])?))
}

async fn bytes(input: &mut (impl AsyncRead + Unpin), length: usize) -> io::Result<Vec<u8>> {
    if length == 0 || length > PATH_BYTES {
        return Err(io::Error::other("limit: export path length"));
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn receive(
    root: &File,
    input: &mut (impl AsyncRead + Unpin),
    active_file: &mut Option<tokio::fs::File>,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let (mut metadata, mut content) = (0u64, 0u64);
    loop {
        let tag = input.read_u8().await?;
        if tag == ABORT {
            return Err(io::Error::other(match input.read_u8().await? {
                b'l' => "limit: disk export helper limit",
                b's' => "spec: disk export contains a special file",
                _ => "failed: disk export helper failed",
            }));
        }
        if tag == STREAM_END {
            if input
                .read_u8()
                .await
                .is_err_and(|error| error.kind() == io::ErrorKind::UnexpectedEof)
            {
                return Ok(());
            }
            return Err(io::Error::other("spec: trailing export stream"));
        }
        if !matches!(
            tag,
            STREAM_DIR | STREAM_FILE | STREAM_EXECUTABLE | STREAM_SYMLINK
        ) {
            return Err(io::Error::other("spec: invalid export type"));
        }
        if seen.len() as u64 >= ENTRIES {
            return Err(io::Error::other("limit: export entries"));
        }
        let length = input.read_u32().await? as usize;
        let path = bytes(input, length).await?;
        metadata += length as u64;
        if metadata > METADATA_BYTES {
            return Err(io::Error::other("limit: export metadata"));
        }
        if !seen.insert(path.clone()) {
            return Err(io::Error::other("spec: duplicate export path"));
        }
        let (parent, name) = entry_parent(root, &path, &directories)?;
        match tag {
            STREAM_DIR => {
                checked(unsafe { nix::libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) })?;
                let directory = open_dir(&parent, &name)?;
                checked(unsafe { nix::libc::fchmod(directory.as_raw_fd(), 0o755) })?;
                directories.insert(path);
            }
            STREAM_SYMLINK => {
                let length = input.read_u32().await? as usize;
                let target = bytes(input, length).await?;
                metadata += length as u64;
                if metadata > METADATA_BYTES {
                    return Err(io::Error::other("limit: export metadata"));
                }
                checked(unsafe {
                    nix::libc::symlinkat(
                        cstring(&target)?.as_ptr(),
                        parent.as_raw_fd(),
                        name.as_ptr(),
                    )
                })?;
            }
            _ => {
                let size = input.read_u64().await?;
                content = content
                    .checked_add(size)
                    .filter(|size| *size <= CONTENT_BYTES)
                    .ok_or_else(|| io::Error::other("limit: export content"))?;
                let mode = if tag == STREAM_EXECUTABLE {
                    0o755
                } else {
                    0o644
                };
                let fd = checked(unsafe {
                    nix::libc::openat(
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        nix::libc::O_WRONLY
                            | nix::libc::O_CREAT
                            | nix::libc::O_EXCL
                            | nix::libc::O_NOFOLLOW
                            | nix::libc::O_CLOEXEC,
                        mode,
                    )
                })?;
                let file = unsafe { File::from_raw_fd(fd) };
                checked(unsafe { nix::libc::fchmod(file.as_raw_fd(), mode) })?;
                *active_file = Some(tokio::fs::File::from_std(file));
                let mut remaining = size;
                let mut buffer = [0; 16384];
                while remaining > 0 {
                    let length = remaining.min(buffer.len() as u64) as usize;
                    input.read_exact(&mut buffer[..length]).await?;
                    active_file
                        .as_mut()
                        .expect("active export file")
                        .write_all(&buffer[..length])
                        .await?;
                    remaining -= length as u64;
                }
                let file = active_file.as_mut().expect("active export file");
                file.flush().await?;
                file.sync_all().await?;
                drop(
                    active_file
                        .take()
                        .expect("active export file")
                        .into_std()
                        .await,
                );
            }
        }
    }
}
