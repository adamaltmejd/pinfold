//! The Apple `container` runtime adapter.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::process::Child;

use crate::core::plan::Plan;
use crate::core::runtime::{
    BoxInfo, BoxStat, BuildRequest, DiskInfo, ImageInfo, MemoryStat, PidsStat, Runtime, inspect,
    list_output, output, parse_json, spawn_error,
};

/// Where Apple `container` forwards `SSH_AUTH_SOCK` inside the box.
pub const GUEST_PROXY_SOCKET: &str = "/var/host-services/ssh-auth.sock";

/// Apple `container`.
pub struct Apple;

impl Runtime for Apple {
    fn up(
        &self,
        plan: &Plan,
        init: &Path,
        _state_dir: &Path,
        proxy_socket: Option<&Path>,
        _seccomp: Option<&Path>,
    ) -> io::Result<Child> {
        let mut extra: Vec<OsString> = vec!["--progress".into(), "none".into()];
        if proxy_socket.is_some() {
            // Off-label: the runtime treats the socket as an SSH agent and
            // forwards it to GUEST_PROXY_SOCKET.
            extra.push("--ssh".into());
        }
        let guest_socket = proxy_socket.map(|_| GUEST_PROXY_SOCKET);
        let mut command = super::up(
            "container",
            plan,
            init,
            guest_socket,
            extra,
            plan.env.iter(),
        )?;
        if let Some(socket) = proxy_socket {
            command.env("SSH_AUTH_SOCK", socket);
        }
        command
            .spawn()
            .map_err(|error| spawn_error("container", error))
    }

    fn preflight(&self) -> io::Result<Option<PathBuf>> {
        let reported = self.version()?;
        let version = reported
            .strip_prefix("container CLI version ")
            .and_then(|value| value.split_whitespace().next())
            .unwrap_or(&reported);
        super::require_version("container", version, [1, 5, 0])?;
        Ok(None)
    }

    fn down(&self, name: &str) -> io::Result<()> {
        // Apple's forced removal races two process-output waiters (#83).
        // Kill through the existing monitor, then remove without forcing.
        // A stopped or absent box can refuse kill; removal still decides
        // success, and cannot force-stop a box whose kill actually failed.
        let _ = output(&["container", "kill", "--signal", "KILL", name]);
        // stderr is captured: an absent box fails `rm` with the runtime's
        // not-found text, which is noise once the box is gone.
        let Err(error) = output(&["container", "rm", name]) else {
            return Ok(());
        };
        // Another process may have removed the box between the list and this
        // call; a box the runtime no longer lists counts as removed.
        if self.list(None)?.iter().all(|box_| box_.id != name) {
            return Ok(());
        }
        Err(error)
    }

