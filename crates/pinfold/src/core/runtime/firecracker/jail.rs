//! The VMM jail `box up` builds before it execs Firecracker (VMM jail).
//!
//! The child of `clone` is a copy of a multithreaded process, so between
//! `clone` and `execve` it makes only system calls: every path, map and
//! argument is built before it, and the child never allocates.

use std::ffi::{CStr, CString, c_char};
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use nix::errno::Errno;
use nix::fcntl::{OFlag, open};
use nix::libc;
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sched::CloneFlags;
use nix::sys::stat::Mode;
use nix::sys::statvfs::{FsFlags, statvfs};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::Pid;

use crate::init::firecracker::{drop_bounding, pidfd_open, set_capabilities};

/// Paths inside the jail. The VM configuration names the kernel, the disks
/// and the vsock socket by these.
pub const FIRECRACKER: &str = "/firecracker";
pub const KERNEL: &str = "/vmlinux";
pub const ROOT_DISK: &str = "/root.ext4";
pub const INIT_DISK: &str = "/init.ext4";
pub const CONFIG: &str = "/config.json";
const VSOCK: &str = "/vsock";
/// The VM's vsock socket. Firecracker adds `_<port>` for a host listener.
pub const VSOCK_PATH: &str = "/vsock/v.sock";
const KVM: &str = "/dev/kvm";

/// The jail path of the box's disk at `index` in [`Jail::disks`].
pub fn disk(index: usize) -> String {
    format!("/disk{index}.ext4")
}

/// What one VMM's jail holds: the host paths the spec lists.
pub struct Jail<'a> {
    /// An empty directory, private to this box, that the jail's tmpfs root
    /// is mounted on in the VMM's own mount namespace.
    pub root: &'a Path,
    pub firecracker: &'a Path,
    pub kernel: &'a Path,
    pub root_disk: &'a Path,
    pub init_disk: &'a Path,
    /// The box's drives after the root and init disks, each with whether
    /// the VM writes it: the box's disks, its mounts' images and its `/tmp`
    /// image.
    pub disks: &'a [(PathBuf, bool)],
    /// The directory that holds the vsock socket and its host listeners.
    pub vsock: &'a File,
    /// The VM configuration file, read with `--config-file`.
    pub config: &'a Path,
}

/// A started VMM. Its owner is a dedicated thread that stays in `waitpid`
/// for the VMM's lifetime.
pub struct Vmm {
    /// Opened before the owner thread can reap the VMM, so a signal sent
    /// through it never reaches a reused pid.
    pidfd: OwnedFd,
    status: oneshot::Receiver<io::Result<i32>>,
    /// The outcome once received: the receiver must not be polled again.
    ended: Option<Result<i32, String>>,
}

impl Vmm {
    /// Wait until the VMM ends: its exit code, or `128+n` for signal `n`.
    /// Cancel-safe, and answers again once it has.
    pub async fn wait(&mut self) -> io::Result<i32> {
        if self.ended.is_none() {
            let outcome = match (&mut self.status).await {
                Ok(outcome) => outcome.map_err(|error| error.to_string()),
                Err(_) => Err("the VMM's owner thread ended without a status".to_string()),
            };
            self.ended = Some(outcome);
        }
        match self.ended.as_ref().expect("an outcome was received") {
            Ok(code) => Ok(*code),
            Err(error) => Err(io::Error::other(error.clone())),
        }
    }

    /// The pidfd becomes readable at exit, before the wait thread publishes status.
    pub fn exited(&self) -> io::Result<bool> {
        let mut fds = [PollFd::new(self.pidfd.as_fd(), PollFlags::POLLIN)];
        Ok(poll(&mut fds, PollTimeout::ZERO)? != 0)
    }

    /// SIGKILL the VMM through its pidfd. One that has ended is left alone.
    pub fn kill(&self) -> io::Result<()> {
        kill(&self.pidfd)
    }
}

/// SIGKILL the process `pidfd` names. One that has ended is left alone.
fn kill(pidfd: &OwnedFd) -> io::Result<()> {
    // SAFETY: pidfd_send_signal on an owned pidfd, no siginfo.
    let sent = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            libc::SIGKILL,
            std::ptr::null::<libc::siginfo_t>(),
            0u32,
        )
    };
    match Errno::result(sent) {
        Ok(_) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(io::Error::other(format!("kill the VMM: {error}"))),
    }
}

