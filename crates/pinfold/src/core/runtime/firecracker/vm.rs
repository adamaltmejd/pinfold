//! One box's VM: what a start prepares, the boot handshake, and `down`.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::process::Listener as UnixListener;

use super::super::exec_transport::put;
use super::jail;
use super::vsock;
use crate::core::artifacts;
use crate::core::disk::{self, Directory, Fill, Seeding};
use crate::core::image::{ImageService, linux};
use crate::core::plan::Plan;
use crate::core::runtime::output;
use crate::core::runtime::sockets::Sockets;
use crate::formatting::terminal_text;
use crate::init::exec_wire::{FAILED, read_frame, write_frame};
use crate::init::firecracker::{BOOT, READY};

/// Exactly these: no secret ever reaches a kernel command line.
const BOOT_ARGS: &str = "console=ttyS0 reboot=k panic=1 pci=off init=/pinfold -- init boot";
/// The guest's CID; the host is always 2.
const GUEST_CID: u32 = 3;
/// The VM size when the spec names none, Apple's defaults.
const DEFAULT_VCPUS: u32 = 4;
const DEFAULT_MIB: u64 = 1024;

/// The VM's memory for a validated spec `memory`.
pub fn memory_mib(memory: Option<&str>) -> u64 {
    memory
        .and_then(crate::core::plan::memory_mib)
        .unwrap_or(DEFAULT_MIB)
}
/// OCI runtimes' PATH for an image that sets none.
const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// What [`prepare`] made for one start.
pub struct Prepared {
    state_dir: PathBuf,
    sockets: Sockets,
    vmm: artifacts::Vmm,
    root_disk: PathBuf,
    init_disk: PathBuf,
    /// Each mount's read-only image and its guest path.
    mounts: Vec<(PathBuf, PathBuf)>,
    /// The image's `Env`, the defaults under the spec's.
    image_env: Vec<(OsString, OsString)>,
    generated: Vec<PathBuf>,
}

/// Install the VMM, check that the image tag still names `image_id`, and
/// make the root disk, this start's init disk and the mounts' images.
pub fn prepare(
    plan: &Plan,
    state_dir: &Path,
    sockets: &Sockets,
    image_id: &str,
    cancel: &AtomicBool,
    generated: &[PathBuf],
) -> io::Result<Prepared> {
    let cancelled = || {
        if cancel.load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "startup cancelled",
            ))
        } else {
            Ok(())
        }
    };
    sockets.validate()?;
    let vmm = artifacts::vmm(Some(cancel))?;
    let reference = plan
        .image
        .as_deref()
        .expect("box up resolves the profile's image before the runtime runs");
    if !linux::Linux
        .resolve_image(reference)?
        .is_ok_and(|image| image.id == image_id)
    {
        return Err(io::Error::other(
            "image-changed: box started a different image",
        ));
    }
    cancelled()?;
    let (root_disk, image_env) = linux::catalog::root(image_id)?;
    cancelled()?;
    let init_disk = state_dir.join("init.ext4");
    write_init_disk(&init_disk)?;
    let mut mounts = Vec::new();
    for (index, mount) in plan.mounts.iter().enumerate() {
        cancelled()?;
        let disk = state_dir.join(format!("mount{index}.ext4"));
        mount_image(&mount.host, &disk)?;
        mounts.push((disk, mount.guest.clone()));
    }
    Ok(Prepared {
        state_dir: state_dir.to_path_buf(),
        sockets: sockets.clone(),
        vmm,
        root_disk,
        init_disk,
        mounts,
        generated: generated.to_vec(),
        image_env: image_env
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
    })
}

/// Write a read-only ext4 init disk at `disk` holding this executable as
/// `/pinfold`, and the empty `/dev` and `/mnt` PID 1 mounts on: a Linux
/// CLI's init for a box, and the guest kernel's root.
pub(crate) fn write_init_disk(disk: &Path) -> io::Result<()> {
    let tree = disk.with_extension("tree");
    let _ = fs::remove_dir_all(&tree);
    fs::create_dir(&tree)?;
    let made = (|| {
        fs::create_dir(tree.join("dev"))?;
        fs::create_dir(tree.join("mnt"))?;
        let init = tree.join("pinfold");
        fs::copy(std::env::current_exe()?, &init)?;
        fs::set_permissions(&init, fs::Permissions::from_mode(0o755))?;
        mount_image(&tree, disk)
    })();
    let _ = fs::remove_dir_all(&tree);
    made
}

