// Prototype guest PID 1 and exec service (ARCHITECTURE.md Lifecycle 5 and 8,
// Transport). std only; syscalls through musl. Not pinfold code.
//
//   PID 1 (init=/sbin/pf-guest): pf.mode= on the kernel command line
//     A   PID 1 mounts /dev/vdc rw at /data, forks the exec service at uid
//         1000 with every capability set empty, keeps only CAP_SYS_ADMIN.
//         `unmount` on port 1026: cgroup.kill the work, sync, umount.
//     C   the exec service mounts /dev/vdc in its own mount namespace, then
//         drops; PID 1 drops to uid 1000 with nothing. `unmount`: kill -1,
//         reap, wait for /sys/fs/ext4/vdc to go.
//     ro  PID 1 mounts /dev/vdc read-only (no noload); otherwise as A.
//   pf-guest serve          the exec service (port 1024), started by PID 1
//   pf-guest probe CID PORT a vsock connect with errno and elapsed ms
//
// Every listener accepts peer CID 2 only and answers any other peer `peer`.
use std::fs;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn mount(src: *const u8, dst: *const u8, fs: *const u8, flags: u64, data: *const u8) -> i32;
    fn umount2(target: *const u8, flags: i32) -> i32;
    fn socket(d: i32, t: i32, p: i32) -> i32;
    fn bind(fd: i32, a: *const u8, l: u32) -> i32;
    fn listen(fd: i32, n: i32) -> i32;
    fn accept4(fd: i32, a: *mut u8, l: *mut u32, f: i32) -> i32;
    fn connect(fd: i32, a: *const u8, l: u32) -> i32;
    fn close(fd: i32) -> i32;
    fn poll(fds: *mut i32, n: u64, ms: i32) -> i32;
    fn waitpid(pid: i32, st: *mut i32, o: i32) -> i32;
    fn fork() -> i32;
    fn prctl(op: i32, ...) -> i32;
    fn setgroups(n: usize, l: *const u32) -> i32;
    fn setresgid(a: u32, b: u32, c: u32) -> i32;
    fn setresuid(a: u32, b: u32, c: u32) -> i32;
    fn syscall(n: i64, ...) -> i64;
    fn unshare(f: i32) -> i32;
    fn sync();
    fn kill(pid: i32, sig: i32) -> i32;
    fn ioctl(fd: i32, req: u64, ...) -> i32;
    fn setsockopt(fd: i32, l: i32, o: i32, v: *const u8, n: u32) -> i32;
    fn execv(p: *const u8, argv: *const *const u8) -> i32;
    fn __errno_location() -> *mut i32;
}

fn errno() -> i32 {
    unsafe { *__errno_location() }
}
fn c(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}
fn ename(e: i32) -> String {
    format!("errno {e} ({})", std::io::Error::from_raw_os_error(e))
}
fn log(s: &str) {
    // PID 1's stdout is the serial console, which the host keeps as console.log.
    println!("pf-guest: {s}");
}

const AF_VSOCK: i32 = 40;
const CAP_SYS_ADMIN: u32 = 21;

