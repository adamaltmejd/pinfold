//! Apple `container` images and its shared BuildKit builder.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use nix::fcntl::{Flock, FlockArg};
use serde::Deserialize;

use super::{allocated_bytes, container, output};
use crate::core::image::{BuildRequest, ImageInfo};
use crate::core::process::CommandGuard;
use crate::core::runtime::{captured, parse_json, spawn_error};
use crate::core::{clean, ownership, process};

/// One `container image list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedImage {
    id: String,
    configuration: ListedImageConfiguration,
    /// Each variant's OCI image config sits at `/config/config`.
    #[serde(default)]
    variants: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct ListedImageConfiguration {
    name: String,
    descriptor: Option<ListedImageDescriptor>,
}

#[derive(Deserialize)]
struct ListedImageDescriptor {
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

/// One image entry and its recorded labels.
fn image_info(image: ListedImage) -> io::Result<ImageInfo> {
    // Build labels are OCI image config labels; a locally built image also
    // carries name annotations on its index descriptor.
    let mut labels = image
        .configuration
        .descriptor
        .map(|descriptor| descriptor.annotations)
        .unwrap_or_default();
    for variant in &image.variants {
        if let Some(found) = variant.pointer("/config/config/Labels") {
            labels.extend(
                BTreeMap::<String, String>::deserialize(found).map_err(|error| {
                    io::Error::other(format!("image {} has invalid labels: {error}", image.id))
                })?,
            );
        }
    }
    Ok(ImageInfo {
        id: image.id,
        reference: image.configuration.name,
        labels,
    })
}

/// Prune target for the builder's unused cache after each build, 16 GB
/// in buildctl's decimal MB. One four-image chain measured 2.5 GB (#392)
/// and its backing storage 3.3 GiB; the target leaves room for several
/// chains. Apple's own GC keeps 32 GiB.
const BUILDER_KEEP_MB: &str = "16000";
const BUILD_LOG_BYTES: usize = 64 * 1024;

/// Prune unused BuildKit records in the shared builder. BuildKit keeps
/// records in active use; deleting the shared builder instead interrupts
/// builds in other processes (#75).
fn prune_builder(flags: &[&str]) -> io::Result<()> {
    let mut command = vec![
        "container",
        "exec",
        "buildkit",
        "buildctl",
        "prune",
        "--all",
    ];
    command.extend_from_slice(flags);
    output(&command).map(drop)
}

/// Return freed guest blocks to the host by online trim, without replacing
/// the builder or stopping active builds (#86).
fn trim_builder() -> io::Result<()> {
    // The guest's ext4 trims freed blocks only once the journal has committed
    // their release; without the sync a trim just after a prune returns
    // little or nothing (#417). Sync only the root filesystem: a plain sync
    // also waits on the builder's VirtioFS mounts, without bound. The sync is
    // best-effort: BusyBox does not report syncfs errors, and the trim's
    // result is what counts.
    let _ = output(&["container", "exec", "buildkit", "sync", "-f", "/"]);
    output(&["container", "clean", "buildkit"]).map(drop)
}

/// Every pinfold build holds this host lock shared; `clean` holds it
/// exclusive while it stops the builder.
const BUILDER_LOCK: &str = "builder:apple";
/// The build-records query took 0.3 s and the stop 0.4 s when measured.
const BUILDER_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const BUILDER_STOP_TIMEOUT: Duration = Duration::from_secs(60);

/// Stop the builder when nothing builds on it. Its host process keeps each
/// build's unlinked export open until it stops (#440); the next build
/// restarts it with its cache.
fn stop_idle_builder() -> io::Result<()> {
    // Held through the stop so no pinfold build starts in between.
    let idle = ownership::try_flock(
        ownership::open_lock(BUILDER_LOCK)?,
        FlockArg::LockExclusiveNonblock,
    )?;
    // Every Apple build waits on the lock, so nothing under it may hang,
    // a write to a stderr nobody drains included: decide, release, print.
    let busy = match idle {
        None => Ok(Some("a build is running")),
        Some(_) => match process::with_deadline(BUILDER_QUERY_TIMEOUT, |_| builder_busy()) {
            Ok(true) => Ok(Some("a build is running")),
            Ok(false) => process::with_deadline(BUILDER_STOP_TIMEOUT, |_| {
                output(&["container", "builder", "stop"]).map(drop)
            })
            .map(|()| None)
            .map_err(|error| match error.kind() {
                io::ErrorKind::Interrupted => io::Error::other(format!(
                    "container builder stop did not finish within {}s",
                    BUILDER_STOP_TIMEOUT.as_secs()
                )),
                _ => error,
            }),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                Ok(Some("the builder did not answer"))
            }
            Err(error) => Err(error),
        },
    };
    drop(idle);
    if let Some(reason) = busy? {
        let _ = writeln!(
            io::stderr(),
            "pinfold clean: build cache: {reason}, so cleanup could not complete; run `pinfold clean` again after it"
        );
    }
    Ok(())
}

