//! A finite build transaction. The VMM sees only its fixed role's disks.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, OFlag};
use nix::poll::PollFlags;
use nix::sys::stat::Mode;
use serde::Serialize;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};

use super::catalog::{self, Verified};
use crate::core::image::{BuildRequest, ImageInfo};
use crate::core::ownership::{self, Claim};
use crate::core::plan::{Egress, Plan};
use crate::core::runtime::firecracker::{jail, vm};
use crate::core::{artifacts, clean, pipe, proxy, runs, sha256_hex};
use crate::init::firecracker::{BOOT, BOOT_PORT, FAILED, READY, read_frame, write_frame};

// Candidate allocations from the executed trial. Final policy is an admission gate.
const CACHE_BYTES: u64 = 8 << 30;
const WORK_BYTES: u64 = 3 << 30;
const JOB_TIME: Duration = Duration::from_secs(300);
const INPUT_BYTES: u64 = 512 << 20;
const BUILD_HOSTS: &[&str] = &[
    "registry-1.docker.io",
    "auth.docker.io",
    "production.cloudfront.docker.com",
    "deb.debian.org",
    "github.com",
    "release-assets.githubusercontent.com",
    "registry.npmjs.org",
    "cli.github.com",
];

#[derive(Serialize)]
struct Base {
    reference: String,
    digest: String,
    directory: String,
}

#[derive(Serialize)]
struct Job<'a> {
    convert: bool,
    cache: bool,
    labels: &'a BTreeMap<String, String>,
    bases: Vec<Base>,
}

struct Request {
    context: PathBuf,
    containerfile: PathBuf,
    cache: bool,
    labels: BTreeMap<String, String>,
    tags: Vec<String>,
    allow: Vec<String>,
}

struct Prepared {
    root: PathBuf,
    vmm: artifacts::Vmm,
    init: PathBuf,
    cache_disk: PathBuf,
    input: PathBuf,
    dirty: PathBuf,
    // Every local reference is a possible Dockerfile context. Keep its snapshot
    // leased through conversion and publication, without parsing the recipe.
    _bases: Vec<(ImageInfo, PathBuf, Flock<File>)>,
}

#[derive(Default)]
struct InputBudget {
    bytes: u64,
}

impl InputBudget {
    fn charge(&mut self, bytes: u64) -> io::Result<()> {
        let allocated = bytes
            .max(1)
            .div_ceil(4096)
            .checked_mul(4096)
            .ok_or_else(|| io::Error::other("resource: build input size"))?;
        self.bytes = self
            .bytes
            .checked_add(allocated)
            .filter(|bytes| *bytes <= INPUT_BYTES)
            .ok_or_else(|| io::Error::other("resource: build input size"))?;
        Ok(())
    }
}

pub fn build(request: &BuildRequest) -> io::Result<Result<(), String>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(transaction(request))
}

async fn transaction(request: &BuildRequest<'_>) -> io::Result<Result<(), String>> {
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let cancel = Arc::new(AtomicBool::new(false));
    let name = super::cache_name()?;
    let mut claim = tokio::select! {
        result = Claim::take(&name) => result?,
        _ = interrupt.recv() => return Err(io::ErrorKind::Interrupted.into()),
        _ = terminate.recv() => return Err(io::ErrorKind::Interrupted.into()),
    }
    .ok_or_else(|| io::Error::other("builder-in-use: another build owns the cache"))?;
    cleanup_abandoned_socket_dirs(&claim)?;
    let state = super::cache_dir()?.join("job");
    let restoring = {
        let state = state.clone();
        tokio::task::spawn_blocking(move || writable_snapshot(&state))
    };
    tokio::pin!(restoring);
    tokio::select! {
        result = &mut restoring => result.map_err(io::Error::other)??,
        _ = interrupt.recv() => { let _ = restoring.await; return Err(io::ErrorKind::Interrupted.into()); },
        _ = terminate.recv() => { let _ = restoring.await; return Err(io::ErrorKind::Interrupted.into()); },
    }
    claim.prepare(&state)?;
    let request = Request {
        context: request.context.to_path_buf(),
        containerfile: request.containerfile.to_path_buf(),
        cache: request.cache,
        labels: request.labels.clone(),
        tags: request.tags.to_vec(),
        allow: request.allow.to_vec(),
    };
    let result = {
        let work = run_transaction(request, state.clone(), cancel.clone(), &mut claim);
        tokio::pin!(work);
        tokio::select! {
            result = &mut work => result,
            _ = interrupt.recv() => {
                cancel.store(true, Ordering::Release);
                let _ = work.await;
                Err(io::ErrorKind::Interrupted.into())
            },
            _ = terminate.recv() => {
                cancel.store(true, Ordering::Release);
                let _ = work.await;
                Err(io::ErrorKind::Interrupted.into())
            },
        }
    };
    // run_transaction never returns until a spawned VMM has been reaped and
    // every host worker has stopped. Only then may the cache claim be released.
    let writable = tokio::task::spawn_blocking(move || writable_snapshot(&state))
        .await
        .map_err(io::Error::other)
        .and_then(|result| result);
    let sockets = cleanup_generation_sockets(&claim);
    let cleanup = claim.finish(writable.is_ok() && sockets.is_ok()).await;
    match result.and(writable).and(sockets).and(cleanup) {
        Ok(()) => Ok(Ok(())),
        Err(error) => Ok(Err(error.to_string())),
    }
}

