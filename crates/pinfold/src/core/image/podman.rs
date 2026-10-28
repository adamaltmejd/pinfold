//! Rootless podman, which builds Linux images and runs no box.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use serde::Deserialize;

use super::service::inspect;
use super::{BuildRequest, ImageInfo, ImageService};
use crate::core::runtime::{output, parse_json};

/// The label podman puts on the intermediate images of a cached build, so
/// `clean` prunes only pinfold's build cache.
const LAYER_LABEL: &str = "dev.pinfold.layer";

pub fn preflight() -> io::Result<()> {
    // A misconfigured host fails closed here, before any box starts.
    let json = output(&["podman", "info", "--format", "json"])?;
    let info: Info = parse_json("podman info", &json)?;
    crate::core::runtime::require_version("podman", &info.version.version, [6, 1, 4])?;
    if !info.host.security.rootless {
        return Err(io::Error::other(
            "rootful podman is not supported; pinfold requires rootless podman",
        ));
    }
    Ok(())
}

pub struct Podman;

impl ImageService for Podman {
    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        let json = output(&["podman", "image", "list", "--format", "json"])?;
        let images: Vec<ListedImage> = parse_json("podman image list", &json)?;
        let mut infos = Vec::new();
        // One entry per name, so Maintenance can remove every tag of an
        // old image; a dangling image is removed by its id. Podman emits
        // one JSON entry per tag and repeats the image's whole `Names` list
        // on each, so keep only the first entry per image.
        let mut seen = BTreeSet::new();
        for image in images {
            if !seen.insert(image.id.clone()) {
                continue;
            }
            let references = if image.names.is_empty() {
                vec![image.id.clone()]
            } else {
                image.names
            };
            for reference in references {
                infos.push(ImageInfo {
                    id: image.id.clone(),
                    reference,
                    labels: image.labels.clone(),
                    digest: None,
                });
            }
        }
        Ok(infos)
    }

    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>> {
        Ok(
            inspect::<InspectedImage>("podman", reference)?.map(|image| ImageInfo {
                id: image.id,
                reference: reference.to_string(),
                labels: image.labels,
                digest: Some(image.digest).filter(|digest| !digest.is_empty()),
            }),
        )
    }

    fn remove_image(&self, reference: &str) -> io::Result<()> {
        // `image rm` also collects the layers no image references.
        output(&["podman", "image", "rm", reference]).map(|_| ())
    }

    fn purge_build_cache(&self) -> io::Result<()> {
        // `image prune` removes only dangling images and repeats until none
        // is left, so an intermediate goes once nothing builds on it, and an
        // intermediate under a kept image stays with it.
        let filter = format!("label={LAYER_LABEL}");
        output(&["podman", "image", "prune", "--force", "--filter", &filter]).map(|_| ())
    }

    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>> {
        let label = format!("{LAYER_LABEL}=true");
        let cache_flags: &[&str] = if request.cache {
            &["--layer-label", &label]
        } else {
            // Every step reruns and no intermediate image is left behind.
            &["--layers=false"]
        };
        super::service::build("podman", cache_flags, request)
    }

    fn build_cache(&self) -> &'static str {
        "the dev.pinfold.layer intermediate images no image builds on"
    }

    fn program(&self) -> &'static str {
        "podman"
    }
}

/// What preflight reads from `podman info`.
#[derive(Debug, Deserialize)]
struct Info {
    host: InfoHost,
    version: InfoVersion,
}

#[derive(Debug, Deserialize)]
struct InfoVersion {
    #[serde(rename = "Version")]
    version: String,
}

#[derive(Debug, Deserialize)]
struct InfoHost {
    security: InfoSecurity,
}

#[derive(Debug, Deserialize)]
struct InfoSecurity {
    rootless: bool,
}

/// One `podman image list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedImage {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Names", default, deserialize_with = "empty_default")]
    names: Vec<String>,
    #[serde(rename = "Labels", default, deserialize_with = "empty_default")]
    labels: BTreeMap<String, String>,
}

/// One `podman image inspect` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct InspectedImage {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Digest", default)]
    digest: String,
    #[serde(rename = "Labels", default, deserialize_with = "empty_default")]
    labels: BTreeMap<String, String>,
}

/// A field podman marshals as `null` when it is empty.
fn empty_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