/// Build the jail and exec Firecracker in it, with `--no-api` and its own
/// seccomp filters. The VMM gets `/dev/null` as stdin, stdout and stderr,
/// from descriptor 3 a read-write descriptor of each image it writes, holding
/// that image's exclusive lock for the VMM's life, no other descriptor,
/// default signal handling and an empty environment: Firecracker's serial
/// console is its stdio, and `box up`'s stdio is the caller's.
///
/// `PR_SET_PDEATHSIG` fires when the thread that created the child exits,
/// not the process (#126). The child is created from a thread that lives
/// exactly as long as the VMM, so the VM ends with its owner process.
pub fn spawn(jail: &Jail) -> io::Result<Vmm> {
    let plan = Plan::jail(jail)?;
    // Retained through child mounting.
    let vsock = jail.vsock.try_clone()?;
    let (started_tx, started_rx) = mpsc::channel();
    let (status_tx, status_rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("vmm".into())
        .spawn(move || {
            let started = (|| {
                // Clone the mount namespace only after this thread has its
                // own cwd pinned to the retained directory. The kernel then
                // rebases that cwd onto the child's copy of the mount.
                nix::sched::unshare(CloneFlags::CLONE_FS)
                    .map_err(|error| io::Error::other(format!("VMM jail: isolate cwd: {error}")))?;
                nix::unistd::fchdir(&vsock).map_err(|error| {
                    io::Error::other(format!("VMM jail: enter socket directory: {error}"))
                })?;
                start(&plan)
            })();
            let (pid, _alive, pidfd) = match started {
                Ok(started) => started,
                Err(error) => {
                    let _ = started_tx.send(Err(error));
                    return;
                }
            };
            if started_tx.send(Ok(pidfd)).is_err() {
                let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
                let _ = wait(pid);
                return;
            }
            // `_alive`'s write end stays open until the VMM has ended.
            let _ = status_tx.send(wait(pid));
        })?;
    let pidfd = started_rx
        .recv()
        .map_err(|_| io::Error::other("the VMM's owner thread ended before it started"))??;
    Ok(Vmm {
        pidfd,
        status: status_rx,
        ended: None,
    })
}

/// Make the jail's namespaces, maps and tmpfs root in a child that then
/// exits, the steps where an unprivileged user namespace is refused.
/// Preflight's `userns` check.
pub fn probe() -> io::Result<()> {
    let plan = Plan::probe()?;
    let (pid, _alive, pidfd) = start(&plan)?;
    loop {
        match crate::core::pipe::wait_ready(
            Some(pidfd.as_fd()),
            PollFlags::POLLIN,
            Instant::now() + Duration::from_millis(100),
            None,
            "namespace probe",
        ) {
            Ok(()) => {
                return match wait(pid)? {
                    0 => Ok(()),
                    code => Err(io::Error::other(format!(
                        "the user namespace probe exited {code}"
                    ))),
                };
            }
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {}
            Err(error) => {
                // The unreaped child and retained pidfd still identify this probe.
                let _ = kill(&pidfd);
                let _ = wait(pid);
                return Err(error);
            }
        }
    }
}

/// Everything the child needs, built before `clone`.
struct Plan {
    uid_map: Vec<u8>,
    gid_map: Vec<u8>,
    /// The tmpfs root, on the host's side of `pivot_root`.
    root: CString,
    binds: Vec<Bind>,
    /// The images the VMM writes, which it holds locked.
    locks: Vec<CString>,
    /// `None` for the probe, which exits after mounting its tmpfs.
    exec: Option<Exec>,
}

struct Bind {
    source: CString,
    target: CString,
    directory: bool,
    /// The flags of a read-only remount, `None` for a writable bind.
    read_only: Option<MsFlags>,
}

struct Exec {
    /// The jail's `/dev`, made in the tmpfs root before the binds.
    dev: CString,
    /// Paths inside the jail with their Landlock rights.
    rules: Vec<(CString, u64)>,
    argv: Vec<CString>,
}

impl Plan {
    fn probe() -> io::Result<Plan> {
        let (uid_map, gid_map) = maps();
        Ok(Plan {
            uid_map,
            gid_map,
            // In the probe's own mount namespace, over the host's root.
            root: c"/".to_owned(),
            binds: Vec::new(),
            locks: Vec::new(),
            exec: None,
        })
    }

    fn jail(jail: &Jail) -> io::Result<Plan> {
        let (uid_map, gid_map) = maps();
        let inside = |name: &str| -> io::Result<CString> { cstring(Path::new(name)) };
        let target = |name: &str| -> io::Result<CString> {
            cstring(&jail.root.join(name.trim_start_matches('/')))
        };
        let read_only = |source: &Path, name: &str| -> io::Result<Bind> {
            Ok(Bind {
                source: cstring(source)?,
                target: target(name)?,
                directory: false,
                read_only: Some(locked_flags(source)?),
            })
        };
        let writable = |source: &Path, name: &str, directory: bool| -> io::Result<Bind> {
            Ok(Bind {
                source: cstring(source)?,
                target: target(name)?,
                directory,
                read_only: None,
            })
        };
        let mut binds = vec![
            read_only(jail.firecracker, FIRECRACKER)?,
            read_only(jail.kernel, KERNEL)?,
            read_only(jail.root_disk, ROOT_DISK)?,
            read_only(jail.init_disk, INIT_DISK)?,
            read_only(jail.config, CONFIG)?,
            writable(Path::new(KVM), KVM, false)?,
            writable(Path::new("."), VSOCK, true)?,
        ];
        let mut rules = vec![
            (
                inside(FIRECRACKER)?,
                landlock::EXECUTE | landlock::READ_FILE,
            ),
            (inside(KERNEL)?, landlock::READ_FILE),
            (inside(ROOT_DISK)?, landlock::READ_FILE),
            (inside(INIT_DISK)?, landlock::READ_FILE),
            (inside(CONFIG)?, landlock::READ_FILE),
            (
                inside(KVM)?,
                landlock::READ_FILE | landlock::WRITE_FILE | landlock::IOCTL_DEV,
            ),
            (
                inside(VSOCK)?,
                landlock::MAKE_SOCK
                    | landlock::RESOLVE_UNIX
                    | landlock::REMOVE_FILE
                    | landlock::READ_DIR,
            ),
        ];
        let mut locks = Vec::new();
        for (index, (source, write)) in jail.disks.iter().enumerate() {
            let name = disk(index);
            if *write {
                locks.push(cstring(source)?);
                binds.push(writable(source, &name, false)?);
                rules.push((inside(&name)?, landlock::READ_FILE | landlock::WRITE_FILE));
            } else {
                binds.push(read_only(source, &name)?);
                rules.push((inside(&name)?, landlock::READ_FILE));
            }
        }
        let argv = [FIRECRACKER, "--config-file", CONFIG, "--no-api"]
            .into_iter()
            .map(|arg| CString::new(arg).expect("no NUL in Firecracker's arguments"))
            .collect();
        Ok(Plan {
            uid_map,
            gid_map,
            root: cstring(jail.root)?,
            binds,
            locks,
            exec: Some(Exec {
                dev: target("/dev")?,
                rules,
                argv,
            }),
        })
    }
}

/// Only the host uid and gid, each mapped to itself.
fn maps() -> (Vec<u8>, Vec<u8>) {
    let uid = nix::unistd::getuid();
    let gid = nix::unistd::getgid();
    (
        format!("{uid} {uid} 1\n").into_bytes(),
        format!("{gid} {gid} 1\n").into_bytes(),
    )
}

fn cstring(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::other(format!("{} contains a NUL byte", path.display())))
}

