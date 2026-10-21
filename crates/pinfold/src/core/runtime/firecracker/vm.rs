//! One box's VM: what a start prepares, the boot handshake, and `down`.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::net::UnixListener;

use super::{Firecracker, jail, vsock};
use crate::core::disk::{self, Fill, Seeding};
use crate::core::plan::Plan;
use crate::core::runtime::{output, podman::Podman};
use crate::core::{artifacts, image};
use crate::dirs;
use crate::init::firecracker::{BOOT, BOOT_PORT, FAILED, READY, read_frame, write_frame};

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
    vmm: artifacts::Vmm,
    root_disk: PathBuf,
    init_disk: PathBuf,
    /// Each mount's read-only image and its guest path.
    mounts: Vec<(PathBuf, PathBuf)>,
    /// The image's `Env`, the defaults under the spec's.
    image_env: Vec<(OsString, OsString)>,
}

/// Install the VMM, check that the image tag still names `image_id`, and
/// make the root disk, this start's init disk and the mounts' images.
pub fn prepare(
    plan: &Plan,
    state_dir: &Path,
    image_id: &str,
    cancel: &AtomicBool,
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
    let vmm = artifacts::vmm(Some(cancel))?;
    let reference = plan
        .image
        .as_deref()
        .expect("box up resolves the profile's image before the runtime runs");
    if !Podman
        .resolve_image(reference)?
        .is_ok_and(|image| image.id == image_id)
    {
        return Err(io::Error::other(
            "image-changed: box started a different image",
        ));
    }
    cancelled()?;
    let root_disk = image::root_disk(image_id)?;
    cancelled()?;
    let init_disk = artifacts::init_disk()?;
    let mut mounts = Vec::new();
    for (index, mount) in plan.mounts.iter().enumerate() {
        cancelled()?;
        let disk = match artifacts::harness_disk(&mount.host)? {
            Some(disk) => {
                dirs::write_once(&disk, |staging| mount_image(&mount.host, staging))?;
                disk
            }
            None => {
                let disk = state_dir.join(format!("mount{index}.ext4"));
                mount_image(&mount.host, &disk)?;
                disk
            }
        };
        mounts.push((disk, mount.guest.clone()));
    }
    Ok(Prepared {
        state_dir: state_dir.to_path_buf(),
        vmm,
        root_disk,
        init_disk,
        mounts,
        image_env: image_env(image_id)?,
    })
}

