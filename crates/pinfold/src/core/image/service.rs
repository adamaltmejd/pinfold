//! Image operations, selected independently of box execution.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read};
use std::path::Path;
use std::process::Stdio;

use serde::de::DeserializeOwned;

#[cfg(not(target_os = "linux"))]
use crate::core::runtime::apple;
use crate::core::runtime::{captured, parse_json, spawn_error};

const BUILD_LOG_BYTES: usize = 64 * 1024;

/// The image service for this OS.
pub fn service() -> &'static dyn ImageService {
    #[cfg(target_os = "linux")]
    return &super::podman::Podman;
    #[cfg(not(target_os = "linux"))]
    return &apple::Apple;
}

/// The runtime's native Linux image architecture.
pub const TARGET_ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "amd64"
};

/// One image the runtime knows about.
#[derive(Debug, Clone)]
pub struct ImageInfo {
    /// The image's content digest; references to one image share it.
    pub id: String,
    /// A reference that names the image, for removal.
    pub reference: String,
    /// The labels recorded on the image.
    pub labels: BTreeMap<String, String>,
    /// The digest the runtime records for the image, `sha256:<hex>`. Only
    /// [`ImageService::resolve_image`] fills it on podman.
    pub digest: Option<String>,
}

/// One image build: a context, a Containerfile, the names to tag the result
/// with, and the labels to record on it.
pub struct BuildRequest<'a> {
    /// The build context.
    pub context: &'a Path,
    /// The Containerfile, inside the context or not.
    pub containerfile: &'a Path,
    /// The image names to tag the built image with.
    pub tags: &'a [String],
    /// Labels to put on the image, for Maintenance to find it by.
    pub labels: &'a BTreeMap<String, String>,
    /// Whether the runtime may reuse its layer cache. Without it every step
    /// reruns.
    pub cache: bool,
}

/// Native image build, resolution, inventory and cache operations.
pub trait ImageService: Sync {
    /// List every image, with the labels recorded on it.
    fn list_images(&self) -> io::Result<Vec<ImageInfo>>;

    /// Resolve `reference` to a local image with the runtime's own inspect,
    /// so every spelling the runtime resolves is accepted. Never pulls. The
    /// inner `Err` is the runtime's message when it cannot resolve it.
    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>>;

    /// Remove one image by reference, and any layers no image references.
    fn remove_image(&self, reference: &str) -> io::Result<()>;

    /// Remove unused build cache without interrupting builds. Apple: the
    /// builder's unused records. podman: the intermediate images pinfold's cached builds
    /// left that no image builds on.
    fn purge_build_cache(&self) -> io::Result<()>;

    /// What [`ImageService::purge_build_cache`] removes, named for `pinfold
    /// clean` and `doctor`.
    fn build_cache(&self) -> &'static str;

    /// Allocated build-cache storage, including its backing filesystem.
    /// None when the runtime cannot measure it separately.
    fn build_cache_bytes(&self) -> io::Result<Option<u64>> {
        Ok(None)
    }

    /// Build an image from [`BuildRequest`]. The inner `Err` is the build's
    /// output, stdout and stderr in order, when the build ran and failed.
    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>>;

    /// The image CLI's program name: `container` on macOS, `podman` on
    /// Linux.
    fn program(&self) -> &'static str;
}

/// Run `<program> image inspect <reference>` and return its first entry. The
/// inner `Err` is the runtime's stderr when it cannot resolve the reference.
pub(crate) fn inspect<T: DeserializeOwned>(
    program: &str,
    reference: &str,
) -> io::Result<Result<T, String>> {
    let json = match captured(&[program, "image", "inspect", reference])? {
        Ok(json) => json,
        Err(stderr) => return Ok(Err(stderr)),
    };
    let what = format!("{program} image inspect");
    let images: Vec<T> = parse_json(&what, &json)?;
    images
        .into_iter()
        .next()
        .map(Ok)
        .ok_or_else(|| io::Error::other(format!("{what} returned no image")))
}

/// Build `request` with `program`; `cache_flags` is how that runtime spells
/// the request's cache choice. stdout and stderr are captured through one
/// pipe, so the output keeps its order. The inner `Err` is that output when
/// the build fails.
pub(crate) fn build(
    program: &str,
    cache_flags: &[&str],
    request: &BuildRequest,
) -> io::Result<Result<(), String>> {
    let (reader, writer) = io::pipe()?;
    let mut command = std::process::Command::new(program);
    if cfg!(target_os = "macos") {
        // Apple includes color settings in the shared builder's identity
        // (GitHub #77). Every pinfold caller must use the same settings.
        command.env_remove("NO_COLOR").env_remove("BUILDKIT_COLORS");
    }
    command
        .arg("build")
        .args(["--build-arg", &format!("TARGETARCH={TARGET_ARCH}")])
        .args(cache_flags)
        .arg("--file")
        .arg(request.containerfile);
    for tag in request.tags {
        command.arg("--tag").arg(tag);
    }
    for (key, value) in request.labels {
        command.arg("--label").arg(format!("{key}={value}"));
    }
    command
        .arg(request.context)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    let child = command.spawn();
    // The command holds the pipe's write ends, and the read below ends only
    // once every write end is closed.
    drop(command);
    let mut child = child.map_err(|error| spawn_error(program, error))?;
    let mut tail = VecDeque::new();
    let mut newlines = 0;
    for byte in io::BufReader::new(reader).bytes() {
        let byte = match byte {
            Ok(byte) => byte,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        tail.push_back(byte);
        newlines += usize::from(byte == b'\n');
        while tail.len() > BUILD_LOG_BYTES
            || newlines + usize::from(tail.back() != Some(&b'\n')) > 40
        {
            newlines -= usize::from(tail.pop_front() == Some(b'\n'));
        }
    }
    if child.wait()?.success() {
        Ok(Ok(()))
    } else {
        let bytes: Vec<u8> = tail.into_iter().collect();
        let mut output = String::from_utf8_lossy(&bytes).into_owned();
        // Replacement characters can expand malformed runtime output.
        let start = output.ceil_char_boundary(output.len().saturating_sub(BUILD_LOG_BYTES));
        output.drain(..start);
        Ok(Err(output))
    }
}
