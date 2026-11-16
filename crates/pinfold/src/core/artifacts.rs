//! Pinned host helper and infrastructure, and this build's init.

#[cfg(target_os = "linux")]
use crate::core::process::CommandGuard;
use crate::core::process::GroupChild;

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use ring::digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::dirs;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const DOWNLOAD_LIMIT: u64 = 512 * 1024 * 1024;

/// The host helper pin, embedded in this build.
static CODEX_HELPER: LazyLock<Artifact> = LazyLock::new(|| {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PinFile {
        host_helper: [Artifact; 1],
    }
    let [helper] = toml::from_str::<PinFile>(include_str!("../../host-helper.toml"))
        .expect("the embedded host-helper.toml parses")
        .host_helper;
    helper
});

/// Firecracker and the guest kernel for Linux boxes.
static VMM: LazyLock<Artifact> = LazyLock::new(|| {
    toml::from_str(include_str!("../../vmm.toml")).expect("the embedded vmm.toml parses")
});

#[cfg(target_os = "linux")]
pub fn vmm_version() -> &'static str {
    &VMM.version
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Builder {
    url: String,
    archive_sha256: String,
    archive_bytes: u64,
    root_sha256: String,
    root_bytes: u64,
    recipe_sha256: String,
}

static BUILDER: LazyLock<Builder> = LazyLock::new(|| {
    toml::from_str(include_str!("../../builder.toml")).expect("the embedded builder.toml parses")
});

#[cfg(target_os = "linux")]
pub fn builder_identity() -> &'static str {
    &BUILDER.archive_sha256
}

fn builder_dir() -> io::Result<PathBuf> {
    Ok(dirs::artifacts_dir()?
        .join("builder")
        .join(&BUILDER.archive_sha256))
}

/// Install the pinned builder from its archive cache or public download.
#[cfg(target_os = "linux")]
pub fn builder(cancel: Option<&AtomicBool>) -> io::Result<PathBuf> {
    let version = builder_dir()?;
    let installed = version.join("linux-x64");
    let deadline = Instant::now() + DOWNLOAD_TIMEOUT;
    dirs::install_dir(&installed, |staging| {
        let archive = version.join(format!(
            "pinfold-builder-linux-x64-{}.tar.gz",
            BUILDER.archive_sha256
        ));
        let snapshot = staging.join(".archive");
        let limit = BUILDER.archive_bytes.min(DOWNLOAD_LIMIT);
        if archive.is_file() {
            builder_file(
                &archive,
                limit,
                &BUILDER.archive_sha256,
                Some(&snapshot),
                deadline,
                cancel,
            )?;
        } else {
            super::download::file(
                &BUILDER.url,
                &snapshot,
                super::pipe::remaining(deadline, cancel, "builder install")?,
                limit,
                cancel,
            )?;
            builder_file(
                &snapshot,
                limit,
                &BUILDER.archive_sha256,
                None,
                deadline,
                cancel,
            )?;
        }
        let root = staging.join("root.ext4");
        let output = fs::File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&root)?;
        // Bootstrap bytes are publisher-pinned. Only their named root is
        // streamed to our fixed path; archive ownership and paths are not applied.
        let mut child = Command::new("tar")
            .arg("-xOzf")
            .arg(&snapshot)
            .arg("root.ext4")
            .stdin(std::process::Stdio::null())
            .stdout(output)
            .stderr(std::process::Stdio::null())
            .spawn_guarded()?;
        let unpacked = (|| {
            loop {
                super::pipe::remaining(deadline, cancel, "builder install")?;
                if let Some(status) = child.try_wait()? {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(io::Error::other("builder archive extraction failed"))
                    };
                }
                super::pipe::wait_ready(
                    None,
                    nix::poll::PollFlags::empty(),
                    deadline,
                    cancel,
                    "builder install",
                )?;
            }
        })();
        if unpacked.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        unpacked?;
        builder_root(&root, deadline, cancel)?;
        fs::remove_file(snapshot)
    })?;
    let root = installed.join("root.ext4");
    builder_root(&root, deadline, cancel)?;
    Ok(root)
}

fn builder_root(path: &Path, deadline: Instant, cancel: Option<&AtomicBool>) -> io::Result<()> {
    builder_file(
        path,
        BUILDER.root_bytes,
        &BUILDER.root_sha256,
        None,
        deadline,
        cancel,
    )
}

