//! Container runtimes: one per OS, behind a trait.

pub mod apple;
pub mod podman;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use tokio::process::Child;

use crate::core::plan::Plan;

/// The container runtime for this OS.
pub fn runtime() -> io::Result<&'static dyn Runtime> {
    if cfg!(target_os = "macos") {
        Ok(&apple::Apple)
    } else if cfg!(target_os = "linux") {
        Ok(&podman::Podman)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no container runtime for this OS",
        ))
    }
}

/// A runtime CLI's `--version` output, trimmed. `doctor` uses it.
fn cli_version(program: &str) -> io::Result<String> {
    let output = std::process::Command::new(program)
        .arg("--version")
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{program} --version: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// One box the runtime knows about, running or not.
#[derive(Debug, Clone)]
pub struct BoxInfo {
    pub id: String,
    pub labels: BTreeMap<String, String>,
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
    /// Start the attached `container run` process that owns the box. When
    /// `proxy_socket` is set, carry that host unix socket into the box.
    fn up(&self, plan: &Plan, init: &Path, proxy_socket: Option<&Path>) -> io::Result<Child>;

    /// Refuse a host the runtime cannot serve, before `up` creates any box
    /// state. The default is a runtime whose binary `up` checks itself.
    fn preflight(&self) -> io::Result<()> {
        Ok(())
    }

    /// Make the carried proxy socket connectable by the box user. Apple only:
    /// the forwarded socket arrives root-owned and mode 000.
    fn make_proxy_connectable(&self, name: &str) -> io::Result<()>;

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

    /// List every image, with the labels recorded on it.
    fn list_images(&self) -> io::Result<Vec<ImageInfo>>;

    /// Remove one image by reference, and any layers no image references.
    fn remove_image(&self, reference: &str) -> io::Result<()>;

    /// Remove the runtime's build cache. Apple: the builder container and
    /// its layers. The default is a runtime whose builds keep no cache; an
    /// adapter with one overrides this.
    fn purge_build_cache(&self) -> io::Result<()> {
        Ok(())
    }

    /// A phrase naming what the runtime's build cache holds, printed by
    /// `pinfold clean`. The default matches [`Runtime::purge_build_cache`]:
    /// a runtime whose builds keep no cache.
    fn build_cache_description(&self) -> &'static str {
        "none"
    }

    /// Build an image from [`BuildRequest`].
    fn build(&self, request: &BuildRequest) -> io::Result<()>;

    /// Pull `reference` and return the digest it resolved to. `None` when
    /// the reference does not resolve, for example `scratch`.
    fn image_digest(&self, reference: &str) -> io::Result<Option<String>>;

    /// The runtime's name, for `doctor`.
    fn name(&self) -> &'static str;

    /// The isolation the runtime gives each box, for `doctor`.
    fn isolation(&self) -> &'static str;

    /// The runtime CLI's version, as the CLI reports it.
    fn version(&self) -> io::Result<String>;
}

/// The path a host path appears at inside the box. The identity on Unix.
pub fn guest_path(host: &Path) -> PathBuf {
    host.to_path_buf()
}

/// The content digest of the image `reference` names, from the runtime's
/// image list. `None` when no image carries the reference. Unlike
/// [`Runtime::image_digest`], this never pulls: the reference is local.
pub fn local_image_id(runtime: &dyn Runtime, reference: &str) -> io::Result<Option<String>> {
    Ok(runtime
        .list_images()?
        .into_iter()
        .find(|image| image.reference == reference)
        .map(|image| image.id))
}

/// The uid:gid PID 1 and all work run as: the spec's, else the host user's.
pub(crate) fn user(plan: &Plan) -> OsString {
    match plan.user {
        Some(user) => {
            let (uid, gid) = (user.uid, user.gid);
            format!("{uid}:{gid}").into()
        }
        None => {
            let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
            format!("{uid}:{gid}").into()
        }
    }
}

/// A `type=bind` mount value, as both runtimes spell it.
pub(crate) fn bind(host: &Path, guest: &Path, readonly: bool) -> OsString {
    let mut value = OsString::from("type=bind,source=");
    value.push(host);
    value.push(",target=");
    value.push(guest);
    if readonly {
        value.push(",readonly");
    }
    value
}
