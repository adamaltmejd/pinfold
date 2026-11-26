//! `pinfold init boot`: PID 1 of a Firecracker box.
//!
//! PID 1 connects to the host's boot port, reads the box's tree from the
//! BOOT frame, mounts it, sets `user.max_user_namespaces` to 0 and forks
//! the exec service as the box user with every capability set empty, in
//! the root-owned work cgroup. Preparation populates fresh disks as that
//! user before attaching generated configuration. PID 1 then keeps only
//! `CAP_SYS_ADMIN`, reaps and answers the unmount at `down`. The exec service
//! copies missing profile seeds from the boot connection and relays
//! `127.0.0.1:3128` to the host's port 3128 when the
//! box has a proxy, reports ready, and serves exec and stat. PID 1 reads no byte past BOOT.
//!
//! Every connection carries frames: a kind byte, a u32 big-endian payload
//! length, the payload. A payload's strings are a u32 length and bytes.
//! Firecracker ends a host-initiated stream once the host half-closes it,
//! so the host never does; stdin's end is the STDIN_END frame.
//!
//! - Boot (guest dials host port [`BOOT_PORT`]): host sends BOOT (uid u32,
//!   gid u32, relay u8 for a box with a proxy, root device, attachment
//!   count u32, then device, guest path, writable u8, fresh u8, deferred u8 each; fill count
//!   u32, then each fill's guest path and a missing u8; directory count u32,
//!   then each directory's disk guest root and relative path; the box CA,
//!   empty for none), then each fill's
//!   copy stream: a new disk's `from`, or
//!   with missing, the profile's seeds, each written only when absent. The
//!   guest answers READY or FAILED (text).
//! - Exec (host dials guest [`EXEC_PORT`]): one request per connection.
//!   ENV (count u32, name and value each) sets the box's environment once,
//!   answered ENV_SET; an EXEC that arrives first waits for it. EXEC is
//!   argv (count u32, strings), tty u8, then the
//!   fixed execution metadata string, followed by STDIN,
//!   STDIN_END, RESIZE (rows u16, cols u16) and OUTPUT_END (stream kind) frames.
//!   CREDIT (stream kind, consumed u32) replenishes each 64 KiB stream window.
//!   The guest sends STDOUT, STDERR and EXIT (i32); output may follow EXIT for
//!   up to one second. STAT is answered STAT with the
//!   work cgroup's memory current and peak, pids current and max, and OOM
//!   kills, each a present u8 and a u64.
//! - Control (host dials guest [`CONTROL_PORT`], PID 1): UNMOUNT kills the
//!   work cgroup, syncs, trims each writable attachment and unmounts every
//!   attachment, answered CLEAN or FAILED (text).

use super::exec_wire::{ENV, EXEC, FAILED, STDERR, read_frame, write_frame};

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nix::libc;
use nix::mount::{MntFlags, MsFlags};
use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::sys::signal::{SigHandler, Signal};
use nix::sys::socket::{AddressFamily, Backlog, SockFlag, SockType, VsockAddr};
use nix::unistd::{ForkResult, Gid, Uid};

/// The host port PID 1 dials for its boot request.
pub const BOOT_PORT: u32 = 1000;
/// The exec service's port. Unprivileged: the service holds no capability.
pub const EXEC_PORT: u32 = 1024;
/// PID 1's port for the unmount at `down`.
pub const CONTROL_PORT: u32 = 1025;
/// The host port of the box's proxy socket.
pub(super) const PROXY_PORT: u32 = 3128;
const HOST_CID: u32 = libc::VMADDR_CID_HOST;
/// The work cgroup's pids cap.
const PIDS_MAX: u32 = 2048;
const WORK: &str = "/sys/fs/cgroup/work";

pub const BOOT: u8 = b'B';
pub const READY: u8 = b'R';
pub const ENV_SET: u8 = b'v';
pub const STAT: u8 = b'S';
pub const UNMOUNT: u8 = b'U';
pub const CLEAN: u8 = b'C';

/// Where PID 1 builds the box's tree before `pivot_root`. The init disk
/// is the kernel's root and holds `/pinfold`, `/dev` and `/mnt`.
const STAGE: &str = "/mnt";
const NEW_ROOT: &str = "/mnt/root";

/// Run as PID 1 of a Firecracker VM. Any failure before ready ends PID 1,
/// which ends the VM.
pub fn boot() -> ! {
    if process::id() != 1 {
        die("init boot runs only as PID 1")
    }
    let mut host = vsock_connect(BOOT_PORT)
        .unwrap_or_else(|error| die(&format!("connect to the host: {error}")));
    let (request, control, trims) = match prepare(&mut host, false) {
        Ok(prepared) => prepared,
        Err(error) => fail(&mut host, &error),
    };
    // The service reports ready only once this pipe closes, after PID 1
    // has dropped to CAP_SYS_ADMIN.
    let (dropped, dropping) = io::pipe().unwrap_or_else(|error| fail(&mut host, &error));
    // Safety: PID 1 has started no thread, so the child is a whole copy.
    let service = match unsafe { nix::unistd::fork() } {
        Ok(ForkResult::Child) => {
            drop((control, dropping, trims));
            exec_service(&request, host, dropped)
        }
        Ok(ForkResult::Parent { child }) => child,
        Err(error) => fail(&mut host, &io::Error::from(error)),
    };
    drop((host, dropped));
    if let Err(error) =
        drop_bounding(1_u64 << CAP_SYS_ADMIN).and_then(|()| set_capabilities(Some(CAP_SYS_ADMIN)))
    {
        die(&format!("keep only CAP_SYS_ADMIN: {error}"))
    }
    drop(dropping);
    // With SIGCHLD ignored the kernel reaps every exited child, orphans
    // reparented to PID 1 included; the service's exit shows on its pidfd.
    // Safety: SIG_IGN runs no code.
    unsafe { nix::sys::signal::signal(Signal::SIGCHLD, SigHandler::SigIgn) }
        .unwrap_or_else(|error| die(&format!("ignore SIGCHLD: {error}")));
    let service = pidfd_open(service.as_raw())
        .unwrap_or_else(|error| die(&format!("watch the exec service: {error}")));
    serve_control(&request, &control, &service, trims)
}