/// A read-only remount in a user namespace must keep the source mount's
/// locked flags, or the kernel refuses it.
fn locked_flags(source: &Path) -> io::Result<MsFlags> {
    let flags = statvfs(source)
        .map_err(|error| io::Error::other(format!("statvfs {}: {error}", source.display())))?
        .flags();
    let mut locked =
        MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY | MsFlags::MS_NOSUID;
    for (statvfs, mount) in [
        (FsFlags::ST_NODEV, MsFlags::MS_NODEV),
        (FsFlags::ST_NOEXEC, MsFlags::MS_NOEXEC),
        (FsFlags::ST_NOATIME, MsFlags::MS_NOATIME),
        (FsFlags::ST_NODIRATIME, MsFlags::MS_NODIRATIME),
        // `ST_RELATIME`, which musl's headers do not name.
        (FsFlags::from_bits_retain(0x1000), MsFlags::MS_RELATIME),
    ] {
        if flags.contains(statvfs) {
            locked |= mount;
        }
    }
    Ok(locked)
}

/// The steps the child reports a failure from, by index.
const STEPS: &[&str] = &[
    "set the parent-death signal",
    "write setgroups",
    "write uid_map",
    "write gid_map",
    "make / private",
    "mount the jail's tmpfs",
    "make a jail directory",
    "make a bind target",
    "bind",
    "remount read-only",
    "remount the jail's root read-only",
    "enter the jail's root",
    "pivot_root",
    "detach the old root",
    "set no_new_privs",
    "read the Landlock ABI",
    "create the Landlock ruleset",
    "open a Landlock path",
    "add a Landlock rule",
    "restrict to the Landlock ruleset",
    "drop the bounding set",
    "clear the capability sets",
    "redirect stdio to /dev/null",
    "reset the signal mask",
    "mark descriptors close-on-exec",
    "keep the image locks",
    "exec Firecracker",
];

