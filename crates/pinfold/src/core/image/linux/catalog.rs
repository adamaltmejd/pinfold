//! Verified OCI receipt and the Linux image store. Guest bytes supply no host paths.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::image::ImageInfo;
use crate::core::{hex, ownership, sha256_hex};
use crate::dirs;

// Candidate resource policy; final admission requires the boundary evidence.
const HEADER_MAX: usize = 8 << 10;
const JSON_MAX: u64 = 1 << 20;
const OBJECT_MAX: usize = 4096;
const PAYLOAD_MAX: u64 = 512 << 20;
const INDEX: &str = "application/vnd.oci.image.index.v1+json";
const MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const CONFIG: &str = "application/vnd.oci.image.config.v1+json";
const IMAGE_OWNER: &str = ".pinfold-image";

fn check_cancel(cancel: Option<&AtomicBool>) -> io::Result<()> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "build cancelled",
        ));
    }
    Ok(())
}

fn invalid(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail.into())
}

pub(super) fn digest_hex(digest: &str) -> io::Result<&str> {
    digest
        .strip_prefix("sha256:")
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| invalid("image-output: invalid sha256 digest"))
}

fn new_file(path: &Path) -> io::Result<File> {
    File::options()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
}

fn directory(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(path)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    digest: String,
    size: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Success {}

fn frame_header(stream: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut prefix = [0u8; 5];
    stream.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix[1..].try_into().unwrap()) as usize;
    if length > HEADER_MAX {
        return Err(invalid("image-output: header limit"));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok((prefix[0], bytes))
}

fn success(stream: &mut impl Read, bytes: &[u8]) -> io::Result<()> {
    serde_json::from_slice::<Success>(bytes).map_err(invalid_json)?;
    let mut trailing = [0];
    if stream.read(&mut trailing)? != 0 {
        return Err(invalid("image-output: trailing bytes"));
    }
    Ok(())
}

/// Native frontend discovery returns one bounded JSON receipt, then success.
pub fn receive_discovery(stream: &mut impl Read) -> io::Result<Vec<Resolution>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Discovery {
        resolutions: Vec<Resolution>,
    }
    let (kind, bytes) = frame_header(stream)?;
    let header: Header = serde_json::from_slice(&bytes).map_err(invalid_json)?;
    if kind != b'I' || header.size > JSON_MAX {
        return Err(invalid("image-output: invalid discovery frame"));
    }
    let mut payload = Vec::with_capacity(header.size as usize);
    copy_checked(stream, &mut payload, header.size, &header.digest, None)?;
    let (kind, bytes) = frame_header(stream)?;
    if kind != b'E' {
        return Err(invalid("image-output: missing discovery success"));
    }
    success(stream, &bytes)?;
    let discovery: Discovery = serde_json::from_slice(&payload).map_err(invalid_json)?;
    if discovery.resolutions.len() > OBJECT_MAX {
        return Err(invalid("image-output: discovery resolution limit"));
    }
    for resolution in &discovery.resolutions {
        for value in [
            Some(resolution.requested.as_str()),
            Some(resolution.effective.as_str()),
            resolution.store.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if value.is_empty()
                || value.len() > HEADER_MAX
                || !value.is_ascii()
                || value
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            {
                return Err(invalid("image-output: invalid discovery resolution"));
            }
        }
        digest_hex(&resolution.digest)?;
    }
    Ok(discovery.resolutions)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub(super) requested: String,
    pub(super) effective: String,
    pub(super) digest: String,
    pub(super) store: Option<String>,
}

/// A converter emits one `D` frame with digest/size and opaque root bytes,
/// then `E {}` and EOF. No guest filesystem is read or mounted on the host.
pub fn receive_root(stream: &mut impl Read, path: &Path) -> io::Result<(String, u64)> {
    let mut file = new_file(path)?;
    let received = (|| {
        let (kind, bytes) = frame_header(stream)?;
        let header: Header = serde_json::from_slice(&bytes).map_err(invalid_json)?;
        if kind != b'D' || header.size == 0 || header.size > PAYLOAD_MAX {
            return Err(invalid("image-output: root frame or size limit"));
        }
        copy_checked(stream, &mut file, header.size, &header.digest, None)?;
        let (kind, bytes) = frame_header(stream)?;
        if kind != b'E' {
            return Err(invalid("image-output: missing root success"));
        }
        success(stream, &bytes)?;
        file.sync_all()?;
        Ok((header.digest, header.size))
    })();
    if received.is_err() {
        let _ = fs::remove_file(path);
    }
    received
}

