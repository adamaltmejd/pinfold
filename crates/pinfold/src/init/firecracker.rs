//! `pinfold init boot`: PID 1 of a Firecracker box.
//!
//! PID 1 connects to the host's boot port, reads the box's tree from the
//! BOOT frame, mounts it, sets `user.max_user_namespaces` to 0 and forks
//! the exec service as the box user with every capability set empty, in
//! the root-owned work cgroup. It keeps only `CAP_SYS_ADMIN`, then only
//! reaps and answers the unmount at `down`. The exec service copies each
//! new disk's `from` stream (the `init receive` format) from the boot
//! connection, relays `127.0.0.1:3128` to the host's port 3128 when the
//! box has a proxy, reports ready, and serves exec and stat. PID 1 reads no byte past BOOT.
//!
//! Every connection carries frames: a kind byte, a u32 big-endian payload
//! length, the payload. A payload's strings are a u32 length and bytes.
//! Firecracker ends a host-initiated stream once the host half-closes it,
//! so the host never does; stdin's end is the STDIN_END frame.
//!
//! - Boot (guest dials host port [`BOOT_PORT`]): host sends BOOT (uid u32,
//!   gid u32, relay u8 for a box with a proxy, root device, attachment
//!   count u32, then device, guest path, writable u8 each; fill count
//!   u32, then each fill's guest path and a missing u8), then each fill's
//!   copy stream: a new disk's `from`, or
//!   with missing, the profile's seeds, each written only when absent. The
//!   guest answers READY or FAILED (text).
//! - Exec (host dials guest [`EXEC_PORT`]): one request per connection.
//!   ENV (count u32, name and value each) sets the box's environment once,
//!   answered ENV_SET; an EXEC that arrives first waits for it. EXEC is
//!   argv (count u32, strings), workdir (empty for `/`), tty u8, rows u16,
//!   cols u16, then extra env pairs (count u32), followed by STDIN,
//!   STDIN_END and RESIZE (rows u16, cols u16) frames; the guest sends
//!   STDOUT, STDERR and finally EXIT (i32). STAT is answered STAT with the
//!   work cgroup's memory current and peak, pids current and max, and OOM
//!   kills, each a present u8 and a u64.
//! - Control (host dials guest [`CONTROL_PORT`], PID 1): UNMOUNT kills the
//!   work cgroup, syncs and unmounts every attachment, answered CLEAN or
//!   FAILED (text).

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Component, Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nix::fcntl::OFlag;
use nix::libc;
use nix::mount::{MntFlags, MsFlags};
use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::sys::signal::{SigHandler, Signal};
use nix::sys::socket::{AddressFamily, Backlog, SockFlag, SockType, VsockAddr};
use nix::sys::stat::Mode;
use nix::unistd::{ForkResult, Gid, Uid};

/// The host port PID 1 dials for its boot request.
pub const BOOT_PORT: u32 = 1000;
/// The exec service's port. Unprivileged: the service holds no capability.
pub const EXEC_PORT: u32 = 1024;
/// PID 1's port for the unmount at `down`.
pub const CONTROL_PORT: u32 = 1025;
/// The host port of the box's proxy socket.
const PROXY_PORT: u32 = 3128;
const HOST_CID: u32 = libc::VMADDR_CID_HOST;
/// The work cgroup's pids cap.
const PIDS_MAX: u32 = 2048;
const WORK: &str = "/sys/fs/cgroup/work";

pub const BOOT: u8 = b'B';
pub const READY: u8 = b'R';
pub const FAILED: u8 = b'F';
pub const ENV: u8 = b'V';
pub const ENV_SET: u8 = b'v';
pub const EXEC: u8 = b'X';
pub const STAT: u8 = b'S';
pub const STDIN: u8 = b'i';
pub const STDIN_END: u8 = b'e';
pub const RESIZE: u8 = b'w';
pub const STDOUT: u8 = b'o';
pub const STDERR: u8 = b'r';
pub const EXIT: u8 = b'x';
pub const UNMOUNT: u8 = b'U';
pub const CLEAN: u8 = b'C';

