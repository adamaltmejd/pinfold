//! Image operations, selected independently of box execution.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

#[cfg(not(target_os = "linux"))]
use crate::core::runtime::apple;
/// The image service for this OS.
pub fn service() -> &'static dyn ImageService {
    #[cfg(target_os = "linux")]
    return &super::linux::Linux;
    #[cfg(not(target_os = "linux"))]
    return &apple::Apple;
}

/// One image the runtime knows about.
#[derive(Debug, Clone)]
pub struct ImageInfo {
    /// The image's content digest; references to one image share it.
    pub id: String,
    /// A reference that names the image, for removal.
    pub reference: String,
    /// The labels recorded on the image.
    pub labels: BTreeMap<String, String>,
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
    /// Additional explicit build-egress hosts; independent of agent egress.
    /// Only Linux builds read it: `check_build_allow` refuses it on macOS.
    #[cfg_attr(not(target_os = "linux"), expect(dead_code))]
    pub allow: &'a [String],
    /// One managed-build argument; absent for caller builds.
    pub build_arg: Option<(&'a str, &'a str)>,
}

/// Native image build, resolution, inventory and cache operations.
pub trait ImageService: Sync {
    /// List every image, with the labels recorded on it.
    fn list_images(&self) -> io::Result<Vec<ImageInfo>>;

    /// Resolve a local image reference or content identity. Never pulls.
    /// The inner `Err` describes an unavailable image.
    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>>;

    /// Remove one image by reference, and any layers no image references.
    fn remove_image(&self, reference: &str) -> io::Result<()>;

    /// Remove unused build cache without interrupting builds: Apple's unused
    /// BuildKit records or Linux's exclusively claimed idle builder cache.
    fn purge_build_cache(&self) -> io::Result<()>;

    /// What [`ImageService::purge_build_cache`] removes, named for `pinfold
    /// clean` and `doctor`.
    fn build_cache(&self) -> &'static str;

    /// Allocated build-cache storage, including its backing filesystem.
    fn build_cache_bytes(&self) -> io::Result<u64>;

    /// Build an image from [`BuildRequest`]. The inner `Err` is the build's
    /// output, stdout and stderr in order, when the build ran and failed.
    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>>;

    /// The build implementation's name for diagnostics.
    fn program(&self) -> &'static str;
}