/// Copy the host directory `host` into a new read-only ext4 image at
/// `disk`, sized to the tree.
pub(crate) fn mount_image(host: &Path, disk: &Path) -> io::Result<()> {
    let (kib, entries) = tree_size(host)?;
    let _ = fs::remove_file(disk);
    // 10% and 16 MiB for metadata, an inode per entry, as the root disk.
    output(&[
        &super::preflight::mkfs_ext4()?.to_string_lossy(),
        "-q",
        "-F",
        "-m",
        "0",
        "-O",
        "^has_journal",
        "-N",
        &(entries + 1024).to_string(),
        "-d",
        &host.to_string_lossy(),
        &disk.to_string_lossy(),
        &format!("{}k", kib + kib / 10 + entries / 4 + 16384),
    ])
    .map(drop)
}

/// A tree's KiB in 4 KiB blocks, and its entries.
fn tree_size(dir: &Path) -> io::Result<(u64, u64)> {
    let (mut kib, mut entries) = (4, 1);
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            let (inner_kib, inner_entries) = tree_size(&entry.path())?;
            kib += inner_kib;
            entries += inner_entries;
        } else {
            kib += metadata.len().div_ceil(4096) * 4;
            entries += 1;
        }
    }
    Ok((kib, entries))
}

/// A box's VM, owned by its `box up`.
pub struct Vm {
    vmm: jail::Vmm,
    sockets: Sockets,
    /// Until the guest dials it, then closed.
    boot: Option<(UnixListener, Boot)>,
    handshake: Option<tokio::task::JoinHandle<io::Result<()>>>,
    boot_cancel: Arc<AtomicBool>,
}

/// What the boot connection carries.
struct Boot {
    request: Vec<u8>,
    fills: Vec<Fill>,
    seeding: Option<Seeding>,
    env: Vec<(OsString, OsString)>,
}