/// A frame's payload limit: argv and env together stay far below it.
const FRAME_MAX: usize = 64 << 20;

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
    let (request, control) = match prepare(&mut host, false) {
        Ok(prepared) => prepared,
        Err(error) => fail(&mut host, &error),
    };
    // The service reports ready only once this pipe closes, after PID 1
    // has dropped to CAP_SYS_ADMIN.
    let (dropped, dropping) = io::pipe().unwrap_or_else(|error| fail(&mut host, &error));
    // Safety: PID 1 has started no thread, so the child is a whole copy.
    let service = match unsafe { nix::unistd::fork() } {
        Ok(ForkResult::Child) => {
            drop((control, dropping));
            exec_service(&request, host, dropped)
        }
        Ok(ForkResult::Parent { child }) => child,
        Err(error) => fail(&mut host, &io::Error::from(error)),
    };
    drop((host, dropped));
    if let Err(error) =
        drop_bounding(Some(CAP_SYS_ADMIN)).and_then(|()| set_capabilities(Some(CAP_SYS_ADMIN)))
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
    serve_control(&request, &control, &service)
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
}

struct Attachment {
    device: String,
    guest: PathBuf,
    writable: bool,
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
        });
    }
    let mut fills = Vec::new();
    for _ in 0..fields.u32()? {
        fills.push((guest_path(&mut fields)?, fields.u8()? != 0));
    }
    fields.end()?;
    attachments.sort_by_key(|attachment| attachment.guest.components().count());
    Ok(Request {
        uid,
        gid,
        relay,
        root,
        attachments,
        fills,
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
/// Runs as root with every capability.
fn prepare(host: &mut UnixStream, builder: bool) -> io::Result<(Request, OwnedFd)> {
    let (kind, payload) = read_frame(host)?;
    if kind != BOOT {
        return Err(io::Error::other(format!("expected BOOT, got frame {kind}")));
    }
    let request = parse_request(&payload)?;
    if builder
        && (request.uid != 0
            || request.gid != 0
            || request.root != "/dev/vdb"
            || !request.fills.is_empty()
            || request.attachments.len() != if request.relay { 3 } else { 2 }
            || !request.attachments.iter().any(|attachment| {
                attachment.device == "/dev/vdc"
                    && attachment.guest == Path::new("/work")
                    && attachment.writable
            })
            || !request.attachments.iter().any(|attachment| {
                attachment.device == "/dev/vdd"
                    && attachment.guest == Path::new("/input")
                    && !attachment.writable
            })
            || (request.relay
                && !request.attachments.iter().any(|attachment| {
                    attachment.device == "/dev/vde"
                        && attachment.guest == Path::new("/selected")
                        && !attachment.writable
                })))
    {
        return Err(io::Error::other("invalid builder devices"));
    }

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
    for attachment in &request.attachments {
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
            nix::unistd::chown(
                &target,
                Some(Uid::from_raw(request.uid)),
                Some(Gid::from_raw(request.gid)),
            )?;
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
    Ok((request, control))
}

/// The trusted image job uses the same mount setup, with fixed devices and
/// no agent state or execution service. The agent entry point stays separate.
#[cfg(target_arch = "x86_64")]
pub(super) fn prepare_builder(host: &mut UnixStream) -> io::Result<bool> {
    let (request, control) = prepare(host, true)?;
    drop(control);
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

#[cfg(target_arch = "x86_64")]
pub(super) fn connect_builder() -> io::Result<UnixStream> {
    vsock_connect(BOOT_PORT)
}

#[cfg(target_arch = "x86_64")]
pub(super) fn relay_builder() -> io::Result<()> {
    let relay = TcpListener::bind(super::RELAY_LISTEN)?;
    super::serve_relay(relay, || vsock_connect(PROXY_PORT))
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
    let target = root.join(guest.strip_prefix("/").unwrap_or(guest));
    let walk = |error: nix::errno::Errno| match error {
        nix::errno::Errno::ELOOP => {
            io::Error::other(format!("{} passes through a symlink", guest.display()))
        }
        error => io::Error::other(format!("make {}: {error}", guest.display())),
    };
    let directory = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let mut dir = nix::fcntl::open(root, directory, Mode::empty()).map_err(walk)?;
    for component in guest.components().skip(1) {
        let name = component.as_os_str();
        match nix::sys::stat::mkdirat(&dir, name, Mode::from_bits_truncate(0o755)) {
            Ok(()) | Err(nix::errno::Errno::EEXIST) => {}
            Err(error) => return Err(walk(error)),
        }
        dir = nix::fcntl::openat(&dir, name, directory, Mode::empty()).map_err(walk)?;
    }
    mount(source, &target, fstype, flags, data)?;
    Ok(target)
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

/// Drop every capability but `keep` from the bounding set. Needs
/// `CAP_SETPCAP`, so it comes before [`set_capabilities`]. Only system
/// calls: the VMM jail's child runs it between `clone` and `execve`.
pub(crate) fn drop_bounding(keep: Option<u32>) -> io::Result<()> {
    for capability in 0..64 {
        if Some(capability) == keep {
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
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = Header {
        version: 0x2008_0522, // _LINUX_CAPABILITY_VERSION_3
        pid: 0,
    };
    let mut data = [Data::default(); 2];
    if let Some(capability) = keep {
        let word = &mut data[(capability / 32) as usize];
        word.effective = 1 << (capability % 32);
        word.permitted = 1 << (capability % 32);
    }
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
fn serve_control(request: &Request, control: &OwnedFd, service: &OwnedFd) -> ! {
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
                .get_or_insert_with(|| unmount(request).map_err(|error| error.to_string()))
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
/// attachment, the nested ones first.
fn unmount(request: &Request) -> io::Result<()> {
    fs::write(format!("{WORK}/cgroup.kill"), "1")?;
    let events = File::open(format!("{WORK}/cgroup.events"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(format!("{WORK}/cgroup.events"))?.contains("populated 0") {
        if Instant::now() > deadline {
            return Err(io::Error::other("box work did not end"));
        }
        // cgroup.events signals a change with POLLPRI.
        let mut fds = [PollFd::new(events.as_fd(), PollFlags::POLLPRI)];
        let _ = nix::poll::poll(&mut fds, PollTimeout::from(100u16));
    }
    nix::unistd::sync();
    for attachment in request.attachments.iter().rev() {
        nix::mount::umount2(&attachment.guest, MntFlags::UMOUNT_NOFOLLOW).map_err(|error| {
            io::Error::other(format!("unmount {}: {error}", attachment.guest.display()))
        })?;
    }
    Ok(())
}

/// The exec service: forked from PID 1, still root until
/// [`become_box_user`].
fn exec_service(request: &Request, host: UnixStream, mut dropped: io::PipeReader) -> ! {
    let mut host = BufReader::new(host);
    let started = (|| -> io::Result<(OwnedFd, Option<TcpListener>)> {
        become_box_user(request.uid, request.gid)?;
        for (fill, missing) in &request.fills {
            super::receive_into(fill, &mut host, *missing).map_err(|error| {
                io::Error::other(format!("copy into {}: {error}", fill.display()))
            })?;
        }
        let exec = vsock_listen(EXEC_PORT)?;
        // Without a proxy nothing listens, so a request finds no listener.
        let relay = (request.relay)
            .then(|| TcpListener::bind(super::RELAY_LISTEN))
            .transpose()?;
        Ok((exec, relay))
    })();
    let mut host = host.into_inner();
    let (exec, relay) = match started {
        Ok(listeners) => listeners,
        Err(error) => fail(&mut host, &error),
    };
    if let Some(relay) = relay
        && let Err(error) = super::serve_relay(relay, || vsock_connect(PROXY_PORT))
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
    // Lowering needs privilege, raising does not: exec'd work goes to 1000.
    fs::write("/proc/self/oom_score_adj", "-1000")?;
    drop_bounding(None)?;
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
            let mut fields = Fields(&payload);
            let vars = pairs(&mut fields)?;
            fields.end()?;
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
            exec(connection, &payload, vars)
        }
        _ => write_frame(
            &mut connection,
            FAILED,
            format!("unknown request {kind}").as_bytes(),
        ),
    }
}

fn pairs(fields: &mut Fields) -> io::Result<Vec<(OsString, OsString)>> {
    (0..fields.u32()?)
        .map(|_| Ok((fields.os()?, fields.os()?)))
        .collect()
}

/// Run one EXEC request in its own session through `init exec`, which sets
/// `oom_score_adj` 1000, and relay its stdio until it exits and its output
/// ends. A host that goes away only closes stdin: pinfold never signals box
/// work through exec.
fn exec(connection: UnixStream, payload: &[u8], vars: Vec<(OsString, OsString)>) -> io::Result<()> {
    let mut fields = Fields(payload);
    let argv: Vec<OsString> = (0..fields.u32()?)
        .map(|_| fields.os())
        .collect::<io::Result<_>>()?;
    let workdir = fields.os()?;
    let tty = fields.u8()? != 0;
    let (rows, cols) = (fields.u16()?, fields.u16()?);
    let extra = pairs(&mut fields)?;
    fields.end()?;
    let Some(program) = argv.first() else {
        return Err(io::Error::other("an exec without argv"));
    };

    // `init exec` restores the environment from Apple's transport names, so
    // a name that itself carries the prefix arrives exactly.
    let transport = vars.into_iter().chain(extra).map(|(name, value)| {
        let mut key = OsString::from(super::BOX_ENV_PREFIX);
        key.push(name);
        let hex: String = value
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        (key, hex)
    });
    // The child is a copy of the service, which holds the exec listener,
    // until its first execve; it stays non-dumpable until then (#158). The
    // init disk holding `/pinfold` is detached, but its magic link execs.
    let mut command = Command::new("/proc/self/exe");
    command
        .args(["init", "exec", "--"])
        .args(&argv)
        .env_clear()
        .envs(transport)
        .current_dir(if workdir.is_empty() {
            OsStr::new("/")
        } else {
            &workdir
        });
    let pty = if tty {
        let (master, slave) = open_pty(rows, cols)?;
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

    let output = Arc::new(Mutex::new(connection.try_clone()?));
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let mut output = output.lock().expect("output lock");
            let message = format!(
                "pinfold init exec: {}: {error}\n",
                Path::new(program).display()
            );
            write_frame(&mut *output, STDERR, message.as_bytes())?;
            // The shell's convention for a command that could not be run.
            let code: i32 = if error.kind() == io::ErrorKind::NotFound {
                127
            } else {
                126
            };
            return write_frame(&mut *output, EXIT, &code.to_be_bytes());
        }
    };
    drop(command);

    let mut pumps = Vec::new();
    let input: Box<dyn Write + Send> = match &pty {
        Some(master) => {
            pumps.push(pump(master.try_clone()?, STDOUT, &output)?);
            Box::new(master.try_clone()?)
        }
        None => {
            pumps.push(pump(child.stdout.take().expect("piped"), STDOUT, &output)?);
            pumps.push(pump(child.stderr.take().expect("piped"), STDERR, &output)?);
            Box::new(child.stdin.take().expect("piped"))
        }
    };
    let resize = match &pty {
        Some(master) => Some(master.try_clone()?),
        None => None,
    };
    let mut reader = connection.try_clone()?;
    thread::Builder::new().spawn(move || relay_input(&mut reader, input, resize.as_ref()))?;

    let status = child.wait()?;
    for pump in pumps {
        let _ = pump.join();
    }
    let code = status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1);
    write_frame(
        &mut *output.lock().expect("output lock"),
        EXIT,
        &code.to_be_bytes(),
    )?;
    // Ends the input relay's read; the host has the exit code.
    let _ = connection.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// Copy `from` into STDOUT or STDERR frames until it ends. After the host
/// goes away it keeps reading and discards, so box work never blocks on a
/// full pipe.
fn pump(
    mut from: impl Read + Send + 'static,
    kind: u8,
    output: &Arc<Mutex<UnixStream>>,
) -> io::Result<thread::JoinHandle<()>> {
    let output = Arc::clone(output);
    thread::Builder::new().spawn(move || {
        let mut buffer = [0; 32 * 1024];
        let mut open = true;
        loop {
            match from.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if open {
                        let mut output = output.lock().expect("output lock");
                        open = write_frame(&mut *output, kind, &buffer[..read]).is_ok();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                // A pty master reads EIO once every slave is closed.
                Err(_) => break,
            }
        }
    })
}

fn relay_input(connection: &mut UnixStream, input: Box<dyn Write + Send>, pty: Option<&File>) {
    let mut input = Some(input);
    while let Ok((kind, payload)) = read_frame(connection) {
        match kind {
            STDIN => {
                if let Some(writer) = input.as_mut()
                    && writer.write_all(&payload).is_err()
                {
                    input = None;
                }
            }
            // A terminal has no end of input to pass on; a pipe closes.
            STDIN_END if pty.is_none() => input = None,
            RESIZE => {
                if let (Some(master), &[r0, r1, c0, c1]) = (pty, payload.as_slice()) {
                    let _ = set_window(
                        master,
                        u16::from_be_bytes([r0, r1]),
                        u16::from_be_bytes([c0, c1]),
                    );
                }
            }
            _ => {}
        }
    }
}

fn open_pty(rows: u16, cols: u16) -> io::Result<(File, File)> {
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
        set_window(&master, rows, cols).map_err(|error| context("set the window", error))?;
        Ok((master, slave))
    }
}

fn context(what: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

fn set_window(master: &File, rows: u16, cols: u16) -> io::Result<()> {
    let size = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // Safety: TIOCSWINSZ reads one winsize.
    if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &size) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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
fn vsock_connect(port: u32) -> io::Result<UnixStream> {
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
    nix::sys::socket::listen(&socket, Backlog::new(16)?)?;
    Ok(socket)
}

/// Accept the next connection from the host. Any other peer is closed at
/// once. The guest kernel has no vsock loopback, so none can reach a guest
/// listener today; the check keeps that true if one ever does. A failed
/// accept backs off. Either way there is no connection.
fn accept_host(listener: &OwnedFd) -> Option<UnixStream> {
    let Ok(fd) = nix::sys::socket::accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC) else {
        super::back_off();
        return None;
    };
    // Safety: accept4 returned a new descriptor, owned here alone.
    let connection = unsafe { OwnedFd::from_raw_fd(fd) };
    match nix::sys::socket::getpeername::<VsockAddr>(connection.as_raw_fd()) {
        Ok(peer) if peer.cid() == HOST_CID => Some(UnixStream::from(connection)),
        _ => None,
    }
}

pub(crate) fn write_frame(output: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(payload.len())
        .ok()
        .filter(|&length| length as usize <= FRAME_MAX)
        .ok_or_else(|| io::Error::other("frame too large"))?;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(kind);
    frame.extend(length.to_be_bytes());
    frame.extend(payload);
    output.write_all(&frame)
}

/// Read one frame exactly, never past it: the boot connection's next bytes
/// belong to the exec service.
pub(crate) fn read_frame(input: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0; 5];
    input.read_exact(&mut head)?;
    let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if length > FRAME_MAX {
        return Err(io::Error::other("frame too large"));
    }
    let mut payload = vec![0; length];
    input.read_exact(&mut payload)?;
    Ok((head[0], payload))
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

    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
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
