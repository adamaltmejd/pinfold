//! Image builds. One path builds every family: a profile's or a project's
//! image from its Containerfile alone, and a caller's image from the
//! caller's own context.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::r#box::RefusalReason;
use crate::core::clean;
use crate::core::plan;
use crate::core::profile;
use crate::core::runtime::{BuildRequest, Runtime, output, runtime};
use crate::dirs;

/// The Containerfile bytes a managed image was built from.
pub const CONTAINERFILE_LABEL: &str = "dev.pinfold.containerfile-sha256";

/// Where a build's files come from.
pub enum Context<'a> {
    /// A fresh, empty context holding only these Containerfile bytes, so
    /// the Containerfile is the build's only input.
    Alone(&'a [u8]),
    /// A directory the caller vouches for, and a Containerfile that may lie
    /// outside it.
    Dir {
        dir: &'a Path,
        containerfile: &'a Path,
    },
}

/// One build of one source.
pub struct Build<'a> {
    /// The family label naming the source on the image, which retention
    /// groups by; a caller's source lives only in its tags, so its label is
    /// `None`.
    pub label: Option<&'static str>,
    /// The source: a profile name, a project id or a caller image name.
    pub source: &'a str,
    pub context: Context<'a>,
    /// Labels beyond the family and base labels.
    pub labels: BTreeMap<String, String>,
    /// Whether the runtime may reuse its layer cache.
    pub cache: bool,
}

/// The image one build made.
pub struct Built {
    /// The unique ref, `pinfold/<family>-<source>:<build>`.
    pub reference: String,
    /// The stable ref, `pinfold/<family>-<source>:latest`, now naming the
    /// same image.
    pub latest: String,
    /// The image's id in the runtime, what a box's `ready` and `list`
    /// report as `image.id`; `None` when the runtime cannot resolve it.
    pub id: Option<String>,
    /// Every label the build put on the image.
    pub labels: BTreeMap<String, String>,
}

/// Build one source's image, tag it uniquely and move the stable ref to it,
/// then run retention for the source's images. The inner `Err` is the
/// build's output when the build ran and failed.
pub fn build(runtime: &dyn Runtime, build: Build) -> io::Result<Result<Built, String>> {
    let id = build_id();
    let mut labels = build.labels;
    // The runtime copies the base image's labels onto the new image, so
    // every family label goes on every image: its own with its source, the
    // other empty. A caller has no label: both go on empty and its name
    // lives only in its tags. Retention then never counts this image as
    // another family's, whatever it builds on.
    for family in clean::FAMILY_LABELS {
        let value = if build.label == Some(family) {
            build.source
        } else {
            ""
        };
        labels.insert(family.to_string(), value.to_string());
    }
    // Likewise the base: the caller set the resolved digest, or this makes
    // it empty, never an inherited copy.
    labels.entry(clean::BASE_LABEL.to_string()).or_default();
    // Caller contexts have other inputs, so they have no source fingerprint.
    // Clear an inherited fingerprint rather than claiming it is their own.
    labels.insert(
        CONTAINERFILE_LABEL.to_string(),
        match &build.context {
            Context::Alone(bytes) => super::sha256_hex(bytes),
            Context::Dir { .. } => String::new(),
        },
    );
    // `pinfold/profile-<name>`, `pinfold/project-<id>` or
    // `pinfold/image-<name>`.
    let stem = match build.label {
        Some(label) => label.trim_start_matches("dev.pinfold."),
        None => "image",
    };
    let repository = format!("pinfold/{stem}-{}", build.source);
    let latest = format!("{repository}:latest");
    let reference = format!("{repository}:{id}");
    let tags = [latest.clone(), reference.clone()];
    let request = |context: &Path, containerfile: &Path| {
        runtime.build(&BuildRequest {
            context,
            containerfile,
            tags: &tags,
            labels: &labels,
            cache: build.cache,
        })
    };
    let result = match build.context {
        Context::Alone(bytes) => {
            let context = dirs::cache_dir()?.join("build").join(&id);
            fs::create_dir_all(&context)?;
            let containerfile = context.join("Containerfile");
            let result =
                fs::write(&containerfile, bytes).and_then(|()| request(&context, &containerfile));
            let _ = fs::remove_dir_all(&context);
            result?
        }
        Context::Dir { dir, containerfile } => request(dir, containerfile)?,
    };
    if let Err(output) = result {
        return Ok(Err(output));
    }
    // The second image is for rollback. A failure here is reported but never
    // fails the build that succeeded.
    if let Err(error) = clean::keep_two_images(runtime, build.label, build.source) {
        eprintln!("pinfold: maintenance: {error}");
    }
    // A runtime that cannot answer does not fail a build that succeeded.
    let id = runtime
        .resolve_image(&reference)
        .ok()
        .and_then(Result::ok)
        .map(|image| image.id);
    Ok(Ok(Built {
        reference,
        latest,
        id,
        labels,
    }))
}

