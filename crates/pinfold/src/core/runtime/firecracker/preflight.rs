//! Linux host requirements for Firecracker boxes (Runtimes), each named by
//! its token. None changes the host.

use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use nix::libc;

use super::jail;

/// One `token: detail` line per missing requirement, each checked
/// independently of the others.
pub fn host_requirements() -> Vec<String> {
    let mut missing = Vec::new();
    if let Err(error) = kvm() {
        missing.push(format!(
            "kvm: /dev/kvm: {error}; the user needs to read, write and use /dev/kvm (the kvm group)"
        ));
    }
    if mkfs_ext4().is_none() {
        missing.push(
            "mkfs: no mkfs.ext4 on PATH, in /usr/sbin or in /sbin; disks need e2fsprogs"
                .to_string(),
        );
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
pub fn mkfs_ext4() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([PathBuf::from("/usr/sbin"), PathBuf::from("/sbin")])
        .map(|dir| dir.join("mkfs.ext4"))
        .find(|path| {
            fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

/// `kernel.apparmor_restrict_unprivileged_userns`, absent where AppArmor
/// has no such restriction.
fn apparmor_restricts_userns() -> bool {
    fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
        .is_ok_and(|value| value.trim() == "1")
}