fn die(detail: &str) -> ! {
    eprintln!("pinfold init boot: {detail}");
    process::exit(1)
}

fn fail(host: &mut UnixStream, error: &io::Error) -> ! {
    // A failed start leaves its disks consistent, like the syncfs at STREAM_END.
    nix::unistd::sync();
    let _ = write_frame(host, FAILED, error.to_string().as_bytes());
    die(&error.to_string())
}

/// The box's tree, as the BOOT frame gives it.
struct Request {
    uid: u32,
    gid: u32,
    /// Whether the box has a proxy for `127.0.0.1:3128` to reach.
    relay: bool,
    root: String,
    /// In mount order: a parent before anything nested in it.
    attachments: Vec<Attachment>,
    /// Each fill's guest path and whether it writes only what is missing.
    fills: Vec<(PathBuf, bool)>,
    /// Paths created as the box user before their read-only mount attaches.
    directories: Vec<(PathBuf, PathBuf)>,
    /// The box CA, PEM, to append to the image's bundle; empty for none.
    trust: Vec<u8>,
}

struct Attachment {
    device: String,
    guest: PathBuf,
    writable: bool,
    fresh: bool,
    deferred: bool,
}

fn parse_request(payload: &[u8]) -> io::Result<Request> {
    let mut fields = Fields(payload);
    let uid = fields.u32()?;
    let gid = fields.u32()?;
    let relay = fields.u8()? != 0;
    let root = device(&mut fields)?;
    let mut attachments = Vec::new();
    for _ in 0..fields.u32()? {
        attachments.push(Attachment {
            device: device(&mut fields)?,
            guest: guest_path(&mut fields)?,
            writable: fields.u8()? != 0,
            fresh: fields.u8()? != 0,
            deferred: fields.u8()? != 0,
        });
    }
    let mut fills = Vec::new();
    for _ in 0..fields.u32()? {
        fills.push((guest_path(&mut fields)?, fields.u8()? != 0));
    }
    // The host checked this layout before it built the request.
    let mut directories = Vec::new();
    for _ in 0..fields.u32()? {
        let disk = guest_path(&mut fields)?;
        let relative = PathBuf::from(OsString::from_vec(fields.bytes()?.to_vec()));
        directories.push((disk, relative));
    }
    let trust = fields.bytes()?.to_vec();
    fields.end()?;
    attachments.sort_by_key(|attachment| attachment.guest.components().count());
    Ok(Request {
        uid,
        gid,
        relay,
        root,
        attachments,
        fills,
        directories,
        trust,
    })
}

fn device(fields: &mut Fields) -> io::Result<String> {
    let name = String::from_utf8(fields.bytes()?.to_vec())
        .ok()
        .filter(|name| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_lowercase()))
        .ok_or_else(|| io::Error::other("a device name is not a block device's"))?;
    Ok(format!("/dev/{name}"))
}

/// A plain absolute path below `/`, as [`mount_at`] walks it. The host's
/// plan already refuses the kernel's and hidden paths.
fn guest_path(fields: &mut Fields) -> io::Result<PathBuf> {
    let path = PathBuf::from(OsString::from_vec(fields.bytes()?.to_vec()));
    let mut components = path.components();
    if components.next() != Some(Component::RootDir)
        || path.components().count() < 2
        || !components.all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::other(format!(
            "guest path {} must be plain, absolute and below /",
            path.display()
        )));
    }
    Ok(path)
}