fn step(name: &str) -> u32 {
    STEPS
        .iter()
        .position(|step| *step == name)
        .expect("a listed step") as u32
}

/// A child failure: the step, the bind or rule index it concerns, and the
/// errno.
type Failure = (u32, u32, Errno);

fn fail(name: &'static str, index: usize) -> impl Fn(Errno) -> Failure {
    move |errno| (step(name), index as u32, errno)
}

/// `clone` the child with the jail's namespaces and return once it has
/// exec'd (or, for the probe, before it exits). The returned pipe end keeps
/// the child's parent-alive check passing; drop it only after `wait`.
fn start(plan: &Plan) -> io::Result<(Pid, OwnedFd, OwnedFd)> {
    // Every descriptor the child uses sits above the slots the locks move to.
    let floor = 3 + plan.locks.len() as RawFd;
    let locks = plan
        .locks
        .iter()
        .map(|image| lock(image, floor))
        .collect::<io::Result<Vec<_>>>()?;
    let lock_fds: Vec<RawFd> = locks.iter().map(AsRawFd::as_raw_fd).collect();
    let (alive_read, alive_write) = nix::unistd::pipe2(OFlag::O_CLOEXEC)?;
    let (report_read, report_write) = nix::unistd::pipe2(OFlag::O_CLOEXEC)?;
    let alive_read = above(alive_read, floor)?;
    let report_write = above(report_write, floor)?;
    let exec = plan.exec.as_ref().map(|exec| {
        let argv: Vec<*const c_char> = exec
            .argv
            .iter()
            .map(|arg| arg.as_ptr())
            .chain([std::ptr::null()])
            .collect();
        (exec, argv)
    });
    let envp: [*const c_char; 1] = [std::ptr::null()];
    // The VMM's stdin, stdout and stderr.
    let null_fd = above(
        open(
            c"/dev/null",
            OFlag::O_RDWR | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?,
        floor,
    )?;
    let null = null_fd.as_raw_fd();
    let probe = plan.exec.is_none();
    let fds = (
        alive_read.as_raw_fd(),
        alive_write.as_raw_fd(),
        report_read.as_raw_fd(),
        report_write.as_raw_fd(),
    );
    let mut stack = vec![0u8; 256 * 1024];
    let child = Box::new(|| {
        let (mut alive_read, alive_write, report_read, mut report_write) = fds;
        // SAFETY: the child's copies of the parent's ends.
        unsafe {
            libc::close(alive_write);
            libc::close(report_read);
            libc::umask(0o077);
        }
        if probe {
            // A probe never execs, so CLOEXEC cannot remove inherited authority.
            // Duplicate first: callers may have started with closed stdio.
            unsafe {
                let alive = libc::fcntl(alive_read, libc::F_DUPFD_CLOEXEC, 5);
                let report = libc::fcntl(report_write, libc::F_DUPFD_CLOEXEC, 5);
                let null = libc::fcntl(null, libc::F_DUPFD_CLOEXEC, 5);
                if alive < 0
                    || report < 0
                    || null < 0
                    || libc::dup2(alive, 3) < 0
                    || libc::dup2(report, 4) < 0
                    || libc::dup2(null, 0) < 0
                    || libc::dup2(null, 1) < 0
                    || libc::dup2(null, 2) < 0
                    || !crate::core::process::close_fds_from(5)
                {
                    libc::_exit(127);
                }
            }
            alive_read = 3;
            report_write = 4;
        }
        let failure = match enter(plan, alive_read) {
            Err(failure) => failure,
            Ok(()) => match &exec {
                None => unsafe { libc::_exit(0) },
                Some((exec, argv)) => match restrict(exec) {
                    Err(failure) => failure,
                    Ok(()) => match exec_ready(null, &lock_fds) {
                        Err(failure) => failure,
                        Ok(()) => {
                            // SAFETY: argv and envp are NUL-terminated arrays
                            // of C strings that outlive the call.
                            unsafe {
                                // Unix bind starts from 0777; the VMM's sockets are 0600.
                                libc::umask(0o177);
                                libc::execve(argv[0], argv.as_ptr(), envp.as_ptr())
                            };
                            (step("exec Firecracker"), 0, Errno::last())
                        }
                    },
                },
            },
        };
        let (step, index, errno) = failure;
        let mut report = [0u8; 12];
        report[..4].copy_from_slice(&step.to_ne_bytes());
        report[4..8].copy_from_slice(&index.to_ne_bytes());
        report[8..].copy_from_slice(&(errno as i32).to_ne_bytes());
        // SAFETY: a write of a stack buffer, then exit without unwinding.
        unsafe {
            libc::write(report_write, report.as_ptr().cast(), report.len());
            libc::_exit(127)
        }
    });
    let flags = CloneFlags::CLONE_NEWUSER
        | CloneFlags::CLONE_NEWNS
        | CloneFlags::CLONE_NEWPID
        | CloneFlags::CLONE_NEWNET
        | CloneFlags::CLONE_NEWIPC
        | CloneFlags::CLONE_NEWUTS;
    // SAFETY: the child runs `child` on `stack` and makes only system calls
    // on memory built above, then execs or exits.
    let pid = {
        let _guard = crate::core::process::fork_guard();
        unsafe { nix::sched::clone(child, &mut stack, flags, Some(libc::SIGCHLD)) }
    };
    // The child's copies are then the only ones, so the locks end with it.
    drop(locks);
    let pid = pid.map_err(|error| io::Error::other(format!("VMM jail: clone: {error}")))?;
    drop(report_write);
    drop(alive_read);
    let mut report_read = File::from(report_read);
    let result = (|| {
        let pidfd = pidfd_open(pid.as_raw())?;
        if probe {
            crate::core::pipe::nonblocking(&report_read, "namespace probe")?;
        }
        let mut report = [0u8; 12];
        let mut filled = 0;
        while filled < report.len() {
            let received = if probe {
                match crate::core::pipe::read(
                    &mut report_read,
                    &mut report[filled..],
                    Instant::now() + Duration::from_millis(100),
                    None,
                    "namespace probe",
                ) {
                    Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
                    result => result,
                }
            } else {
                match nix::unistd::read(&report_read, &mut report[filled..]) {
                    Ok(read) => Ok(read),
                    Err(Errno::EINTR) => continue,
                    Err(error) => {
                        return Err(io::Error::other(format!("VMM jail: read report: {error}")));
                    }
                }
            };
            match received {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(error) => return Err(error),
            }
        }
        if filled == 0 {
            return Ok(pidfd);
        }
        if filled < report.len() {
            return Err(io::Error::other(format!(
                "VMM jail: the child's failure report was cut short at {filled} bytes"
            )));
        }
        let field =
            |range: std::ops::Range<usize>| u32::from_ne_bytes(report[range].try_into().unwrap());
        let (step, index) = (field(0..4) as usize, field(4..8) as usize);
        let errno = Errno::from_raw(field(8..12) as i32);
        let what = STEPS.get(step).copied().unwrap_or("an unknown step");
        let path = match what {
            "make a jail directory" => plan.exec.as_ref().map(|exec| &exec.dev),
            "make a bind target" | "bind" | "remount read-only" => {
                plan.binds.get(index).map(|bind| &bind.target)
            }
            "open a Landlock path" | "add a Landlock rule" => plan
                .exec
                .as_ref()
                .and_then(|exec| exec.rules.get(index))
                .map(|(path, _)| path),
            _ => None,
        };
        let detail = path
            .map(|path| format!(" {}", path.to_string_lossy()))
            .unwrap_or_default();
        let error = io::Error::from(errno);
        Err(io::Error::new(
            error.kind(),
            format!("VMM jail: {what}{detail}: {error}"),
        ))
    })();
    match result {
        Ok(pidfd) => Ok((pid, alive_write, pidfd)),
        Err(error) => {
            drop(alive_write);
            // The child is still unreaped, so this PID cannot have been reused.
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
            let _ = wait(pid);
            Err(error)
        }
    }
}

/// A copy of `fd` at `floor` or above, close-on-exec.
fn above(fd: OwnedFd, floor: RawFd) -> io::Result<OwnedFd> {
    if fd.as_raw_fd() >= floor {
        return Ok(fd);
    }
    // SAFETY: F_DUPFD_CLOEXEC on an open descriptor.
    let raised = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, floor) };
    // SAFETY: a new descriptor the kernel just returned.
    Ok(unsafe { OwnedFd::from_raw_fd(Errno::result(raised)?) })
}

