//! Firecracker on Linux: one VM per box, its VMM in the jail `box up`
//! builds. Rootless podman builds the images and holds them.

pub mod jail;
pub mod preflight;
mod vm;
mod vsock;

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use nix::unistd::Pid;

pub use vm::{Prepared, Vm, prepare};

use crate::core::artifacts::VMM;
use crate::core::ownership;
use crate::core::runtime::podman::Podman;
use crate::core::runtime::{
    BoxInfo, BoxStat, BuildRequest, DiskInfo, ImageInfo, MemoryStat, PidsStat, Runtime, output,
};
use crate::dirs;

/// Firecracker, run by pinfold itself.
pub struct Firecracker;

impl Firecracker {
    /// The box's 0700 vsock socket directory.
    fn vsock_dir(state_dir: &Path) -> io::Result<PathBuf> {
        let dir = state_dir.join("vsock");
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        Ok(dir)
    }

    /// The state dir of the box `list` names `name`.
    fn state_dir(name: &str) -> io::Result<PathBuf> {
        ownership::record(name)?
            .map(|record| record.state_dir)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no box named {name:?}"))
            })
    }
}

impl Runtime for Firecracker {
    fn preflight(&self) -> io::Result<()> {
        let missing = preflight::host_requirements();
        if !missing.is_empty() {
            return Err(io::Error::other(missing.join("; ")));
        }
        Podman.preflight()
    }

    fn proxy_socket(&self, state_dir: &Path) -> io::Result<PathBuf> {
        Ok(Self::vsock_dir(state_dir)?.join("v.sock_3128"))
    }

    // The VMM ends with its owner's thread (PR_SET_PDEATHSIG), so an owner
    // that is gone left no VM; its state goes with its record.
    fn down(&self, _name: &str) -> io::Result<()> {
        Ok(())
    }

    fn exec(
        &self,
        name: &str,
        _init: &Path,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus> {
        let code = vsock::exec(&Self::state_dir(name)?, tty, workdir, argv)?;
        Ok(super::exit_status(code))
    }

    /// `box exec` of this executable, for a caller that pipes its stdio.
    fn exec_command(
        &self,
        name: &str,
        _init: &Path,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> std::process::Command {
        let mut command = std::process::Command::new(
            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/proc/self/exe")),
        );
        command.args(["box", "exec"]);
        if tty {
            command.arg("--tty");
        } else {
            command.process_group(0);
        }
        if let Some(dir) = workdir {
            command.arg("--workdir").arg(dir);
        }
        command.arg(name).arg("--").args(argv);
        command
    }

    fn stat(&self, name: &str) -> io::Result<BoxStat> {
        let record = ownership::record(name)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no box named {name:?}"))
        })?;
        let memory = record.described.and_then(|described| described.memory);
        let [current, peak, pids, pids_max, oom_kills] = vsock::stat(&record.state_dir)?;
        Ok(BoxStat {
            name: name.to_string(),
            oom_kills,
            memory: MemoryStat {
                current,
                peak,
                limit: Some(vm::memory_mib(memory.as_deref()) << 20),
            },
            pids: PidsStat {
                current: pids,
                limit: pids_max,
            },
        })
    }