/// Read BOOT, mount the box's tree, enter it and make the work cgroup.
/// Runs as root with every capability. Also returns a descriptor on each
/// writable attachment's root for the trim at stop: PID 1 keeps no DAC
/// override, so it could not open a root the box user makes private.
fn prepare(host: &mut UnixStream, builder: bool) -> io::Result<(Request, OwnedFd, Vec<OwnedFd>)> {
    let (kind, payload) = read_frame(host)?;
    if kind != BOOT {
        return Err(io::Error::other(format!("expected BOOT, got frame {kind}")));
    }
    let request = parse_request(&payload)?;

    let nodev = MsFlags::MS_NOSUID | MsFlags::MS_NODEV;
    let stage = Path::new(STAGE);
    mount("tmpfs", stage, "tmpfs", nodev, Some("mode=0700"))?;
    for dir in ["lower", "upper/data", "upper/work", "root"] {
        fs::create_dir_all(stage.join(dir))?;
    }
    mount(
        &request.root,
        &stage.join("lower"),
        "ext4",
        nodev | MsFlags::MS_RDONLY,
        None,
    )?;
    // The image's root is read-only; an overlay on tmpfs makes the mount
    // points and the empty resolv.conf, then becomes read-only itself.
    let root = Path::new(NEW_ROOT);
    mount(
        "overlay",
        root,
        "overlay",
        nodev,
        Some("lowerdir=/mnt/lower,upperdir=/mnt/upper/data,workdir=/mnt/upper/work"),
    )?;
    let etc = root.join("etc");
    fs::create_dir_all(&etc)?;
    match fs::remove_file(etc.join("resolv.conf")) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => File::create(etc.join("resolv.conf")).map(drop)?,
    }
    if !request.trust.is_empty() {
        write_bundle(root, &request.trust)
            .map_err(|error| io::Error::other(format!("write {}: {error}", super::BUNDLE)))?;
    }

    let noexec = nodev | MsFlags::MS_NOEXEC;
    mount_at(root, "/proc", "proc", "proc", noexec, None)?;
    mount_at(
        root,
        "/sys",
        "sysfs",
        "sysfs",
        noexec | MsFlags::MS_RDONLY,
        None,
    )?;
    mount_at(root, "/sys/fs/cgroup", "cgroup2", "cgroup2", noexec, None)?;
    mount_at(
        root,
        "/dev",
        "devtmpfs",
        "devtmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
        None,
    )?;
    mount_at(
        root,
        "/dev/pts",
        "devpts",
        "devpts",
        // Not nodev: the ptys are its device nodes.
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
        Some("newinstance,ptmxmode=0666,mode=0620"),
    )?;
    mount_at(root, "/dev/shm", "tmpfs", "tmpfs", nodev, Some("mode=1777"))?;
    mount_at(root, "/tmp", "tmpfs", "tmpfs", nodev, Some("mode=1777"))?;
    let mut trims = Vec::new();
    for deferred in [false, true] {
        for attachment in request
            .attachments
            .iter()
            .filter(|attachment| attachment.deferred == deferred)
        {
            if builder && attachment.guest == Path::new("/selected") {
                // The host populates this fixed read-only device after discovery.
                // Do not probe it before that host-authorized transition.
                fs::create_dir(root.join("selected"))?;
                continue;
            }
            let flags = if attachment.writable {
                nodev
            } else {
                nodev | MsFlags::MS_RDONLY
            };
            for (disk, relative) in &request.directories {
                if disk.join(relative) == attachment.guest {
                    let directory = disk_root(root, disk)?;
                    as_box_user(request.uid, request.gid, || {
                        super::pid1::make_directories(directory, relative).map(drop)
                    })
                    .map_err(|error| {
                        let path = disk.join(relative);
                        io::Error::other(format!("prepare {}: {error}", path.display()))
                    })?;
                }
            }
            // pinfold's disks leave their inode tables uninitialized, and a
            // disk's host image grows only with what the box writes.
            let target = mount_at(
                root,
                attachment.guest.as_os_str(),
                &attachment.device,
                "ext4",
                flags,
                Some("noinit_itable"),
            )?;
            if attachment.writable {
                trims.push(super::pid1::open_directory(&target)?);
                if attachment.fresh {
                    super::pid1::own_fresh_root(&target, request.uid, request.gid)?;
                }
            }
            for (guest, missing) in &request.fills {
                if !missing && *guest == attachment.guest {
                    let directory = disk_root(root, guest)?;
                    as_box_user(request.uid, request.gid, || {
                        nix::unistd::fchdir(&directory)?;
                        // Unbuffered reads leave the next fill or seed stream for its owner.
                        super::pid1::receive_into(Path::new("."), host, false)
                    })
                    .map_err(|error| {
                        io::Error::other(format!("populate {}: {error}", guest.display()))
                    })?;
                }
            }
        }
    }
    nix::mount::mount(
        None::<&str>,
        root,
        None::<&str>,
        MsFlags::MS_REMOUNT | MsFlags::MS_BIND | MsFlags::MS_RDONLY | nodev,
        None::<&str>,
    )
    .map_err(|error| io::Error::other(format!("make the root read-only: {error}")))?;

    loopback_up()?;
    // Detach the init disk and the staging tree: the box sees only its own.
    nix::unistd::chdir(root)?;
    nix::unistd::pivot_root(".", ".")?;
    nix::mount::umount2(".", MntFlags::MNT_DETACH)?;
    nix::unistd::chdir("/")?;

    fs::write("/proc/sys/user/max_user_namespaces", "0")?;
    fs::write("/sys/fs/cgroup/cgroup.subtree_control", "+pids +memory")?;
    fs::create_dir(WORK)?;
    fs::write(format!("{WORK}/pids.max"), PIDS_MAX.to_string())?;
    // Bound while PID 1 still holds every capability; the host dials it.
    let control = vsock_listen(CONTROL_PORT)?;
    Ok((request, control, trims))
}

