//! Container runtimes: one per OS, behind a trait.

pub mod apple;

use std::io;
use std::path::{Path, PathBuf};

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

/// One OS's container runtime. Command lines are built as data.
pub trait Runtime: Sync {
    /// Start the attached `container run` process that owns the box.
    fn up(&self, plan: &Plan, init: &Path) -> io::Result<Child>;

    /// Stop and remove the box.
    fn down(&self, name: &str) -> io::Result<()>;
}

/// The path a host path appears at inside the box. The identity on Unix.
pub fn guest_path(host: &Path) -> PathBuf {
    host.to_path_buf()
}