    /// The limits Apple reports, and null for what the VM cannot answer
    /// (kills, memory use, pids use).
    fn stat(&self, name: &str) -> io::Result<BoxStat> {
        let resources = container(name)?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no box named {name:?}"))
            })?
            .configuration
            .resources;
        Ok(BoxStat {
            name: name.to_string(),
            oom_kills: None,
            memory: MemoryStat {
                limit: resources.memory,
                ..MemoryStat::default()
            },
            pids: PidsStat::default(),
        })
    }

    fn list(&self, cancel: Option<&AtomicBool>) -> io::Result<Vec<BoxInfo>> {
        Ok(containers(cancel)?
            .into_iter()
            .map(|container| {
                let configuration = container.configuration;
                let digest = configuration.image.descriptor.digest;
                BoxInfo {
                    id: container.id,
                    labels: configuration.labels,
                    image_id: digest
                        .strip_prefix("sha256:")
                        .unwrap_or(&digest)
                        .to_string(),
                    image_ref: configuration.image.reference,
                    created: configuration.created,
                    running: container.status.state == "running",
                }
            })
            .collect())
    }

    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        let json = output(&["container", "image", "list", "--format", "json"])?;
        let images: Vec<ListedImage> = parse_json("container image list", &json)?;
        images.into_iter().map(image_info).collect()
    }

    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>> {
        // `image inspect` prints the same entries as `image list`.
        match inspect::<ListedImage>("container", reference)? {
            Ok(image) => image_info(image).map(Ok),
            Err(stderr) => Ok(Err(stderr)),
        }
    }

    fn remove_image(&self, reference: &str) -> io::Result<()> {
        // `image delete` also collects the layers no image references.
        output(&["container", "image", "delete", reference]).map(|_| ())
    }

    fn purge_build_cache(&self) -> io::Result<()> {
        let Some(builder) = builder()? else {
            return Ok(());
        };
        if builder.status.state != "running" {
            // `builder start` may replace an existing instance when its
            // defaults changed; start this instance without replacing it.
            output(&["container", "start", "buildkit"])?;
        }
        // BuildKit holds active records while pruning. Deleting the shared
        // builder instead interrupts builds in other processes (#75).
        output(&[
            "container",
            "exec",
            "buildkit",
            "buildctl",
            "prune",
            "--all",
        ])?;
        // Pruning frees guest blocks; online trim returns them to the host
        // without replacing the builder or stopping active builds (#86).
        output(&["container", "clean", "buildkit"]).map(drop)
    }

    fn build_cache(&self) -> &'static str {
        "the shared builder's backing storage (unused cache is pruned)"
    }

    fn build_cache_bytes(&self) -> io::Result<Option<u64>> {
        let Some(builder) = builder()? else {
            return Ok(Some(0));
        };
        // The exports mount records the runtime's actual app root, including
        // a custom root. Count allocated blocks: rootfs.ext4 is sparse and
        // BuildKit's logical cache size misses retained filesystem space (#74).
        let exports = builder
            .configuration
            .mounts
            .iter()
            .find(|mount| mount.destination == "/var/lib/container-builder-shim/exports")
            .map(|mount| Path::new(&mount.source))
            .ok_or_else(|| io::Error::other("builder exports mount is missing"))?;
        let root = exports
            .parent()
            .ok_or_else(|| io::Error::other("builder exports mount has no parent"))?;
        allocated_bytes(&root.join("containers/buildkit")).map(Some)
    }

    fn list_disks(&self) -> io::Result<Vec<DiskInfo>> {
        let json = list_output(&["container", "volume", "list", "--format", "json"], None)?;
        let volumes: Vec<ListedVolume> = parse_json("container volume list", &json)?;
        volumes
            .into_iter()
            .map(|volume| {
                let configuration = volume.configuration;
                Ok(DiskInfo {
                    allocated: allocated_bytes(Path::new(&configuration.source))?,
                    name: configuration.name,
                    size: configuration.size,
                })
            })
            .collect()
    }

    fn create_disk(&self, name: &str, bytes: u64) -> io::Result<()> {
        let size = format!("{}G", bytes >> 30);
        output(&["container", "volume", "create", "-s", &size, name]).map(drop)
    }

    fn remove_disk(&self, name: &str) -> io::Result<()> {
        output(&["container", "volume", "rm", name]).map(drop)
    }

    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>> {
        let cache_flags: &[&str] = if request.cache { &[] } else { &["--no-cache"] };
        super::build("container", cache_flags, request)
    }

    fn program(&self) -> &'static str {
        "container"
    }

    fn name(&self) -> &'static str {
        "Apple container"
    }

    fn isolation(&self) -> &'static str {
        "one VM per box"
    }
}

/// Wait, bounded, until `list` reports box `name` running. Apple records
/// `.running` only after the box's first process starts, so `ready` can
/// arrive first; an exec is refused in that gap.
pub fn wait_until_running(name: &str) -> io::Result<BoxInfo> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = Apple.list(None)?.into_iter().find(|box_| box_.id == name);
        match state {
            Some(box_) if box_.running => return Ok(box_),
            _ if Instant::now() >= deadline => {
                return Err(io::Error::other(match state {
                    Some(_) => format!("box {name:?} is not running after ready"),
                    _ => format!("the runtime does not list box {name:?} after ready"),
                }));
            }
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Make the forwarded proxy socket in box `name` connectable by the box
/// user: it arrives root-owned and mode 000. The one transient root exec.
pub fn make_proxy_connectable(name: &str) -> io::Result<()> {
    output(&[
        "container",
        "exec",
        "--user",
        "0:0",
        name,
        "chmod",
        "666",
        GUEST_PROXY_SOCKET,
    ])
    .map(drop)
}

/// Give new volumes' roots, which arrive `0:0 755`, to the box user. A box's
/// root exec cannot: the box drops every capability. So a transient
/// container of the box's image runs init as root with only `CAP_CHOWN`
/// and no network. It takes the box's name, still held by its claim, and
/// its owner and generation labels, so `clean` sees a leftover as a dead
/// box and removes it.
pub fn own_disks(
    names: &[&str],
    image: &str,
    init: &Path,
    name: &str,
    labels: &BTreeMap<String, String>,
) -> io::Result<()> {
    let init_dir = init
        .parent()
        .expect("box::up refuses an init without a directory");
    let owner = format!("{}:{}", nix::unistd::getuid(), nix::unistd::getgid());
    let mut command = std::process::Command::new("container");
    command
        .args(["run", "--rm", "--progress", "none", "--network", "none"])
        .args(["--read-only", "--cap-drop", "ALL", "--cap-add", "CAP_CHOWN"])
        .args(["--name", name])
        .args(["--user", "0:0", "--mount"])
        .arg(super::bind(init_dir, init_dir, true));
    for (key, value) in labels {
        command.arg("--label").arg(format!("{key}={value}"));
    }
    let mut setup: Vec<OsString> = vec!["init".into(), "setup".into()];
    for (index, name) in names.iter().enumerate() {
        let guest = format!("/pinfold-disk/{index}");
        command
            .arg("--mount")
            .arg(format!("type=volume,source={name},target={guest}"));
        setup.extend(["--own".into(), owner.clone().into(), guest.into()]);
    }
    let result = command
        .arg("--entrypoint")
        .arg(init)
        .arg(image)
        .args(setup)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| spawn_error("container", error))?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "give new disks to the box user: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    Ok(())
}

