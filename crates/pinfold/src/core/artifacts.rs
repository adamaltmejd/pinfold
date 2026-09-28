//! Pinned harnesses, and the init the CLI's own build provides.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::dirs;

/// The pin file, parsed once. It is embedded, so a parse error is a bug in
/// this build.
pub static HARNESSES: LazyLock<Vec<Harness>> = LazyLock::new(|| {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PinFile {
        harness: Vec<Harness>,
    }
    toml::from_str::<PinFile>(include_str!("../../harnesses.toml"))
        .expect("the embedded harnesses.toml parses")
        .harness
});

/// One pinned harness: a row of `harnesses.toml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harness {
    pub name: String,
    pub version: String,
    /// Env defaults in the box; the spec's own env wins.
    pub env: BTreeMap<String, String>,
    asset: Vec<Asset>,
}

/// One release asset. It serializes as `artifacts` reports it: the url and
/// its sha256.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    #[serde(rename = "os-arch", skip_serializing)]
    os_arch: String,
    url: String,
    sha256: String,
    #[serde(skip_serializing)]
    install: Install,
    /// Where it is installed, relative to the `<os-arch>/` directory.
    #[serde(rename = "as", skip_serializing)]
    path: PathBuf,
}

#[derive(Deserialize)]
enum Install {
    /// The archive's one top-level entry is installed.
    #[serde(rename = "tar.gz")]
    TarGz,
    /// The download is the executable.
    #[serde(rename = "executable")]
    Executable,
}

/// The pinned harness named `name`.
pub fn harness(name: &str) -> Option<&'static Harness> {
    HARNESSES.iter().find(|harness| harness.name == name)
}

/// Where the harness `name` is mounted in the box.
pub fn guest(name: &str) -> PathBuf {
    Path::new("/opt/pinfold").join(name)
}

impl Harness {
    /// The host directory to mount at [`guest`] for a box on this host,
    /// downloading and installing the assets if the cache lacks them.
    pub fn install(&self) -> io::Result<PathBuf> {
        Ok(self.install_os_arch(OS_ARCH)?.join(&self.name))
    }

    /// The directory of this harness's host helpers for this host, installed
    /// the same way. It never enters a box.
    pub fn install_host(&self) -> io::Result<PathBuf> {
        self.install_os_arch(HOST_OS_ARCH)
    }

    /// Download and install the assets for `os_arch` if the cache lacks
    /// them, and return their directory.
    fn install_os_arch(&self, os_arch: &str) -> io::Result<PathBuf> {
        let assets = self.assets(os_arch);
        let dir = self.dir(os_arch)?;
        dirs::install_dir(&dir, |staging| {
            assets
                .iter()
                .try_for_each(|asset| self.fetch(asset, staging))
        })?;
        Ok(dir)
    }

    /// This harness's assets for `os_arch`.
    fn assets(&self, os_arch: &str) -> Vec<&Asset> {
        self.asset
            .iter()
            .filter(|asset| asset.os_arch == os_arch)
            .collect()
    }

    /// The cache directory of this harness for `os_arch`, installed or not.
    fn dir(&self, os_arch: &str) -> io::Result<PathBuf> {
        Ok(dirs::artifacts_dir()?
            .join(&self.name)
            .join(&self.version)
            .join(os_arch))
    }

    /// Download `asset` into `staging`, verify its sha256, and install it at
    /// its path there.
    fn fetch(&self, asset: &Asset, staging: &Path) -> io::Result<()> {
        let url = &asset.url;
        let download = staging.join(".download");
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
            .arg(&download)
            .arg(url)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!("curl {url}: {status}")));
        }
        let actual = super::sha256_hex(fs::read(&download)?);
        if actual != asset.sha256 {
            return Err(io::Error::other(format!(
                "{} {} {url}: checksum mismatch: expected {}, got {actual}",
                self.name, self.version, asset.sha256
            )));
        }
        let target = staging.join(&asset.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        match asset.install {
            Install::Executable => {
                fs::set_permissions(&download, fs::Permissions::from_mode(0o755))?;
                fs::rename(&download, &target)
            }
            Install::TarGz => {
                let unpacked = staging.join(".unpack");
                fs::create_dir(&unpacked)?;
                let status = Command::new("tar")
                    .arg("-xzf")
                    .arg(&download)
                    .arg("-C")
                    .arg(&unpacked)
                    .status()?;
                if !status.success() {
                    return Err(io::Error::other(format!("tar {url}: {status}")));
                }
                let entries = dirs::entries(&unpacked)?;
                let [entry] = entries.as_slice() else {
                    return Err(io::Error::other(format!(
                        "{url}: {} top-level entries, expected one",
                        entries.len()
                    )));
                };
                fs::rename(entry.path(), &target)?;
                fs::remove_dir(&unpacked)?;
                fs::remove_file(&download)
            }
        }
    }
}

/// The `os-arch` of a box on this host: Apple `container` and podman run
/// native images.
const OS_ARCH: &str = if cfg!(target_arch = "aarch64") {
    "linux-arm64"
} else {
    "linux-x64"
};

/// The `os-arch` of a host helper on this host, as its rows name it. The
/// macOS CLI is arm64 only.
const HOST_OS_ARCH: &str = if cfg!(target_os = "macos") {
    "host-darwin-arm64"
} else if cfg!(target_arch = "aarch64") {
    "host-linux-arm64"
} else {
    "host-linux-x64"
};

/// The Linux init binary to mount into a box.
///
/// On macOS the CLI embeds the `aarch64-unknown-linux-musl` build and
/// extracts it once per content into the artifact cache. On Linux the CLI
/// is itself a static Linux binary and mounts its own executable.
pub fn init() -> io::Result<PathBuf> {
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

/// One pinned harness's cache state, as `doctor` and `artifacts` report it.
/// Nothing here downloads.
#[derive(Serialize)]
pub struct Pin {
    pub name: &'static str,
    pub version: &'static str,
    /// The host directory mounted at `/opt/pinfold/<name>`.
    pub path: PathBuf,
    /// Whether the cache holds this host's assets.
    pub cached: bool,
    /// This host's assets.
    assets: Vec<&'static Asset>,
}

/// Every pinned harness and whether the cache holds it. Never downloads.
pub fn pins() -> io::Result<Vec<Pin>> {
    HARNESSES
        .iter()
        .map(|harness| {
            let dir = harness.dir(OS_ARCH)?;
            Ok(Pin {
                name: &harness.name,
                version: &harness.version,
                path: dir.join(&harness.name),
                cached: dir.is_dir(),
                assets: harness.assets(OS_ARCH),
            })
        })
        .collect()
}

/// Every `<name>/<version>` in the cache that no harness pins, names no
/// harness has included. The embedded init lives under `init/` and is never
/// listed.
pub fn unpinned_versions() -> io::Result<Vec<PathBuf>> {
    let mut unpinned = Vec::new();
    for name in dirs::entries(&dirs::artifacts_dir()?)? {
        if name.file_name() == "init" {
            continue;
        }
        for version in dirs::entries(&name.path())? {
            let pinned = HARNESSES.iter().any(|harness| {
                name.file_name() == harness.name.as_str()
                    && version.file_name() == harness.version.as_str()
            });
            if !pinned {
                unpinned.push(version.path());
            }
        }
    }
    Ok(unpinned)
}
