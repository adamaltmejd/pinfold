// Prototype of the VMM jail `box up` builds (ARCHITECTURE.md "VMM jail").
// std only; syscalls through musl's libc symbols. Not pinfold code.
//
//   fcjail [--no-jail] STATE FIRECRACKER KERNEL ROOTFS INITDISK DATADISK
//
// STATE holds config.json, vsock/ and an empty jail/ directory. The
// launcher stays the VMM's parent (as `box up` would) and exits with its
// status. Jailed, Firecracker sees:
//   /firecracker /vmlinux /rootfs.ext4 /init.ext4 /data.ext4 /config.json
//   /dev/kvm /vsock/
// FCJAIL_PROBE=1 runs escape probes in place of execve (same credentials,
// namespaces, mounts and Landlock domain as Firecracker would get).
// FCJAIL_NO_LANDLOCK=1 skips Landlock (the probe's control).
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::time::Instant;

type CLong = i64;
unsafe extern "C" {
    fn syscall(n: CLong, ...) -> CLong;
    fn prctl(op: i32, ...) -> i32;
    fn mount(src: *const u8, dst: *const u8, fs: *const u8, flags: u64, data: *const u8) -> i32;
    fn umount2(target: *const u8, flags: i32) -> i32;
    fn chdir(p: *const u8) -> i32;
    fn statvfs(p: *const u8, buf: *mut u64) -> i32;
    fn execve(p: *const u8, argv: *const *const u8, envp: *const *const u8) -> i32;
    fn waitpid(pid: i32, status: *mut i32, opts: i32) -> i32;
    fn setgroups(n: usize, list: *const u32) -> i32;
    fn pipe(fds: *mut i32) -> i32;
    fn close(fd: i32) -> i32;
    fn read(fd: i32, buf: *mut u8, n: usize) -> isize;
    fn open(p: *const u8, flags: i32, ...) -> i32;
    fn ioctl(fd: i32, req: u64, ...) -> i32;
    fn socket(d: i32, t: i32, p: i32) -> i32;
    fn connect(fd: i32, addr: *const u8, len: u32) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
    fn mknod(p: *const u8, mode: u32, dev: u64) -> i32;
    fn ftruncate(fd: i32, len: i64) -> i32;
    fn __errno_location() -> *mut i32;
}

const SYS_CLONE: CLong = 56;
const SYS_PIVOT_ROOT: CLong = 155;
const SYS_CAPSET: CLong = 126;
const SYS_LL_CREATE: CLong = 444;
const SYS_LL_ADD: CLong = 445;
const SYS_LL_RESTRICT: CLong = 446;

const CLONE_NEWNS: u64 = 0x20000;
const CLONE_NEWUTS: u64 = 0x0400_0000;
const CLONE_NEWIPC: u64 = 0x0800_0000;
const CLONE_NEWUSER: u64 = 0x1000_0000;
const CLONE_NEWPID: u64 = 0x2000_0000;
const CLONE_NEWNET: u64 = 0x4000_0000;
const SIGCHLD: u64 = 17;

const MS_RDONLY: u64 = 1;
const MS_NOSUID: u64 = 2;
const MS_NODEV: u64 = 4;
const MS_NOEXEC: u64 = 8;
const MS_REMOUNT: u64 = 32;
const MS_NOATIME: u64 = 1024;
const MS_NODIRATIME: u64 = 2048;
const MS_BIND: u64 = 4096;
const MS_REC: u64 = 16384;
const MS_PRIVATE: u64 = 1 << 18;
const MS_RELATIME: u64 = 1 << 21;
const MNT_DETACH: i32 = 2;

const PR_SET_PDEATHSIG: i32 = 1;
const PR_CAPBSET_DROP: i32 = 24;
const PR_SET_NO_NEW_PRIVS: i32 = 38;
const PR_CAP_AMBIENT: i32 = 47;