/// Open `image` and take its exclusive lock, which the VMM keeps for its
/// life (VMM jail). The descriptor is read-write because NFS grants
/// `LOCK_EX` only on one; it is opened outside the jail, so Landlock does
/// not govern it. A liveness probe holds a shared lock for an instant, so a
/// held lock is retried for up to a second; one held longer refuses the
/// start rather than block it.
fn lock(image: &CStr, floor: RawFd) -> io::Result<OwnedFd> {
    let path = image.to_string_lossy();
    let fd = open(image, OFlag::O_RDWR | OFlag::O_CLOEXEC, Mode::empty())
        .map_err(|error| io::Error::other(format!("VMM jail: open {path}: {error}")))?;
    let fd = above(fd, floor)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        // SAFETY: flock on an owned descriptor.
        match Errno::result(unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) {
            Ok(_) => return Ok(fd),
            Err(Errno::EINTR) => {}
            Err(Errno::EWOULDBLOCK) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(Errno::EWOULDBLOCK) => {
                return Err(io::Error::other(format!("disk-in-use: {path} is locked")));
            }
            Err(error) => {
                return Err(io::Error::other(format!("VMM jail: lock {path}: {error}")));
            }
        }
    }
}

/// Reap the child: its exit code, or `128+n` for signal `n`.
fn wait(pid: Pid) -> io::Result<i32> {
    loop {
        match waitpid(pid, None) {
            Ok(WaitStatus::Exited(_, code)) => return Ok(code),
            Ok(WaitStatus::Signaled(_, signal, _)) => return Ok(128 + signal as i32),
            Ok(_) | Err(Errno::EINTR) => {}
            Err(error) => return Err(io::Error::other(format!("wait for the VMM: {error}"))),
        }
    }
}