impl Vm {
    /// Write the VM configuration and start the VMM in its jail. The guest
    /// dials the boot listener; [`Vm::ready`] answers it.
    pub fn up(
        prepared: Prepared,
        plan: &Plan,
        proxy_socket: Option<&Path>,
        fills: &[Fill],
        seeding: Option<&Seeding>,
        directories: &[Directory],
    ) -> io::Result<Vm> {
        let state_dir = prepared.state_dir;
        let sockets = prepared.sockets;
        let vsock_directory = sockets.vsock_directory()?;
        let listener = sockets.bind_boot()?;
        listener.set_nonblocking(true)?;
        let listener = UnixListener::from_std(listener)?;

        // vda is the init disk, vdb the root disk, then the box's disks and
        // the mounts' images in this order.
        let mut drives: Vec<(PathBuf, bool)> = Vec::new();
        let mut attachments = Vec::new();
        for disk in &plan.disks {
            drives.push((super::disk_path(&disk.name)?, !disk.readonly));
            let fresh = !disk.readonly && disk::unfilled(&disk.name)?;
            attachments.push((
                crate::core::plan::guest_path(&disk.guest),
                !disk.readonly,
                fresh,
                false,
            ));
        }
        for (image, guest) in &prepared.mounts {
            drives.push((image.clone(), false));
            let guest = crate::core::plan::guest_path(guest);
            let deferred = prepared.generated.contains(&guest);
            attachments.push((guest, false, false, deferred));
        }
        let device = |index: usize| format!("vd{}", char::from(b'a' + index as u8));

        let config = config(plan, &drives);
        let config_path = state_dir.join("config.json");
        fs::write(&config_path, config.to_string())?;

        let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
        let mut request = Vec::new();
        request.extend(uid.as_raw().to_be_bytes());
        request.extend(gid.as_raw().to_be_bytes());
        request.push(u8::from(proxy_socket.is_some()));
        put(&mut request, device(1).as_bytes());
        request.extend((attachments.len() as u32).to_be_bytes());
        for (index, (guest, writable, fresh, deferred)) in attachments.iter().enumerate() {
            put(&mut request, device(index + 2).as_bytes());
            put(&mut request, guest.as_os_str().as_bytes());
            request.push(u8::from(*writable));
            request.push(u8::from(*fresh));
            request.push(u8::from(*deferred));
        }
        // Population precedes nested attachments, so its streams follow the
        // same stable parent-first order as the guest's mount traversal.
        let mut fills = fills.to_vec();
        for fill in &mut fills {
            fill.guest = crate::core::plan::guest_path(&fill.guest);
        }
        fills.sort_by_key(|fill| fill.guest.components().count());
        let filled = fills
            .iter()
            .map(|fill| (fill.guest.clone(), false))
            .chain(seeding.map(|seeding| (crate::core::plan::guest_path(&seeding.guest), true)));
        let filled: Vec<_> = filled.collect();
        request.extend((filled.len() as u32).to_be_bytes());
        for (guest, missing) in filled {
            put(&mut request, guest.as_os_str().as_bytes());
            request.push(u8::from(missing));
        }
        request.extend((directories.len() as u32).to_be_bytes());
        for directory in directories {
            put(&mut request, directory.disk.as_os_str().as_bytes());
            put(&mut request, directory.relative.as_os_str().as_bytes());
        }

        let root = state_dir.join("jail");
        fs::create_dir_all(&root)?;
        let vmm = jail::spawn(
            &jail::Jail {
                root: &root,
                firecracker: &prepared.vmm.firecracker,
                kernel: &prepared.vmm.kernel,
                root_disk: &prepared.root_disk,
                init_disk: &prepared.init_disk,
                disks: &drives,
                vsock: vsock_directory,
                config: &config_path,
            },
            &state_dir,
        )?;
        Ok(Vm {
            vmm,
            boot: Some((
                listener,
                Boot {
                    request,
                    fills,
                    seeding: seeding.cloned(),
                    env: env(plan, prepared.image_env),
                },
            )),
            sockets,
            handshake: None,
            boot_cancel: Arc::default(),
        })
    }

    /// Answer the guest's boot: BOOT, the fills' streams, then READY or
    /// FAILED; then the box's environment over the exec port.
    pub async fn ready(&mut self) -> Result<(), String> {
        let (listener, boot) = self.boot.take().ok_or("the box already booted")?;
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            _ = self.vmm.wait() => return Err("box exited before ready".to_string()),
        };
        drop(listener);
        let (stream, _) = accepted.map_err(|error| format!("boot connection: {error}"))?;
        let stream = stream
            .into_std()
            .and_then(|stream| stream.set_nonblocking(false).map(|()| stream))
            .map_err(|error| format!("boot connection: {error}"))?;
        let sockets = self.sockets.clone();
        let cancel = self.boot_cancel.clone();
        self.handshake = Some(tokio::task::spawn_blocking(move || {
            boot.run(stream, &sockets, &cancel)
        }));
        let result = tokio::select! {
            booted = self.handshake.as_mut().expect("retained BOOT worker") => {
                booted.map_err(|error| error.to_string()).and_then(|result| result.map_err(|error| error.to_string()))
            },
            _ = self.vmm.wait() => {
                let _ = self.cancel_boot().await;
                Err("box exited before ready".to_string())
            },
        };
        self.handshake.take();
        result
    }

    pub fn exited(&mut self) -> io::Result<bool> {
        self.vmm.exited()
    }

    /// Wait until the VM ends on its own or is stopped.
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.vmm.wait().await.map(super::exit_status);
        let worker = self.cancel_boot().await;
        status.and_then(|status| worker.map(|()| status))
    }

    async fn cancel_boot(&mut self) -> io::Result<()> {
        self.boot_cancel.store(true, Ordering::Release);
        self.boot.take();
        if let Some(worker) = self.handshake.as_mut() {
            let joined = worker.await;
            self.handshake.take();
            let _ = joined.map_err(io::Error::other)?;
        }
        Ok(())
    }

    /// Stop a start that ended before ready, as [`Vm::down`].
    pub async fn abort(mut self, name: &str) -> (io::Result<bool>, io::Result<ExitStatus>) {
        let clean = self.down(name).await;
        (clean, self.wait().await)
    }

    /// Unmount the box's disks through PID 1, then end the VMM. True when
    /// the guest synced and unmounted them.
    pub async fn down(&mut self, _name: &str) -> io::Result<bool> {
        let worker = self.cancel_boot().await;
        if !self.vmm.running() {
            self.vmm.wait().await?;
            return worker.map(|()| false);
        }
        let sockets = self.sockets.clone();
        let clean = matches!(
            tokio::task::spawn_blocking(move || vsock::unmount(&sockets)).await,
            Ok(Ok(()))
        );
        self.vmm.kill()?;
        self.vmm.wait().await?;
        worker.map(|()| clean)
    }
}

