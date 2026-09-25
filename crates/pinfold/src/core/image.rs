//! Image builds. One path builds every family: a profile's or a project's
//! image from its Containerfile alone, and a caller's image from the
//! caller's own context.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::r#box::RefusalReason;
use crate::core::clean;
use crate::core::profile;
use crate::core::runtime::{BuildRequest, Runtime, runtime};
use crate::dirs;

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
    /// The image name both tags share, e.g. `pinfold/profile-default`.
    pub repository: String,
    /// The label naming the source on the image, which retention groups by.
    pub label: &'static str,
    /// The source: a profile name, a project id or a caller image name.
    pub source: &'a str,
    pub context: Context<'a>,
    /// Labels beyond the source and build labels.
    pub labels: BTreeMap<String, String>,
    /// Whether the runtime may reuse its layer cache.
    pub cache: bool,
}

/// The image one build made.
pub struct Built {
    /// The unique ref, `<repository>:<build>`.
    pub reference: String,
    /// The stable ref, `<repository>:latest`, now naming the same image.
    pub latest: String,
    /// Every label the build put on the image.
    pub labels: BTreeMap<String, String>,
}

/// Build one source's image, tag it uniquely and move the stable ref to it,
/// then keep the source's newest two images. The inner `Err` is the build's
/// output when the build ran and failed.
pub fn build(runtime: &dyn Runtime, build: Build) -> io::Result<Result<Built, String>> {
    let id = build_id();
    let mut labels = build.labels;
    // The runtime copies the base image's labels onto the new image, so
    // every family label goes on every image: its own with its source, the
    // other two empty. Retention then never counts this image as another
    // family's, whatever it builds on.
    for family in [
        clean::PROFILE_LABEL,
        clean::PROJECT_LABEL,
        clean::IMAGE_LABEL,
    ] {
        labels.insert(
            family.to_string(),
            if family == build.label {
                build.source.to_string()
            } else {
                String::new()
            },
        );
    }
    // Likewise the base: the caller set the resolved digest, or this makes
    // it empty, never an inherited copy.
    labels.entry(clean::BASE_LABEL.to_string()).or_default();
    // The unique build label is what makes every build a distinct image,
    // cached or not.
    labels.insert(clean::BUILD_LABEL.to_string(), id.clone());
    let latest = format!("{}:latest", build.repository);
    let reference = format!("{}:{id}", build.repository);
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
    Ok(Ok(Built {
        reference,
        latest,
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
    if !profile::valid_name(&request.name) {
        return Err(spec(format!(
            "image name {:?} must start alphanumeric and hold only [a-z0-9._-]",
            request.name
        )));
    }
    if let Some(key) = request
        .labels
        .keys()
        .find(|key| key.starts_with("dev.pinfold."))
    {
        return Err(spec(format!(
            "label {key:?} is pinfold's: a caller label may not start with dev.pinfold."
        )));
    }
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
            repository: format!("pinfold/image-{}", request.name),
            label: clean::IMAGE_LABEL,
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

/// A new build's id. It starts with the nanoseconds since the epoch in hex,
/// so it orders builds.
fn build_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{}", std::process::id())
}

/// The digest of the image a Containerfile's first `FROM` pulls, when it
/// names one the runtime can resolve.
pub fn base_digest(runtime: &dyn Runtime, containerfile: &[u8]) -> io::Result<Option<String>> {
    Ok(profile::base_image(containerfile)
        .map(|base| runtime.image_digest(base))
        .transpose()?
        .flatten())
}
