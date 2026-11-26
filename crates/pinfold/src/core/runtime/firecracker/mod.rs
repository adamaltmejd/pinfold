//! Firecracker on Linux: one VM per box, its VMM in the jail `box up`
//! builds. Completed image roots come from the isolated image service.

pub mod jail;
pub mod preflight;
pub(crate) mod vm;
mod vsock;

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use nix::unistd::Pid;

pub use vm::{Prepared, Vm, prepare};

use crate::core::disk::{Directory, Fill, Seeding};
use crate::core::ownership;
use crate::core::plan::Plan;
use crate::core::process::CommandGuard;
use crate::core::runtime::{
    BoxInfo, BoxStat, DiskInfo, ExecContext, MemoryStat, PidsStat, Runtime, spawn_error,
};
use crate::dirs;

/// The host's absolute deadline for the guest's UNMOUNT answer.
pub(crate) const UNMOUNT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Firecracker, run by pinfold itself.
pub struct Firecracker;

impl Firecracker {
    /// The ownership record of the box `list` names `name`.
    fn record(name: &str) -> io::Result<ownership::Record> {
        ownership::record(name)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no box named {name:?}"))
        })
    }

    fn state(record: &ownership::Record) -> io::Result<Option<VmmState>> {
        if record.retiring {
            return Ok(None);
        }
        match vmm_state(&record.state_dir) {
            Ok(state) => Ok(Some(state)),
            Err(error) => {
                // Retirement is published before the identity file is removed.
                let current = ownership::record(&record.name)?;
                if current.is_none_or(|current| {
                    current.generation != record.generation || current.retiring
                }) {
                    Ok(None)
                } else {
                    Err(error)
                }
            }
        }
    }

    fn info(record: ownership::Record, running: Option<bool>) -> Option<BoxInfo> {
        let described = record.described?;
        Some(BoxInfo {
            memory_limit: Some(vm::memory_mib(described.memory.as_deref()) << 20),
            running: running == Some(true),
            stopped: running == Some(false),
            id: record.name,
            labels: described.labels,
            image_id: described.image_id,
            image_ref: described.image_ref,
            created: described.created,
        })
    }

    fn known_info(record: ownership::Record) -> io::Result<Option<BoxInfo>> {
        let running = match Self::state(&record)? {
            Some(VmmState::Alive) => true,
            Some(VmmState::Dead) => false,
            Some(VmmState::NotStarted) | None => return Ok(None),
        };
        Ok(Self::info(record, Some(running)))
    }
}

impl Runtime for Firecracker {
    fn preflight(&self) -> io::Result<()> {
        let missing = preflight::host_requirements();
        if !missing.is_empty() {
            return Err(io::Error::other(missing.join("; ")));
        }
        Ok(())
    }