impl Boot {
    fn run(
        self,
        stream: std::os::unix::net::UnixStream,
        sockets: &Sockets,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        stream.set_nonblocking(true)?;
        let mut transport = super::super::exec_transport::Cancellable::startup(&stream, cancel);
        let sent = (|| {
            write_frame(&mut transport, BOOT, &self.request)?;
            for fill in &self.fills {
                disk::send(fill, &mut transport)?;
            }
            if let Some(seeding) = &self.seeding {
                disk::send_seeds(&seeding.relative, &seeding.seeds, &mut transport)?;
            }
            Ok::<_, io::Error>(())
        })();
        // A guest that failed early has stopped reading; its FAILED says why.
        let answer = read_frame(&mut transport);
        match answer {
            Ok((READY, _)) => sent?,
            Ok((FAILED, text)) => {
                return Err(io::Error::other(terminal_text(&String::from_utf8_lossy(
                    &text,
                ))));
            }
            Ok((kind, _)) => {
                return Err(io::Error::other(format!(
                    "the box answered boot with frame {kind}"
                )));
            }
            Err(error) => {
                sent?;
                return Err(io::Error::other(format!(
                    "box exited before ready: {error}"
                )));
            }
        }
        drop(stream);
        vsock::set_env(sockets, &self.env, cancel)
    }
}

/// The VM configuration, read by Firecracker inside the jail: paths are
/// the jail's.
pub(crate) fn config(plan: &Plan, drives: &[(PathBuf, bool)]) -> serde_json::Value {
    let drive = |id: String, path: &str, root: bool, read_only: bool| {
        serde_json::json!({
            "drive_id": id,
            "path_on_host": path,
            "is_root_device": root,
            "is_read_only": read_only,
        })
    };
    let mut all = vec![
        drive("init".into(), jail::INIT_DISK, true, true),
        drive("root".into(), jail::ROOT_DISK, false, true),
    ];
    for (index, (_, writable)) in drives.iter().enumerate() {
        all.push(drive(
            format!("disk{index}"),
            &jail::disk(index),
            false,
            !writable,
        ));
    }
    let vcpus = plan.cpus.map_or(DEFAULT_VCPUS, |cpus| cpus.ceil() as u32);
    let mib = memory_mib(plan.memory.as_deref());
    serde_json::json!({
        "boot-source": {
            "kernel_image_path": jail::KERNEL,
            "boot_args": BOOT_ARGS,
        },
        "drives": all,
        "machine-config": {
            "vcpu_count": vcpus,
            "mem_size_mib": mib,
        },
        "vsock": {
            "guest_cid": GUEST_CID,
            "uds_path": jail::VSOCK_PATH,
        },
    })
}

/// The box's environment: the image's `Env`, under the spec's, under the
/// always-applied variables.
fn env(plan: &Plan, image_env: Vec<(OsString, OsString)>) -> Vec<(OsString, OsString)> {
    let mut vars: std::collections::BTreeMap<OsString, OsString> = image_env.into_iter().collect();
    vars.entry("PATH".into())
        .or_insert_with(|| DEFAULT_PATH.into());
    vars.extend(
        crate::core::runtime::guest_vars(plan)
            .into_iter()
            .map(|(name, value)| (name.into(), value)),
    );
    vars.into_iter().collect()
}
