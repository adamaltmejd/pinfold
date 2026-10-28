//! The Apple `container` runtime adapter.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::core::disk::{self, Fill, Seeding};
use crate::core::image::service::inspect;
use crate::core::image::{BuildRequest, ImageInfo, ImageService};
use crate::core::plan::Plan;
use crate::core::runtime::{
    BoxInfo, BoxStat, DiskInfo, MemoryStat, PidsStat, Runtime, output, parse_json, spawn_error,
};
use crate::core::{clean, ownership};
use crate::dirs;
use crate::init::BOX_ENV_PREFIX;

/// Where Apple `container` forwards `SSH_AUTH_SOCK` inside the box.
pub const GUEST_PROXY_SOCKET: &str = "/var/host-services/ssh-auth.sock";

/// Apple `container`.
pub struct Apple;

/// The volume behind disk `name`. Apple's volumes are host-wide and a disk
/// belongs to one state root, so the root's key prefixes the name.
fn volume(name: &str) -> io::Result<String> {
    Ok(format!("{}-{name}", dirs::state_root_key()?))
}

/// Start the attached `container run` process that owns the box: stdin and
/// stdout piped. When `proxy_socket` is set, carry that host unix socket
/// into the box, where init relays to it.
///
/// Guest values are encoded under prefixed names in the child's
/// environment, so they cannot configure the host client, a spec `PATH`
/// cannot choose the runtime (GitHub #70), argv holds only names, and `ps`
/// cannot read a secret.
///
/// `init` is a host path; it and its directory are mounted read-only at the
/// same path, so it is also the path PID 1 runs.
#[cfg_attr(target_os = "linux", expect(dead_code))]
pub fn up(plan: &Plan, init: &Path, proxy_socket: Option<&Path>) -> io::Result<Client> {
    let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
    let mut command = Command::new("container");
    command
        .args(["run", "-i", "--name", &plan.name])
        .args(["--network", "none", "--cap-drop", "ALL"])
        .args(["--read-only", "--tmpfs", "/tmp"])
        .arg("--user")
        .arg(format!("{uid}:{gid}"))
        .args(["--progress", "none"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(socket) = proxy_socket {
        // Off-label: the runtime treats the socket as an SSH agent and
        // forwards it to GUEST_PROXY_SOCKET.
        command.arg("--ssh").env("SSH_AUTH_SOCK", socket);
    }
    if let Some(cpus) = plan.cpus {
        command.arg("--cpus").arg(cpus.to_string());
    }
    if let Some(memory) = &plan.memory {
        command.args(["--memory", memory]);
    }
    for (key, value) in &plan.labels {
        command.arg("--label").arg(format!("{key}={value}"));
    }
    for (name, value) in super::guest_vars(plan) {
        guest_env(command.as_std_mut(), &name, &value);
    }
    for mount in &plan.mounts {
        command.arg("--mount").arg(bind(&mount.host, &mount.guest));
    }
    for disk in &plan.disks {
        let mut value = OsString::from(format!(
            "type=volume,source={},target=",
            volume(&disk.name)?
        ));
        value.push(&disk.guest);
        if disk.readonly {
            value.push(",readonly");
        }
        command.arg("--mount").arg(value);
    }
    command
        .arg("--mount")
        .arg(init_mount(init))
        .arg("--entrypoint")
        .arg(init)
        .arg(
            plan.image
                .as_deref()
                .expect("box up resolves the profile's image before the runtime runs"),
        )
        .arg("init")
        .args(proxy_socket.map(|_| GUEST_PROXY_SOCKET));
    command
        .spawn()
        .map(Client)
        .map_err(|error| spawn_error("container", error))
}

/// Pass guest variable `name` through the child's environment, hex-encoded
/// under a prefixed name, so argv holds only the name.
fn guest_env(command: &mut std::process::Command, name: &str, value: &OsStr) {
    let transport = format!("{BOX_ENV_PREFIX}{name}");
    let encoded: String = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    command.args(["--env", &transport]).env(transport, encoded);
}

/// A read-only `type=bind` mount value.
fn bind(host: &Path, guest: &Path) -> OsString {
    let mut value = OsString::from("type=bind,source=");
    value.push(host);
    value.push(",target=");
    value.push(guest);
    value.push(",readonly");
    value
}

/// Init's directory, mounted read-only at the same path.
fn init_mount(init: &Path) -> OsString {
    let dir = init
        .parent()
        .expect("init lives in the artifacts directory");
    bind(dir, dir)
}

/// The attached `container run` process that owns a box.
#[cfg_attr(target_os = "linux", expect(dead_code))]
pub struct Client(Child);

#[cfg_attr(target_os = "linux", expect(dead_code))]
impl Client {
    /// Wait until init reports ready. The error is why it did not.
    pub async fn ready(&mut self) -> Result<(), String> {
        let stdout = self
            .0
            .stdout
            .take()
            .ok_or("the runtime did not pipe the box's stdout")?;
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) if line == "ready" => break,
                Ok(Some(_)) => {}
                Ok(None) => return Err("box exited before ready".to_string()),
                Err(error) => return Err(format!("box output failed: {error}")),
            }
        }
        // Keep the pipe drained so a talkative box cannot block on it.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        Ok(())
    }

    /// Wait until the box ends on its own or is stopped.
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.0.wait().await
    }

    /// Stop and remove a ready box. True when the stop was one pinfold
    /// requested of a running box, which syncs its disks.
    pub async fn down(&mut self, name: &str) -> io::Result<bool> {
        let requested = matches!(self.0.try_wait(), Ok(None));
        // Runtime removal waits for attached output to close. Keep its
        // drain running on the owner while the runtime command blocks.
        let name = name.to_string();
        let removed = tokio::task::spawn_blocking(move || Apple.down(&name))
            .await
            .map_err(io::Error::other)
            .and_then(|result| result);
        let _ = self.0.wait().await;
        removed.map(|()| requested)
    }

    /// Stop a start that ended before ready and remove its box, returning
    /// whether the stop synced its disks and how the box ended.
    pub async fn abort(mut self, name: &str) -> (io::Result<bool>, io::Result<ExitStatus>) {
        let requested = matches!(self.0.try_wait(), Ok(None));
        // The client goes first, so one still creating the box cannot
        // finish after the removal. The box exists by name even when
        // readiness failed.
        let _ = self.0.start_kill();
        let status = self.0.wait().await;
        (Apple.down(name).map(|()| requested), status)
    }
}