fn builder_file(
    path: &Path,
    bytes: u64,
    expected: &str,
    copy: Option<&Path>,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<()> {
    let mut input = fs::File::open(path)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.len() != bytes {
        return Err(io::Error::other("builder-integrity: size mismatch"));
    }
    let mut output = copy
        .map(|path| {
            fs::File::options()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
        })
        .transpose()?;
    let mut hash = digest::Context::new(&digest::SHA256);
    let mut total = 0;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        super::pipe::remaining(deadline, cancel, "builder verification")?;
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > bytes {
            return Err(io::Error::other("builder-integrity: size mismatch"));
        }
        hash.update(&buffer[..read]);
        if let Some(output) = &mut output {
            output.write_all(&buffer[..read])?;
        }
    }
    if total != bytes || super::hex(hash.finish().as_ref()) != expected {
        return Err(io::Error::other("builder-integrity: checksum mismatch"));
    }
    Ok(())
}

/// One pinned host helper or infrastructure release.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    version: String,
    asset: Vec<Asset>,
}

/// One pinned release asset.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    #[serde(rename = "os-arch")]
    os_arch: String,
    url: String,
    sha256: String,
    install: Install,
    /// Where it is installed, relative to the `<os-arch>/` directory.
    #[serde(rename = "as")]
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

/// Install the host Codex helper and return its executable. It never enters a box.
pub fn codex_helper(cancel: Option<&AtomicBool>) -> io::Result<PathBuf> {
    Ok(CODEX_HELPER
        .install_os_arch(HOST_OS_ARCH, cancel)?
        .join("codex-app-server"))
}

impl Artifact {
    /// Download and install the assets for `os_arch` if the cache lacks
    /// them, and return their directory. A host helper's (`HOST_OS_ARCH`)
    /// never enters a box.
    fn install_os_arch(&self, os_arch: &str, cancel: Option<&AtomicBool>) -> io::Result<PathBuf> {
        let assets = self.assets(os_arch);
        let dir = self.dir(os_arch)?;
        dirs::install_dir(&dir, |staging| {
            assets
                .iter()
                .try_for_each(|asset| self.fetch(asset, staging, cancel))
        })?;
        Ok(dir)
    }

    fn assets(&self, os_arch: &str) -> Vec<&Asset> {
        self.asset
            .iter()
            .filter(|asset| asset.os_arch == os_arch)
            .collect()
    }

    /// The cache directory for `os_arch`, installed or not.
    fn dir(&self, os_arch: &str) -> io::Result<PathBuf> {
        Ok(dirs::artifacts_dir()?
            .join(&self.name)
            .join(&self.version)
            .join(os_arch))
    }