async fn run_transaction(
    request: Request,
    state: PathBuf,
    cancel: Arc<AtomicBool>,
    claim: &mut Claim,
) -> io::Result<()> {
    let preparing = {
        let state = state.clone();
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            let prepared = prepare(&request, &state, &cancel)?;
            Ok::<_, io::Error>((request, prepared))
        })
    };
    let prepared = watch_worker(preparing, &cancel, claim, None).await?;
    let (request, prepared) = prepared;
    check_cancel(&cancel)?;
    let generation = claim.generation().to_string();
    runs::create(&generation)?;
    let vsock = socket_dir(&generation, true)?;
    private_dir(&vsock)?;
    let mut hosts: Vec<String> = BUILD_HOSTS.iter().map(|host| (*host).to_string()).collect();
    hosts.extend_from_slice(&request.allow);
    runs::write_launch(
        &generation,
        serde_json::json!({
            "kind": "build",
            "pinfold": env!("CARGO_PKG_VERSION"),
            "tags": request.tags,
            "builder": artifacts::builder_identity(),
            "egress": { "allow": hosts, "routes": {} },
            "resources": {
                "policy": "candidate",
                "vcpus": 2, "memory_mib": 2560,
                "cache_bytes": CACHE_BYTES, "converter_work_bytes": WORK_BYTES,
                "input_bytes": INPUT_BYTES,
                "job_deadline_seconds": JOB_TIME.as_secs(),
            },
        }),
    )?;
    let proxy_socket = vsock.join("v.sock_3128");
    let egress_log = runs::egress_log(&generation)?;
    let mut audit = proxy::start(
        &proxy_socket,
        &Egress {
            allow: hosts,
            routes: BTreeMap::new(),
        },
        None,
        egress_log.clone(),
    )?;
    eprintln!(
        "pinfold: build run {generation}; egress log {}",
        egress_log.display()
    );
    fs::set_permissions(proxy_socket, fs::Permissions::from_mode(0o600))?;
    let built = run_job(
        &state.join("build"),
        &prepared,
        &prepared.cache_disk,
        &prepared.input,
        true,
        &request.labels,
        &state.join("oci"),
        &cancel,
        claim,
        &mut audit,
    )
    .await?;
    let Received::Oci(verified) = built else {
        return Err(io::Error::other("image-output: unexpected root"));
    };
    let recipe = sha256_hex(
        [
            include_bytes!("../../../../builder.toml").as_slice(),
            include_bytes!("../../../../src/init/builder.rs").as_slice(),
        ]
        .concat(),
    );
    let converting = {
        let state = state.clone();
        let cancel = cancel.clone();
        let recipe = recipe.clone();
        let cache_disk = prepared.cache_disk.clone();
        let dirty = prepared.dirty.clone();
        tokio::task::spawn_blocking(move || {
            // E/EOF follows guest unmount. Sync the backing file before
            // advertising a clean cache, and keep the runtime responsive.
            File::open(cache_disk)?.sync_all()?;
            fs::remove_file(&dirty)?;
            File::open(dirty.parent().expect("cache marker has a parent"))?.sync_all()?;
            check_cancel(&cancel)?;
            let cached = catalog::complete_root(&verified.id, &recipe)?;
            check_cancel(&cancel)?;
            if cached.is_none() {
                prepare_conversion(&verified, &state, &cancel)?;
            }
            Ok::<_, io::Error>((verified, cached))
        })
    };
    let conversion = watch_worker(converting, &cancel, claim, Some(&mut audit)).await?;
    let (verified, cached) = conversion;
    let converted = if let Some(cached) = cached {
        cached
    } else {
        let root_file = state.join("converted.ext4");
        match run_job(
            &state.join("convert"),
            &prepared,
            &state.join("converter.ext4"),
            &state.join("conversion-input"),
            false,
            &request.labels,
            &root_file,
            &cancel,
            claim,
            &mut audit,
        )
        .await?
        {
            Received::Root(sha, bytes) => (root_file, sha, bytes),
            _ => return Err(io::Error::other("image-output: unexpected OCI")),
        }
    };
    check_cancel(&cancel)?;
    let worker_cancel = cancel.clone();
    let publishing = tokio::task::spawn_blocking(move || {
        check_cancel(&worker_cancel)?;
        catalog::publish(
            verified,
            &converted.0,
            &converted.1,
            converted.2,
            &recipe,
            &request.tags,
            &worker_cancel,
        )
    });
    watch_worker(publishing, &cancel, claim, Some(&mut audit)).await?;
    Ok(())
}