/// The trusted image job uses the same mount setup, with fixed devices and
/// no agent state or execution service. The agent entry point stays separate.
#[cfg(target_arch = "x86_64")]
pub(super) fn prepare_builder(host: &mut UnixStream) -> io::Result<bool> {
    let (request, control, trims) = prepare(host, true)?;
    drop((control, trims));
    mount(
        "tmpfs",
        Path::new("/run"),
        "tmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=0755"),
    )?;
    fs::write(format!("{WORK}/cgroup.subtree_control"), "+pids +memory")?;
    fs::create_dir(format!("{WORK}/service"))?;
    fs::create_dir(format!("{WORK}/jobs"))?;
    Ok(request.relay)
}

#[cfg(target_arch = "x86_64")]
pub(super) fn mount_builder_input() -> io::Result<()> {
    let device = File::open("/dev/vde")?;
    // BLKFLSBUF is permitted on a read-only FD with the trusted service's
    // existing SYS_ADMIN. Init leaves this device unopened until host handoff.
    // SAFETY: this no-argument block-device ioctl reads no userspace memory.
    if unsafe { libc::ioctl(device.as_raw_fd(), 0x1261) } < 0 {
        return Err(io::Error::last_os_error());
    }
    drop(device);
    mount(
        "/dev/vde",
        Path::new("/selected"),
        "ext4",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_RDONLY,
        Some("noinit_itable"),
    )
}

fn mount(
    source: &str,
    target: &Path,
    fstype: &str,
    flags: MsFlags,
    data: Option<&str>,
) -> io::Result<()> {
    nix::mount::mount(Some(source), target, Some(fstype), flags, data).map_err(|error| {
        io::Error::other(format!("mount {source} on {}: {error}", target.display()))
    })
}

/// Mount `source` at `guest` inside `root`, making the mount point. Each
/// component is made and opened without following a symlink, which a box
/// may have left on a disk, so PID 1 creates nothing through one and
/// refuses the mount. Mounting by path afterwards is safe: box work has
/// not started, so nothing can swap a component in between.
fn mount_at(
    root: &Path,
    guest: impl AsRef<OsStr>,
    source: &str,
    fstype: &str,
    flags: MsFlags,
    data: Option<&str>,
) -> io::Result<PathBuf> {
    let guest = Path::new(guest.as_ref());
    let relative = guest.strip_prefix("/").unwrap_or(guest);
    let target = root.join(relative);
    // Every walk error is an errno; name it as nix does.
    let walk = |error: io::Error| match error.raw_os_error().map(nix::errno::Errno::from_raw) {
        Some(nix::errno::Errno::ELOOP) => {
            io::Error::other(format!("{} passes through a symlink", guest.display()))
        }
        Some(errno) => io::Error::other(format!("make {}: {errno}", guest.display())),
        None => io::Error::other(format!("make {}: {error}", guest.display())),
    };
    super::pid1::open_directory(root)
        .and_then(|root| super::pid1::make_directories(root, relative))
        .map_err(walk)?;
    mount(source, &target, fstype, flags, data)?;
    Ok(target)
}

/// Replace the image's CA bundle in the overlay with itself and `ca`
/// appended, walking the path without following a link.
fn write_bundle(root: &Path, ca: &[u8]) -> io::Result<()> {
    use nix::fcntl::OFlag;
    let root = super::pid1::open_directory(root)?;
    let mut bundle = super::pid1::image_bundle(&root)?;
    if !bundle.is_empty() && !bundle.ends_with(b"\n") {
        bundle.push(b'\n');
    }
    bundle.extend_from_slice(ca);
    let path = Path::new(super::BUNDLE)
        .strip_prefix("/")
        .expect("absolute");
    let certs = super::pid1::make_directories(root, path.parent().expect("a directory"))?;
    let name = path.file_name().expect("a file name");
    match nix::unistd::unlinkat(&certs, name, nix::unistd::UnlinkatFlags::NoRemoveDir) {
        Ok(()) | Err(nix::errno::Errno::ENOENT) => {}
        Err(error) => return Err(error.into()),
    }
    let file = nix::fcntl::openat(
        &certs,
        name,
        OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        nix::sys::stat::Mode::from_bits_truncate(0o644),
    )?;
    File::from(file).write_all(&bundle)
}

/// An open disk descriptor avoids the private staging tree's root-only
/// ancestors.
fn disk_root(root: &Path, disk: &Path) -> io::Result<OwnedFd> {
    super::pid1::open_directory(&root.join(disk.strip_prefix("/").unwrap()))
}

/// The child has no box work or threads and cannot regain root.
fn as_box_user(uid: u32, gid: u32, operation: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    let (mut errors, mut report) = io::pipe()?;
    // Safety: prepare runs before PID 1 starts threads or ordinary box work.
    let child = match unsafe { nix::unistd::fork() }? {
        ForkResult::Child => {
            drop(errors);
            let result = super::pid1::drop_identity(uid, gid).and_then(|()| operation());
            if let Err(error) = result {
                let _ = write!(report, "{error}");
                process::exit(1);
            }
            process::exit(0);
        }
        ForkResult::Parent { child } => child,
    };
    drop(report);
    let mut detail = String::new();
    let read = errors.read_to_string(&mut detail);
    let status = nix::sys::wait::waitpid(child, None)?;
    read?;
    if status != nix::sys::wait::WaitStatus::Exited(child, 0) {
        return Err(io::Error::other(format!(
            "prepare as the box user: {status:?}: {detail}"
        )));
    }
    Ok(())
}

