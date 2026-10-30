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
#[cfg(target_os = "linux")]
pub(crate) mod linux;
pub(crate) mod service;

#[cfg(not(target_os = "linux"))]
use crate::core::runtime::output;
use crate::dirs;
#[cfg(not(target_os = "linux"))]
use service::TARGET_ARCH;
pub use service::{BuildRequest, ImageInfo, ImageService, service};

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
    /// Whether the runtime may reuse its layer cache.
    pub cache: bool,
    /// Labels beyond the family and base labels.
    pub labels: BTreeMap<String, String>,
    pub allow: Vec<String>,
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
pub fn build(images: &dyn ImageService, build: Build) -> io::Result<Result<Built, String>> {
    check_build_allow(&build.allow)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    // A new build's id, the `<build>` of its unique tag. It starts with the
    // build's nanoseconds since the epoch in hex, which retention orders
    // builds by. It is never a label, so a cached build of unchanged inputs
    // returns the existing image.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let id = format!("{nanos:x}-{}", std::process::id());
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
    let stem = match build.label {
        Some(label) => label.trim_start_matches("dev.pinfold."),
        None => "image",
    };
    let repository = format!("localhost/pinfold/{stem}-{}", build.source);
    let latest = format!("{repository}:latest");
    let reference = format!("{repository}:{id}");
    let tags = [latest.clone(), reference.clone()];
    let request = |context: &Path, containerfile: &Path| {
        images.build(&BuildRequest {
            context,
            containerfile,
            tags: &tags,
            labels: &labels,
            cache: build.cache,
            allow: &build.allow,
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
    if let Err(error) = clean::keep_two_images(images, Some((build.label, build.source))) {
        eprintln!("pinfold: maintenance: {error}");
    }
    // A runtime that cannot answer does not fail a build that succeeded.
    let id = images
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
    pub allow: Vec<String>,
}

/// Why a caller build made no image.
pub enum ImageError {
    /// Refused before the build ran; nothing was made.
    Refused(RefusalReason, String),
    /// The build ran and failed, or the runtime failed around it: the
    /// build's output, or the error.
    Failed(String),
}

/// Build a caller's image `localhost/pinfold/image-<name>` from its own context.
pub fn build_image(request: ImageRequest) -> Result<Built, ImageError> {
    let spec = |detail: String| ImageError::Refused(RefusalReason::Spec, detail);
    profile::check_name("image", &request.name).map_err(|error| spec(error.to_string()))?;
    plan::check_reserved_labels(&request.labels).map_err(spec)?;
    check_build_allow(&request.allow).map_err(spec)?;
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
    let images = service();
    // Linux's outer error is admission; running job errors are inner results.
    // A missing Apple runtime binary shows first as a failed spawn.
    let failed = |error: io::Error| {
        if cfg!(target_os = "linux") || error.kind() == io::ErrorKind::NotFound {
            ImageError::Refused(RefusalReason::Runtime, error.to_string())
        } else {
            ImageError::Failed(error.to_string())
        }
    };
    let labels = request.labels;
    refresh_base(images, &containerfile).map_err(failed)?;
    build(
        images,
        Build {
            label: None,
            source: &request.name,
            context: Context::Dir {
                dir: &request.context,
                containerfile: &request.containerfile,
            },
            cache: request.cache,
            labels,
            allow: request.allow,
        },
    )
    .map_err(failed)?
    .map_err(ImageError::Failed)
}

/// Build egress has exact DNS hosts only, independently of box egress.
pub(crate) fn check_build_allow(hosts: &[String]) -> Result<(), String> {
    if cfg!(target_os = "macos") && !hosts.is_empty() {
        return Err("build-allow-unsupported: --allow is supported only for Linux builds".into());
    }
    for host in hosts {
        if host.starts_with('.') || rustls::pki_types::DnsName::try_from(host.as_str()).is_err() {
            return Err(format!(
                "build --allow {host:?} must name an exact DNS host, without a scheme, port or suffix"
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn refresh_base(_images: &dyn ImageService, _containerfile: &[u8]) -> io::Result<()> {
    // BuildKit resolves sources inside the isolated builder.
    Ok(())
}

/// Refresh Apple's external base on the requested platform before building.
/// Local pinfold bases and unresolved arguments never trigger a pull.
#[cfg(not(target_os = "linux"))]
pub fn refresh_base(images: &dyn ImageService, containerfile: &[u8]) -> io::Result<()> {
    let Ok(text) = std::str::from_utf8(containerfile) else {
        return Ok(());
    };
    let mut platform = None;
    let base = text
        .lines()
        .map(str::split_whitespace)
        .find_map(|mut words| {
            if !words.next()?.eq_ignore_ascii_case("FROM") {
                return None;
            }
            for word in words {
                if let Some(value) = word.strip_prefix("--platform=") {
                    platform = Some(value);
                } else if !word.starts_with("--") {
                    return Some(word);
                }
            }
            None
        });
    // Build arguments are resolved by the builder. Guessing an unresolved
    // platform here can refresh every architecture (GitHub #78).
    if platform.is_some_and(|platform| platform.contains('$')) {
        return Ok(());
    }
    match base {
        Some("scratch") => Ok(()),
        Some(base) if !base.contains('$') => {
            // A pinfold-built base can never be pulled, so read its digest
            // locally before spending the retries on a doomed pull. Every
            // pinfold build records [`clean::BASE_LABEL`], empty when its
            // own base did not resolve, so its presence marks the image; a
            // caller image's family labels are all empty.
            if let Ok(image) = images.resolve_image(base)?
                && image.labels.contains_key(clean::BASE_LABEL)
            {
                return Ok(());
            }
            // A floating tag must be pulled for its digest to be current and
            // present to inspect. An unresolvable base has no digest.
            let native = format!("linux/{TARGET_ARCH}");
            let platform =
                platform.or_else(|| cfg!(target_os = "macos").then_some(native.as_str()));
            let mut arguments = vec![images.program(), "image", "pull"];
            if let Some(platform) = platform {
                arguments.extend(["--platform", platform]);
            }
            arguments.push(base);
            let _ = output(&arguments);
            Ok(())
        }
        _ => Ok(()),
    }
}