    fn finish_start(
        &self,
        name: &str,
        _init: &Path,
        _proxy_socket: Option<&Path>,
        _seeding: Option<&Seeding>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<BoxInfo> {
        // The owner holds this name. Other boxes cannot decide its readiness.
        Self::known_info(Self::record(name)?)?.ok_or_else(|| {
            io::Error::other(format!(
                "the runtime does not list box {name:?} after ready"
            ))
        })
    }

    // An orphan is not our child to reap. Confirm its recorded VMM ended
    // before removing the files it could still use.
    fn down(&self, name: &str) -> io::Result<()> {
        if let Some(record) = ownership::record(name)? {
            match Self::state(&record)? {
                Some(VmmState::NotStarted | VmmState::Dead) | None => {}
                Some(VmmState::Alive) => {
                    return Err(io::Error::other("the orphan VMM is still running"));
                }
            }
        }
        Ok(())
    }

    fn exec(
        &self,
        name: &str,
        _init: &Path,
        context: &ExecContext<'_>,
        argv: &[String],
    ) -> io::Result<ExitStatus> {
        let sockets = super::sockets::Sockets::open(&Self::record(name)?.generation)?;
        let code = vsock::exec(&sockets, context, argv)?;
        Ok(exit_status(code))
    }

    /// `box exec` of this executable, for a caller that pipes its stdio.
    fn exec_command(&self, name: &str, _init: &Path, argv: &[String]) -> std::process::Command {
        let mut command = std::process::Command::new("/proc/self/exe");
        command
            .process_group(0)
            .args(["box", "exec", name, "--"])
            .args(argv);
        command
    }

    fn stat(&self, name: &str) -> io::Result<BoxStat> {
        let record = Self::record(name)?;
        let sockets = super::sockets::Sockets::open(&record.generation)?;
        let memory = record.described.and_then(|described| described.memory);
        let [current, peak, pids, pids_max, oom_kills] = vsock::stat(&sockets)?;
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
        let mut boxes = Vec::new();
        for result in ownership::record_results()? {
            let record = match result {
                Ok(record) => record,
                Err(error) => {
                    eprintln!("pinfold: box inventory: {error}");
                    continue;
                }
            };
            if record.described.is_none() {
                continue;
            }
            let running = match Self::state(&record) {
                Ok(Some(VmmState::NotStarted) | None) => continue,
                Ok(Some(VmmState::Alive)) => Some(true),
                Ok(Some(VmmState::Dead)) => Some(false),
                Err(error) => {
                    eprintln!("pinfold: box inventory {:?}: {error}", record.name);
                    // Unknown liveness must retain this generation's disk claims.
                    None
                }
            };
            if let Some(box_) = Self::info(record, running) {
                boxes.push(box_);
            }
        }
        Ok(boxes)
    }

    fn inspect(&self, name: &str) -> io::Result<Option<BoxInfo>> {
        match ownership::record(name)? {
            Some(record) => Self::known_info(record),
            None => Ok(None),
        }
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
        let made = file.set_len(bytes).and_then(|()| format_disk(&path));
        if let Err(error) = made {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(())
    }

    // create_disk sets root_owner while formatting the image.
    fn own_disks(
        &self,
        _names: &[&str],
        _plan: &Plan,
        _init: &Path,
        _generation: &str,
        _directories: &[Directory],
        _fills: &[Fill],
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        Ok(())
    }

    fn remove_disk(&self, name: &str) -> io::Result<()> {
        crate::core::absent_ok(fs::remove_file(disk_path(name)?))
    }

    /// Firecracker's default cache ignores guest flushes, but each guest
    /// write it completed is already in the image file.
    fn sync_disk(&self, name: &str) -> io::Result<()> {
        fs::File::open(disk_path(name)?)?.sync_all()
    }

    fn describe(&self) -> &'static str {
        "Firecracker (one VM per box, pinned guest kernel)"
    }

    fn version(&self) -> io::Result<String> {
        Ok(crate::core::artifacts::vmm_version().to_owned())
    }
}

/// The format of every disk pinfold creates, as mke2fs's whole profile.
/// Init's trim takes one 128 MiB block group of 4 KiB blocks per range, so
/// no size type may change the block size and no host profile may add
/// `bigalloc`. The features are ones every e2fsprogs since 1.43 knows; an
/// unknown name fails the format on an older host.
const DISK_FORMAT: &str = "\
[defaults]
    base_features = sparse_super,large_file,filetype,resize_inode,dir_index,ext_attr
    default_mntopts = acl,user_xattr
    enable_periodic_fsck = 0
    blocksize = 4096
    inode_size = 256
    inode_ratio = 16384

[fs_types]
    ext4 = {
        features = has_journal,extent,huge_file,flex_bg,metadata_csum,64bit,dir_nlink,extra_isize
    }
";

/// Format the image at `path` with [`DISK_FORMAT`], its root the user's.
/// MKE2FS_CONFIG names pinfold's own profile, so neither the host's
/// mke2fs.conf nor the caller's MKE2FS_CONFIG is read. An empty or missing
/// profile pins nothing: mke2fs then falls back to ext2-like or built-in
/// defaults.
fn format_disk(path: &Path) -> io::Result<()> {
    let mkfs = preflight::mkfs_ext4()?;
    // Beside the image, whose create_new made the name this call's.
    let profile = path.with_extension("mke2fs.conf");
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&profile)?
        .write_all(DISK_FORMAT.as_bytes())?;
    let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
    let ran = Command::new(&mkfs)
        .args(["-q", "-F", "-m", "0", "-E"])
        .arg(format!(
            "lazy_itable_init=1,lazy_journal_init=1,root_owner={uid}:{gid}"
        ))
        .arg(path)
        .env("MKE2FS_CONFIG", &profile)
        .stdin(Stdio::null())
        .output_guarded();
    let _ = fs::remove_file(&profile);
    let ran = ran.map_err(|error| spawn_error(&mkfs.to_string_lossy(), error))?;
    if !ran.status.success() {
        return Err(io::Error::other(format!(
            "{}: {}",
            mkfs.display(),
            String::from_utf8_lossy(&ran.stderr).trim()
        )));
    }
    Ok(())
}