fn prepare(request: &Request, state: &Path, cancel: &AtomicBool) -> io::Result<Prepared> {
    check_cancel(cancel)?;
    let root = artifacts::builder(Some(cancel))?;
    let vmm = artifacts::vmm(Some(cancel))?;
    let init = state.join("init.ext4");
    let init_tree = state.join("init.tree");
    private_dir(&init_tree)?;
    fs::create_dir(init_tree.join("dev"))?;
    fs::create_dir(init_tree.join("mnt"))?;
    copy_file(
        &std::env::current_exe()?,
        &init_tree.join("pinfold"),
        cancel,
        &mut InputBudget::default(),
    )?;
    fs::set_permissions(init_tree.join("pinfold"), fs::Permissions::from_mode(0o755))?;
    input_disk(&init_tree, &init, cancel)?;
    fs::remove_dir_all(init_tree)?;
    let cache = super::cache_dir()?;
    let cache_disk = cache.join("cache.ext4");
    let dirty = cache.join("unclean");
    if dirty.exists() {
        crate::core::absent_ok(fs::remove_file(&cache_disk))?;
    }
    let mut marker = File::create(&dirty)?;
    marker.write_all(b"in-use\n")?;
    marker.sync_all()?;
    File::open(&cache)?.sync_all()?;
    if !cache_disk.exists() {
        empty_disk(&cache_disk, CACHE_BYTES, cancel)?;
    }
    let bases = catalog::bases()?;
    let input = state.join("input");
    let mut budget = InputBudget::default();
    private_dir(&input.join("context"))?;
    copy_tree(
        &fs::canonicalize(&request.context)?,
        &input.join("context"),
        cancel,
        &mut budget,
    )?;
    private_dir(&input.join("recipe"))?;
    copy_file(
        &fs::canonicalize(&request.containerfile)?,
        &input.join("recipe/Dockerfile"),
        cancel,
        &mut budget,
    )?;
    // The project CLI resolved its selected profile before this snapshot.
    // Keep that identity even if another publisher has since moved latest.
    let selected_base = request
        .labels
        .get(clean::BASE_LABEL)
        .filter(|base| !base.is_empty())
        .filter(|_| {
            request
                .labels
                .get(clean::PROJECT_LABEL)
                .is_some_and(|project| !project.is_empty())
        })
        .map(|expected| {
            bases
                .iter()
                .find(|(image, _, _)| {
                    &image.id == expected
                        && image
                            .labels
                            .get(clean::PROFILE_LABEL)
                            .is_some_and(|profile| !profile.is_empty())
                })
                .ok_or_else(|| {
                    io::Error::other(
                        "image-base-changed: selected local profile is no longer present",
                    )
                })
        })
        .transpose()?;
    let selected_reference = selected_base.map(|(image, _, _)| {
        format!(
            "localhost/pinfold/profile-{}",
            image.labels[clean::PROFILE_LABEL]
        )
    });
    let mut contexts = Vec::new();
    let mut layouts: BTreeMap<String, String> = BTreeMap::new();
    for (image, path, _lease) in &bases {
        let reference = image
            .reference
            .strip_suffix(":latest")
            .unwrap_or(&image.reference);
        let (image, path) = if selected_reference.as_deref() == Some(reference) {
            let (image, path, _) = selected_base.expect("a selected reference has a selected base");
            (image, path)
        } else {
            (image, path)
        };
        let relative = if let Some(relative) = layouts.get(&image.id) {
            relative.clone()
        } else {
            let relative = format!("bases/{}", layouts.len());
            let target = input.join(&relative);
            private_dir(&target)?;
            copy_tree(path, &target, cancel, &mut budget)?;
            layouts.insert(image.id.clone(), relative.clone());
            relative
        };
        contexts.push(Base {
            reference: reference.to_string(),
            digest: image.id.clone(),
            directory: format!("/input/{relative}"),
        });
    }
    write_job(
        &input,
        &Job {
            convert: false,
            cache: request.cache,
            labels: &request.labels,
            bases: contexts,
        },
        &mut budget,
    )?;
    budget.charge(4096)?;
    source_policy(&input, "docker-image://localhost/pinfold/*")?;
    check_cancel(cancel)?;
    Ok(Prepared {
        root,
        vmm,
        init,
        cache_disk,
        input,
        dirty,
        _bases: bases,
    })
}