fn sockaddr_vm(cid: u32, port: u32) -> [u8; 16] {
    let mut a = [0u8; 16];
    a[0..2].copy_from_slice(&(AF_VSOCK as u16).to_ne_bytes());
    a[4..8].copy_from_slice(&port.to_ne_bytes());
    a[8..12].copy_from_slice(&cid.to_ne_bytes());
    a
}
fn vlisten(port: u32) -> i32 {
    unsafe {
        let fd = socket(AF_VSOCK, 1 | 0o2000000, 0);
        let a = sockaddr_vm(u32::MAX, port); // VMADDR_CID_ANY
        assert!(bind(fd, a.as_ptr(), 16) == 0, "bind {port}: {}", ename(errno()));
        assert!(listen(fd, 16) == 0);
        fd
    }
}
fn vaccept(lfd: i32) -> (fs::File, u32) {
    let mut a = [0u8; 16];
    let mut l = 16u32;
    let fd = unsafe { accept4(lfd, a.as_mut_ptr(), &mut l, 0o2000000) };
    assert!(fd >= 0, "accept: {}", ename(errno()));
    let cid = u32::from_ne_bytes(a[8..12].try_into().unwrap());
    (unsafe { fs::File::from_raw_fd(fd) }, cid)
}
fn vconnect(cid: u32, port: u32) -> Result<fs::File, i32> {
    unsafe {
        let fd = socket(AF_VSOCK, 1 | 0o2000000, 0);
        let a = sockaddr_vm(cid, port);
        if connect(fd, a.as_ptr(), 16) < 0 {
            let e = errno();
            close(fd);
            return Err(e);
        }
        Ok(fs::File::from_raw_fd(fd))
    }
}
fn read_line(f: &mut fs::File) -> String {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    while let Ok(1) = f.read(&mut b) {
        if b[0] == b'\n' {
            break;
        }
        out.push(b[0]);
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn mnt(src: &str, dst: &str, fstype: &str, flags: u64, data: &str) -> Result<(), i32> {
    let _ = fs::create_dir_all(dst);
    let r = unsafe { mount(c(src).as_ptr() as _, c(dst).as_ptr() as _, c(fstype).as_ptr() as _, flags, c(data).as_ptr() as _) };
    if r < 0 { Err(errno()) } else { Ok(()) }
}

// Bounding set reduced to `keep` (needs CAP_SETPCAP, so first); then
// effective and permitted to `keep`, inheritable and ambient empty.
fn drop_bset(keep: Option<u32>) {
    for cap in 0..64u64 {
        if Some(cap as u32) == keep {
            continue;
        }
        if unsafe { prctl(24, cap, 0u64, 0u64, 0u64) } < 0 {
            assert!(errno() == 22, "PR_CAPBSET_DROP {cap}: {}", ename(errno()));
            break;
        }
    }
}
fn capset(keep: Option<u32>) {
    unsafe { prctl(47, 4u64, 0u64, 0u64, 0u64) };
    let hdr = [0x2008_0522u32, 0];
    let mut data = [0u32; 6];
    if let Some(k) = keep {
        data[0] = 1 << k; // effective
        data[1] = 1 << k; // permitted
    }
    assert!(unsafe { syscall(126, hdr.as_ptr(), data.as_ptr()) } == 0, "capset: {}", ename(errno()));
}
fn caps_keep(keep: Option<u32>) {
    drop_bset(keep);
    capset(keep);
}
fn become_user() {
    drop_bset(None);
    unsafe {
        assert!(setgroups(0, std::ptr::null()) == 0, "setgroups: {}", ename(errno()));
        assert!(setresgid(1000, 1000, 1000) == 0, "setresgid: {}", ename(errno()));
        assert!(setresuid(1000, 1000, 1000) == 0, "setresuid: {}", ename(errno()));
    }
    capset(None);
    assert!(unsafe { prctl(38, 1u64, 0u64, 0u64, 0u64) } == 0); // no_new_privs
}

fn lo_up() {
    unsafe {
        let s = socket(2, 2, 0);
        let mut ifr = [0u8; 40];
        ifr[..2].copy_from_slice(b"lo");
        ioctl(s, 0x8913, ifr.as_mut_ptr()); // SIOCGIFFLAGS
        ifr[16] |= 1; // IFF_UP
        ioctl(s, 0x8914, ifr.as_mut_ptr()); // SIOCSIFFLAGS
        close(s);
    }
}

fn init() -> ! {
    if let Err(e) = mnt("proc", "/proc", "proc", 0, "") { log(&format!("mount {}: {}", "/proc", ename(e))) }
    if let Err(e) = mnt("sys", "/sys", "sysfs", 0, "") { log(&format!("mount {}: {}", "/sys", ename(e))) }
    if let Err(e) = mnt("dev", "/dev", "devtmpfs", 0, "") { log(&format!("mount {}: {}", "/dev", ename(e))) }
    if let Err(e) = mnt("tmp", "/tmp", "tmpfs", 2 | 4, "mode=1777") { log(&format!("mount {}: {}", "/tmp", ename(e))) }
    if let Err(e) = mnt("run", "/run", "tmpfs", 2 | 4, "mode=0755") { log(&format!("mount {}: {}", "/run", ename(e))) }
    if let Err(e) = mnt("cg", "/sys/fs/cgroup", "cgroup2", 2 | 4 | 8, "") { log(&format!("mount {}: {}", "/sys/fs/cgroup", ename(e))) }
    lo_up();
    let cmdline = fs::read_to_string("/proc/cmdline").unwrap();
    std::panic::set_hook(Box::new(|i| { log(&format!("PANIC {i}")); std::thread::sleep(Duration::from_secs(1)); }));
    let mode = cmdline.split_whitespace().find_map(|w| w.strip_prefix("pf.mode=")).unwrap_or("A").to_string();
    log(&format!("mode {mode} kernel {}", fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim()));
    match mode.as_str() {
        "A" => log(&format!("mount /data rw: {:?}", mnt("/dev/vdc", "/data", "ext4", 2 | 4, ""))),
        "ro" => log(&format!("mount /data ro: {}", match mnt("/dev/vdc", "/data", "ext4", 1 | 2 | 4, "") { Ok(()) => "ok".into(), Err(e) => ename(e) })),
        _ => {}
    }
    // Work cgroup: a pids cap, and the handle PID 1 ends the work with.
    let _ = fs::write("/sys/fs/cgroup/cgroup.subtree_control", "+pids");
    fs::create_dir_all("/sys/fs/cgroup/work").unwrap();
    let _ = fs::write("/sys/fs/cgroup/work/pids.max", "1024");
    let ctl = vlisten(1026);
    let pid = unsafe { fork() };
    if pid == 0 {
        unsafe { close(ctl) };
        fs::write("/sys/fs/cgroup/work/cgroup.procs", "0").unwrap();
        if mode == "C" {
            assert!(unsafe { unshare(0x20000) } == 0); // CLONE_NEWNS
            if let Err(e) = mnt("", "/", "", 16384 | (1 << 18), "") { log(&format!("mount {}: {}", "/", ename(e))) } // rprivate
            log(&format!("C: mount /data rw in child ns: {:?}", mnt("/dev/vdc", "/data", "ext4", 2 | 4, "")));
        }
        become_user();
        let a = [c("/sbin/pf-guest"), c("serve")];
        let av = [a[0].as_ptr() as *const u8, a[1].as_ptr() as _, std::ptr::null()];
        unsafe { execv(av[0], av.as_ptr()) };
        log(&format!("exec serve: {}", ename(errno())));
        std::process::exit(1)
    }
    if mode == "C" {
        become_user();
    } else {
        caps_keep(Some(CAP_SYS_ADMIN));
    }
    log(&format!("pid1 caps set, exec service pid {pid}"));
    let mut pfd = [ctl, 1 /* POLLIN */];
    loop {
        unsafe {
            let mut st = 0;
            while waitpid(-1, &mut st, 1 /* WNOHANG */) > 0 {}
            if poll(pfd.as_mut_ptr(), 1, 500) <= 0 {
                continue;
            }
        }
        pfd[1] = 1;
        let (mut s, cid) = vaccept(ctl);
        if cid != 2 {
            let _ = s.write_all(b"peer\n");
            log(&format!("ctl: refused peer cid {cid}"));
            continue;
        }
        let req = read_line(&mut s);
        log(&format!("ctl: peer cid {cid} request {req:?}"));
        let t = Instant::now();
        let reply = match (req.as_str(), mode.as_str()) {
            ("unmount", "C") => {
                unsafe { kill(-1, 9) };
                let mut st = 0;
                while unsafe { waitpid(-1, &mut st, 0) } > 0 {}
                let reaped = t.elapsed().as_millis();
                // Superblock teardown may lag the last exit; wait for it.
                while fs::metadata("/sys/fs/ext4/vdc").is_ok() && t.elapsed() < Duration::from_secs(10) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                let gone = fs::metadata("/sys/fs/ext4/vdc").is_err();
                format!("reaped_ms={reaped} ext4_gone={gone} ms={}", t.elapsed().as_millis())
            }
            ("unmount-nokill", _) => {
                unsafe { sync() };
                let r = unsafe { umount2(c("/data").as_ptr() as _, 0) };
                format!("umount {}", if r < 0 { ename(errno()) } else { "ok".into() })
            }
            ("unmount", _) => {
                let k = fs::write("/sys/fs/cgroup/work/cgroup.kill", "1");
                while !fs::read_to_string("/sys/fs/cgroup/work/cgroup.events").unwrap_or_default().contains("populated 0") {
                    let mut st = 0;
                    while unsafe { waitpid(-1, &mut st, 1) } > 0 {}
                    std::thread::sleep(Duration::from_millis(1));
                }
                unsafe { sync() };
                let r = unsafe { umount2(c("/data").as_ptr() as _, 0) };
                format!(
                    "cgroup.kill {:?} umount {} ms={}",
                    k.map_err(|e| e.to_string()),
                    if r < 0 { ename(errno()) } else { "ok".into() },
                    t.elapsed().as_millis()
                )
            }
            _ => "unknown".into(),
        };
        log(&format!("ctl: {reply}"));
        let _ = s.write_all(format!("{reply}\n").as_bytes());
    }
}

fn serve() -> ! {
    // Reap children on a thread; an ignored SIGCHLD would be inherited by sh.
    std::thread::spawn(|| loop {
        let mut st = 0;
        if unsafe { waitpid(-1, &mut st, 0) } < 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let lfd = vlisten(1024);
    // The relay of 127.0.0.1:3128 to the host's port 3128.
    let _ = Command::new("socat")
        .args(["TCP-LISTEN:3128,bind=127.0.0.1,reuseaddr,fork", "VSOCK-CONNECT:2:3128"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn();
    let ver = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap();
    // Wait until the relay listens before reporting ready.
    while std::net::TcpStream::connect("127.0.0.1:3128").is_err() {
        std::thread::sleep(Duration::from_millis(1));
    }
    match vconnect(2, 1000) {
        Ok(mut f) => { let _ = writeln!(f, "ready {}", ver.trim()); }
        Err(e) => eprintln!("ready: {}", ename(e)),
    }
    loop {
        let (mut s, cid) = vaccept(lfd);
        if cid != 2 {
            let _ = s.write_all(b"peer\n");
            continue;
        }
        let cmd = read_line(&mut s);
        let out = s.try_clone().unwrap();
        let err = s.try_clone().unwrap();
        let wrapped = format!("echo peer_cid={cid}; {cmd}; echo exit=$?");
        let _ = Command::new("/bin/sh").args(["-c", &wrapped])
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", "/tmp")
            .env("HTTPS_PROXY", "http://127.0.0.1:3128")
            .stdin(Stdio::null()).stdout(Stdio::from(out)).stderr(Stdio::from(err))
            .spawn();
    }
}

fn probe(cid: u32, port: u32) {
    let t = Instant::now();
    let r = vconnect(cid, port);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    match r {
        Err(e) => println!("probe cid={cid} port={port}: {} after {ms:.1} ms", ename(e)),
        Ok(mut f) => {
            let tv = [2i64, 0];
            unsafe { setsockopt(std::os::fd::AsRawFd::as_raw_fd(&f), 1, 20, tv.as_ptr() as _, 16) };
            let mut b = [0u8; 64];
            let n = f.read(&mut b);
            let reply = match n {
                Ok(n) => format!("{:?}", String::from_utf8_lossy(&b[..n])),
                Err(e) => e.to_string(),
            };
            println!("probe cid={cid} port={port}: connected after {ms:.1} ms, reply {reply}");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if std::process::id() == 1 {
        init()
    }
    match args.get(1).map(|s| s.as_str()) {
        Some("serve") => serve(),
        Some("probe") => probe(args[2].parse().unwrap(), args[3].parse().unwrap()),
        _ => {
            eprintln!("usage: pf-guest serve | probe CID PORT");
            std::process::exit(2)
        }
    }
}
