//! Linux builds and completed image representations, owned by pinfold.

pub(crate) mod catalog;
mod job;

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::{BuildRequest, ImageInfo, ImageService};
use crate::core::{artifacts, ownership, sha256_hex};
use crate::dirs;

pub struct Linux;

pub fn cache_name() -> io::Result<String> {
    let root = dirs::resolve_host_path(&dirs::cache_dir()?)?;
    Ok(format!(
        "builder-{}",
        &sha256_hex(root.as_os_str().as_encoded_bytes())[..32]
    ))
}

pub fn cache_dir() -> io::Result<std::path::PathBuf> {
    Ok(dirs::cache_dir()?
        .join("builder")
        .join(artifacts::builder_identity()))
}

impl ImageService for Linux {
    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        catalog::list()
    }
    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>> {
        catalog::resolve(reference)
    }
    fn remove_image(&self, reference: &str) -> io::Result<()> {
        catalog::remove(reference)
    }
    fn purge_build_cache(&self) -> io::Result<()> {
        // Immutable output uses the catalog lock and image leases. A live
        // builder prevents only removal of its separate writable cache.
        catalog::collect_unreferenced()?;
        let Some(_idle) = ownership::try_name_lock(&cache_name()?)? else {
            return Ok(());
        };
        job::writable_snapshot(&cache_dir()?.join("job"))?;
        match fs::remove_dir_all(cache_dir()?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
    fn build_cache(&self) -> &'static str {
        "the idle isolated builder cache"
    }
    fn build_cache_bytes(&self) -> io::Result<Option<u64>> {
        Ok(Some(allocated_bytes(&cache_dir()?)?))
    }
    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>> {
        job::build(request)
    }
    fn program(&self) -> &'static str {
        "BuildKit"
    }
}

fn allocated_bytes(path: &Path) -> io::Result<u64> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if metadata.is_file() {
        return Ok(metadata.blocks() * 512);
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut bytes = 0;
    for entry in fs::read_dir(path)? {
        bytes += allocated_bytes(&entry?.path())?;
    }
    Ok(bytes)
}