/// Whether BuildKit has a build in progress, pinfold's or another caller's:
/// a build record without a completion time. A killed client's build
/// completes its record at once.
fn builder_busy() -> io::Result<bool> {
    #[derive(Deserialize)]
    struct Event {
        record: Record,
    }
    #[derive(Deserialize)]
    struct Record {
        #[serde(rename = "CompletedAt")]
        completed_at: Option<serde::de::IgnoredAny>,
    }
    let json = output(&[
        "container",
        "exec",
        "buildkit",
        "buildctl",
        "debug",
        "histories",
        "--format",
        "{{json .}}",
    ])?;
    for event in serde_json::Deserializer::from_slice(&json).into_iter::<Event>() {
        let event = event.map_err(|error| {
            io::Error::other(format!(
                "buildctl debug histories returned invalid JSON: {error}"
            ))
        })?;
        if event.record.completed_at.is_none() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn list_images() -> io::Result<Vec<ImageInfo>> {
    let json = output(&["container", "image", "list", "--format", "json"])?;
    let images: Vec<ListedImage> = parse_json("container image list", &json)?;
    images.into_iter().map(image_info).collect()
}

pub fn resolve_image(reference: &str) -> io::Result<Option<ImageInfo>> {
    // `image inspect` prints the same entries as `image list`.
    let argv = ["container", "image", "inspect", reference];
    let json = match captured(&argv)? {
        Ok(json) => json,
        Err(stderr) if stderr.starts_with("Error: image not found:") => return Ok(None),
        Err(stderr) => {
            return Err(super::stopped_service(io::Error::other(format!(
                "{}: {stderr}",
                argv.join(" ")
            ))));
        }
    };
    let images: Vec<ListedImage> = parse_json("container image inspect", &json)?;
    let image = images
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::other("container image inspect returned no image"))?;
    image_info(image).map(Some)
}

pub fn remove_image(reference: &str) -> io::Result<()> {
    // `image delete` also collects the layers no image references.
    output(&["container", "image", "delete", reference]).map(|_| ())
}

pub fn purge_build_cache() -> io::Result<()> {
    let Some(builder) = container("buildkit")? else {
        return Ok(());
    };
    if builder.status.state != "running" {
        // `builder start` may replace an existing instance when its
        // defaults changed; start this instance without replacing it.
        output(&["container", "start", "buildkit"])?;
    }
    prune_builder(&[])?;
    trim_builder()?;
    stop_idle_builder()
}

pub fn build_cache_bytes() -> io::Result<u64> {
    let Some(builder) = container("buildkit")? else {
        return Ok(0);
    };
    // The exports mount records the runtime's actual app root, including
    // a custom root. Count allocated blocks: rootfs.ext4 is sparse and
    // BuildKit's logical cache size misses retained filesystem space (#74).
    let exports = builder
        .configuration
        .mounts
        .iter()
        .find(|mount| mount.destination == "/var/lib/container-builder-shim/exports")
        .map(|mount| Path::new(&mount.source))
        .ok_or_else(|| io::Error::other("builder exports mount is missing"))?;
    let root = exports
        .parent()
        .ok_or_else(|| io::Error::other("builder exports mount has no parent"))?;
    allocated_bytes(&root.join("containers/buildkit"))
}

pub fn build(request: &BuildRequest) -> io::Result<Result<(), String>> {
    // Through the post-build prune and trim, which also need the builder.
    let _building = Flock::lock(ownership::open_lock(BUILDER_LOCK)?, FlockArg::LockShared)
        .map_err(|(_, error)| io::Error::from(error))?;
    refresh_base(&std::fs::read(request.containerfile)?)?;
    let result = run_build(request)?;
    // Neither step fails the build.
    if prune_builder(&["--keep-storage", BUILDER_KEEP_MB]).is_err() {
        eprintln!("pinfold: could not prune the builder cache");
    }
    if trim_builder().is_err() {
        eprintln!("pinfold: could not reclaim unused builder blocks");
    }
    Ok(result)
}

/// Refresh Apple's external base on the requested platform before building.
/// Local pinfold bases and unresolved arguments never trigger a pull.
fn refresh_base(containerfile: &[u8]) -> io::Result<()> {
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
            // An unreadable local index falls through to the pull.
            if let Ok(Some(image)) = resolve_image(base)
                && image.labels.contains_key(clean::BASE_LABEL)
            {
                return Ok(());
            }
            // A floating tag must be pulled for its digest to be current and
            // present to inspect. An unresolvable base has no digest.
            let platform = platform.unwrap_or("linux/arm64");
            let _ = output(&["container", "image", "pull", "--platform", platform, base]);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Run `container build` for `request`. stdout and stderr are captured
/// through one pipe, so the output keeps its order. The inner `Err` is that
/// output when the build fails.
fn run_build(request: &BuildRequest) -> io::Result<Result<(), String>> {
    let (reader, writer) = crate::core::process::pipe()?;
    let mut command = std::process::Command::new("container");
    // Apple includes color settings in the shared builder's identity
    // (GitHub #77). Every pinfold caller must use the same settings.
    command.env_remove("NO_COLOR").env_remove("BUILDKIT_COLORS");
    command
        .arg("build")
        // The macOS CLI is arm64 only.
        .args(["--build-arg", "TARGETARCH=arm64"])
        .args(if request.cache {
            None
        } else {
            Some("--no-cache")
        })
        .arg("--file")
        .arg(request.containerfile);
    if let Some((name, value)) = request.build_arg {
        command.arg("--build-arg").arg(format!("{name}={value}"));
    }
    for tag in request.tags {
        command.arg("--tag").arg(tag);
    }
    for (key, value) in request.labels {
        command.arg("--label").arg(format!("{key}={value}"));
    }
    command
        // A bare relative context such as `--json` would parse as an option.
        .arg(std::path::absolute(request.context)?)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    let child = command.spawn_guarded();
    // The command holds the pipe's write ends, and the read below ends only
    // once every write end is closed.
    drop(command);
    let mut child = child.map_err(|error| spawn_error("container", error))?;
    let mut tail = VecDeque::new();
    let mut newlines = 0;
    for byte in io::Read::bytes(io::BufReader::new(reader)) {
        let byte = match byte {
            Ok(byte) => byte,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        tail.push_back(byte);
        newlines += usize::from(byte == b'\n');
        while tail.len() > BUILD_LOG_BYTES
            || newlines + usize::from(tail.back() != Some(&b'\n')) > 40
        {
            newlines -= usize::from(tail.pop_front() == Some(b'\n'));
        }
    }
    if child.wait()?.success() {
        Ok(Ok(()))
    } else {
        let bytes: Vec<u8> = tail.into_iter().collect();
        let mut output = String::from_utf8_lossy(&bytes).into_owned();
        // Replacement characters can expand malformed runtime output.
        let start = output.ceil_char_boundary(output.len().saturating_sub(BUILD_LOG_BYTES));
        output.drain(..start);
        Ok(Err(output))
    }
}