/// An admitted single linux/amd64 runnable manifest and its complete OCI layout.
pub struct Verified {
    pub id: String,
    pub labels: BTreeMap<String, String>,
    pub env: Vec<(String, String)>,
    pub directory: PathBuf,
    pub sources: Vec<Source>,
}

/// Native discovery's resolved image inputs, independent of runnable identity.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub(super) uri: String,
    pub(super) digest: String,
}

/// Frames: kind byte (`I`, `B`, `E`), big-endian u32 JSON-header length,
/// header, then the declared raw payload. I/B headers contain only digest
/// and size. E is `{}` without payload, followed by EOF. I must be first.
pub fn receive(
    stream: &mut impl Read,
    staging: &Path,
    expected: &BTreeMap<String, String>,
) -> io::Result<Verified> {
    fs::DirBuilder::new().mode(0o700).create(staging)?;
    let received = (|| {
        directory(&staging.join("blobs/sha256"))?;
        let mut objects = BTreeMap::new();
        let mut total = 0u64;
        let mut indexed = false;
        loop {
            let (kind, bytes) = frame_header(stream)?;
            if kind == b'E' {
                success(stream, &bytes)?;
                break;
            }
            let header: Header = serde_json::from_slice(&bytes).map_err(invalid_json)?;
            let digest = digest_hex(&header.digest)?;
            total = total
                .checked_add(header.size)
                .filter(|total| *total <= PAYLOAD_MAX)
                .ok_or_else(|| invalid("image-output: payload limit"))?;
            let path = match kind {
                b'I' if !indexed && objects.is_empty() => {
                    if header.size > JSON_MAX {
                        return Err(invalid("image-output: index limit"));
                    }
                    indexed = true;
                    staging.join("index.json")
                }
                b'B' if indexed => {
                    if objects.len() >= OBJECT_MAX
                        || objects.insert(header.digest.clone(), header.size).is_some()
                    {
                        return Err(invalid("image-output: duplicate object or object limit"));
                    }
                    staging.join("blobs/sha256").join(digest)
                }
                _ => return Err(invalid("image-output: unexpected frame")),
            };
            let mut file = new_file(&path)?;
            copy_checked(stream, &mut file, header.size, &header.digest, None)?;
            file.sync_all()?;
        }
        if !indexed {
            return Err(invalid("image-output: missing index"));
        }
        let verified = validate(staging, &objects, expected, true, None)?;
        let mut layout = new_file(&staging.join("oci-layout"))?;
        layout.write_all(b"{\"imageLayoutVersion\":\"1.0.0\"}")?;
        layout.sync_all()?;
        File::open(staging.join("blobs/sha256"))?.sync_all()?;
        File::open(staging.join("blobs"))?.sync_all()?;
        File::open(staging)?.sync_all()?;
        Ok(verified)
    })();
    if received.is_err() {
        let _ = fs::remove_dir_all(staging);
    }
    received
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    invalid(format!("image-output: invalid JSON: {error}"))
}