/// Run `init receive GUEST` in box `name` as the box user with `stream`
/// written to its stdin: a new disk's `from` copy.
pub fn copy_in(
    name: &str,
    init: &Path,
    guest: &Path,
    stream: impl FnOnce(&mut dyn io::Write) -> io::Result<()>,
) -> io::Result<()> {
    let mut child = std::process::Command::new("container")
        .args(["exec", "-i", name])
        .arg(init)
        .args(["init", "receive"])
        .arg(guest)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| spawn_error("container", error))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let sent = stream(&mut stdin);
    drop(stdin);
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "copy into {}: {}",
            guest.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    sent
}

/// One `container list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedContainer {
    id: String,
    #[serde(default)]
    configuration: ListedConfiguration,
    #[serde(default)]
    status: ListedStatus,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ListedConfiguration {
    labels: BTreeMap<String, String>,
    image: ListedBoxImage,
    /// ISO 8601, which is RFC 3339.
    #[serde(rename = "creationDate")]
    created: String,
    resources: ListedResources,
    mounts: Vec<ListedMount>,
}

#[derive(Deserialize)]
struct ListedMount {
    source: String,
    destination: String,
}

fn allocated_bytes(path: &Path) -> io::Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    let mut bytes = metadata.blocks() * 512;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            bytes += allocated_bytes(&entry?.path())?;
        }
    }
    Ok(bytes)
}

/// The image a box runs, as the runtime records it.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ListedBoxImage {
    reference: String,
    descriptor: ListedBoxImageDescriptor,
}

#[derive(Default, Deserialize)]
struct ListedBoxImageDescriptor {
    /// `sha256:<hex>`; the hex is the image list's `id`.
    #[serde(default)]
    digest: String,
}

/// The limits the runtime reports for a box.
#[derive(Default, Deserialize)]
struct ListedResources {
    #[serde(rename = "memoryInBytes")]
    memory: Option<u64>,
}

#[derive(Default, Deserialize)]
struct ListedStatus {
    #[serde(default)]
    state: String,
}

/// Every box `container list` reports, running or not.
fn containers(cancel: Option<&AtomicBool>) -> io::Result<Vec<ListedContainer>> {
    let json = list_output(&["container", "list", "--all", "--format", "json"], cancel)?;
    parse_json("container list", &json)
}

/// The listed container with this id, if the runtime has one.
fn container(name: &str) -> io::Result<Option<ListedContainer>> {
    Ok(containers(None)?.into_iter().find(|entry| entry.id == name))
}

/// The shared BuildKit builder, if the runtime has one.
fn builder() -> io::Result<Option<ListedContainer>> {
    container("buildkit")
}

/// One `container volume list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedVolume {
    configuration: ListedVolumeConfiguration,
}

#[derive(Deserialize)]
struct ListedVolumeConfiguration {
    name: String,
    #[serde(rename = "sizeInBytes")]
    size: u64,
    /// The volume's image on the host.
    source: String,
}

/// One `container image list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedImage {
    id: String,
    configuration: ListedImageConfiguration,
    /// Each variant's OCI image config sits at `/config/config`.
    #[serde(default)]
    variants: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct ListedImageConfiguration {
    name: String,
    descriptor: Option<ListedImageDescriptor>,
}

#[derive(Deserialize)]
struct ListedImageDescriptor {
    #[serde(default)]
    annotations: BTreeMap<String, String>,
    /// `sha256:<hex>`.
    digest: Option<String>,
}

/// One image entry, with its descriptor's digest.
fn image_info(image: ListedImage) -> io::Result<ImageInfo> {
    // Build labels are OCI image config labels; a locally built image also
    // carries name annotations on its index descriptor.
    let (mut labels, digest) = image
        .configuration
        .descriptor
        .map(|descriptor| (descriptor.annotations, descriptor.digest))
        .unwrap_or_default();
    for variant in &image.variants {
        if let Some(found) = variant.pointer("/config/config/Labels") {
            labels.extend(
                BTreeMap::<String, String>::deserialize(found).map_err(|error| {
                    io::Error::other(format!("image {} has invalid labels: {error}", image.id))
                })?,
            );
        }
    }
    Ok(ImageInfo {
        id: image.id,
        reference: image.configuration.name,
        labels,
        digest,
    })
}