/// A VMM's exit code, or `128+n` for signal `n`, as the exit status a
/// runtime client would report: `exited` names 137 for a SIGKILLed VMM.
fn exit_status(code: i32) -> ExitStatus {
    std::os::unix::process::ExitStatusExt::from_raw((code & 0xff) << 8)
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

pub(crate) fn prepare_vmm(state_dir: &Path) -> io::Result<()> {
    write_identity(state_dir, b"not-started\n")
}

fn record_vmm(state_dir: &Path, pid: Pid) -> io::Result<()> {
    let started = start_time(pid)?.ok_or_else(|| io::Error::other("the VMM ended at once"))?;
    write_identity(
        state_dir,
        format!("{} {started}\n", pid.as_raw()).as_bytes(),
    )
}

fn write_identity(state_dir: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = state_dir.join("vmm.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, vmm_file(state_dir))?;
    fs::File::open(state_dir)?.sync_all()
}

enum VmmState {
    NotStarted,
    Alive,
    Dead,
}

/// An unreadable or incomplete identity never proves resource release.
fn vmm_state(state_dir: &Path) -> io::Result<VmmState> {
    fn unknown(error: impl std::fmt::Display) -> io::Error {
        io::Error::other(format!("unknown VMM liveness: {error}"))
    }
    let text = fs::read_to_string(vmm_file(state_dir)).map_err(unknown)?;
    if text == "not-started\n" {
        return Ok(VmmState::NotStarted);
    }
    let mut fields = text.split_whitespace();
    let (Some(Ok(pid)), Some(Ok(started)), None) = (
        fields.next().map(str::parse::<i32>),
        fields.next().map(str::parse::<u64>),
        fields.next(),
    ) else {
        return Err(unknown("invalid identity"));
    };
    if pid <= 0 || started == 0 || text != format!("{pid} {started}\n") {
        return Err(unknown("invalid identity"));
    }
    Ok(
        if start_time(Pid::from_raw(pid)).map_err(unknown)? == Some(started) {
            VmmState::Alive
        } else {
            VmmState::Dead
        },
    )
}

/// A live process's start time from `/proc/PID/stat`; None for a zombie or
/// no process.
fn start_time(pid: Pid) -> io::Result<Option<u64>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let invalid = || io::Error::other("invalid VMM process identity");
    // Fields after the command, which may hold spaces, start at the state.
    let mut fields = stat
        .rsplit_once(')')
        .ok_or_else(invalid)?
        .1
        .split_whitespace();
    let state = fields.next().ok_or_else(invalid)?;
    if state == "Z" || state == "X" {
        return Ok(None);
    }
    // starttime is field 22; the state is field 3.
    fields
        .nth(18)
        .ok_or_else(invalid)?
        .parse()
        .map(Some)
        .map_err(|_| invalid())
}