fn prepare_conversion(verified: &Verified, state: &Path, cancel: &AtomicBool) -> io::Result<()> {
    let input = state.join("conversion-input");
    let mut budget = InputBudget::default();
    for path in ["context", "recipe", "bases/0"] {
        private_dir(&input.join(path))?;
    }
    copy_tree(
        &verified.directory,
        &input.join("bases/0"),
        cancel,
        &mut budget,
    )?;
    budget.charge(4096)?;
    fs::write(
        input.join("recipe/Dockerfile"),
        b"FROM --platform=linux/amd64 prepared\n",
    )?;
    write_job(
        &input,
        &Job {
            convert: true,
            cache: false,
            labels: &verified.labels,
            bases: vec![Base {
                reference: "prepared".into(),
                digest: verified.id.clone(),
                directory: "/input/bases/0".into(),
            }],
        },
        &mut budget,
    )?;
    budget.charge(4096)?;
    source_policy(&input, "docker-image://*")?;
    empty_disk(&state.join("converter.ext4"), WORK_BYTES, cancel)
}

enum Received {
    Oci(Verified),
    Root(String, u64),
}

#[allow(clippy::too_many_arguments)]
async fn run_job(
    state: &Path,
    prepared: &Prepared,
    work: &Path,
    input: &Path,
    build: bool,
    expected: &BTreeMap<String, String>,
    output_path: &Path,
    cancel: &Arc<AtomicBool>,
    claim: &mut Claim,
    audit: &mut tokio::sync::oneshot::Receiver<()>,
) -> io::Result<Received> {
    check_cancel(cancel)?;
    private_dir(state)?;
    let vsock = socket_dir(claim.generation(), build)?;
    private_dir(&vsock)?;
    let disk = state.join("input.ext4");
    let formatting = {
        let input = input.to_path_buf();
        let disk = disk.clone();
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || input_disk(&input, &disk, &cancel))
    };
    watch_worker(formatting, cancel, claim, Some(&mut *audit)).await?;
    check_cancel(cancel)?;
    let listener = UnixListener::bind(vsock.join(format!("v.sock_{BOOT_PORT}")))?;
    let drives = vec![(work.to_path_buf(), true), (disk, false)];
    let plan = Plan {
        cpus: Some(2.0),
        memory: Some("2560M".into()),
        ..Plan::default()
    };
    let mut config = vm::config(&plan, &drives);
    config["boot-source"]["boot_args"] =
        "console=ttyS0 reboot=k panic=1 pci=off init=/pinfold -- init builder".into();
    let config_path = state.join("config.json");
    fs::write(&config_path, config.to_string())?;
    let jail_root = state.join("jail");
    private_dir(&jail_root)?;
    let mut child = jail::spawn(&jail::Jail {
        root: &jail_root,
        firecracker: &prepared.vmm.firecracker,
        kernel: &prepared.vmm.kernel,
        root_disk: &prepared.root,
        init_disk: &prepared.init,
        disks: &drives,
        vsock: &vsock,
        config: &config_path,
    })?;
    let deadline = Instant::now() + JOB_TIME;
    // All fallible work after spawn is inside this future. Every result flows
    // through kill/wait below before the caller can release ownership.
    let received = async {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted.map(|(stream,_)|stream),
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err(io::Error::other("resource: builder job deadline")),
            _ = child.wait() => Err(io::Error::other("builder exited before handshake")),
            _ = cancelled(cancel) => Err(io::ErrorKind::Interrupted.into()),
            _ = claim.stop_requested() => { cancel.store(true, Ordering::Release); Err(io::ErrorKind::Interrupted.into()) },
            _ = &mut *audit => { cancel.store(true, Ordering::Release); Err(io::Error::other("audit-log: builder audit failed")) },
        }?;
        drop(listener);
        let stream = accepted.into_std()?;
        stream.set_nonblocking(true)?;
        let cancellation = cancel.clone();
        let expected = expected.clone();
        let destination = output_path.to_path_buf();
        let reader = tokio::task::spawn_blocking(move || {
            let mut reader = JobReader { stream, deadline, cancel: cancellation };
            let mut request = vec![0; 8];
            request.push(u8::from(build));
            put(&mut request, b"vdb");
            request.extend(2u32.to_be_bytes());
            for (device, path, writable) in [
                (b"vdc".as_slice(), b"/work".as_slice(), true),
                (b"vdd".as_slice(), b"/input".as_slice(), false),
            ] {
                put(&mut request, device); put(&mut request, path);
                request.push(u8::from(writable));
            }
            request.extend(0u32.to_be_bytes());
            write_frame(&mut reader, BOOT, &request)?;
            let (kind, payload) = read_frame(&mut reader)?;
            match kind {
                READY if payload.is_empty() => {},
                FAILED => return Err(io::Error::other(String::from_utf8_lossy(&payload).into_owned())),
                _ => return Err(io::Error::other("image-output: missing successful job handshake")),
            }
            if build { catalog::receive(&mut reader, &destination, &expected).map(Received::Oci) }
            else { catalog::receive_root(&mut reader, &destination).map(|(sha,size)| Received::Root(sha,size)) }
        });
        tokio::pin!(reader);
        tokio::select! {
            result = &mut reader => result.map_err(io::Error::other)?,
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                cancel.store(true, Ordering::Release); let _ = reader.await;
                Err(io::Error::other("resource: builder job deadline"))
            },
            _ = claim.stop_requested() => {
                cancel.store(true, Ordering::Release); let _ = reader.await;
                Err(io::ErrorKind::Interrupted.into())
            },
            _ = &mut *audit => {
                cancel.store(true, Ordering::Release); let _ = reader.await;
                Err(io::Error::other("audit-log: builder audit failed"))
            },
        }
    }.await;
    let killed = child.kill();
    let reaped = child.wait().await;
    killed?;
    reaped?;
    received
}