impl Runtime for Apple {
    fn preflight(&self) -> io::Result<()> {
        let reported = self.version()?;
        let version = reported
            .strip_prefix("container CLI version ")
            .and_then(|value| value.split_whitespace().next())
            .unwrap_or(&reported);
        super::require_version("container", version, [1, 6, 0])
    }

    fn finish_start(
        &self,
        name: &str,
        init: &Path,
        proxy_socket: Option<&Path>,
        fills: &[Fill],
        seeding: Option<&Seeding>,
    ) -> io::Result<BoxInfo> {
        // Init can report ready before Apple records the box running.
        // Wait even without egress, before the caller's first exec.
        let actual = wait_until_running(name)?;
        if proxy_socket.is_some() {
            make_proxy_connectable(name)?;
        }
        for fill in fills {
            copy_in(name, init, &fill.guest, false, |out| disk::send(fill, out))?;
        }
        if let Some(seeding) = seeding {
            copy_in(name, init, &seeding.guest, true, |out| {
                disk::send_seeds(&seeding.relative, &seeding.seeds, out)
            })?;
        }
        Ok(actual)
    }

    fn down(&self, name: &str) -> io::Result<()> {
        // Apple's forced removal races two process-output waiters (#83).
        // Kill through the existing monitor, then remove without forcing.
        // A stopped or absent box can refuse kill; removal still decides
        // success.
        let _ = output(&["container", "kill", "--signal", "KILL", name]);
        // Apple can report a killed box `running` for a moment, and `rm`
        // refuses it then; `stop` returns once it has stopped. Only a box
        // that refused kill while running gets stop's SIGTERM, 5 s grace and
        // kill, which is #83's second waiter.
        let _ = output(&["container", "stop", name]);
        // stderr is captured: an absent box fails `rm` with the runtime's
        // not-found text, which is noise once the box is gone.
        let Err(error) = output(&["container", "rm", name]) else {
            return Ok(());
        };
        // Another process may have removed the box between the list and this
        // call; a box the runtime no longer lists counts as removed.
        if self.list()?.iter().all(|box_| box_.id != name) {
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

    fn list(&self) -> io::Result<Vec<BoxInfo>> {
        Ok(containers()?
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
                    stopped: container.status.state == "stopped"
                        && container.status.started_date.is_some(),
                }
            })
            .collect())
    }

    fn list_disks(&self) -> io::Result<Vec<DiskInfo>> {
        let json = output(&["container", "volume", "list", "--format", "json"])?;
        let volumes: Vec<ListedVolume> = parse_json("container volume list", &json)?;
        // Another root's volumes, and any pinfold did not make, are no disks.
        let prefix = volume("")?;
        let mut disks = Vec::new();
        for listed in volumes {
            let configuration = listed.configuration;
            let Some(name) = configuration.name.strip_prefix(&prefix) else {
                continue;
            };
            disks.push(DiskInfo {
                allocated: allocated_bytes(Path::new(&configuration.source))?,
                name: name.to_string(),
                size: configuration.size,
            });
        }
        Ok(disks)
    }

    fn create_disk(&self, name: &str, bytes: u64) -> io::Result<()> {
        let size = format!("{}G", bytes >> 30);
        output(&["container", "volume", "create", "-s", &size, &volume(name)?]).map(drop)
    }

    fn remove_disk(&self, name: &str) -> io::Result<()> {
        output(&["container", "volume", "rm", &volume(name)?]).map(drop)
    }

    /// The guest synced; what the volume keeps through a host crash is
    /// the runtime's.
    fn sync_disk(&self, _name: &str) -> io::Result<()> {
        Ok(())
    }

    fn exec(
        &self,
        name: &str,
        init: &Path,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus> {
        self.exec_command(name, init, tty, workdir, argv)
            .status()
            .map_err(|error| spawn_error("container", error))
    }

    fn exec_command(
        &self,
        name: &str,
        init: &Path,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> std::process::Command {
        let mut command = std::process::Command::new("container");
        command.args(["exec", "-i"]);
        if tty {
            // Keep the host's terminal identity: the runtime otherwise
            // reports its own `TERM`.
            command.arg("-t");
            for variable in ["TERM", "COLORTERM"] {
                if let Some(value) = std::env::var_os(variable) {
                    guest_env(&mut command, variable, &value);
                }
            }
        } else {
            // Without a TTY the runtime's exec gets its own process group, so
            // a terminal signal reaches pinfold and not the exec.
            command.process_group(0);
        }
        if let Some(dir) = workdir {
            command.arg("--workdir").arg(dir);
        }
        command
            .arg(name)
            .arg(init)
            .args(["init", "exec", "--"])
            .args(argv);
        command
    }

    /// A transient helper owns only the new disks, under the caller's
    /// name claim. It runs without network and with only CAP_CHOWN.
    fn own_disks(
        &self,
        names: &[&str],
        plan: &Plan,
        init: &Path,
        generation: &str,
    ) -> io::Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let name = &plan.name;
        let image = plan.image.as_deref().expect("up resolved the image");
        let labels = BTreeMap::from([
            (
                clean::OWNER_LABEL.to_string(),
                plan.labels[clean::OWNER_LABEL].clone(),
            ),
            (
                ownership::GENERATION_LABEL.to_string(),
                generation.to_string(),
            ),
        ]);
        let owner = format!("{}:{}", nix::unistd::getuid(), nix::unistd::getgid());
        let mut command = std::process::Command::new("container");
        command
            .args(["run", "--progress", "none", "--network", "none"])
            .args(["--read-only", "--cap-drop", "ALL", "--cap-add", "CAP_CHOWN"])
            .args(["--name", name])
            .args(["--user", "0:0", "--mount"])
            .arg(init_mount(init));
        for (key, value) in &labels {
            command.arg("--label").arg(format!("{key}={value}"));
        }
        let mut setup: Vec<OsString> = vec!["init".into(), "setup".into()];
        for (index, name) in names.iter().enumerate() {
            let guest = format!("/pinfold-disk/{index}");
            command.arg("--mount").arg(format!(
                "type=volume,source={},target={guest}",
                volume(name)?
            ));
            setup.extend(["--own".into(), owner.clone().into(), guest.into()]);
        }
        let result = command
            .arg("--entrypoint")
            .arg(init)
            .arg(image)
            .args(setup)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|error| spawn_error("container", error));
        // The box reuses the helper's name, so the helper must be gone first.
        // Apple reports a container `running` for a moment after `run` returns,
        // and `rm` refuses it then; `run --rm` removes it just as late. `stop`
        // returns once it has stopped, after which `rm` succeeds.
        let stopped = output(&["container", "stop", name]);
        let removed = output(&["container", "rm", name]);
        let result = result?;
        if !result.status.success() {
            // Name a failed stop or removal too, so a leftover helper shows.
            let leftover = match (&stopped, &removed) {
                (Ok(_), Ok(_)) => String::new(),
                _ => format!(
                    "; removing helper {name}: {:?}, {:?}",
                    stopped.err(),
                    removed.err()
                ),
            };
            return Err(io::Error::other(format!(
                "give new disks to the box user: {}{leftover}",
                String::from_utf8_lossy(&result.stderr).trim()
            )));
        }
        stopped?;
        removed.map(drop)
    }

    fn describe(&self) -> &'static str {
        "Apple container (one VM per box)"
    }

    /// The CLI's version, as the CLI reports it.
    fn version(&self) -> io::Result<String> {
        let stdout = output(&["container", "--version"])?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_string())
    }
}

