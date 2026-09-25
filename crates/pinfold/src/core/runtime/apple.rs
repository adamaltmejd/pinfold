//! The Apple `container` runtime adapter.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::Path;

use serde::Deserialize;
use tokio::process::Child;

use crate::core::plan::Plan;
use crate::core::runtime::{
    BoxInfo, BoxStat, BuildCache, BuildRequest, ImageIdentity, ImageInfo, MemoryStat, PidsStat,
    Preflight, Runtime, inspect, output, parse_json, run, spawn_error,
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
        );
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
                }
            })
            .collect())
    }

    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        let json = output(&["container", "image", "list", "--format", "json"])?;
        let images: Vec<ListedImage> = parse_json("container image list", &json)?;
        Ok(images
            .into_iter()
            .map(|image| image_info(image).0)
            .collect())
    }

    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageIdentity, String>> {
        // `image inspect` prints the same entries as `image list`.
        Ok(
            inspect::<ListedImage>("container", reference)?.map(|image| {
                let (image, digest) = image_info(image);
                ImageIdentity {
                    id: image.id,
                    labels: image.labels,
                    digest,
                }
            }),
        )
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

/// The limits the runtime reports for a box.
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
    parse_json("container list", &json)
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
    /// `sha256:<hex>`.
    digest: Option<String>,
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

/// One image entry, with its descriptor's digest.
fn image_info(image: ListedImage) -> (ImageInfo, Option<String>) {
    // Build labels are OCI image config labels; a locally built image also
    // carries name annotations on its index descriptor.
    let (mut labels, digest) = image
        .configuration
        .descriptor
        .map(|descriptor| (descriptor.annotations, descriptor.digest))
        .unwrap_or_default();
    for variant in image.variants {
        if let Some(config) = variant.config.and_then(|config| config.config) {
            labels.extend(config.labels);
        }
    }
    let info = ImageInfo {
        id: image.id,
        reference: image.configuration.name,
        labels,
    };
    (info, digest)
}