// Landlock access bits (uapi/linux/landlock.h).
const LL_EXECUTE: u64 = 1;
const LL_WRITE_FILE: u64 = 2;
const LL_READ_FILE: u64 = 4;
const LL_READ_DIR: u64 = 8;
const LL_REMOVE_FILE: u64 = 32;
const LL_MAKE_SOCK: u64 = 512;
const LL_TRUNCATE: u64 = 1 << 14;
const LL_IOCTL_DEV: u64 = 1 << 15;

fn errno() -> i32 {
    unsafe { *__errno_location() }
}
fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}
fn die(what: &str) -> ! {
    let e = errno();
    eprintln!("fcjail: {what}: errno {e} ({})", std::io::Error::from_raw_os_error(e));
    std::process::exit(125)
}
fn chk(r: i64, what: &str) {
    if r < 0 {
        die(what)
    }
}

fn write_file(p: &str, s: &str) {
    if let Err(e) = fs::write(p, s) {
        eprintln!("fcjail: write {p}: {e}");
        std::process::exit(125)
    }
}

// Locked flags of the source mount must survive a remount in a user
// namespace, or the kernel refuses it (EPERM).
fn locked_flags(path: &str) -> u64 {
    let mut b = [0u64; 32];
    chk(unsafe { statvfs(c(path).as_ptr() as _, b.as_mut_ptr()) } as i64, "statvfs");
    let f = b[9]; // f_flag on x86_64 musl
    let mut m = 0;
    for (st, ms) in [
        (2, MS_NOSUID),
        (4, MS_NODEV),
        (8, MS_NOEXEC),
        (1024, MS_NOATIME),
        (2048, MS_NODIRATIME),
        (4096, MS_RELATIME),
    ] {
        if f & st != 0 {
            m |= ms;
        }
    }
    m
}

fn bind(src: &str, dst: &str, ro: bool, dir: bool) {
    if dir {
        fs::create_dir(dst).unwrap_or_else(|e| panic!("{dst}: {e}"));
    } else {
        fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(dst)
            .unwrap_or_else(|e| panic!("{dst}: {e}"));
    }
    let (s, d) = (c(src), c(dst));
    chk(unsafe { mount(s.as_ptr() as _, d.as_ptr() as _, std::ptr::null(), MS_BIND, std::ptr::null()) } as i64, dst);
    if ro {
        let fl = MS_BIND | MS_REMOUNT | MS_RDONLY | MS_NOSUID | locked_flags(src);
        chk(unsafe { mount(std::ptr::null(), d.as_ptr() as _, std::ptr::null(), fl, std::ptr::null()) } as i64, dst);
    }
}

#[repr(C)]
struct RulesetAttr {
    fs: u64,
    net: u64,
    scoped: u64,
}
#[repr(C, packed)]
struct PathBeneath {
    allowed: u64,
    fd: i32,
}

fn landlock() -> i32 {
    let abi = unsafe { syscall(SYS_LL_CREATE, 0usize, 0usize, 1u32) } as i32;
    if abi < 1 {
        die("landlock unavailable")
    }
    let mut fs_all: u64 = (1 << 13) - 1; // ABI 1
    if abi >= 2 { fs_all |= 1 << 13 } // REFER
    if abi >= 3 { fs_all |= LL_TRUNCATE }
    if abi >= 5 { fs_all |= LL_IOCTL_DEV }
    let attr = RulesetAttr {
        fs: fs_all,
        net: if abi >= 4 { 3 } else { 0 }, // bind and connect TCP: no rules, so denied
        scoped: if abi >= 6 { 3 } else { 0 }, // abstract unix sockets, signals
    };
    let size = if abi >= 6 { 24 } else if abi >= 4 { 16 } else { 8 };
    let rs = unsafe { syscall(SYS_LL_CREATE, &attr as *const _, size as usize, 0u32) } as i32;
    chk(rs as i64, "landlock_create_ruleset");
    let file = LL_EXECUTE | LL_WRITE_FILE | LL_READ_FILE | LL_TRUNCATE | LL_IOCTL_DEV;
    let rules: [(&str, u64); 9] = [
        ("/firecracker", LL_EXECUTE | LL_READ_FILE),
        ("/vmlinux", LL_READ_FILE),
        ("/rootfs.ext4", LL_READ_FILE),
        ("/init.ext4", LL_READ_FILE),
        ("/config.json", LL_READ_FILE),
        ("/data.ext4", LL_READ_FILE | LL_WRITE_FILE),
        ("/dev/kvm", LL_READ_FILE | LL_WRITE_FILE | LL_IOCTL_DEV),
        ("/vsock", LL_READ_DIR | LL_MAKE_SOCK | LL_REMOVE_FILE),
        ("/dev", 0),
    ];
    for (p, acc) in rules {
        let acc = acc & fs_all & if p == "/vsock" { u64::MAX } else { file };
        if acc == 0 {
            continue;
        }
        let fd = unsafe { open(c(p).as_ptr() as _, 0o10000000 /* O_PATH */ | 0o2000000 /* O_CLOEXEC */) };
        chk(fd as i64, p);
        let pb = PathBeneath { allowed: acc, fd };
        chk(unsafe { syscall(SYS_LL_ADD, rs, 1u32, &pb as *const _, 0u32) }, p);
        unsafe { close(fd) };
    }
    chk(unsafe { syscall(SYS_LL_RESTRICT, rs, 0u32) }, "landlock_restrict_self");
    abi
}