/// Copy the host directory `host` into a new read-only ext4 image at
/// `disk`, sized to the tree.
fn mount_image(host: &Path, disk: &Path) -> io::Result<()> {
    let (kib, entries) = tree_size(host)?;
    let _ = fs::remove_file(disk);
    // 10% and 16 MiB for metadata, an inode per entry, as the root disk.
    output(&[
        &image::mkfs_ext4()?.to_string_lossy(),
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

/// The image's config `Env`.
fn image_env(image_id: &str) -> io::Result<Vec<(OsString, OsString)>> {
    let json = output(&[
        "podman",
        "image",
        "inspect",
        "--format",
        "{{json .Config.Env}}",
        image_id,
    ])?;
    let env: Option<Vec<String>> = serde_json::from_slice(&json)
        .map_err(|error| io::Error::other(format!("podman image inspect: {error}")))?;
    Ok(env
        .unwrap_or_default()
        .into_iter()
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            Some((name.into(), value.into()))
        })
        .collect())
}

/// A box's VM, owned by its `box up`.
pub struct Vm {
    vmm: jail::Vmm,
    state_dir: PathBuf,
    /// Until the guest dials it, then closed and unlinked.
    boot: Option<(UnixListener, Boot)>,
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
    ) -> io::Result<Vm> {
        let state_dir = prepared.state_dir;
        let vsock_dir = Firecracker::vsock_dir(&state_dir)?;
        if let Some(socket) = proxy_socket {
            fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
        }
        let boot_socket = vsock_dir.join(format!("v.sock_{BOOT_PORT}"));
        let _ = fs::remove_file(&boot_socket);
        let listener = UnixListener::bind(&boot_socket)?;

        // vda is the init disk, vdb the root disk, then the box's disks and
        // the mounts' images in this order.
        let mut drives: Vec<(PathBuf, bool)> = Vec::new();
        let mut attachments: Vec<(&Path, bool)> = Vec::new();
        for disk in &plan.disks {
            drives.push((super::disk_path(&disk.name)?, !disk.readonly));
            attachments.push((&disk.guest, !disk.readonly));
        }
        for (image, guest) in &prepared.mounts {
            drives.push((image.clone(), false));
            attachments.push((guest, false));
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
        for (index, (guest, writable)) in attachments.iter().enumerate() {
            put(&mut request, device(index + 2).as_bytes());
            put(&mut request, guest.as_os_str().as_bytes());
            request.push(u8::from(*writable));
        }
        let filled = fills
            .iter()
            .map(|fill| (&fill.guest, false))
            .chain(seeding.map(|seeding| (&seeding.guest, true)));
        let filled: Vec<_> = filled.collect();
        request.extend((filled.len() as u32).to_be_bytes());
        for (guest, missing) in filled {
            put(&mut request, guest.as_os_str().as_bytes());
            request.push(u8::from(missing));
        }

        let root = state_dir.join("jail");
        fs::create_dir_all(&root)?;
        let vmm = jail::spawn(&jail::Jail {
            root: &root,
            firecracker: &prepared.vmm.firecracker,
            kernel: &prepared.vmm.kernel,
            root_disk: &prepared.root_disk,
            init_disk: &prepared.init_disk,
            disks: &drives,
            vsock: &vsock_dir,
            config: &config_path,
        })?;
        super::record_vmm(&state_dir, vmm.pid)?;
        Ok(Vm {
            vmm,
            boot: Some((
                listener,
                Boot {
                    request,
                    fills: fills.to_vec(),
                    seeding: seeding.cloned(),
                    env: env(plan, prepared.image_env),
                },
            )),
            state_dir,
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
        let boot_socket =
            vsock::socket(&self.state_dir).with_file_name(format!("v.sock_{BOOT_PORT}"));
        let _ = fs::remove_file(boot_socket);
        let (stream, _) = accepted.map_err(|error| format!("boot connection: {error}"))?;
        let stream = stream
            .into_std()
            .and_then(|stream| stream.set_nonblocking(false).map(|()| stream))
            .map_err(|error| format!("boot connection: {error}"))?;
        let state_dir = self.state_dir.clone();
        let handshake = tokio::task::spawn_blocking(move || boot.run(stream, &state_dir));
        tokio::select! {
            booted = handshake => booted.map_err(|error| error.to_string())?.map_err(|error| error.to_string()),
            _ = self.vmm.wait() => Err("box exited before ready".to_string()),
        }
    }

    /// Wait until the VM ends on its own or is stopped.
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.vmm.wait().await.map(super::super::exit_status)
    }

    /// Stop a start that ended before ready, as [`Vm::down`].
    pub async fn abort(mut self, name: &str) -> (io::Result<bool>, io::Result<ExitStatus>) {
        let clean = self.down(name).await;
        (clean, self.wait().await)
    }

    /// Unmount the box's disks through PID 1, then end the VMM. True when
    /// the guest synced and unmounted them.
    pub async fn down(&mut self, _name: &str) -> io::Result<bool> {
        if !self.vmm.running() {
            return Ok(false);
        }
        let state_dir = self.state_dir.clone();
        let clean = matches!(
            tokio::task::spawn_blocking(move || vsock::unmount(&state_dir)).await,
            Ok(Ok(()))
        );
        self.vmm.kill()?;
        self.vmm.wait().await?;
        Ok(clean)
    }
}

impl Boot {
    fn run(self, mut stream: std::os::unix::net::UnixStream, state_dir: &Path) -> io::Result<()> {
        let sent = (|| {
            write_frame(&mut stream, BOOT, &self.request)?;
            for fill in &self.fills {
                disk::send(fill, &mut stream)?;
            }
            if let Some(seeding) = &self.seeding {
                disk::send_seeds(&seeding.relative, &seeding.seeds, &mut stream)?;
            }
            Ok::<_, io::Error>(())
        })();
        // A guest that failed early has stopped reading; its FAILED says why.
        let answer = read_frame(&mut stream);
        match answer {
            Ok((READY, _)) => sent?,
            Ok((FAILED, text)) => {
                return Err(io::Error::other(
                    String::from_utf8_lossy(&text).into_owned(),
                ));
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
        vsock::set_env(state_dir, &self.env)
    }
}

fn put(payload: &mut Vec<u8>, bytes: &[u8]) {
    payload.extend((bytes.len() as u32).to_be_bytes());
    payload.extend(bytes);
}

/// The VM configuration, read by Firecracker inside the jail: paths are
/// the jail's.
fn config(plan: &Plan, drives: &[(PathBuf, bool)]) -> serde_json::Value {
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