fn put(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend((bytes.len() as u32).to_be_bytes());
    output.extend(bytes);
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}

async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn watch_worker<T>(
    mut worker: tokio::task::JoinHandle<io::Result<T>>,
    cancel: &AtomicBool,
    claim: &mut Claim,
    audit: Option<&mut tokio::sync::oneshot::Receiver<()>>,
) -> io::Result<T> {
    let audit_failed = async {
        if let Some(audit) = audit {
            let _ = audit.await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        result = &mut worker => result.map_err(io::Error::other)?,
        _ = claim.stop_requested() => {
            cancel.store(true, Ordering::Release);
            let _ = worker.await;
            Err(io::ErrorKind::Interrupted.into())
        },
        _ = audit_failed => {
            cancel.store(true, Ordering::Release);
            let _ = worker.await;
            Err(io::Error::other("audit-log: builder audit failed"))
        },
    }
}

struct JobReader {
    stream: UnixStream,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
}
impl Read for JobReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        pipe::read(
            &mut self.stream,
            buffer,
            self.deadline,
            Some(&self.cancel),
            "image-output",
        )
    }
}
impl Write for JobReader {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        pipe::write_all(
            &mut self.stream,
            buffer,
            self.deadline,
            Some(&self.cancel),
            "image-input",
        )?;
        Ok(buffer.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

fn socket_prefix() -> io::Result<String> {
    Ok(format!(
        "b-{}-",
        super::cache_name()?.trim_start_matches("builder-")
    ))
}

fn socket_path(generation: &str, build: bool) -> io::Result<PathBuf> {
    if generation.len() != 64 || !generation.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::other("invalid builder generation"));
    }
    Ok(ownership::host_dir()?.join(format!(
        "{}{}-{}",
        socket_prefix()?,
        &generation[..24],
        if build { 'b' } else { 'c' }
    )))
}

