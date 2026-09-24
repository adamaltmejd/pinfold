//! Pinned release binaries.
//!
//! A pin names a version and the sha256 of each supported `os-arch` release
//! archive. First use downloads, verifies and unpacks it into
//! `~/.cache/pinfold/artifacts/<name>/<version>/<os-arch>/`; later calls use
//! the cache.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::dirs;

/// Where the pinned harness directory is mounted in the box.
pub const GUEST_PI: &str = "/opt/pinfold/pi";

/// pi's pinned release.
const PI_VERSION: &str = "0.87.1";

/// The sha256 of pi's release archive per `os-arch`.
const PI_PINS: &[(&str, &str)] = &[
    (
        "linux-arm64",
        "364b4a9f8491450b27a4857d4e3c780dbaf696790821c176a873e860cbbc3b89",
    ),
    (
        "linux-x64",
        "80d78dd62d50049a006b981d994c61255bcc10e730b0c278d4ea0a755909764c",
    ),
];

/// The host path of the pinned pi binary for a box on this host.
///
/// On first use the release is downloaded, verified against its pin and
/// unpacked into the artifact cache; a checksum mismatch leaves nothing in
/// the cache. Later calls use the cache and touch no network.
pub fn pi() -> io::Result<PathBuf> {
    let os_arch = box_os_arch()?;
    let sha256 = pi_sha256(os_arch);
    let binary = pi_binary(os_arch)?;
    if binary.is_file() {
        return Ok(binary);
    }

    let dir = binary
        .parent()
        .and_then(Path::parent)
        .expect("the pi binary path has an os-arch directory")
        .to_path_buf();
    let version_dir = dir
        .parent()
        .expect("the os-arch directory has a version directory")
        .to_path_buf();
    fs::create_dir_all(&version_dir)?;
    // Stage and rename, like the embedded init: a failed or killed unpack
    // never leaves a half-written artifact where the next call looks.
    let staging = version_dir.join(format!(".tmp-{os_arch}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let url = format!(
        "https://github.com/earendil-works/pi/releases/download/v{PI_VERSION}/pi-{os_arch}.tar.gz"
    );
    if let Err(error) = unpack(&staging, &url, sha256, os_arch) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    match fs::rename(&staging, &dir) {
        Ok(()) => Ok(binary),
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            // A concurrent call may have installed the same version first.
            if binary.is_file() {
                Ok(binary)
            } else {
                Err(error)
            }
        }
    }
}

/// The pinned sha256 of pi's release archive for `os_arch`.
fn pi_sha256(os_arch: &str) -> &'static str {
    PI_PINS
        .iter()
        .find_map(|(name, sha256)| (*name == os_arch).then_some(*sha256))
        .expect("every supported os-arch has a pin")
}

/// One pinned artifact's cache state, as `doctor` and `artifacts` report it.
/// Nothing here downloads.
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
    let os_arch = box_os_arch()?;
    let path = pi_binary(os_arch)?;
    Ok(vec![Pin {
        name: "pi",
        version: PI_VERSION,
        sha256: pi_sha256(os_arch),
        cached: path.is_file(),
        path,
    }])
}

/// The host path of the pinned pi binary for `os_arch`, cached or not.
fn pi_binary(os_arch: &str) -> io::Result<PathBuf> {
    Ok(dirs::artifacts_dir()?
        .join("pi")
        .join(PI_VERSION)
        .join(os_arch)
        .join("pi")
        .join("pi"))
}

/// Artifact versions under `pi/` that no pin names. The embedded init lives
/// under `init/` and is named by no pin, so it is never listed.
pub fn unpinned_versions() -> io::Result<Vec<PathBuf>> {
    let pi = dirs::artifacts_dir()?.join("pi");
    let entries = match fs::read_dir(&pi) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut unpinned = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_str() == Some(PI_VERSION) {
            continue;
        }
        unpinned.push(entry.path());
    }
    Ok(unpinned)
}

/// Remove artifact versions no pin names.
pub fn prune_unpinned() -> io::Result<()> {
    for path in unpinned_versions()? {
        remove(&path)?;
    }
    Ok(())
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// The `os-arch` of a box on this host. Apple `container` and podman run
/// native images.
fn box_os_arch() -> io::Result<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Ok("linux-arm64"),
        "x86_64" => Ok("linux-x64"),
        arch => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("no pinned pi release for host architecture {arch}"),
        )),
    }
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
    let actual = sha256(&archive)?;
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
    if !staging.join("pi").join("pi").is_file() {
        return Err(io::Error::other(format!(
            "pi {PI_VERSION} {os_arch}: archive has no pi/pi binary"
        )));
    }
    fs::remove_file(&archive)
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}