/// In the child: the parent-death signal, the maps, the mount tree and
/// `pivot_root`.
fn enter(plan: &Plan, alive: RawFd) -> Result<(), Failure> {
    nix::sys::prctl::set_pdeathsig(nix::sys::signal::Signal::SIGKILL)
        .map_err(fail("set the parent-death signal", 0))?;
    // The owner may have died before the signal was set: then the pipe's
    // only write end is gone.
    let mut poll = libc::pollfd {
        fd: alive,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one pollfd on the stack.
    if unsafe { libc::poll(&mut poll, 1, 0) } != 0 {
        // SAFETY: exit without unwinding.
        unsafe { libc::_exit(127) }
    }
    write_proc(c"/proc/self/setgroups", b"deny").map_err(fail("write setgroups", 0))?;
    write_proc(c"/proc/self/uid_map", &plan.uid_map).map_err(fail("write uid_map", 0))?;
    write_proc(c"/proc/self/gid_map", &plan.gid_map).map_err(fail("write gid_map", 0))?;
    let none = None::<&CStr>;
    mount(
        none,
        c"/",
        none,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        none,
    )
    .map_err(fail("make / private", 0))?;
    let tmpfs_data = c"mode=0755,size=64k";
    mount(
        Some(c"tmpfs"),
        plan.root.as_c_str(),
        Some(c"tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some(tmpfs_data),
    )
    .map_err(fail("mount the jail's tmpfs", 0))?;
    let Some(exec) = &plan.exec else {
        return Ok(());
    };
    nix::unistd::mkdir(exec.dev.as_c_str(), Mode::from_bits_truncate(0o755))
        .map_err(fail("make a jail directory", 0))?;
    for (index, bind) in plan.binds.iter().enumerate() {
        if bind.directory {
            nix::unistd::mkdir(bind.target.as_c_str(), Mode::from_bits_truncate(0o700))
                .map_err(fail("make a bind target", index))?;
        } else {
            open(
                bind.target.as_c_str(),
                OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_WRONLY | OFlag::O_CLOEXEC,
                Mode::from_bits_truncate(0o600),
            )
            .map_err(fail("make a bind target", index))?;
        }
        mount(
            Some(bind.source.as_c_str()),
            bind.target.as_c_str(),
            none,
            MsFlags::MS_BIND,
            none,
        )
        .map_err(fail("bind", index))?;
        if let Some(flags) = bind.read_only {
            mount(none, bind.target.as_c_str(), none, flags, none)
                .map_err(fail("remount read-only", index))?;
        }
    }
    // Nothing new can be made in the jail's root.
    mount(
        none,
        plan.root.as_c_str(),
        none,
        MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some(tmpfs_data),
    )
    .map_err(fail("remount the jail's root read-only", 0))?;
    nix::unistd::chdir(plan.root.as_c_str()).map_err(fail("enter the jail's root", 0))?;
    nix::unistd::pivot_root(c".", c".").map_err(fail("pivot_root", 0))?;
    umount2(c".", MntFlags::MNT_DETACH).map_err(fail("detach the old root", 0))?;
    nix::unistd::chdir(c"/").map_err(fail("enter the jail's root", 0))?;
    Ok(())
}

/// In the child, last before `execve`: `/dev/null` as stdio, an empty
/// signal mask and default dispositions (Rust's runtime ignores SIGPIPE),
/// every descriptor above stderr close-on-exec, the inherited ones
/// included, then the image locks at 3, 4, … without it. The report pipe
/// stays open until the exec succeeds.
fn exec_ready(null: RawFd, locks: &[RawFd]) -> Result<(), Failure> {
    // `/dev/null` is above stderr (`start`), so dup2 never lands on itself.
    for target in 0..=2 {
        // SAFETY: dup2 of an open descriptor.
        if unsafe { libc::dup2(null, target) } < 0 {
            return Err(fail("redirect stdio to /dev/null", 0)(Errno::last()));
        }
    }
    // SAFETY: an empty set on the stack.
    let mut empty: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: sigemptyset and sigprocmask on that set.
    if unsafe {
        libc::sigemptyset(&mut empty);
        libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut())
    } != 0
    {
        return Err(fail("reset the signal mask", 0)(Errno::last()));
    }
    for signal in 1..=64 {
        // SIGKILL and SIGSTOP refuse; every other one takes the default.
        // SAFETY: signal with SIG_DFL.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
    // SAFETY: close_range with the cloexec flag closes nothing now.
    if unsafe {
        libc::syscall(
            libc::SYS_close_range,
            3 as libc::c_uint,
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        )
    } != 0
    {
        return Err(fail("mark descriptors close-on-exec", 0)(Errno::last()));
    }
    // Each lock is above its slot, so dup2 never lands on itself, which
    // would keep close-on-exec. Firecracker's own dup2 near the descriptor
    // limit stays clear of these low slots.
    for (slot, &fd) in locks.iter().enumerate() {
        // SAFETY: dup2 of an open descriptor.
        if unsafe { libc::dup2(fd, 3 + slot as RawFd) } < 0 {
            return Err(fail("keep the image locks", slot)(Errno::last()));
        }
    }
    Ok(())
}

fn write_proc(path: &CStr, bytes: &[u8]) -> Result<(), Errno> {
    let fd = open(path, OFlag::O_WRONLY | OFlag::O_CLOEXEC, Mode::empty())?;
    let written = nix::unistd::write(&fd, bytes)?;
    if written == bytes.len() {
        Ok(())
    } else {
        Err(Errno::EIO)
    }
}

/// In the child, inside the jail: `no_new_privs`, Landlock, then no
/// capabilities.
fn restrict(exec: &Exec) -> Result<(), Failure> {
    nix::sys::prctl::set_no_new_privs().map_err(fail("set no_new_privs", 0))?;
    let abi = landlock::abi().map_err(fail("read the Landlock ABI", 0))?;
    if abi < 5 {
        return Err(fail("read the Landlock ABI", 0)(Errno::EOPNOTSUPP));
    }
    let ruleset = landlock::ruleset(abi).map_err(fail("create the Landlock ruleset", 0))?;
    for (index, (path, access)) in exec.rules.iter().enumerate() {
        let fd = open(
            path.as_c_str(),
            OFlag::O_PATH | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(fail("open a Landlock path", index))?;
        // A right the running ABI lacks is not handled, so not allowed.
        landlock::allow(&ruleset, &fd, access & landlock::handled_fs(abi))
            .map_err(fail("add a Landlock rule", index))?;
    }
    landlock::restrict(&ruleset).map_err(fail("restrict to the Landlock ruleset", 0))?;
    // A raw OS error carries its errno without allocating.
    let errno = |error: io::Error| Errno::from_raw(error.raw_os_error().unwrap_or(0));
    drop_bounding(0).map_err(|error| fail("drop the bounding set", 0)(errno(error)))?;
    set_capabilities(None).map_err(|error| fail("clear the capability sets", 0)(errno(error)))
}

/// Landlock through raw system calls (`linux/landlock.h`).
pub mod landlock {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use nix::errno::Errno;
    use nix::libc;

    pub const EXECUTE: u64 = 1 << 0;
    pub const WRITE_FILE: u64 = 1 << 1;
    pub const READ_FILE: u64 = 1 << 2;
    pub const READ_DIR: u64 = 1 << 3;
    pub const REMOVE_FILE: u64 = 1 << 5;
    pub const MAKE_SOCK: u64 = 1 << 9;
    pub const IOCTL_DEV: u64 = 1 << 15;
    pub const RESOLVE_UNIX: u64 = 1 << 16;

    /// Every filesystem right of accepted ABIs 5 to 10: the first 16 in
    /// ABI 5, `RESOLVE_UNIX` in 9. ABIs 4 and 10 add network rights, 6 adds
    /// scopes, 7 and 8 add none. A newer ABI's
    /// new rights stay unhandled until they are listed here (ARCHITECTURE.md
    /// "VMM jail").
    pub fn handled_fs(abi: i32) -> u64 {
        let mut rights = (1 << 16) - 1;
        if abi >= 9 {
            rights |= RESOLVE_UNIX;
        }
        rights
    }

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
        handled_access_net: u64,
        scoped: u64,
    }

    #[repr(C, packed)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    /// The running kernel's Landlock ABI version.
    pub fn abi() -> Result<i32, Errno> {
        const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
        // SAFETY: the version query takes no attribute.
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        Errno::result(abi).map(|abi| abi as i32)
    }

    /// A ruleset handling every right of `abi`: filesystem, TCP bind and
    /// connect (ABI 4), abstract unix socket and signal scoping (ABI 6), and
    /// UDP bind and send (ABI 10).
    pub fn ruleset(abi: i32) -> Result<OwnedFd, Errno> {
        let attr = RulesetAttr {
            handled_access_fs: handled_fs(abi),
            // TCP bind and connect, then UDP bind and send, with no rule
            // allowing any.
            handled_access_net: if abi >= 10 { 0b1111 } else { 0b11 },
            // Abstract unix sockets and signals.
            scoped: if abi >= 6 { 0b11 } else { 0 },
        };
        // The kernel accepts a larger struct whose unknown fields are zero.
        // SAFETY: attr is a valid ruleset attribute of the given size.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        };
        // SAFETY: a new descriptor the kernel just returned.
        Errno::result(fd).map(|fd| unsafe { OwnedFd::from_raw_fd(fd as i32) })
    }

    /// Allow `access` beneath the file or directory `fd` opens.
    pub fn allow(ruleset: &OwnedFd, fd: &OwnedFd, access: u64) -> Result<(), Errno> {
        const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;
        let attr = PathBeneathAttr {
            allowed_access: access,
            parent_fd: fd.as_raw_fd(),
        };
        // SAFETY: attr is a valid path-beneath attribute.
        let result = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset.as_raw_fd(),
                LANDLOCK_RULE_PATH_BENEATH,
                &attr as *const PathBeneathAttr,
                0u32,
            )
        };
        Errno::result(result).map(drop)
    }

    pub fn restrict(ruleset: &OwnedFd) -> Result<(), Errno> {
        // SAFETY: a ruleset descriptor and no flags.
        let result =
            unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0u32) };
        Errno::result(result).map(drop)
    }
}