fn drop_caps() {
    for cap in 0..64 {
        if unsafe { prctl(PR_CAPBSET_DROP, cap as u64, 0u64, 0u64, 0u64) } < 0 {
            if errno() == 22 { break } // EINVAL: past the last cap
            die("PR_CAPBSET_DROP")
        }
    }
    unsafe { prctl(PR_CAP_AMBIENT, 4u64, 0u64, 0u64, 0u64) }; // CLEAR_ALL
    let hdr = [0x2008_0522u32, 0];
    let data = [0u32; 6];
    chk(unsafe { syscall(SYS_CAPSET, hdr.as_ptr(), data.as_ptr()) }, "capset");
}

// Runs as the VMM would, after every restriction, and reports what a
// compromised VMM could reach.
fn probe() -> ! {
    let t = |name: &str, r: i64| {
        let e = if r < 0 { errno() } else { 0 };
        println!("probe {name}: {}", if r < 0 { format!("errno {e} ({})", std::io::Error::from_raw_os_error(e)) } else { format!("ok ({r})") });
    };
    // Positive controls: what Firecracker needs.
    let kvm = unsafe { open(c("/dev/kvm").as_ptr() as _, 2 | 0o2000000) };
    t("open /dev/kvm rw", kvm as i64);
    t("ioctl KVM_GET_API_VERSION", unsafe { ioctl(kvm, 0xAE00, 0u64) } as i64);
    t("open /data.ext4 rw", unsafe { open(c("/data.ext4").as_ptr() as _, 2) } as i64);
    t("open /vmlinux ro", unsafe { open(c("/vmlinux").as_ptr() as _, 0) } as i64);
    let mut sun = [0u8; 110];
    sun[0] = 1;
    let path = b"/vsock/v.sock_3128";
    sun[2..2 + path.len()].copy_from_slice(path);
    let u = unsafe { socket(1, 1, 0) };
    t("connect /vsock/v.sock_3128 (the proxy)", unsafe { connect(u, sun.as_ptr(), (2 + path.len()) as u32) } as i64);
    // Refusals from the mounts.
    t("open /vmlinux rw", unsafe { open(c("/vmlinux").as_ptr() as _, 2) } as i64);
    t("create /x", unsafe { open(c("/x").as_ptr() as _, 0o100 | 1, 0o600) } as i64);
    t("open /proc/self/status", unsafe { open(c("/proc/self/status").as_ptr() as _, 0) } as i64);
    t("open /home", unsafe { open(c("/home").as_ptr() as _, 0) } as i64);
    t("open /etc/passwd", unsafe { open(c("/etc/passwd").as_ptr() as _, 0) } as i64);
    // Refusals Landlock adds inside the tree (the writable vsock dir and disk).
    t("create regular file /vsock/f", unsafe { open(c("/vsock/f").as_ptr() as _, 0o100 | 1, 0o600) } as i64);
    t("mknod fifo /vsock/p", unsafe { mknod(c("/vsock/p").as_ptr() as _, 0o10600, 0) } as i64);
    let d = unsafe { open(c("/data.ext4").as_ptr() as _, 2) };
    t("ftruncate /data.ext4", unsafe { ftruncate(d, 1 << 20) } as i64);
    // Refusals from the namespaces (and Landlock's network and scope rules).
    let s = unsafe { socket(2, 1, 0) };
    let sin: [u8; 16] = [2, 0, 0x1f, 0x90, 127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    t("tcp connect 127.0.0.1:8080", unsafe { connect(s, sin.as_ptr(), 16) } as i64);
    let u = unsafe { socket(1, 1, 0) };
    let mut sun = [0u8; 110];
    sun[0] = 1;
    let name = b"\0fcjail-probe-abstract";
    sun[2..2 + name.len()].copy_from_slice(name);
    t("connect abstract @fcjail-probe-abstract", unsafe { connect(u, sun.as_ptr(), (2 + name.len()) as u32) } as i64);
    let host_pid: i32 = std::env::var("FCJAIL_PROBE_KILL").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    t(&format!("kill(0 signal) host pid {host_pid}"), unsafe { kill(host_pid, 0) } as i64);
    t("setgroups([])", unsafe { setgroups(0, std::ptr::null()) } as i64);
    // Last: the one executable, which replaces this process.
    let argv = [c("/firecracker"), c("--version")];
    let av: Vec<*const u8> = argv.iter().map(|a| a.as_ptr() as _).chain([std::ptr::null()]).collect();
    t("execve /firecracker --version", unsafe { execve(av[0], av.as_ptr(), [std::ptr::null()].as_ptr()) } as i64);
    std::process::exit(0)
}

fn main() {
    let t0 = Instant::now();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let jail = if args.first().map(|s| s == "--no-jail").unwrap_or(false) {
        args.remove(0);
        false
    } else {
        true
    };
    let [state, fc, kernel, rootfs, initd, data] = <[String; 6]>::try_from(args).unwrap_or_else(|_| {
        eprintln!("usage: fcjail [--no-jail] STATE FIRECRACKER KERNEL ROOTFS INITDISK DATADISK");
        std::process::exit(2)
    });
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    let probe_mode = set("FCJAIL_PROBE");
    let no_landlock = set("FCJAIL_NO_LANDLOCK");
    let (uid, gid) = {
        let st = fs::read_to_string("/proc/self/status").unwrap();
        let f = |k: &str| st.lines().find(|l| l.starts_with(k)).unwrap().split_whitespace().nth(1).unwrap().to_string();
        (f("Uid:"), f("Gid:"))
    };
    // Outside the namespaces: can the launcher drop supplementary groups?
    if jail {
        let r = unsafe { setgroups(0, std::ptr::null()) };
        eprintln!("fcjail: setgroups([]) before unshare: {}", if r < 0 { format!("errno {}", errno()) } else { "ok".into() });
    }
    // The pipe's write end lives only in the parent: EOF tells the child its
    // owner died before PR_SET_PDEATHSIG took hold.
    let mut p = [0i32; 2];
    chk(unsafe { pipe(p.as_mut_ptr()) } as i64, "pipe");
    let flags = if jail {
        CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWPID | CLONE_NEWNET | CLONE_NEWIPC | CLONE_NEWUTS | SIGCHLD
    } else {
        SIGCHLD
    };
    let pid = unsafe { syscall(SYS_CLONE, flags, 0usize, 0usize, 0usize, 0usize) } as i32;
    chk(pid as i64, "clone");
    if pid > 0 {
        unsafe { close(p[0]) };
        fs::write(format!("{state}/vmm.pid"), format!("{pid}\n")).unwrap();
        let mut st = 0;
        unsafe { waitpid(pid, &mut st, 0) };
        let code = if st & 0x7f == 0 { (st >> 8) & 0xff } else { 128 + (st & 0x7f) };
        eprintln!("fcjail: vmm {pid} ended, status {code}");
        std::process::exit(code)
    }
    // Child.
    unsafe { close(p[1]) };
    chk(unsafe { prctl(PR_SET_PDEATHSIG, 9u64, 0u64, 0u64, 0u64) } as i64, "PDEATHSIG");
    let mut b = [0u8; 1];
    {
        // Parent alive? Its write end is open, so a nonblocking read gets EAGAIN.
        let f = fs::OpenOptions::new().read(true).custom_flags(0o4000).open(format!("/proc/self/fd/{}", p[0]));
        if let Ok(mut f) = f {
            if let Ok(0) = f.read(&mut b) {
                std::process::exit(125)
            }
        }
    }
    let target = if jail {
        write_file("/proc/self/setgroups", "deny");
        write_file("/proc/self/uid_map", &format!("{uid} {uid} 1\n"));
        write_file("/proc/self/gid_map", &format!("{gid} {gid} 1\n"));
        let j = format!("{state}/jail");
        let jc = c(&j);
        unsafe {
            chk(mount(std::ptr::null(), c("/").as_ptr() as _, std::ptr::null(), MS_REC | MS_PRIVATE, std::ptr::null()) as i64, "make / private");
            chk(mount(c("tmpfs").as_ptr() as _, jc.as_ptr() as _, c("tmpfs").as_ptr() as _, MS_NOSUID | MS_NODEV, c("mode=0755,size=64k,nr_inodes=32").as_ptr() as _) as i64, "tmpfs");
        }
        bind(&fc, &format!("{j}/firecracker"), true, false);
        bind(&kernel, &format!("{j}/vmlinux"), true, false);
        bind(&rootfs, &format!("{j}/rootfs.ext4"), true, false);
        bind(&initd, &format!("{j}/init.ext4"), true, false);
        bind(&format!("{state}/config.json"), &format!("{j}/config.json"), true, false);
        bind(&data, &format!("{j}/data.ext4"), false, false);
        fs::create_dir(format!("{j}/dev")).unwrap();
        bind("/dev/kvm", &format!("{j}/dev/kvm"), false, false);
        bind(&format!("{state}/vsock"), &format!("{j}/vsock"), false, true);
        // The jail root itself read-only: nothing new can be created in it.
        unsafe {
            chk(mount(std::ptr::null(), jc.as_ptr() as _, std::ptr::null(), MS_REMOUNT | MS_RDONLY | MS_NOSUID | MS_NODEV, c("mode=0755,size=64k,nr_inodes=32").as_ptr() as _) as i64, "tmpfs ro");
            chk(chdir(jc.as_ptr() as _) as i64, "chdir jail");
            chk(syscall(SYS_PIVOT_ROOT, c(".").as_ptr(), c(".").as_ptr()), "pivot_root");
            chk(umount2(c(".").as_ptr() as _, MNT_DETACH) as i64, "detach old root");
            chk(chdir(c("/").as_ptr() as _) as i64, "chdir /");
        }
        chk(unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1u64, 0u64, 0u64, 0u64) } as i64, "NNP");
        if !no_landlock {
            let abi = landlock();
            eprintln!("fcjail: landlock abi {abi}");
        }
        let r = unsafe { setgroups(0, std::ptr::null()) };
        eprintln!("fcjail: setgroups([]) inside userns: {}", if r < 0 { format!("errno {}", errno()) } else { "ok".into() });
        drop_caps();
        if probe_mode {
            probe()
        }
        ("/firecracker".to_string(), "/config.json".to_string())
    } else {
        (fc.clone(), format!("{state}/config.json"))
    };
    eprintln!("fcjail: setup_us={}", t0.elapsed().as_micros());
    let _ = std::io::stderr().flush();
    let argv = [c(&target.0), c("--config-file"), c(&target.1), c("--no-api")];
    let av: Vec<*const u8> = argv.iter().map(|a| a.as_ptr() as _).chain([std::ptr::null()]).collect();
    unsafe { execve(av[0], av.as_ptr(), [std::ptr::null()].as_ptr()) };
    die("execve firecracker")
}
