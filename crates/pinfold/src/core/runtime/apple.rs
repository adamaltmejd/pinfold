//! The Apple `container` runtime adapter.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use serde::Deserialize;
use tokio::process::Child;

use crate::core::plan::Plan;
use crate::core::runtime::{
    BoxInfo, BoxStat, BuildCache, BuildRequest, ImageIdentity, ImageInfo, MemoryStat, PidsStat,
    Preflight, Runtime, bind, output, run, spawn_error, up_command, user,
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
        proxy_socket: Option<&Path>,
        _preflight: &Preflight,
    ) -> io::Result<Child> {
        let argv = up_argv(plan, init, proxy_socket);
        let mut command = up_command(&argv, &mut plan.env.iter(), plan.egress.is_some());
        if let Some(socket) = proxy_socket {
            command.env("SSH_AUTH_SOCK", socket);
        }
        command
            .spawn()
            .map_err(|error| spawn_error("container", error))
    }

    fn make_proxy_connectable(&self, name: &str) -> io::Result<()> {
        // The one transient root exec, which makes the forwarded socket
        // connectable by the box user.
        run(&[
            "container",
            "exec",
            "--user",
            "0:0",
            name,
            "chmod",
            "666",
            GUEST_PROXY_SOCKET,
        ])
    }

    fn preflight(&self) -> io::Result<Preflight> {
        // A missing `container` binary is refused before `up` creates any
        // state.
        self.version().map(|_| Preflight::default())
    }

    fn down(&self, name: &str) -> io::Result<()> {
        // stderr is captured: an absent box fails `rm` with the runtime's
        // not-found text, which is noise once the box is gone.
        let Err(error) = output(&["container", "rm", "-f", name]) else {
            return Ok(());
        };
        // Another process may have removed the box between the list and this
        // call; a box the runtime no longer lists counts as removed.
        if self.list()?.iter().all(|box_| box_.id != name) {
            return Ok(());
        }
        Err(error)
    }

    fn exec(
        &self,
        name: &str,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus> {
        super::exec("container", name, tty, workdir, argv)
    }

    /// The limits Apple reports, and null for what the VM cannot answer
    /// (kills, memory use, pids use).
    fn stat(&self, name: &str) -> io::Result<BoxStat> {
        let resources = containers()?
            .into_iter()
            .find(|container| container.id == name)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no box named {name:?}"))
            })?
            .configuration
            .resources;
        Ok(BoxStat {
            name: name.to_string(),
            oom_kills: None,
            memory: MemoryStat {
                current: None,
                peak: None,
                limit: resources.memory,
            },
            pids: PidsStat {
                current: None,
                limit: None,
            },
        })
    }

    fn list(&self) -> io::Result<Vec<BoxInfo>> {
        Ok(containers()?
            .into_iter()
            .map(|container| {
                let ListedConfiguration {
                    labels,
                    image,
                    created,
                    ..
                } = container.configuration;
                let digest = image.descriptor.digest;
                BoxInfo {
                    id: container.id,
                    labels,
                    image_id: digest
                        .strip_prefix("sha256:")
                        .unwrap_or(&digest)
                        .to_string(),
                    image_ref: image.reference,
                    created,
                    running: container.status.state == "running",
                }
            })
            .collect())
    }

    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        let json = output(&["container", "image", "list", "--format", "json"])?;
        parse_images(&json)
    }

    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageIdentity, String>> {
        let output = std::process::Command::new("container")
            .args(["image", "inspect", reference])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| spawn_error("container", error))?;
        if !output.status.success() {
            return Ok(Err(String::from_utf8_lossy(&output.stderr)
                .trim()
                .to_string()));
        }
        // `image inspect` prints the same entries as `image list`.
        let image = parse_images(&output.stdout)?
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::other("container image inspect returned no image"))?;
        Ok(Ok(ImageIdentity {
            id: image.id,
            labels: image.labels,
        }))
    }

    fn remove_image(&self, reference: &str) -> io::Result<()> {
        // `image delete` also collects the layers no image references.
        output(&["container", "image", "delete", reference]).map(|_| ())
    }

    fn purge_build_cache(&self) -> io::Result<()> {
        // The builder container holds the build cache. `--force` removes a
        // running builder too; a missing builder is not an error.
        run(&["container", "builder", "delete", "--force"])
    }

    fn build_cache(&self) -> io::Result<BuildCache> {
        Ok(BuildCache::Named("the runtime's builder container"))
    }

    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>> {
        // Without the cache every step reruns, so a rebuild picks up base
        // updates instead of replaying a cached `RUN` layer.
        let cache_flags: &[&str] = if request.cache { &[] } else { &["--no-cache"] };
        super::build("container", cache_flags, request)
    }

    fn image_digest(&self, reference: &str) -> io::Result<Option<String>> {
        super::image_digest("container", reference, parse_digest)
    }

    fn name(&self) -> &'static str {
        "Apple container"
    }

    fn isolation(&self) -> &'static str {
        "one VM per box"
    }

    fn version(&self) -> io::Result<String> {
        super::cli_version("container")
    }
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
struct ListedConfiguration {
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    image: ListedBoxImage,
    /// ISO 8601, which is RFC 3339.
    #[serde(default, rename = "creationDate")]
    created: String,
    #[serde(default)]
    resources: ListedResources,
}

/// The image a box runs, as the runtime records it.
#[derive(Default, Deserialize)]
struct ListedBoxImage {
    #[serde(default)]
    reference: String,
    #[serde(default)]
    descriptor: ListedBoxImageDescriptor,
}