    /// Download `asset` into `staging`, verify its sha256, and install it at
    /// its path there.
    fn fetch(&self, asset: &Asset, staging: &Path, cancel: Option<&AtomicBool>) -> io::Result<()> {
        let url = &asset.url;
        let download = staging.join(".download");
        super::download::file(url, &download, DOWNLOAD_TIMEOUT, DOWNLOAD_LIMIT, cancel)?;
        let actual = super::download::sha256_file(&download)?;
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
                let deadline = Instant::now() + DOWNLOAD_TIMEOUT;
                super::pipe::remaining(deadline, cancel, "artifact extraction")?;
                let mut child = GroupChild::spawn(
                    Command::new("tar")
                        .arg("-xzf")
                        .arg(&download)
                        .arg("-C")
                        .arg(&unpacked)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null()),
                )?;
                let status = loop {
                    super::pipe::remaining(deadline, cancel, "artifact extraction")?;
                    if child.exited()? {
                        break child.finish()?;
                    }
                    super::pipe::wait_ready(
                        None,
                        nix::poll::PollFlags::empty(),
                        deadline,
                        cancel,
                        "artifact extraction",
                    )?;
                };
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

/// The installed VMM: the Firecracker binary and the guest kernel.
#[cfg(target_os = "linux")]
pub struct Vmm {
    pub firecracker: PathBuf,
    pub kernel: PathBuf,
}

/// Install [`VMM`] for this host if the cache lacks it.
#[cfg(target_os = "linux")]
pub fn vmm(cancel: Option<&AtomicBool>) -> io::Result<Vmm> {
    let dir = VMM.install_os_arch(OS_ARCH, cancel)?;
    let (firecracker, _) = VMM
        .version
        .split_once('+')
        .expect("vmm.toml's version is firecracker+kernel");
    Ok(Vmm {
        firecracker: dir
            .join("firecracker")
            .join(format!("firecracker-v{firecracker}-x86_64")),
        kernel: dir.join("vmlinux"),
    })
}

/// The `os-arch` of a box on this host: Apple `container` and Firecracker
/// run native images.
const OS_ARCH: &str = if cfg!(target_arch = "aarch64") {
    "linux-arm64"
} else {
    "linux-x64"
};

/// The `os-arch` of a host helper on this host, as its rows name it. The
/// macOS CLI is arm64 only; Linux arm64 is unsupported.
const HOST_OS_ARCH: &str = if cfg!(target_os = "macos") {
    "host-darwin-arm64"
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
        .join(super::hex(&Sha256::digest(INIT)))
        .join("linux-arm64");
    dirs::install_dir(&dir, |staging| {
        let path = staging.join("pinfold");
        fs::write(&path, INIT)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
    })?;
    Ok(dir.join("pinfold"))
}

/// One pin's cache state, as `doctor` and `artifacts` report it. Nothing
/// here downloads.
#[derive(Serialize)]
pub struct Pin {
    pub name: &'static str,
    pub version: &'static str,
    /// The host helper executable or infrastructure install directory.
    pub path: PathBuf,
    /// Whether the cache holds this host's assets.
    pub cached: bool,
    /// This host's assets.
    assets: Vec<PinAsset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recipe_sha256: Option<&'static str>,
}

#[derive(Serialize)]
struct PinAsset {
    url: &'static str,
    sha256: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
}

/// The host helper and, on Linux x64, infrastructure cache state. Never downloads.
pub fn pins() -> io::Result<Vec<Pin>> {
    let pin = |artifact: &'static Artifact,
               os_arch: &str,
               path: fn(PathBuf) -> PathBuf|
     -> io::Result<Pin> {
        let dir = artifact.dir(os_arch)?;
        Ok(Pin {
            name: &artifact.name,
            version: &artifact.version,
            path: path(dir.clone()),
            cached: dirs::installed(&dir)?,
            assets: artifact
                .assets(os_arch)
                .into_iter()
                .map(|asset| PinAsset {
                    url: &asset.url,
                    sha256: &asset.sha256,
                    bytes: None,
                })
                .collect(),
            recipe_sha256: None,
        })
    };
    let mut pins = vec![pin(&CODEX_HELPER, HOST_OS_ARCH, |dir| {
        dir.join("codex-app-server")
    })?];
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        pins.push(pin(&VMM, OS_ARCH, |dir| dir)?);
        let path = builder_dir()?.join("linux-x64");
        pins.push(Pin {
            name: "builder",
            version: &BUILDER.archive_sha256,
            cached: dirs::installed(&path)?
                && builder_root(
                    &path.join("root.ext4"),
                    Instant::now() + DOWNLOAD_TIMEOUT,
                    None,
                )
                .is_ok(),
            path,
            assets: vec![PinAsset {
                url: &BUILDER.url,
                sha256: &BUILDER.archive_sha256,
                bytes: Some(BUILDER.archive_bytes),
            }],
            recipe_sha256: Some(&BUILDER.recipe_sha256),
        });
    }
    Ok(pins)
}

/// Every unpinned `<name>/<version>` in the cache. The embedded init lives
/// under `init/` and is never listed.
pub fn unpinned_versions() -> io::Result<Vec<PathBuf>> {
    let mut unpinned = Vec::new();
    for name in dirs::entries(&dirs::artifacts_dir()?)? {
        if name.file_name() == "init" {
            continue;
        }
        for version in dirs::entries(&name.path())? {
            let pinned = [&*CODEX_HELPER, &*VMM].iter().any(|artifact| {
                name.file_name() == artifact.name.as_str()
                    && version.file_name() == artifact.version.as_str()
            });
            let builder = name.file_name() == "builder"
                && version.file_name() == BUILDER.archive_sha256.as_str();
            if !pinned && !builder {
                unpinned.push(version.path());
            }
        }
    }
    Ok(unpinned)
}