fn socket_dir(generation: &str, build: bool) -> io::Result<PathBuf> {
    // The cache claim checked this common namespace through open_lock.
    // Only sockets leave the cache path; immutable OCI staging stays there.
    let path = socket_path(generation, build)?;
    match fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    check_socket_dir(&path)?;
    Ok(path)
}

fn check_socket_dir(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other("unsafe builder socket directory"));
    }
    Ok(())
}

// Called only after both VMMs and host workers ended, before finishing this
// claim. It names this generation's two paths, never another job's sockets.
fn cleanup_generation_sockets(claim: &Claim) -> io::Result<()> {
    for build in [true, false] {
        let path = socket_path(claim.generation(), build)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                check_socket_dir(&path)?;
                fs::remove_dir_all(path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

// Taking this cache's exclusive Claim precedes orphan recovery. An old job's
// ordinary state may already be pruned, so its cache-specific prefix is durable.
fn cleanup_abandoned_socket_dirs(_cache_claim: &Claim) -> io::Result<()> {
    let prefix = socket_prefix()?;
    for entry in crate::dirs::entries(&ownership::host_dir()?)? {
        let name = entry.file_name();
        let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix(&prefix)) else {
            continue;
        };
        let Some((generation, role)) = suffix.split_once('-') else {
            continue;
        };
        if generation.len() != 24
            || !generation.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !matches!(role, "b" | "c")
        {
            continue;
        }
        check_socket_dir(&entry.path())?;
        fs::remove_dir_all(entry.path())?;
    }
    Ok(())
}

fn empty_disk(path: &Path, bytes: u64, cancel: &AtomicBool) -> io::Result<()> {
    check_cancel(cancel)?;
    File::options()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?
        .set_len(bytes)?;
    let mut command = mkfs()?;
    command.arg(path);
    run_process(command, cancel)
}

fn mkfs() -> io::Result<Command> {
    let mut command = Command::new(crate::core::runtime::firecracker::preflight::mkfs_ext4()?);
    command.args(["-q", "-F", "-m", "0"]);
    Ok(command)
}

fn input_disk(input: &Path, disk: &Path, cancel: &AtomicBool) -> io::Result<()> {
    let (kib, entries) = tree_size(input, cancel)?;
    let mut command = mkfs()?;
    command
        .args(["-O", "^has_journal", "-N"])
        .arg((entries + 1024).to_string())
        .arg("-d")
        .arg(input)
        .arg(disk)
        .arg(format!("{}k", kib + kib / 10 + entries / 4 + 16384));
    run_process(command, cancel)
}

fn run_process(mut command: Command, cancel: &AtomicBool) -> io::Result<()> {
    check_cancel(cancel)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + JOB_TIME;
    let result = (|| loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                check_cancel(cancel)
            } else {
                Err(io::Error::other(format!(
                    "image-prepare: mkfs.ext4 exited {status}"
                )))
            };
        }
        pipe::wait_ready(
            None,
            PollFlags::empty(),
            deadline,
            Some(cancel),
            "image-prepare",
        )?;
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let reaped = child.wait();
    result.and(reaped.map(|_| ()))
}

fn tree_size(path: &Path, cancel: &AtomicBool) -> io::Result<(u64, u64)> {
    check_cancel(cancel)?;
    let (mut kib, mut entries) = (4, 1);
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            let (inner_kib, inner_entries) = tree_size(&entry.path(), cancel)?;
            kib += inner_kib;
            entries += inner_entries;
        } else {
            kib += metadata.len().div_ceil(4096) * 4;
            entries += 1;
        }
        check_cancel(cancel)?;
    }
    Ok((kib, entries))
}

