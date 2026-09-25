//! Pinned release binaries, and the init the CLI's own build provides.
//!
//! A pin names a version and the sha256 of each supported `os-arch` release
//! archive. First use downloads, verifies and unpacks it into
//! `~/.cache/pinfold/artifacts/<name>/<version>/<os-arch>/`; later calls use
//! the cache.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::dirs;

/// Where the pinned harness directory is mounted in the box.
pub const GUEST_PI: &str = "/opt/pinfold/pi";

/// pi's pinned release.
const PI_VERSION: &str = "0.87.1";

/// The host directory of the pinned pi release for a box on this host: what
/// the box mounts at [`GUEST_PI`], holding the `pi` binary.
pub fn pi() -> io::Result<PathBuf> {
    let (os_arch, sha256) = pin()?;
    let dir = pi_dir(os_arch)?;
    let url = format!(
        "https://github.com/earendil-works/pi/releases/download/v{PI_VERSION}/pi-{os_arch}.tar.gz"
    );
    dirs::install_dir(&dir, |staging| unpack(staging, &url, sha256, os_arch))?;
    Ok(dir.join("pi"))
}

/// The `os-arch` of pi's release for a box on this host (Apple `container`
/// and podman run native images) and the pinned sha256 of its archive.
fn pin() -> io::Result<(&'static str, &'static str)> {
    match std::env::consts::ARCH {
        "aarch64" => Ok((
            "linux-arm64",
            "364b4a9f8491450b27a4857d4e3c780dbaf696790821c176a873e860cbbc3b89",
        )),
        "x86_64" => Ok((
            "linux-x64",
            "80d78dd62d50049a006b981d994c61255bcc10e730b0c278d4ea0a755909764c",
        )),
        arch => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("no pinned pi release for host architecture {arch}"),
        )),
    }
}

/// The Linux init binary to mount into a box.
///
/// On macOS the CLI embeds the `aarch64-unknown-linux-musl` build and
/// extracts it once per content into the artifact cache. On Linux the CLI
/// is itself a static Linux binary and mounts its own executable.
pub fn init() -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    if cfg!(not(target_os = "macos")) {
        return std::env::current_exe();
    }
    const INIT: &[u8] = include_bytes!(env!("PINFOLD_INIT"));
    let dir = dirs::artifacts_dir()?
        .join("init")
        .join(super::sha256_hex(INIT))
        .join("linux-arm64");
    dirs::install_dir(&dir, |staging| {
        let path = staging.join("pinfold");
        fs::write(&path, INIT)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
    })?;
    Ok(dir.join("pinfold"))
}

/// One pinned artifact's cache state, as `doctor` and `artifacts` report it.
/// Nothing here downloads.
#[derive(serde::Serialize)]
pub struct Pin {
    /// The artifact's name, e.g. `pi`.
    pub name: &'static str,
    /// The pinned version.
    pub version: &'static str,
    /// The sha256 of the release archive this host's artifact was unpacked
    /// from.
    pub sha256: &'static str,
    /// The host path of the binary in the cache.
    pub path: PathBuf,
    /// Whether the binary is already in the cache.
    pub cached: bool,
}

/// Every pinned artifact and whether the cache holds it. Never downloads.
pub fn pins() -> io::Result<Vec<Pin>> {
    let (os_arch, sha256) = pin()?;
    let path = pi_dir(os_arch)?.join("pi").join("pi");
    Ok(vec![Pin {
        name: "pi",
        version: PI_VERSION,
        sha256,
        cached: path.is_file(),
        path,
    }])
}

/// The cache directory of the pinned pi release for `os_arch`, unpacked or
/// not; the binary is its `pi/pi`.
fn pi_dir(os_arch: &str) -> io::Result<PathBuf> {
    Ok(dirs::artifacts_dir()?
        .join("pi")
        .join(PI_VERSION)
        .join(os_arch))
}

/// Artifact versions under `pi/` that no pin names. The embedded init lives
/// under `init/` and is named by no pin, so it is never listed.
pub fn unpinned_versions() -> io::Result<Vec<PathBuf>> {
    Ok(dirs::entries(&dirs::artifacts_dir()?.join("pi"))?
        .into_iter()
        .filter(|entry| entry.file_name() != PI_VERSION)
        .map(|entry| entry.path())
        .collect())
}

/// Download `url` into `staging`, verify its sha256, and unpack it there.
fn unpack(staging: &Path, url: &str, expected: &str, os_arch: &str) -> io::Result<()> {
    let archive = staging.join("pi.tar.gz");
    // Fail a stalled first contact, but no --max-time: a slow, moving
    // download must still finish.
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "15",
            "--speed-limit",
            "1",
            "--speed-time",
            "30",
            "--output",
        ])
        .arg(&archive)
        .arg(url)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!("curl {url}: {status}")));
    }
    let actual = super::sha256_hex(fs::read(&archive)?);
    if actual != expected {
        return Err(io::Error::other(format!(
            "pi {PI_VERSION} {os_arch}: checksum mismatch: expected {expected}, got {actual}"
        )));
    }
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(staging)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!("tar {url}: {status}")));
    }
    fs::remove_file(&archive)
}