/// Wait, bounded, until `list` reports box `name` running. Apple records
/// `.running` only after the box's first process starts, so `ready` can
/// arrive first; an exec is refused in that gap.
fn wait_until_running(name: &str) -> io::Result<BoxInfo> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = Apple.list()?.into_iter().find(|box_| box_.id == name);
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
fn make_proxy_connectable(name: &str) -> io::Result<()> {
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

/// Run `init receive GUEST` in box `name` as the box user with `stream`
/// written to its stdin: a new disk's `from` copy, or with `missing`, the
/// profile's seeds, each written only when absent.
fn copy_in(
    name: &str,
    init: &Path,
    guest: &Path,
    missing: bool,
    stream: impl FnOnce(&mut dyn io::Write) -> io::Result<()>,
) -> io::Result<()> {
    let mut child = std::process::Command::new("container")
        .args(["exec", "-i", name])
        .arg(init)
        .args(["init", "receive"])
        .args(missing.then_some("--missing"))
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
    /// Set once the box has run. A box still booting lists `stopped`
    /// without it.
    #[serde(default, rename = "startedDate")]
    started_date: Option<String>,
}

/// Every box `container list` reports, running or not.
fn containers() -> io::Result<Vec<ListedContainer>> {
    let json = output(&["container", "list", "--all", "--format", "json"])?;
    parse_json("container list", &json)
}

/// The listed container with this id, if the runtime has one.
fn container(name: &str) -> io::Result<Option<ListedContainer>> {
    Ok(containers()?.into_iter().find(|entry| entry.id == name))
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
}

/// One image entry and its recorded labels.
fn image_info(image: ListedImage) -> io::Result<ImageInfo> {
    // Build labels are OCI image config labels; a locally built image also
    // carries name annotations on its index descriptor.
    let mut labels = image
        .configuration
        .descriptor
        .map(|descriptor| descriptor.annotations)
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
    })
}

impl ImageService for Apple {
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
        let Some(builder) = container("buildkit")? else {
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
        let Some(builder) = container("buildkit")? else {
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

    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>> {
        crate::core::image::check_build_allow(request.allow)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let cache_flags: &[&str] = if request.cache { &[] } else { &["--no-cache"] };
        crate::core::image::service::build("container", cache_flags, request)
    }

    fn program(&self) -> &'static str {
        "container"
    }
}