fn copy_file(
    from: &Path,
    to: &Path,
    cancel: &AtomicBool,
    budget: &mut InputBudget,
) -> io::Result<()> {
    let source = File::options()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(from)?;
    copy_open_file(source, to, cancel, budget)
}

fn copy_open_file(
    mut source: File,
    to: &Path,
    cancel: &AtomicBool,
    budget: &mut InputBudget,
) -> io::Result<()> {
    let metadata = source.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("build-input: input changed file type"));
    }
    budget.charge(metadata.len())?;
    let mut destination = File::options()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)?;
    let mut bytes = [0; 64 << 10];
    let mut copied = 0u64;
    loop {
        check_cancel(cancel)?;
        let read = source.read(&mut bytes)?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .filter(|bytes| *bytes <= metadata.len())
            .ok_or_else(|| io::Error::other("build-input: input changed size"))?;
        destination.write_all(&bytes[..read])?;
    }
    if copied != metadata.len() {
        return Err(io::Error::other("build-input: input changed size"));
    }
    fs::set_permissions(to, metadata.permissions())
}

fn copy_tree(
    from: &Path,
    to: &Path,
    cancel: &AtomicBool,
    budget: &mut InputBudget,
) -> io::Result<()> {
    let from = fs::canonicalize(from)?;
    if fs::canonicalize(to)?.starts_with(&from) {
        return Err(io::Error::other(
            "build-input: context contains its own snapshot directory",
        ));
    }
    // Keep the parent inode open while resolving each child. A caller changing
    // a directory into a symlink cannot turn the copy into an outside-tree read.
    let directory = File::options()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
        .open(from)?;
    copy_directory(&directory, to, cancel, budget)
}

fn copy_directory(
    directory: &File,
    to: &Path,
    cancel: &AtomicBool,
    budget: &mut InputBudget,
) -> io::Result<()> {
    budget.charge(4096)?;
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    for entry in fs::read_dir(anchored)? {
        check_cancel(cancel)?;
        let entry = entry?;
        let name = entry.file_name();
        let target = to.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            let child = File::from(nix::fcntl::openat(
                directory,
                name.as_os_str(),
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )?);
            private_dir(&target)?;
            copy_directory(&child, &target, cancel, budget)?;
        } else if kind.is_file() {
            let source = File::from(nix::fcntl::openat(
                directory,
                name.as_os_str(),
                OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
                Mode::empty(),
            )?);
            copy_open_file(source, &target, cancel, budget)?;
        } else if kind.is_symlink() {
            budget.charge(4096)?;
            std::os::unix::fs::symlink(
                nix::fcntl::readlinkat(directory, name.as_os_str())?,
                target,
            )?;
        } else {
            return Err(io::Error::other(
                "build-input: unsupported context file type",
            ));
        }
    }
    fs::set_permissions(to, directory.metadata()?.permissions())
}

fn write_job(input: &Path, job: &Job<'_>, budget: &mut InputBudget) -> io::Result<()> {
    let bytes = serde_json::to_vec(job)?;
    if bytes.len() > 1 << 20 {
        return Err(io::Error::other("resource: builder job metadata"));
    }
    budget.charge(bytes.len() as u64)?;
    fs::write(input.join("job.json"), bytes)
}

// Snapshot directory modes belong in the input disk. Restore owner access only
// after teardown, so ordinary cleanup can retire a caller's read-only tree.
pub(super) fn writable_snapshot(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() {
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
        )?;
        for entry in fs::read_dir(path)? {
            writable_snapshot(&entry?.path())?;
        }
    }
    Ok(())
}

fn source_policy(input: &Path, identifier: &str) -> io::Result<()> {
    let policy = serde_json::json!({"version":1,"rules":[{"action":"DENY","selector":{"identifier":identifier,"match_type":"WILDCARD"}}]});
    fs::write(
        input.join("source-policy.json"),
        serde_json::to_vec(&policy)?,
    )
}
