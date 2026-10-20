// Scratch escape probes, run as /firecracker inside pinfold's jail (not pinfold code).
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

fn errno() -> i32 { unsafe { *__errno_location() } }
fn c(s: &str) -> CString { CString::new(s).unwrap() }
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
    t("open /disk0.ext4 rw", unsafe { open(c("/disk0.ext4").as_ptr() as _, 2) } as i64);
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
    let d = unsafe { open(c("/disk0.ext4").as_ptr() as _, 2) };
    t("ftruncate /disk0.ext4", unsafe { ftruncate(d, 1 << 20) } as i64);
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
    let host_pid: i32 = std::env::var("PJ_KILL").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    t(&format!("kill(0 signal) host pid {host_pid}"), unsafe { kill(host_pid, 0) } as i64);
    t("setgroups([])", unsafe { setgroups(0, std::ptr::null()) } as i64);
    t("open /root.ext4 ro", unsafe { open(c("/root.ext4").as_ptr() as _, 0) } as i64);
    t("open /root.ext4 rw", unsafe { open(c("/root.ext4").as_ptr() as _, 2) } as i64);
    std::process::exit(0)
}

fn main() { probe() }
