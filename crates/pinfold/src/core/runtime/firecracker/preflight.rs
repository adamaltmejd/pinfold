//! Linux host requirements for Firecracker boxes and image jobs
//! (Runtimes), each named by its token. None changes the host.

use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

use nix::libc;

use super::jail;

/// One `token: detail` line per missing requirement, each checked
/// independently of the others.
pub fn host_requirements() -> Vec<String> {
    let mut missing = Vec::new();
    if nix::unistd::getuid().is_root() {
        missing.push("root: pinfold runs as uid 0; Firecracker runs only as the user".to_string());
    }
    match std::env::current_exe().and_then(|exe| has_interpreter(&exe)) {
        Ok(false) => {}
        Ok(true) => missing.push(
            "static: pinfold's executable names a dynamic loader (PT_INTERP); the guest init needs a statically linked build".to_string(),
        ),
        Err(error) => missing.push(format!("static: pinfold's executable: {error}")),
    }
    if let Err(error) = kvm() {
        missing.push(format!(
            "kvm: /dev/kvm: {error}; the user needs to read, write and use /dev/kvm (the kvm group)"
        ));
    }
    if let Err(error) = mkfs_ext4() {
        missing.push(format!("{error}; disks need e2fsprogs"));
    }
    if let Err(error) = jail::probe() {
        // From the sysctl, not the error: AppArmor's refusal can surface as
        // any of the probe's steps.
        if apparmor_restricts_userns() {
            missing.push(format!(
                "userns-apparmor: {error}; AppArmor restricts unprivileged user namespaces and no profile grants this executable `userns`; load `pinfold apparmor-profile` as root once"
            ));
        } else {
            missing.push(format!(
                "userns: {error}; the VMM jail needs unprivileged user namespaces"
            ));
        }
    }
    match jail::landlock::abi() {
        Ok(abi) if abi >= 5 => {}
        Ok(abi) => missing.push(format!(
            "landlock: Landlock ABI {abi}; the VMM jail needs ABI 5 or later (Linux 6.10+)"
        )),
        Err(error) => missing.push(format!(
            "landlock: Landlock is unavailable: {error}; the VMM jail needs ABI 5 or later (Linux 6.10+)"
        )),
    }
    missing
}

/// Whether an ELF64 little-endian executable has a `PT_INTERP` program
/// header. The executable is also the guest init, and the init disk holds
/// no dynamic loader.
fn has_interpreter(path: &std::path::Path) -> io::Result<bool> {
    use std::os::unix::fs::FileExt;
    const PT_INTERP: u32 = 3;
    let file = fs::File::open(path)?;
    let mut header = [0u8; 64];
    file.read_exact_at(&mut header, 0)?;
    if header[..4] != *b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return Err(io::Error::other("not an ELF64 little-endian file"));
    }
    let offset = u64::from_le_bytes(header[0x20..0x28].try_into().expect("8 bytes"));
    let size = u16::from_le_bytes([header[0x36], header[0x37]]);
    let count = u16::from_le_bytes([header[0x38], header[0x39]]);
    if size < 4 {
        return Err(io::Error::other("ELF program header size below 4"));
    }
    for index in 0..u64::from(count) {
        let mut kind = [0u8; 4];
        file.read_exact_at(&mut kind, offset + index * u64::from(size))?;
        if u32::from_le_bytes(kind) == PT_INTERP {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Open `/dev/kvm` and use it: access, not group membership.
fn kvm() -> io::Result<()> {
    const KVM_GET_API_VERSION: libc::c_ulong = 0xAE00;
    const KVM_API_VERSION: libc::c_int = 12;
    let device = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")?;
    // SAFETY: an ioctl without an argument on an open descriptor.
    let version = unsafe { libc::ioctl(device.as_raw_fd(), KVM_GET_API_VERSION as _, 0) };
    if version < 0 {
        return Err(io::Error::last_os_error());
    }
    if version != KVM_API_VERSION {
        return Err(io::Error::other(format!("KVM API version {version}")));
    }
    Ok(())
}

/// The first executable `mkfs.ext4` on PATH, then in `/usr/sbin` and
/// `/sbin`, which Debian leaves off a user's PATH.
pub fn mkfs_ext4() -> io::Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([PathBuf::from("/usr/sbin"), PathBuf::from("/sbin")])
        .map(|dir| dir.join("mkfs.ext4"))
        .find(|path| {
            path.is_file() && nix::unistd::access(path, nix::unistd::AccessFlags::X_OK).is_ok()
        })
        .ok_or_else(|| io::Error::other("mkfs: no mkfs.ext4 on PATH, in /usr/sbin or in /sbin"))
}

/// `kernel.apparmor_restrict_unprivileged_userns`, absent where AppArmor
/// has no such restriction.
fn apparmor_restricts_userns() -> bool {
    fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
        .is_ok_and(|value| value.trim() == "1")
}
