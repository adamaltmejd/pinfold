//! Image operations, selected independently of box execution. Each target
//! provides the same native functions:
//!
//! - `list_images`: every image, with the labels recorded on it.
//! - `resolve_image`: resolve a local image reference or content identity.
//!   Never pulls. `None` when the runtime has no such image.
//! - `remove_image`: remove one image by reference, and any layers no image
//!   references.
//! - `purge_build_cache`: remove unused build cache without interrupting
//!   builds: Apple's unused BuildKit records or Linux's exclusively claimed
//!   idle builder cache. Apple then stops the builder when no build is
//!   running.
//! - `build_cache_bytes`: allocated build-cache storage, including its
//!   backing filesystem.
//! - `build`: build an image from [`BuildRequest`]. The inner `Err` is the
//!   build's output, stdout and stderr in order, when the build ran and
//!   failed.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub use super::linux::{
    build, build_cache_bytes, list_images, purge_build_cache, remove_image, resolve_image,
};
#[cfg(not(target_os = "linux"))]
pub use crate::core::runtime::apple::{
    build, build_cache_bytes, list_images, purge_build_cache, remove_image, resolve_image,
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
}

/// One image build: a context, a Containerfile, the names to tag the result
/// with, and the labels to record on it.
pub struct BuildRequest {
    /// The build context.
    pub context: PathBuf,
    /// The Containerfile, inside the context or not.
    pub containerfile: PathBuf,
    /// The image names to tag the built image with.
    pub tags: Vec<String>,
    /// Labels to put on the image, for Maintenance to find it by.
    pub labels: BTreeMap<String, String>,
    /// Whether the runtime may reuse its layer cache. Without it every step
    /// reruns.
    pub cache: bool,
    /// Additional explicit build-egress hosts; independent of agent egress.
    /// Only Linux builds read it: `check_build_allow` refuses it on macOS.
    #[cfg_attr(not(target_os = "linux"), expect(dead_code))]
    pub allow: Vec<String>,
    /// One managed-build argument; absent for caller builds.
    pub build_arg: Option<(String, String)>,
}
