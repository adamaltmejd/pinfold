//! Container runtimes: one per OS, behind a trait.

pub mod apple;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use tokio::process::Child;

use crate::core::plan::Plan;

/// The container runtime for this OS.
pub fn runtime() -> io::Result<&'static dyn Runtime> {
    if cfg!(target_os = "macos") {
        Ok(&apple::Apple)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no container runtime for this OS",
        ))
    }
}

/// One box the runtime knows about, running or not.
#[derive(Debug, Clone)]
pub struct BoxInfo {
    pub id: String,
    pub labels: BTreeMap<String, String>,
}

/// One image build: an empty context holding only a Containerfile, the
/// names to tag the result with, and the labels to record on it.
pub struct BuildRequest<'a> {
    /// The build context; it holds only the Containerfile.
    pub context: &'a Path,
    /// The Containerfile inside the context.
    pub containerfile: &'a Path,
    /// The image names to tag the built image with.
    pub tags: &'a [String],
    /// Labels to put on the image, for Maintenance to find it by.
    pub labels: &'a BTreeMap<String, String>,
}

/// One OS's container runtime. Command lines are built as data.
pub trait Runtime: Sync {
    /// Start the attached `container run` process that owns the box.
    fn up(&self, plan: &Plan, init: &Path) -> io::Result<Child>;

    /// Stop and remove the box.
    fn down(&self, name: &str) -> io::Result<()>;

    /// Run a command in a running box with inherited stdio.
    fn exec(
        &self,
        name: &str,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus>;

    /// List every box, running or not.
    fn list(&self) -> io::Result<Vec<BoxInfo>>;

    /// Build an image from [`BuildRequest`].
    fn build(&self, request: &BuildRequest) -> io::Result<()>;

    /// Pull `reference` and return the digest it resolved to. `None` when
    /// the reference does not resolve, for example `scratch`.
    fn image_digest(&self, reference: &str) -> io::Result<Option<String>>;
}

/// The path a host path appears at inside the box. The identity on Unix.
pub fn guest_path(host: &Path) -> PathBuf {
    host.to_path_buf()
}