/// What a caller asks `pinfold image build` for.
pub struct ImageRequest {
    /// The image's name, validated like a profile name.
    pub name: String,
    pub containerfile: PathBuf,
    /// The context the caller vouches for, as it vouches for its mounts.
    pub context: PathBuf,
    /// The caller's labels; none may start with `dev.pinfold.`.
    pub labels: BTreeMap<String, String>,
    /// Whether the runtime may reuse its layer cache.
    pub cache: bool,
}

/// Why a caller build made no image.
pub enum ImageError {
    /// Refused before the build ran; nothing was made.
    Refused(RefusalReason, String),
    /// The build ran and failed, or the runtime failed around it: the
    /// build's output, or the error.
    Failed(String),
}

/// Build a caller's image `pinfold/image-<name>` from its own context.
pub fn build_image(request: ImageRequest) -> Result<Built, ImageError> {
    let spec = |detail: String| ImageError::Refused(RefusalReason::Spec, detail);
    profile::check_name("image", &request.name).map_err(|error| spec(error.to_string()))?;
    plan::check_reserved_labels(&request.labels).map_err(spec)?;
    if !request.context.is_dir() {
        return Err(spec(format!(
            "context {} is not a directory",
            request.context.display()
        )));
    }
    let containerfile = fs::read(&request.containerfile).map_err(|error| {
        spec(format!(
            "containerfile {}: {error}",
            request.containerfile.display()
        ))
    })?;
    let runtime = runtime();
    // A missing runtime binary shows first as a failed spawn; nothing ran.
    let failed = |error: io::Error| {
        if error.kind() == io::ErrorKind::NotFound {
            ImageError::Refused(RefusalReason::Runtime, error.to_string())
        } else {
            ImageError::Failed(error.to_string())
        }
    };
    let mut labels = request.labels;
    if let Some(digest) = base_digest(runtime, &containerfile).map_err(failed)? {
        labels.insert(clean::BASE_LABEL.to_string(), digest);
    }
    build(
        runtime,
        Build {
            label: None,
            source: &request.name,
            context: Context::Dir {
                dir: &request.context,
                containerfile: &request.containerfile,
            },
            labels,
            cache: request.cache,
        },
    )
    .map_err(failed)?
    .map_err(ImageError::Failed)
}

/// A new build's id, the `<build>` of its unique tag. It starts with the
/// build's nanoseconds since the epoch in hex, which retention orders
/// builds by. It is never a label, so a cached build of unchanged inputs
/// returns the existing image.
fn build_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{}", std::process::id())
}

/// The digest of the image a Containerfile's first `FROM` names, when the
/// runtime can resolve it. A pinfold-built base has no registry, so its
/// digest is read from local storage and nothing is pulled; any other base
/// is pulled first, so a floating tag's digest is current. A `FROM` that
/// names a variable has none.
pub fn base_digest(runtime: &dyn Runtime, containerfile: &[u8]) -> io::Result<Option<String>> {
    let Ok(text) = std::str::from_utf8(containerfile) else {
        return Ok(None);
    };
    let base = text
        .lines()
        .map(str::split_whitespace)
        .find_map(|mut words| {
            if !words.next()?.eq_ignore_ascii_case("FROM") {
                return None;
            }
            words.find(|word| !word.starts_with("--"))
        });
    match base {
        Some(base) if !base.contains('$') => {
            // A pinfold-built base can never be pulled, so read its digest
            // locally before spending the retries on a doomed pull. Every
            // pinfold build records [`clean::BASE_LABEL`], empty when its
            // own base did not resolve, so its presence marks the image; a
            // caller image's family labels are all empty.
            if let Ok(image) = runtime.resolve_image(base)?
                && image.labels.contains_key(clean::BASE_LABEL)
            {
                return Ok(image.digest);
            }
            // A floating tag must be pulled for its digest to be current and
            // present to inspect. `scratch` and other non-registry references
            // cannot be pulled; they simply have no digest.
            let _ = output(&[runtime.program(), "image", "pull", base]);
            Ok(runtime
                .resolve_image(base)?
                .ok()
                .and_then(|image| image.digest))
        }
        _ => Ok(None),
    }
}