#[derive(Default, Deserialize)]
struct ListedBoxImageDescriptor {
    /// `sha256:<hex>`; the hex is the image list's `id`.
    #[serde(default)]
    digest: String,
}

/// The limits the runtime reports for a box. The VM exposes no kill or use
/// counters, so `stat` answers null for those.
#[derive(Default, Deserialize)]
struct ListedResources {
    #[serde(default, rename = "memoryInBytes")]
    memory: Option<u64>,
}

#[derive(Default, Deserialize)]
struct ListedStatus {
    #[serde(default)]
    state: String,
}

/// Every box `container list` reports, running or not.
fn containers() -> io::Result<Vec<ListedContainer>> {
    let json = output(&["container", "list", "--all", "--format", "json"])?;
    serde_json::from_slice(&json)
        .map_err(|error| io::Error::other(format!("container list returned invalid JSON: {error}")))
}

/// One `container image list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedImage {
    id: String,
    configuration: ListedImageConfiguration,
    #[serde(default)]
    variants: Vec<ListedImageVariant>,
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

#[derive(Deserialize)]
struct ListedImageVariant {
    config: Option<ListedImageConfig>,
}

#[derive(Deserialize)]
struct ListedImageConfig {
    config: Option<ListedImageLabels>,
}

#[derive(Deserialize)]
struct ListedImageLabels {
    #[serde(default, rename = "Labels")]
    labels: BTreeMap<String, String>,
}

fn parse_images(json: &[u8]) -> io::Result<Vec<ImageInfo>> {
    let images: Vec<ListedImage> = serde_json::from_slice(json).map_err(|error| {
        io::Error::other(format!(
            "container image list returned invalid JSON: {error}"
        ))
    })?;
    Ok(images
        .into_iter()
        .map(|image| {
            let ListedImage {
                id,
                configuration,
                variants,
            } = image;
            // Build labels are OCI image config labels; a locally built
            // image also carries name annotations on its index descriptor.
            let mut labels = configuration
                .descriptor
                .map(|descriptor| descriptor.annotations)
                .unwrap_or_default();
            for variant in &variants {
                if let Some(config) = variant
                    .config
                    .as_ref()
                    .and_then(|config| config.config.as_ref())
                {
                    labels.extend(config.labels.clone());
                }
            }
            ImageInfo {
                id,
                reference: configuration.name,
                labels,
            }
        })
        .collect())
}

/// One `container image inspect` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct InspectedImage {
    configuration: InspectedConfiguration,
}

#[derive(Deserialize)]
struct InspectedConfiguration {
    descriptor: InspectedDescriptor,
}

#[derive(Deserialize)]
struct InspectedDescriptor {
    digest: String,
}

fn parse_digest(json: &[u8]) -> io::Result<Option<String>> {
    let images: Vec<InspectedImage> = serde_json::from_slice(json).map_err(|error| {
        io::Error::other(format!(
            "container image inspect returned invalid JSON: {error}"
        ))
    })?;
    Ok(images
        .into_iter()
        .next()
        .map(|image| image.configuration.descriptor.digest))
}

/// The `container run` argv for a box, as data.
///
/// `init` is a host path; it and its directory are mounted read-only at the
/// same path, so it is also the path PID 1 runs. When `proxy_socket` is set,
/// the box carries the socket in over `--ssh` and init relays to the guest
/// path as its argument.
fn up_argv(plan: &Plan, init: &Path, proxy_socket: Option<&Path>) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        "container".into(),
        "run".into(),
        "-i".into(),
        "--progress".into(),
        "none".into(),
        "--name".into(),
        plan.name.clone().into(),
        "--network".into(),
        "none".into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--read-only".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--user".into(),
        user(plan),
    ];
    if proxy_socket.is_some() {
        // Off-label: the runtime treats the socket as an SSH agent and
        // forwards it to GUEST_PROXY_SOCKET.
        argv.push("--ssh".into());
    }
    if let Some(cpus) = plan.cpus {
        argv.push("--cpus".into());
        argv.push(cpus.to_string().into());
    }
    if let Some(memory) = &plan.memory {
        argv.push("--memory".into());
        argv.push(memory.into());
    }
    for (key, value) in &plan.labels {
        argv.push("--label".into());
        argv.push(format!("{key}={value}").into());
    }
    for name in plan.env.keys() {
        // Names only: `container` reads the value from our environment.
        argv.push("--env".into());
        argv.push(name.into());
    }
    // Always applied: Node's fetch reads the proxy variables only with this
    // set.
    argv.push("--env".into());
    argv.push("NODE_USE_ENV_PROXY".into());
    if plan.egress.is_some() {
        argv.push("--env".into());
        argv.push("HTTPS_PROXY".into());
        argv.push("--env".into());
        argv.push("http_proxy".into());
    }
    for mount in &plan.mounts {
        argv.push("--mount".into());
        argv.push(bind(&mount.host, &mount.guest, mount.readonly));
    }

    // The init binary comes from the host and runs as PID 1. box::up refuses
    // an init without a directory, so the parent is a real directory here.
    let init_dir = init.parent().unwrap_or(Path::new("/"));
    argv.push("--mount".into());
    argv.push(bind(init_dir, init_dir, true));
    argv.push("--entrypoint".into());
    argv.push(init.into());
    argv.push(
        plan.image
            .clone()
            .expect("box up resolves the profile's image before the runtime runs")
            .into(),
    );
    argv.push("init".into());
    if proxy_socket.is_some() {
        argv.push(GUEST_PROXY_SOCKET.into());
    }
    argv
}