fn copy_checked(
    source: &mut impl Read,
    destination: &mut impl Write,
    size: u64,
    digest: &str,
    cancel: Option<&AtomicBool>,
) -> io::Result<()> {
    digest_hex(digest)?;
    let mut hash = Sha256::new();
    let mut remaining = size;
    let mut buffer = [0u8; 64 << 10];
    while remaining != 0 {
        check_cancel(cancel)?;
        let take = remaining.min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..take])?;
        destination.write_all(&buffer[..take])?;
        hash.update(&buffer[..take]);
        remaining -= take as u64;
    }
    if hex(&hash.finalize()) != digest_hex(digest)? {
        return Err(invalid("image-output: digest mismatch"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    media_type: String,
    digest: String,
    size: u64,
    #[serde(default)]
    urls: Vec<String>,
    platform: Option<Platform>,
    artifact_type: Option<String>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
    data: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Platform {
    architecture: String,
    os: String,
    variant: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    schema_version: u32,
    media_type: Option<String>,
    manifests: Vec<Descriptor>,
    subject: Option<serde_json::Value>,
    artifact_type: Option<String>,
}

fn index_shape(index: &Index, manifests: usize) -> io::Result<()> {
    if index.schema_version != 2
        || index.media_type.as_deref() != Some(INDEX)
        || index.manifests.len() != manifests
        || index.subject.is_some()
        || index.artifact_type.is_some()
    {
        return Err(invalid(format!(
            "image-output: invalid OCI index; expected {manifests} descriptors, received {}",
            index.manifests.len()
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    media_type: Option<String>,
    config: Descriptor,
    layers: Option<Vec<Descriptor>>,
    subject: Option<Descriptor>,
    artifact_type: Option<String>,
}

#[derive(Deserialize)]
struct ImageConfig {
    architecture: String,
    os: String,
    variant: Option<String>,
    config: Option<ContainerConfig>,
    rootfs: Rootfs,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ContainerConfig {
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    env: Vec<String>,
}

#[derive(Deserialize)]
struct Rootfs {
    #[serde(rename = "type")]
    kind: String,
    diff_ids: Option<Vec<String>>,
}

fn native(platform: &Platform) -> bool {
    platform.os == "linux"
        && platform.architecture == "amd64"
        && platform.variant.as_deref().is_none_or(str::is_empty)
}

fn descriptor(
    descriptor: &Descriptor,
    objects: &BTreeMap<String, u64>,
    referenced: &mut BTreeSet<String>,
) -> io::Result<()> {
    digest_hex(&descriptor.digest)?;
    if descriptor.data.is_some()
        || !descriptor.urls.is_empty()
        || descriptor.artifact_type.is_some()
        || descriptor
            .annotations
            .contains_key("vnd.docker.reference.type")
        || descriptor
            .annotations
            .contains_key("vnd.docker.reference.digest")
        || descriptor.platform.as_ref().is_some_and(|p| !native(p))
        || objects.get(&descriptor.digest) != Some(&descriptor.size)
    {
        return Err(invalid("image-output: invalid local descriptor"));
    }
    referenced.insert(descriptor.digest.clone());
    Ok(())
}

fn json_object<T: serde::de::DeserializeOwned>(
    directory: &Path,
    descriptor: &Descriptor,
    json_bytes: &mut u64,
) -> io::Result<T> {
    *json_bytes = json_bytes
        .checked_add(descriptor.size)
        .filter(|size| *size <= JSON_MAX)
        .ok_or_else(|| invalid("image-output: JSON limit"))?;
    let path = directory
        .join("blobs/sha256")
        .join(digest_hex(&descriptor.digest)?);
    serde_json::from_slice(&fs::read(path)?).map_err(invalid_json)
}

fn validate(
    directory: &Path,
    objects: &BTreeMap<String, u64>,
    expected: &BTreeMap<String, String>,
    normalize: bool,
    cancel: Option<&AtomicBool>,
) -> io::Result<Verified> {
    check_cancel(cancel)?;
    let index_metadata = fs::symlink_metadata(directory.join("index.json"))?;
    if !index_metadata.is_file() || index_metadata.len() > JSON_MAX {
        return Err(invalid("image-output: index limit"));
    }
    let index_bytes = fs::read(directory.join("index.json"))?;
    let mut json_bytes = index_bytes.len() as u64;
    let index: Index = serde_json::from_slice(&index_bytes).map_err(invalid_json)?;
    index_shape(&index, 1)?;
    let mut referenced = BTreeSet::new();
    let runnable = &index.manifests[0];
    // Single-platform OCI export may omit descriptor.platform. The config
    // below always supplies the required linux/amd64 platform check.
    if runnable.media_type != MANIFEST {
        return Err(invalid("image-output: unsupported manifest media"));
    }
    descriptor(runnable, objects, &mut referenced)?;
    let manifest: Manifest = json_object(directory, runnable, &mut json_bytes)?;
    let layers = manifest.layers.unwrap_or_default();
    if manifest.schema_version != 2
        || manifest
            .media_type
            .as_deref()
            .is_some_and(|media| media != MANIFEST)
        || manifest.subject.is_some()
        || manifest.artifact_type.is_some()
        || layers.len() > OBJECT_MAX - 2
        || manifest.config.media_type != CONFIG
    {
        return Err(invalid("image-output: unsupported runnable manifest"));
    }
    descriptor(&manifest.config, objects, &mut referenced)?;
    let config: ImageConfig = json_object(directory, &manifest.config, &mut json_bytes)?;
    // The pinned exporter represents a scratch image's empty layers and diff
    // IDs as null. Their counts must still agree before admitting any image.
    let diff_ids = config.rootfs.diff_ids.unwrap_or_default();
    if !native(&Platform {
        architecture: config.architecture,
        os: config.os,
        variant: config.variant,
    }) || config.rootfs.kind != "layers"
        || diff_ids.len() != layers.len()
    {
        return Err(invalid("image-output: invalid image configuration"));
    }
    for (layer, diff_id) in layers.iter().zip(&diff_ids) {
        check_cancel(cancel)?;
        if !matches!(
            layer.media_type.as_str(),
            "application/vnd.oci.image.layer.v1.tar"
                | "application/vnd.oci.image.layer.v1.tar+gzip"
                | "application/vnd.oci.image.layer.v1.tar+zstd"
        ) {
            return Err(invalid("image-output: unsupported layer media"));
        }
        digest_hex(diff_id)?;
        descriptor(layer, objects, &mut referenced)?;
    }
    if referenced.len() != objects.len() {
        return Err(invalid("image-output: unreferenced object"));
    }
    let config = config.config.unwrap_or_default();
    if expected
        .iter()
        .any(|(key, value)| config.labels.get(key) != Some(value))
    {
        return Err(invalid("image-output: expected label mismatch"));
    }
    let env = config
        .env
        .into_iter()
        .map(|entry| {
            let (name, value) = entry
                .split_once('=')
                .filter(|(name, value)| {
                    !name.is_empty() && !name.contains('\0') && !value.contains('\0')
                })
                .ok_or_else(|| invalid("image-output: invalid image environment"))?;
            Ok((name.to_string(), value.to_string()))
        })
        .collect::<io::Result<Vec<_>>>()?;
    if normalize {
        // Exporter index annotations vary independently of runnable identity.
        let canonical = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": INDEX,
            "manifests": [{
                "mediaType": MANIFEST, "digest": runnable.digest, "size": runnable.size,
                "platform": { "os": "linux", "architecture": "amd64" }
            }]
        }))?;
        check_cancel(cancel)?;
        let mut index_file = File::options()
            .write(true)
            .truncate(true)
            .open(directory.join("index.json"))?;
        index_file.write_all(&canonical)?;
        index_file.sync_all()?;
    }
    Ok(Verified {
        id: runnable.digest.clone(),
        labels: config.labels,
        env,
        directory: directory.to_path_buf(),
        sources: Vec::new(),
    })
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    references: BTreeMap<String, String>,
    images: BTreeMap<String, Image>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    labels: BTreeMap<String, String>,
    env: Vec<(String, String)>,
    recipe: String,
    root_sha: String,
    root_bytes: u64,
    sources: Vec<Source>,
}

fn store() -> io::Result<PathBuf> {
    Ok(dirs::cache_dir()?.join("images"))
}

fn key(suffix: &str) -> io::Result<String> {
    let canonical = dirs::resolve_host_path(&store()?)?;
    Ok(format!(
        "images:{}:{suffix}",
        sha256_hex(canonical.as_os_str().as_bytes())
    ))
}

fn catalog_lock() -> io::Result<Flock<File>> {
    Flock::lock(
        ownership::open_lock(&key("catalog")?)?,
        FlockArg::LockExclusive,
    )
    .map_err(|(_, error)| error.into())
}

fn shared_lease(id: &str) -> io::Result<Flock<File>> {
    digest_hex(id)?;
    Flock::lock(ownership::open_lock(&key(id)?)?, FlockArg::LockShared)
        .map_err(|(_, error)| error.into())
}

/// Whether an image has a live reader. The catalog lock prevents new snapshots
/// while a removal uses this answer.
pub fn leased(id: &str) -> io::Result<bool> {
    digest_hex(id)?;
    match Flock::lock(
        ownership::open_lock(&key(id)?)?,
        FlockArg::LockExclusiveNonblock,
    ) {
        Ok(lock) => {
            drop(lock);
            Ok(false)
        }
        Err((_, nix::errno::Errno::EWOULDBLOCK)) => Ok(true),
        Err((_, error)) => Err(error.into()),
    }
}

fn read_catalog() -> io::Result<Catalog> {
    let bytes = match fs::read(store()?.join("catalog.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Catalog::default()),
        Err(error) => return Err(error),
    };
    let catalog: Catalog = serde_json::from_slice(&bytes).map_err(invalid_json)?;
    for (id, image) in &catalog.images {
        digest_hex(id)?;
        digest_hex(&image.root_sha)?;
        for input in &image.sources {
            digest_hex(&input.digest)?;
        }
    }
    if catalog
        .references
        .values()
        .any(|id| !catalog.images.contains_key(id))
    {
        return Err(invalid("image-store: dangling reference"));
    }
    Ok(catalog)
}

fn image_dir(id: &str) -> io::Result<PathBuf> {
    Ok(store()?.join(digest_hex(id)?))
}

fn root_path(id: &str, image: &Image) -> io::Result<PathBuf> {
    let derivation = sha256_hex(format!("{id}\n{}\n", image.recipe));
    Ok(image_dir(id)?.join(format!(
        "{derivation}-{}.ext4",
        digest_hex(&image.root_sha)?
    )))
}

fn info(id: &str, reference: &str, image: &Image) -> ImageInfo {
    ImageInfo {
        id: id.to_string(),
        reference: reference.to_string(),
        labels: image.labels.clone(),
    }
}

/// A read-only atomic generation snapshot. No cache or lock is created.
pub fn list() -> io::Result<Vec<ImageInfo>> {
    let catalog = read_catalog()?;
    Ok(catalog
        .references
        .iter()
        .map(|(reference, id)| info(id, reference, &catalog.images[id]))
        .collect())
}

/// Resolve tags or an admitted runnable manifest digest locally.
pub fn resolve(reference: &str) -> io::Result<Result<ImageInfo, String>> {
    let catalog = read_catalog()?;
    let id = catalog
        .references
        .get(reference)
        .map(String::as_str)
        .unwrap_or(reference);
    Ok(match catalog.images.get(id) {
        Some(image) => Ok(info(id, reference, image)),
        None => Err(
            "image-missing: rebuild this image with pinfold build or pinfold image build"
                .to_string(),
        ),
    })
}

/// Resolve and protect the same generation before starting a box.
pub fn resolve_leased(reference: &str) -> io::Result<Result<(ImageInfo, Flock<File>), String>> {
    let _lock = catalog_lock()?;
    let catalog = read_catalog()?;
    let id = catalog
        .references
        .get(reference)
        .map(String::as_str)
        .unwrap_or(reference);
    Ok(match catalog.images.get(id) {
        Some(image) => Ok((info(id, reference, image), shared_lease(id)?)),
        None => Err(
            "image-missing: rebuild this image with pinfold build or pinfold image build"
                .to_string(),
        ),
    })
}

/// The completed opaque root and image environment. This never converts.
pub fn root(id: &str) -> io::Result<(PathBuf, Vec<(String, String)>)> {
    let catalog = read_catalog()?;
    let image = catalog
        .images
        .get(id)
        .ok_or_else(|| invalid("image-missing: rebuild image"))?;
    Ok((verify_root(id, image)?, image.env.clone()))
}

fn verify_root(id: &str, image: &Image) -> io::Result<PathBuf> {
    let path = root_path(id, image)?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() != image.root_bytes {
        return Err(invalid(
            "image-root: completed root missing or damaged; rebuild image",
        ));
    }
    let mut file = File::open(&path)?;
    copy_checked(
        &mut file,
        &mut io::sink(),
        image.root_bytes,
        &image.root_sha,
        None,
    )?;
    Ok(path)
}

/// Reuse only a complete verified representation of this manifest and exact
/// converter recipe. Missing or damaged output requires a fresh conversion.
pub fn complete_root(id: &str, recipe: &str) -> io::Result<Option<(PathBuf, String, u64)>> {
    let catalog = read_catalog()?;
    let Some(image) = catalog
        .images
        .get(id)
        .filter(|image| image.recipe == recipe)
    else {
        return Ok(None);
    };
    match verify_root(id, image) {
        Ok(path) => Ok(Some((path, image.root_sha.clone(), image.root_bytes))),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidData
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Once the catalog rename succeeds, the build is visible. Later cancellation,
/// audit failure or directory-sync failure cannot report an unchanged failure.
pub enum Publication {
    Committed { durability_error: Option<String> },
}

fn commit(catalog: &Catalog, cancel: Option<&AtomicBool>) -> io::Result<Publication> {
    let directory = store()?;
    let bytes = serde_json::to_vec(catalog)?;
    check_cancel(cancel)?;
    let path = directory.join("catalog.json");
    let staging = path.with_extension(format!("tmp-{}", std::process::id()));
    crate::core::absent_ok(fs::remove_file(&staging))?;
    let written = (|| {
        let mut file = new_file(&staging)?;
        for chunk in bytes.chunks(64 << 10) {
            check_cancel(cancel)?;
            file.write_all(chunk)?;
        }
        file.sync_all()?;
        // Cancellation must still preserve the prior generation after the
        // potentially slow staging write and sync, immediately before rename.
        check_cancel(cancel)?;
        fs::rename(&staging, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(staging);
    }
    written?;
    // The rename above is the commit point. A later fsync error affects crash
    // durability, not whether the new generation is already visible to callers.
    let durability_error = File::open(directory)
        .and_then(|directory| directory.sync_all())
        .err()
        .map(|error| error.to_string());
    Ok(Publication::Committed { durability_error })
}

fn verify_layout(
    path: &Path,
    expected: &BTreeMap<String, String>,
    cancel: Option<&AtomicBool>,
) -> io::Result<Verified> {
    check_cancel(cancel)?;
    for directory in [
        path.to_path_buf(),
        path.join("blobs"),
        path.join("blobs/sha256"),
    ] {
        if !fs::symlink_metadata(directory)?.is_dir() {
            return Err(invalid("image-store: invalid OCI layout directory"));
        }
    }
    let layout_path = path.join("oci-layout");
    let layout_metadata = fs::symlink_metadata(&layout_path)?;
    if !layout_metadata.is_file()
        || layout_metadata.len() > HEADER_MAX as u64
        || fs::read(layout_path)? != b"{\"imageLayoutVersion\":\"1.0.0\"}"
    {
        return Err(invalid("image-store: invalid OCI layout version"));
    }
    let mut objects = BTreeMap::new();
    let mut total = 0u64;
    for entry in fs::read_dir(path.join("blobs/sha256"))? {
        check_cancel(cancel)?;
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("image-store: invalid object name"))?;
        let digest = format!("sha256:{name}");
        digest_hex(&digest)?;
        let metadata = fs::symlink_metadata(entry.path())?;
        total = total
            .checked_add(metadata.len())
            .filter(|total| *total <= PAYLOAD_MAX)
            .ok_or_else(|| invalid("image-store: payload limit"))?;
        if !metadata.is_file() || objects.len() >= OBJECT_MAX {
            return Err(invalid("image-store: invalid object or object limit"));
        }
        copy_checked(
            &mut File::open(entry.path())?,
            &mut io::sink(),
            metadata.len(),
            &digest,
            cancel,
        )?;
        objects.insert(digest, metadata.len());
    }
    validate(path, &objects, expected, false, cancel)
}

/// Install immutable verified OCI bytes and a completed conversion before
/// publishing every build ref in one atomic catalog generation.
#[allow(clippy::too_many_arguments)]
pub fn publish(
    verified: Verified,
    root_source: &Path,
    root_sha: &str,
    root_bytes: u64,
    recipe: &str,
    tags: &[String],
    cancel: &AtomicBool,
) -> io::Result<Publication> {
    check_cancel(Some(cancel))?;
    digest_hex(&verified.id)?;
    digest_hex(root_sha)?;
    if root_bytes > PAYLOAD_MAX
        || tags.is_empty()
        || tags.iter().any(|tag| {
            !tag.starts_with("localhost/pinfold/") || tag.chars().any(char::is_whitespace)
        })
    {
        return Err(invalid("image-store: invalid publication"));
    }
    let _lock = catalog_lock()?;
    check_cancel(Some(cancel))?;
    directory(&store()?)?;
    let mut catalog = read_catalog()?;
    let image = Image {
        labels: verified.labels,
        env: verified.env,
        recipe: recipe.to_string(),
        root_sha: root_sha.to_string(),
        root_bytes,
        sources: verified.sources,
    };
    let target = image_dir(&verified.id)?;
    claim_image_directory(&target, &verified.id)?;
    let layout = target.join("oci");
    if !layout.exists() {
        // The receipt's layout contains only host-created fixed names.
        fs::rename(&verified.directory, &layout)?;
        File::open(&target)?.sync_all()?;
    } else {
        let existing = verify_layout(&layout, &image.labels, Some(cancel))?;
        if existing.id != verified.id
            || existing.labels != image.labels
            || existing.env != image.env
        {
            return Err(invalid("image-store: existing OCI identity mismatch"));
        }
        if verified.directory != layout {
            fs::remove_dir_all(&verified.directory)?;
        }
    }
    let root = root_path(&verified.id, &image)?;
    let complete = (|| {
        let mut existing = File::open(&root)?;
        if existing.metadata()?.len() != root_bytes {
            return Err(invalid("image-store: existing root size mismatch"));
        }
        copy_checked(
            &mut existing,
            &mut io::sink(),
            root_bytes,
            root_sha,
            Some(cancel),
        )
    })();
    if let Err(error) = complete {
        if !matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::InvalidData
        ) {
            return Err(error);
        }
        let staging = target.join(format!(".root-{}", std::process::id()));
        crate::core::absent_ok(fs::remove_file(&staging))?;
        let result = (|| {
            let mut source = File::open(root_source)?;
            if source.metadata()?.len() != root_bytes {
                return Err(invalid("image-output: root size mismatch"));
            }
            let mut destination = new_file(&staging)?;
            copy_checked(
                &mut source,
                &mut destination,
                root_bytes,
                root_sha,
                Some(cancel),
            )?;
            destination.sync_all()?;
            check_cancel(Some(cancel))?;
            fs::rename(&staging, &root)?;
            File::open(&target)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }
        result?;
    }
    for tag in tags {
        catalog.references.insert(tag.clone(), verified.id.clone());
    }
    catalog.images.insert(verified.id, image);
    commit(&catalog, Some(cancel))
}

pub struct BaseMetadata {
    pub id: String,
    pub references: Vec<String>,
    pub labels: BTreeMap<String, String>,
    pub manifest: Vec<u8>,
    pub config_digest: String,
    pub config: Vec<u8>,
}

fn metadata_blob(layout: &Path, digest: &str, cancel: &AtomicBool) -> io::Result<Vec<u8>> {
    check_cancel(Some(cancel))?;
    let mut file = File::options()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(layout.join("blobs/sha256").join(digest_hex(digest)?))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > JSON_MAX {
        return Err(invalid("image-store: invalid base metadata"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    copy_checked(&mut file, &mut bytes, metadata.len(), digest, Some(cancel))?;
    Ok(bytes)
}

/// Snapshot only immutable manifest/config metadata. Native frontend discovery
/// needs no layers, and unrelated images acquire no lifetime lease.
pub fn base_metadata(cancel: &AtomicBool) -> io::Result<Vec<BaseMetadata>> {
    check_cancel(Some(cancel))?;
    if !store()?.join("catalog.json").exists() {
        return Ok(Vec::new());
    }
    let _lock = catalog_lock()?;
    let catalog = read_catalog()?;
    let mut references: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (reference, id) in &catalog.references {
        references.entry(id).or_default().push(reference.clone());
    }
    let mut total = 0u64;
    let mut snapshots = Vec::new();
    for (id, references) in references {
        let layout = image_dir(id)?.join("oci");
        let manifest = metadata_blob(&layout, id, cancel)?;
        let parsed: Manifest = serde_json::from_slice(&manifest).map_err(invalid_json)?;
        if parsed.schema_version != 2
            || parsed.media_type.as_deref() != Some(MANIFEST)
            || parsed.subject.is_some()
            || parsed.artifact_type.is_some()
            || parsed.config.media_type != CONFIG
            || !parsed.config.urls.is_empty()
            || parsed.config.data.is_some()
            || parsed.config.artifact_type.is_some()
        {
            return Err(invalid("image-store: invalid base manifest"));
        }
        let config = metadata_blob(&layout, &parsed.config.digest, cancel)?;
        if config.len() as u64 != parsed.config.size {
            return Err(invalid("image-store: invalid base configuration size"));
        }
        total = total
            .checked_add(manifest.len() as u64 + config.len() as u64)
            .filter(|total| *total <= PAYLOAD_MAX)
            .ok_or_else(|| invalid("resource: local base metadata size"))?;
        snapshots.push(BaseMetadata {
            id: id.to_string(),
            references,
            labels: catalog.images[id].labels.clone(),
            manifest,
            config_digest: parsed.config.digest,
            config,
        });
    }
    Ok(snapshots)
}

/// Only identities selected by native discovery acquire leases and full-layout
/// verification. A removal during discovery refuses rather than retargeting.
pub fn lease_bases(
    ids: &BTreeSet<String>,
    cancel: &AtomicBool,
) -> io::Result<Vec<(String, PathBuf, Flock<File>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let _lock = catalog_lock()?;
    let catalog = read_catalog()?;
    let mut bases = Vec::new();
    for id in ids {
        check_cancel(Some(cancel))?;
        let image = catalog
            .images
            .get(id)
            .ok_or_else(|| invalid("image-base-changed: selected local base was removed"))?;
        let guard = shared_lease(id)?;
        let layout = image_dir(id)?.join("oci");
        let verified = verify_layout(&layout, &image.labels, Some(cancel))?;
        if verified.id != *id || verified.labels != image.labels || verified.env != image.env {
            return Err(invalid("image-store: local base OCI identity mismatch"));
        }
        bases.push((id.clone(), layout, guard));
    }
    Ok(bases)
}

/// Remove a reference; a last reference is retained while a box or build owns
/// its image. Catalog replacement precedes removal of immutable bytes.
pub fn remove(reference: &str) -> io::Result<()> {
    let _lock = catalog_lock()?;
    let mut catalog = read_catalog()?;
    let Some(id) = catalog.references.get(reference).cloned() else {
        return Ok(());
    };
    let last = catalog
        .references
        .values()
        .filter(|candidate| **candidate == id)
        .count()
        == 1;
    if last && leased(&id)? {
        return Err(io::Error::new(io::ErrorKind::WouldBlock, "image-in-use"));
    }
    catalog.references.remove(reference);
    if last {
        catalog.images.remove(&id);
    }
    let Publication::Committed { durability_error } = commit(&catalog, None)?;
    if let Some(error) = durability_error {
        eprintln!("pinfold: image-store: committed removal durability: {error}");
    }
    if last {
        crate::core::absent_ok(fs::remove_dir_all(image_dir(&id)?))?;
    }
    Ok(())
}

fn owner_bytes(id: &str) -> Vec<u8> {
    format!("pinfold-image-v1\n{id}\n").into_bytes()
}

fn owned_image_directory(path: &Path, id: &str) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Ok(false);
    }
    let marker = path.join(IMAGE_OWNER);
    let expected = owner_bytes(id);
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() != expected.len() as u64
    {
        return Ok(false);
    }
    let mut bytes = Vec::new();
    File::options()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(marker)?
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes == expected)
}

fn claim_image_directory(path: &Path, id: &str) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {
            // Write ownership before any OCI/root bytes. An interruption before
            // this small marker completes leaves an unowned directory intact.
            let mut marker = new_file(&path.join(IMAGE_OWNER))?;
            marker.write_all(&owner_bytes(id))?;
            marker.sync_all()?;
            File::open(path)?.sync_all()?;
            File::open(path.parent().expect("image directory has a parent"))?.sync_all()
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if owned_image_directory(path, id)? {
                Ok(())
            } else {
                Err(invalid(
                    "image-store: output directory has no matching ownership marker",
                ))
            }
        }
        Err(error) => Err(error),
    }
}

/// Collect marked immutable output left before publication or after the last
/// ref's removal. The catalog lock excludes publishers and new image leases.
pub fn collect_unreferenced() -> io::Result<()> {
    let directory = store()?;
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(invalid("image-store: invalid store directory")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    let _lock = catalog_lock()?;
    let catalog = read_catalog()?;
    let mut removed = false;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let id = format!("sha256:{name}");
        if digest_hex(&id).is_err()
            || catalog.images.contains_key(&id)
            || !owned_image_directory(&entry.path(), &id)?
            || leased(&id)?
        {
            continue;
        }
        fs::remove_dir_all(entry.path())?;
        removed = true;
    }
    if removed {
        File::open(directory)?.sync_all()?;
    }
    Ok(())
}