    /// pinfold's own claims whose VMM started, running while it lives. No
    /// runtime is asked, so nothing can stall it.
    fn list(&self) -> io::Result<Vec<BoxInfo>> {
        Ok(ownership::records()?
            .into_iter()
            .filter_map(|record| {
                if !vmm_file(&record.state_dir).exists() {
                    return None;
                }
                let described = record.described?;
                Some(BoxInfo {
                    running: vmm_alive(&record.state_dir),
                    id: record.name,
                    labels: described.labels,
                    image_id: described.image_id,
                    image_ref: described.image_ref,
                    created: described.created,
                })
            })
            .collect())
    }

    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        Podman.list_images()
    }

    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>> {
        Podman.resolve_image(reference)
    }

    fn remove_image(&self, reference: &str) -> io::Result<()> {
        Podman.remove_image(reference)
    }

    fn purge_build_cache(&self) -> io::Result<()> {
        Podman.purge_build_cache()
    }

    fn build_cache(&self) -> &'static str {
        Podman.build_cache()
    }

    fn list_disks(&self) -> io::Result<Vec<DiskInfo>> {
        let dir = disks_dir()?;
        let mut disks = Vec::new();
        for entry in dirs::entries(&dir)? {
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".ext4"))
            else {
                continue;
            };
            let metadata = entry.metadata()?;
            disks.push(DiskInfo {
                name: name.to_string(),
                size: metadata.len(),
                allocated: metadata.blocks() * 512,
            });
        }
        Ok(disks)
    }

    /// A sparse ext4 image, mode 0600, its root the user's.
    fn create_disk(&self, name: &str, bytes: u64) -> io::Result<()> {
        let path = disk_path(name)?;
        fs::create_dir_all(path.parent().expect("a disk path has a directory"))?;
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let made = file.set_len(bytes).and_then(|()| {
            let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
            output(&[
                &preflight::mkfs_ext4()
                    .ok_or_else(|| io::Error::other("mkfs: no mkfs.ext4"))?
                    .to_string_lossy(),
                "-q",
                "-F",
                "-m",
                "0",
                "-E",
                &format!("lazy_itable_init=1,lazy_journal_init=1,root_owner={uid}:{gid}"),
                &path.to_string_lossy(),
            ])
        });
        if let Err(error) = made {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(())
    }

    fn remove_disk(&self, name: &str) -> io::Result<()> {
        match fs::remove_file(disk_path(name)?) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Firecracker's default cache ignores guest flushes, but each guest
    /// write it completed is already in the image file.
    fn sync_disk(&self, name: &str) -> io::Result<()> {
        fs::File::open(disk_path(name)?)?.sync_all()
    }

    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>> {
        Podman.build(request)
    }

    fn program(&self) -> &'static str {
        "podman"
    }

    fn name(&self) -> &'static str {
        "Firecracker"
    }

    fn isolation(&self) -> &'static str {
        "one VM per box, pinned guest kernel"
    }

    fn version(&self) -> io::Result<String> {
        Ok(VMM.version.clone())
    }
}

/// `$XDG_STATE_HOME/pinfold/disks`.
fn disks_dir() -> io::Result<PathBuf> {
    Ok(dirs::state_dir()?.join("disks"))
}

fn disk_path(name: &str) -> io::Result<PathBuf> {
    Ok(disks_dir()?.join(format!("{name}.ext4")))
}

/// The file that names a box's VMM: its pid and start time, so a reused
/// pid is never taken for it.
fn vmm_file(state_dir: &Path) -> PathBuf {
    state_dir.join("vmm")
}

fn record_vmm(state_dir: &Path, pid: Pid) -> io::Result<()> {
    let started = start_time(pid).ok_or_else(|| io::Error::other("the VMM ended at once"))?;
    fs::write(vmm_file(state_dir), format!("{} {started}\n", pid.as_raw()))
}

/// Whether the VMM `record_vmm` named still runs.
fn vmm_alive(state_dir: &Path) -> bool {
    let Ok(text) = fs::read_to_string(vmm_file(state_dir)) else {
        return false;
    };
    let mut fields = text.split_whitespace();
    let (Some(Ok(pid)), Some(started)) = (fields.next().map(str::parse::<i32>), fields.next())
    else {
        return false;
    };
    start_time(Pid::from_raw(pid)).is_some_and(|now| now == started)
}

/// A live process's start time from `/proc/PID/stat`; None for a zombie or
/// no process.
fn start_time(pid: Pid) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the command, which may hold spaces, start at the state.
    let mut fields = stat.rsplit_once(')')?.1.split_whitespace();
    let state = fields.next()?;
    if state == "Z" || state == "X" {
        return None;
    }
    // starttime is field 22; the state is field 3.
    fields.nth(18).map(str::to_string)
}
