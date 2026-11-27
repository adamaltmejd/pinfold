//! Firecracker on Linux: one VM per box, its VMM in the jail `box up`
//! builds. Completed image roots come from the isolated image service.

pub mod jail;
pub mod preflight;
pub(crate) mod vm;
mod vsock;

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use nix::fcntl::FlockArg;

pub use vm::{Prepared, Vm, prepare};

use crate::core::disk::{Directory, Fill, Seeding};
use crate::core::ownership;
use crate::core::plan::Plan;
use crate::core::runtime::{
    BoxInfo, BoxStat, DiskInfo, ExecContext, MemoryStat, PidsStat, Runtime, output,
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

    /// Whether the claim's VMM runs; `None` once its retirement is published.
    fn running(record: &ownership::Record) -> io::Result<Option<bool>> {
        if record.retiring {
            return Ok(None);
        }
        vmm_alive(record).map(Some)
    }

    fn info(record: ownership::Record, running: bool) -> Option<BoxInfo> {
        let described = record.described?;
        Some(BoxInfo {
            memory_limit: Some(vm::memory_mib(described.memory.as_deref()) << 20),
            running,
            stopped: !running,
            id: record.name,
            labels: described.labels,
            image_id: described.image_id,
            image_ref: described.image_ref,
            created: described.created,
        })
    }

    fn known_info(record: ownership::Record) -> io::Result<Option<BoxInfo>> {
        Ok(Self::running(&record)?.and_then(|running| Self::info(record, running)))
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
        Self::known_info(Self::record(name)?)?
            .filter(|box_| box_.running)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "the runtime does not list box {name:?} running after ready"
                ))
            })
    }

    // An orphan is not our child to reap. Confirm its VMM ended before
    // removing the files it could still use.
    fn down(&self, name: &str) -> io::Result<()> {
        if let Some(record) = ownership::record(name)?
            && Self::running(&record)? == Some(true)
        {
            return Err(io::Error::other("the orphan VMM is still running"));
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

    /// pinfold's own described claims, running while their VMM lives. No
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
            let running = match Self::running(&record) {
                Ok(None) => continue,
                Ok(Some(running)) => running,
                Err(error) => {
                    eprintln!("pinfold: box inventory {:?}: {error}", record.name);
                    // A VMM that cannot be probed may still write its disks.
                    true
                }
            };
            if let Some(box_) = Self::info(record, running) {
                boxes.push(box_);
            }
        }
        Ok(boxes)
    }

    fn inspect(&self, name: &str) -> io::Result<Option<BoxInfo>> {
        ownership::record(name)?.map_or(Ok(None), Self::known_info)
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
                &preflight::mkfs_ext4()?.to_string_lossy(),
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

/// Whether the claim's VMM runs. Each VMM holds every image it writes
/// exclusively locked for its life (`jail::spawn`): a box's VMM its
/// writable disks and `/tmp` image, a build job's its work disk. A missing
/// image is held by none.
fn vmm_alive(record: &ownership::Record) -> io::Result<bool> {
    let disks = record
        .disks
        .iter()
        .filter(|disk| !disk.readonly)
        .map(|disk| disk_path(&disk.name))
        .collect::<io::Result<Vec<_>>>()?;
    for image in disks
        .into_iter()
        .chain([vm::tmp_image(&record.state_dir)])
        .chain(crate::core::image::linux::work_images(&record.state_dir))
    {
        let file = match fs::File::open(&image) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        // A shared probe never refuses another probe.
        if ownership::try_flock(file, FlockArg::LockSharedNonblock)?.is_none() {
            return Ok(true);
        }
    }
    Ok(false)
}