fn loopback_up() -> io::Result<()> {
    let socket = nix::sys::socket::socket(
        AddressFamily::Inet,
        SockType::Datagram,
        SockFlag::SOCK_CLOEXEC,
        None,
    )?;
    // Safety: ifreq is plain data; the ioctls read and write only it.
    unsafe {
        let mut request: libc::ifreq = std::mem::zeroed();
        request.ifr_name[0] = b'l' as libc::c_char;
        request.ifr_name[1] = b'o' as libc::c_char;
        if libc::ioctl(socket.as_raw_fd(), libc::SIOCGIFFLAGS as _, &mut request) < 0 {
            return Err(io::Error::last_os_error());
        }
        request.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
        if libc::ioctl(socket.as_raw_fd(), libc::SIOCSIFFLAGS as _, &request) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

const CAP_SYS_ADMIN: u32 = 21;

/// Drop every capability outside the `keep` mask from the bounding set. Needs
/// `CAP_SETPCAP`, so it comes before [`set_capabilities`]. Only system
/// calls: the VMM jail's child runs it between `clone` and `execve`.
pub(crate) fn drop_bounding(keep: u64) -> io::Result<()> {
    for capability in 0..64 {
        if keep & (1_u64 << capability) != 0 {
            continue;
        }
        // Safety: prctl with integer arguments, passed as the kernel's
        // unsigned longs.
        if unsafe {
            libc::prctl(
                libc::PR_CAPBSET_DROP,
                capability as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            )
        } < 0
        {
            let error = io::Error::last_os_error();
            // EINVAL: past the kernel's last capability. Capability 0
            // always exists, so EINVAL there is a failure.
            if error.raw_os_error() == Some(libc::EINVAL) && capability > 0 {
                break;
            }
            return Err(error);
        }
    }
    Ok(())
}

/// Make `keep` the only effective and permitted capability, with the
/// inheritable and ambient sets empty. Only system calls, like
/// [`drop_bounding`].
pub(crate) fn set_capabilities(keep: Option<u32>) -> io::Result<()> {
    clear_ambient()?;
    capset(keep.map_or(0, |capability| 1_u64 << capability))
}

pub(super) fn clear_ambient() -> io::Result<()> {
    // Safety: prctl with integer arguments, as unsigned longs.
    if unsafe {
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Set effective and permitted capabilities to `keep`, with no inheritable
/// capabilities. Only system calls, like [`drop_bounding`].
pub(super) fn capset(keep: u64) -> io::Result<()> {
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = Header {
        version: 0x2008_0522, // _LINUX_CAPABILITY_VERSION_3
        pid: 0,
    };
    let data = [0, 32].map(|shift| Data {
        effective: (keep >> shift) as u32,
        permitted: (keep >> shift) as u32,
        inheritable: 0,
    });
    // Safety: capset reads the header and two data words.
    if unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn pidfd_open(pid: i32) -> io::Result<OwnedFd> {
    // Safety: pidfd_open returns a new descriptor or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::c_long, 0 as libc::c_long) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // Safety: the descriptor is new and owned here alone.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

/// PID 1 after boot: answer UNMOUNT, and end the VM if the exec service
/// ends before one.
fn serve_control(
    request: &Request,
    control: &OwnedFd,
    service: &OwnedFd,
    mut trims: Vec<OwnedFd>,
) -> ! {
    let mut unmounted = None;
    loop {
        let mut fds = vec![PollFd::new(control.as_fd(), PollFlags::POLLIN)];
        if unmounted.is_none() {
            fds.push(PollFd::new(service.as_fd(), PollFlags::POLLIN));
        }
        match nix::poll::poll(&mut fds, PollTimeout::NONE) {
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(error) => die(&format!("poll: {error}")),
        }
        if fds
            .get(1)
            .and_then(PollFd::revents)
            .is_some_and(|events| !events.is_empty())
        {
            die("the exec service ended")
        }
        if !fds[0].any().unwrap_or(false) {
            continue;
        }
        let Some(mut connection) = accept_host(control) else {
            continue;
        };
        let reply = match read_frame(&mut connection) {
            Ok((UNMOUNT, _)) => unmounted
                .get_or_insert_with(|| {
                    unmount(request, std::mem::take(&mut trims)).map_err(|error| error.to_string())
                })
                .clone(),
            Ok((kind, _)) => Err(format!("unknown control frame {kind}")),
            Err(error) => Err(error.to_string()),
        };
        let _ = match reply {
            Ok(()) => write_frame(&mut connection, CLEAN, &[]),
            Err(detail) => write_frame(&mut connection, FAILED, detail.as_bytes()),
        };
    }
}

/// End box work, so nothing keeps a disk busy, then sync and unmount every
/// attachment, the nested ones first. Trimming each writable disk first
/// returns its freed blocks to the host as holes in the image.
fn unmount(request: &Request, trims: Vec<OwnedFd>) -> io::Result<()> {
    // The host waits 30 s for the answer. Trims start only in the first
    // 20, so the unmounts keep the rest.
    let started = Instant::now();
    stop_work(Some(started + Duration::from_secs(10)))?;
    nix::unistd::sync();
    for disk in &trims {
        trim(disk, started + Duration::from_secs(20));
    }
    // A held descriptor would make its unmount EBUSY.
    drop(trims);
    for attachment in request.attachments.iter().rev() {
        nix::mount::umount2(&attachment.guest, MntFlags::UMOUNT_NOFOLLOW).map_err(|error| {
            io::Error::other(format!("unmount {}: {error}", attachment.guest.display()))
        })?;
    }
    Ok(())
}

/// FITRIM the filesystem `disk` is on, 128 MiB at a time, starting no
/// range after `deadline`; one FITRIM cannot be interrupted. Ranges past
/// the end fail, as does any other error, which leaves only space
/// unreclaimed and does not fail the stop.
fn trim(disk: &OwnedFd, deadline: Instant) {
    // linux/fs.h: _IOWR('X', 121, struct fstrim_range).
    const FITRIM: libc::c_ulong = 0xc018_5879;
    // One ext4 block group. With MINLEN a range sends at most 128 discards,
    // so the last one started ends long before the host's deadline
    // (docs/archive/2026-11-26-linux-disk-discard.md).
    const RANGE: u64 = 128 << 20;
    // ext4 forgets what it trimmed at unmount, so each stop discards every
    // free extent again; extents under 1 MiB are skipped.
    const MINLEN: u64 = 1 << 20;
    let mut start = 0;
    while Instant::now() < deadline {
        let mut range: [u64; 3] = [start, RANGE, MINLEN];
        // SAFETY: the kernel reads and writes only the 24-byte fstrim_range.
        if unsafe { libc::ioctl(disk.as_raw_fd(), FITRIM as _, range.as_mut_ptr()) } < 0 {
            return;
        }
        start += RANGE;
    }
}

/// Kill the work cgroup and wait until it is empty, failing after
/// `deadline` if there is one.
pub(super) fn stop_work(deadline: Option<Instant>) -> io::Result<()> {
    fs::write(format!("{WORK}/cgroup.kill"), "1")?;
    let mut events = File::open(format!("{WORK}/cgroup.events"))?;
    loop {
        // cgroup v2 signals a change with POLLPRI|POLLERR on the descriptor
        // last read, which stays ready until it is read again from 0.
        events.seek(SeekFrom::Start(0))?;
        let mut state = String::new();
        events.read_to_string(&mut state)?;
        if state.lines().any(|line| line == "populated 0") {
            return Ok(());
        }
        let timeout = match deadline {
            None => PollTimeout::NONE,
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(io::Error::other("box work did not end"));
                }
                PollTimeout::try_from(left).unwrap_or(PollTimeout::MAX)
            }
        };
        let mut fds = [PollFd::new(
            events.as_fd(),
            PollFlags::POLLPRI | PollFlags::POLLERR,
        )];
        match nix::poll::poll(&mut fds, timeout) {
            Ok(_) | Err(nix::errno::Errno::EINTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// The exec service: forked from PID 1, still root until
/// [`become_box_user`].
fn exec_service(request: &Request, host: UnixStream, mut dropped: io::PipeReader) -> ! {
    let mut host = BufReader::new(host);
    let started = (|| -> io::Result<(OwnedFd, Option<TcpListener>)> {
        become_box_user(request.uid, request.gid)?;
        for (fill, _) in request.fills.iter().filter(|(_, missing)| *missing) {
            super::pid1::receive_into(fill, &mut host, true).map_err(|error| {
                io::Error::other(format!("copy into {}: {error}", fill.display()))
            })?;
        }
        let exec = vsock_listen(EXEC_PORT)?;
        // Without a proxy nothing listens, so a request finds no listener.
        let relay = (request.relay)
            .then(|| TcpListener::bind(super::pid1::RELAY_LISTEN))
            .transpose()?;
        Ok((exec, relay))
    })();
    let mut host = host.into_inner();
    let (exec, relay) = match started {
        Ok(listeners) => listeners,
        Err(error) => fail(&mut host, &error),
    };
    if let Some(relay) = relay
        && let Err(error) = super::pid1::serve_relay(relay, || vsock_connect(PROXY_PORT))
    {
        fail(&mut host, &error)
    }
    // EOF once PID 1 holds only CAP_SYS_ADMIN, or has died.
    let _ = dropped.read(&mut [0]);
    drop(dropped);
    if let Err(error) = write_frame(&mut host, READY, &[]) {
        die(&format!("report ready: {error}"))
    }
    drop(host);

    let env = Arc::new(Env::default());
    loop {
        if let Some(connection) = accept_host(&exec) {
            let env = Arc::clone(&env);
            // A thread that cannot start, at the pids cap, drops its
            // connection; the service stays up. `thread::spawn` would panic.
            let _ = thread::Builder::new().spawn(move || serve(connection, &env));
        }
    }
}

/// Join the work cgroup, out of the OOM killer's reach, then drop to the
/// box user with every capability set empty and `no_new_privs`.
fn become_box_user(uid: u32, gid: u32) -> io::Result<()> {
    fs::write(format!("{WORK}/cgroup.procs"), "0")?;
    // The modern ABI sets the inherited unprivileged floor. The deprecated
    // ABI keeps the service immune without lowering that floor (#199).
    fs::write("/proc/self/oom_score_adj", "-999")?;
    fs::write("/proc/self/oom_adj", "-17")?;
    drop_bounding(0)?;
    nix::unistd::setgroups(&[])?;
    let (uid, gid) = (Uid::from_raw(uid), Gid::from_raw(gid));
    nix::unistd::setresgid(gid, gid, gid)?;
    // Leaving root also clears dumpable, so the service's /proc entries stay
    // root's and box work cannot take its descriptors with pidfd_getfd.
    // Nothing may set it again (#158).
    nix::unistd::setresuid(uid, uid, uid)?;
    set_capabilities(None)?;
    nix::sys::prctl::set_no_new_privs()?;
    Ok(())
}

/// The box's environment, set once by `up` and held in memory.
#[derive(Default)]
struct Env {
    vars: Mutex<Option<Vec<(OsString, OsString)>>>,
    set: Condvar,
}

/// Answer one request; a request that fails before its answer gets
/// FAILED with the reason.
fn serve(connection: UnixStream, env: &Env) {
    let Ok(mut failed) = connection.try_clone() else {
        return;
    };
    if let Err(error) = request(connection, env) {
        let _ = write_frame(&mut failed, FAILED, error.to_string().as_bytes());
    }
}

fn request(mut connection: UnixStream, env: &Env) -> io::Result<()> {
    let (kind, payload) = read_frame(&mut connection)?;
    match kind {
        ENV => {
            let vars = decode_env(&payload)?;
            let mut held = env.vars.lock().expect("env lock");
            if held.is_some() {
                return write_frame(&mut connection, FAILED, b"the environment is already set");
            }
            *held = Some(vars);
            env.set.notify_all();
            write_frame(&mut connection, ENV_SET, &[])
        }
        STAT => write_frame(&mut connection, STAT, &stat()),
        EXEC => {
            let vars = env
                .set
                .wait_while(env.vars.lock().expect("env lock"), |vars| vars.is_none())
                .expect("env lock")
                .clone()
                .unwrap_or_default();
            exec(connection, &payload, Some(vars), None)
        }
        _ => write_frame(
            &mut connection,
            FAILED,
            format!("unknown request {kind}").as_bytes(),
        ),
    }
}

/// An ENV payload's names and values.
pub(super) fn decode_env(payload: &[u8]) -> io::Result<Vec<(OsString, OsString)>> {
    let mut fields = Fields(payload);
    let vars = (0..fields.u32()?)
        .map(|_| Ok((fields.os()?, fields.os()?)))
        .collect::<io::Result<_>>()?;
    fields.end()?;
    Ok(vars)
}

/// Run one EXEC request in its own session through `init exec`, which sets
/// `oom_score_adj` 1000, and relay its stdio until it exits and its output
/// ends. Project commands stop their process group when their client departs.
/// `vars` is the box's environment for the child; without it (Apple) the
/// child keeps this process's environment and reads the box's from PID 1.
pub(super) fn exec(
    connection: UnixStream,
    payload: &[u8],
    vars: Option<Vec<(OsString, OsString)>>,
    departure: Option<BorrowedFd<'_>>,
) -> io::Result<()> {
    let mut fields = Fields(payload);
    let argv: Vec<OsString> = (0..fields.u32()?)
        .map(|_| fields.os())
        .collect::<io::Result<_>>()?;
    let tty = fields.u8()? != 0;
    let mut metadata = super::ExecMetadata::decode(
        fields
            .os()?
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF-8 execution metadata"))?,
    )?;
    // Apply a known size when creating the PTY; init exec must not
    // apply it a second time after the command's terminal exists.
    let window = metadata.window.take();
    let project = metadata.project_directory.is_some();
    fields.end()?;
    let Some(program) = argv.first() else {
        return Err(io::Error::other("an exec without argv"));
    };

    // The child is a copy of the service, which holds the exec listener,
    // until its first execve; it stays non-dumpable until then (#158). The
    // init disk holding `/pinfold` is detached, but its magic link execs.
    let mut command = Command::new("/proc/self/exe");
    command.args(["init", "exec"]);
    match vars {
        // `init exec` restores the environment from these transport names, so
        // a name that itself carries the prefix arrives exactly.
        Some(vars) => {
            command
                .env_clear()
                .envs(vars.into_iter().map(|(name, value)| {
                    let mut key = OsString::from(super::pid1::BOX_ENV_PREFIX);
                    key.push(name);
                    (key, super::hex(value.as_bytes()))
                }));
        }
        None => {
            command.arg("--box-env");
            // Apple's own TTY exec defaulted TERM below the image, spec and
            // host values; public exec keeps that default.
            if tty && !project && std::env::var_os("TERM").is_none() {
                command.env("TERM", "xterm");
            }
        }
    }
    command
        .arg("--")
        .args(&argv)
        .env(super::EXEC_METADATA, metadata.encode());
    if !project {
        command.current_dir("/");
    }
    let pty = if tty {
        let (master, slave) = open_pty(window)?;
        command
            .stdin(slave.try_clone()?)
            .stdout(slave.try_clone()?)
            .stderr(slave);
        Some(master)
    } else {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        None
    };
    // Safety: the closure makes only async-signal-safe calls.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 || (tty && libc::ioctl(0, libc::TIOCSCTTY, 0) < 0) {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let mut output = &connection;
            let message = format!(
                "pinfold init exec: {}: {error}\n",
                super::formatting::terminal_text(&program.to_string_lossy())
            );
            if project {
                return write_frame(&mut output, FAILED, message.as_bytes());
            }
            write_frame(&mut output, STDERR, message.as_bytes())?;
            // The public process interface keeps the shell's spawn codes.
            let code: i32 = if error.kind() == io::ErrorKind::NotFound {
                127
            } else {
                126
            };
            return write_frame(&mut output, super::exec_wire::EXIT, &code.to_be_bytes());
        }
    };
    drop(command);

    super::session_io::relay(connection, child, pty, project, departure)
}

fn open_pty(window: Option<(u16, u16)>) -> io::Result<(File, File)> {
    // Safety: posix_openpt returns a new descriptor or -1; the slave path
    // comes from ptsname_r into a local buffer.
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC);
        if master < 0 {
            return Err(context("open a pty", io::Error::last_os_error()));
        }
        let master = File::from_raw_fd(master);
        if libc::grantpt(master.as_raw_fd()) < 0 || libc::unlockpt(master.as_raw_fd()) < 0 {
            return Err(context("unlock a pty", io::Error::last_os_error()));
        }
        let mut name = [0 as libc::c_char; 128];
        let failed = libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len());
        if failed != 0 {
            return Err(io::Error::from_raw_os_error(failed));
        }
        let slave = libc::open(
            name.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        );
        if slave < 0 {
            let path = std::ffi::CStr::from_ptr(name.as_ptr()).to_string_lossy();
            return Err(context(&format!("open {path}"), io::Error::last_os_error()));
        }
        let slave = File::from_raw_fd(slave);
        if let Some((rows, cols)) = window {
            super::pid1::set_window(&master, rows, cols)
                .map_err(|error| context("set the window", error))?;
        }
        Ok((master, slave))
    }
}

fn context(what: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

/// The work cgroup's memory, pids and OOM kills, from the guest kernel.
fn stat() -> Vec<u8> {
    let read = |name: &str| fs::read_to_string(format!("{WORK}/{name}")).ok();
    let number = |name: &str| read(name).and_then(|value| value.trim().parse::<u64>().ok());
    let oom_kills = read("memory.events").and_then(|events| {
        events
            .lines()
            .find_map(|line| line.strip_prefix("oom_kill "))
            .and_then(|count| count.trim().parse().ok())
    });
    let mut payload = Vec::new();
    for value in [
        number("memory.current"),
        number("memory.peak"),
        number("pids.current"),
        number("pids.max"),
        oom_kills,
    ] {
        payload.push(u8::from(value.is_some()));
        payload.extend(value.unwrap_or(0).to_be_bytes());
    }
    payload
}

fn vsock_socket() -> io::Result<OwnedFd> {
    Ok(nix::sys::socket::socket(
        AddressFamily::Vsock,
        SockType::Stream,
        SockFlag::SOCK_CLOEXEC,
        None,
    )?)
}

/// A vsock stream to the host. It travels as a `UnixStream`, whose read,
/// write, shutdown and clone are the socket calls any stream socket takes;
/// its address methods would be wrong and are never called.
pub(super) fn vsock_connect(port: u32) -> io::Result<UnixStream> {
    let socket = vsock_socket()?;
    nix::sys::socket::connect(socket.as_raw_fd(), &VsockAddr::new(HOST_CID, port))?;
    Ok(UnixStream::from(socket))
}

fn vsock_listen(port: u32) -> io::Result<OwnedFd> {
    let socket = vsock_socket()?;
    nix::sys::socket::bind(
        socket.as_raw_fd(),
        &VsockAddr::new(libc::VMADDR_CID_ANY, port),
    )?;
    // -1 takes the kernel's somaxconn, 4096 in the guest: a burst of
    // concurrent execs queues instead of being reset (#484).
    nix::sys::socket::listen(&socket, Backlog::new(-1)?)?;
    Ok(socket)
}

/// Accept the next connection from the host. Any other peer is closed at
/// once. The guest kernel has no vsock loopback, so none can reach a guest
/// listener today; the check keeps that true if one ever does. A failed
/// accept backs off. Either way there is no connection.
fn accept_host(listener: &OwnedFd) -> Option<UnixStream> {
    let Ok(connection) = nix::sys::socket::accept4(listener, SockFlag::SOCK_CLOEXEC) else {
        super::pid1::back_off();
        return None;
    };
    match nix::sys::socket::getpeername::<VsockAddr>(connection.as_raw_fd()) {
        Ok(peer) if peer.cid() == HOST_CID => Some(UnixStream::from(connection)),
        _ => None,
    }
}

/// A frame payload's fields, read in order.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take(&mut self, count: usize) -> io::Result<&[u8]> {
        if self.0.len() < count {
            return Err(io::Error::other("a frame ended early"));
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn bytes(&mut self) -> io::Result<&[u8]> {
        let length = self.u32()? as usize;
        self.take(length)
    }

    fn os(&mut self) -> io::Result<OsString> {
        Ok(OsStr::from_bytes(self.bytes()?).to_os_string())
    }

    fn end(self) -> io::Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other("a frame has trailing bytes"))
        }
    }
}
