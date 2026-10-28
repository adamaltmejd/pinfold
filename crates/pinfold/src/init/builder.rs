//! The trusted Linux image job. Host inputs are read-only; output is bytes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{self, Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use nix::libc;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::firecracker::{self, FAILED, READY, write_frame};

const PATH: &str = "/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const PROXY: &str = "http://127.0.0.1:3128";
const SOCKET: &str = "unix:///run/buildkit/buildkitd.sock";
const LOG_BYTES: usize = 64 * 1024;
// Candidate admission cap; final runtime policy still needs boundary evidence.
const OUTPUT_BYTES: u64 = 512 * 1024 * 1024;
const JSON_BYTES: u64 = 1024 * 1024;
const DEFINITION_OPS: usize = 4096;
const CAPABILITIES: u64 = 0xa82425fb;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    phase: Phase,
    cache: bool,
    labels: BTreeMap<String, String>,
    bases: Vec<Base>,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Discover,
    Build,
    Convert,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Base {
    reference: String,
    digest: String,
    directory: String,
}

pub(super) fn boot() -> ! {
    if process::id() != 1 {
        eprintln!("pinfold init builder runs only as PID 1");
        process::exit(1);
    }
    let mut host = match firecracker::connect_builder() {
        Ok(host) => host,
        Err(error) => {
            eprintln!("pinfold init builder: {error}");
            process::exit(1);
        }
    };
    let result = run(&mut host);
    if let Err(error) = result {
        let _ = fs::write("/sys/fs/cgroup/work/cgroup.kill", "1");
        nix::unistd::sync();
        let mut detail = error.to_string();
        let start = detail.ceil_char_boundary(detail.len().saturating_sub(LOG_BYTES));
        detail.drain(..start);
        let _ = write_frame(&mut host, FAILED, detail.as_bytes());
        process::exit(1);
    }
    process::exit(0);
}

fn run(host: &mut UnixStream) -> io::Result<()> {
    let relay = firecracker::prepare_builder(host)?;
    let mut bytes = Vec::new();
    File::open("/input/job.json")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(io::Error::other("resource: builder job metadata"));
    }
    let job: Job = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if relay == (job.phase == Phase::Convert) {
        return Err(io::Error::other("invalid builder egress role"));
    }
    capabilities()?;
    if relay {
        firecracker::relay_builder()?;
    }
    fs::create_dir("/run/docker")?;
    match fs::remove_dir_all("/work/oci") {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let log = Arc::new(Mutex::new(Tail::default()));
    let daemon_log = Arc::new(Mutex::new(Tail::default()));
    let mut daemon =
        Daemon::start(&job, Arc::clone(&daemon_log)).map_err(|error| failed(error, &daemon_log))?;
    let built = build(&job, Arc::clone(&log));
    let stopped = daemon.stop();
    built.map_err(|error| failed(error, &log))?;
    stopped.map_err(|error| failed(error, &daemon_log))?;
    let output = match job.phase {
        Phase::Discover => Path::new("/work/discovery.json"),
        Phase::Build => Path::new("/work/oci"),
        Phase::Convert => {
            convert(Arc::clone(&log)).map_err(|error| failed(error, &log))?;
            Path::new("/work/root.ext4")
        }
    };
    // No RUN descendant may keep changing an output after its daemon exited.
    stop_work()?;
    nix::unistd::sync();
    write_frame(host, READY, &[])?;
    match job.phase {
        Phase::Discover => {
            send(host, b'I', output)?;
            fs::remove_file(output)?;
        }
        Phase::Convert => send(host, b'D', output)?,
        Phase::Build => {
            send(host, b'I', &output.join("index.json"))?;
            let mut blobs =
                fs::read_dir(output.join("blobs/sha256"))?.collect::<Result<Vec<_>, _>>()?;
            blobs.sort_by_key(|entry| entry.file_name());
            for blob in blobs {
                send(host, b'B', &blob.path())?;
            }
            fs::remove_dir_all(output)?;
        }
    }
    nix::unistd::sync();
    nix::mount::umount("/input")?;
    nix::mount::umount("/work")?;
    write_frame(host, b'E', b"{}")?;
    host.flush()
}

fn capabilities() -> io::Result<()> {
    // This role intentionally retains mount privileges. Agent work still uses
    // the separate entry point's empty bounding set.
    for capability in 0..64 {
        if CAPABILITIES & (1_u64 << capability) != 0 {
            continue;
        }
        // SAFETY: prctl takes integer arguments and changes this process only.
        if unsafe { libc::prctl(libc::PR_CAPBSET_DROP, capability, 0, 0, 0) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINVAL) && capability > 0 {
                break;
            }
            return Err(error);
        }
    }
    // SAFETY: clear all ambient capabilities and prohibit gaining privilege.
    if unsafe {
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        )
    } < 0
        || unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = Header {
        version: 0x2008_0522,
        pid: 0,
    };
    let data = [
        Data {
            effective: CAPABILITIES as u32,
            permitted: CAPABILITIES as u32,
            inheritable: 0,
        },
        Data {
            effective: (CAPABILITIES >> 32) as u32,
            permitted: (CAPABILITIES >> 32) as u32,
            inheritable: 0,
        },
    ];
    // SAFETY: capset reads the versioned header and two data words.
    if unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn command(program: &str, proxy: bool) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", PATH)
        .env("HOME", "/run")
        .env("DOCKER_CONFIG", "/run/docker")
        .env("BUILDKIT_HOST", SOCKET)
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .stdin(Stdio::null());
    if proxy {
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            command.env(name, PROXY);
        }
    }
    // SAFETY: only async-signal-safe calls run between fork and exec. Writing
    // zero moves the writing child into the cgroup, including its descendants.
    unsafe {
        command.pre_exec(|| {
            let file = libc::open(
                c"/sys/fs/cgroup/work/service/cgroup.procs".as_ptr(),
                libc::O_WRONLY | libc::O_CLOEXEC,
            );
            if file < 0 {
                return Err(io::Error::last_os_error());
            }
            let written = libc::write(file, b"0".as_ptr().cast(), 1);
            let error = io::Error::last_os_error();
            libc::close(file);
            if written == 1 { Ok(()) } else { Err(error) }
        });
    }
    command
}

#[derive(Default)]
struct Tail {
    bytes: VecDeque<u8>,
    newlines: usize,
}

impl Tail {
    fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.bytes.push_back(byte);
            self.newlines += usize::from(byte == b'\n');
            while self.bytes.len() > LOG_BYTES
                || self.newlines + usize::from(self.bytes.back() != Some(&b'\n')) > 40
            {
                self.newlines -= usize::from(self.bytes.pop_front() == Some(b'\n'));
            }
        }
    }

    fn text(&self) -> String {
        let bytes: Vec<_> = self.bytes.iter().copied().collect();
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        let start = text.ceil_char_boundary(text.len().saturating_sub(LOG_BYTES));
        text.drain(..start);
        text
    }
}

fn failed(error: io::Error, log: &Mutex<Tail>) -> io::Error {
    let text = log
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .text();
    if text.is_empty() {
        return error;
    }
    let mut tail = Tail::default();
    tail.push(text.as_bytes());
    if !text.ends_with('\n') {
        tail.push(b"\n");
    }
    // A resource refusal must retain its reason after progress diagnostics.
    tail.push(error.to_string().as_bytes());
    io::Error::other(tail.text())
}

struct Daemon {
    child: Child,
    reader: Option<thread::JoinHandle<io::Result<()>>>,
    stopped: bool,
}

impl Daemon {
    fn start(job: &Job, log: Arc<Mutex<Tail>>) -> io::Result<Self> {
        let (mut reader, writer) = io::pipe()?;
        let mut command = command("buildkitd", job.phase != Phase::Convert);
        command
            .args([
                "--addr",
                SOCKET,
                "--root",
                "/work/cache",
                "--oci-worker-snapshotter=overlayfs",
                "--oci-worker-net=host",
                "--containerd-worker=false",
                "--oci-worker-gc=false",
            ])
            .stdout(writer.try_clone()?)
            .stderr(writer);
        let child = command.spawn()?;
        drop(command);
        let (ready, started) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let mut pending = Some(ready);
            let mut line = VecDeque::new();
            let mut buffer = [0; 4096];
            loop {
                let count = reader.read(&mut buffer)?;
                if count == 0 {
                    return Ok(());
                }
                log.lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .push(&buffer[..count]);
                for &byte in &buffer[..count] {
                    line.push_back(byte);
                    if line.len() > 4096 {
                        line.pop_front();
                    }
                    if byte == b'\n' {
                        if pending.is_some()
                            && line
                                .make_contiguous()
                                .windows(b"running server".len())
                                .any(|word| word == b"running server")
                        {
                            let _ = pending.take().unwrap().send(());
                        }
                        line.clear();
                    }
                }
            }
        });
        let mut daemon = Self {
            child,
            reader: Some(reader),
            stopped: false,
        };
        if started.recv().is_err() {
            let _ = daemon.stop();
            return Err(io::Error::other("BuildKit exited before ready"));
        }
        Ok(daemon)
    }

    fn stop(&mut self) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        match nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(self.child.id() as i32),
            nix::sys::signal::Signal::SIGTERM,
        ) {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
            Err(error) => return Err(error.into()),
        }
        self.child.wait()?;
        stop_work()?;
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| io::Error::other("BuildKit log reader panicked"))??;
        }
        Ok(())
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn logged(mut command: Command, log: Arc<Mutex<Tail>>) -> io::Result<()> {
    let (mut reader, writer) = io::pipe()?;
    command.stdout(writer.try_clone()?).stderr(writer);
    let child = command.spawn();
    drop(command);
    let mut child = child?;
    let mut buffer = [0; 4096];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        if count == 0 {
            break;
        }
        log.lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(&buffer[..count]);
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("image job: {status}")))
    }
}

fn stop_work() -> io::Result<()> {
    fs::write("/sys/fs/cgroup/work/cgroup.kill", "1")?;
    let mut events = File::open("/sys/fs/cgroup/work/cgroup.events")?;
    loop {
        events.seek(SeekFrom::Start(0))?;
        let mut state = String::new();
        events.read_to_string(&mut state)?;
        if state.lines().any(|line| line == "populated 0") {
            return Ok(());
        }
        let mut watch = [nix::poll::PollFd::new(
            events.as_fd(),
            nix::poll::PollFlags::POLLPRI | nix::poll::PollFlags::POLLERR,
        )];
        match nix::poll::poll(&mut watch, nix::poll::PollTimeout::NONE) {
            Ok(_) | Err(nix::errno::Errno::EINTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn build(job: &Job, log: Arc<Mutex<Tail>>) -> io::Result<()> {
    let mut build = command("buildctl", job.phase != Phase::Convert);
    build.args([
        "build",
        "--source-policy-file",
        "/input/source-policy.json",
        "--frontend",
        "dockerfile.v0",
        // Presence suppresses caller #syntax forwarding in the pinned frontend.
        "--opt",
        "cmdline=",
        "--local",
        "context=/input/context",
        "--local",
        "dockerfile=/input/recipe",
        "--opt",
        "build-arg:TARGETARCH=amd64",
        "--opt",
        "platform=linux/amd64",
        "--opt",
        "cgroup-parent=/work/jobs",
    ]);
    if !job.cache {
        build.arg("--no-cache");
    }
    if job.phase != Phase::Convert {
        build.args(["--opt", "image-resolve-mode=pull"]);
        if job.phase == Phase::Build {
            build.args(["--opt", "attest:provenance=mode=min,version=v1"]);
        }
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            build.arg("--opt").arg(format!("build-arg:{name}={PROXY}"));
        }
        for name in ["NO_PROXY", "no_proxy"] {
            build.arg("--opt").arg(format!("build-arg:{name}="));
        }
    }
    for (key, value) in &job.labels {
        build.arg("--opt").arg(format!("label:{key}={value}"));
    }
    for (index, base) in job.bases.iter().enumerate() {
        if !base
            .directory
            .strip_prefix("/input/bases/")
            .is_some_and(|value| {
                !value.is_empty()
                    && value.len() <= 4
                    && value.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(io::Error::other("invalid local base directory"));
        }
        build
            .arg("--oci-layout")
            .arg(format!("base{index}={}", base.directory));
        build.arg("--opt").arg(format!(
            "context:{}=oci-layout://base{index}@{}",
            base.reference, base.digest
        ));
    }
    if job.phase == Phase::Discover {
        build.args(["--opt", "requestid=frontend.convertllb"]);
        let bytes = capture_discovery(build, log)?;
        let result = bytes
            .strip_prefix(b"result.json\n")
            .ok_or_else(|| io::Error::other("discovery: missing native frontend result"))?;
        let result: serde_json::Value = serde_json::from_slice(result)
            .map_err(|_| io::Error::other("discovery: invalid native frontend result"))?;
        let definition = result
            .get("def")
            .and_then(serde_json::Value::as_object)
            .filter(|definition| definition.len() <= DEFINITION_OPS)
            .ok_or_else(|| io::Error::other("resource: discovery definition"))?;
        let mut sources = BTreeSet::new();
        for operation in definition.values() {
            if let Some(source) = operation.get("source") {
                let identifier = source
                    .get("identifier")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| io::Error::other("discovery: invalid native source"))?;
                if identifier.starts_with("oci-layout://")
                    || identifier.starts_with("docker-image://")
                {
                    sources.insert(identifier.to_string());
                }
            }
        }
        let metadata = serde_json::to_vec(&serde_json::json!({"sources": sources}))?;
        if metadata.len() as u64 > JSON_BYTES {
            return Err(io::Error::other("resource: discovery sources"));
        }
        return fs::write("/work/discovery.json", metadata);
    }
    build.arg("--output").arg(if job.phase == Phase::Convert {
        "type=tar,dest=/work/root.tar"
    } else {
        "type=oci,dest=/work/oci,tar=false,oci-mediatypes=true,oci-artifact=true,name=localhost/pinfold/build-result:latest"
    });
    logged(build, log)
}

fn capture_discovery(mut command: Command, log: Arc<Mutex<Tail>>) -> io::Result<Vec<u8>> {
    let (mut reader, writer) = io::pipe()?;
    command.stdout(Stdio::piped()).stderr(writer);
    let child = command.spawn();
    drop(command);
    let mut child = child?;
    let errors = thread::spawn(move || -> io::Result<()> {
        let mut buffer = [0; 4096];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                return Ok(());
            }
            log.lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(&buffer[..count]);
        }
    });
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("discovery stdout is piped")
        .take(JSON_BYTES + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > JSON_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        let _ = errors.join();
        read?;
        return Err(io::Error::other("resource: discovery JSON"));
    }
    let status = child.wait()?;
    errors
        .join()
        .map_err(|_| io::Error::other("discovery log reader panicked"))??;
    if !status.success() {
        return Err(io::Error::other(format!("discovery: {status}")));
    }
    Ok(bytes)
}

fn convert(log: Arc<Mutex<Tail>>) -> io::Result<()> {
    fs::create_dir("/work/tree")?;
    let mut tar = command("tar", false);
    tar.args([
        "--numeric-owner",
        "--same-owner",
        "--same-permissions",
        "--xattrs",
        "--xattrs-include=*",
        "--acls",
        "-xpf",
        "/work/root.tar",
        "-C",
        "/work/tree",
    ]);
    logged(tar, Arc::clone(&log))?;
    let (kib, entries) = tree_size(Path::new("/work/tree"))?;
    let size = kib
        .checked_add(kib / 10)
        .and_then(|size| size.checked_add(entries / 4))
        .and_then(|size| size.checked_add(16384))
        .and_then(|size| size.checked_mul(1024))
        .filter(|&size| size <= OUTPUT_BYTES)
        .ok_or_else(|| io::Error::other("resource: converted root size"))?;
    let file = File::create("/work/root.ext4")?;
    file.set_len(size)?;
    drop(file);
    let mut mkfs = command("mkfs.ext4", false);
    mkfs.args(["-q", "-F", "-m", "0", "-O", "^has_journal", "-N"])
        .arg((entries + 1024).to_string())
        .args(["-d", "/work/tree", "/work/root.ext4"]);
    logged(mkfs, log)
}

fn tree_size(path: &Path) -> io::Result<(u64, u64)> {
    let (blocks, entries) = tree_blocks(path, &mut BTreeSet::new())?;
    Ok((blocks.div_ceil(2), entries))
}

fn tree_blocks(path: &Path, seen: &mut BTreeSet<(u64, u64)>) -> io::Result<(u64, u64)> {
    let metadata = fs::symlink_metadata(path)?;
    if !seen.insert((metadata.dev(), metadata.ino())) {
        return Ok((0, 1));
    }
    let (mut blocks, mut entries) = (metadata.blocks(), 1_u64);
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            let (child_blocks, child_entries) = tree_blocks(&entry?.path(), seen)?;
            blocks = blocks
                .checked_add(child_blocks)
                .ok_or_else(|| io::Error::other("resource: root tree size"))?;
            entries = entries
                .checked_add(child_entries)
                .ok_or_else(|| io::Error::other("resource: root entry count"))?;
        }
    }
    Ok((blocks, entries))
}

fn send(host: &mut UnixStream, kind: u8, path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > OUTPUT_BYTES {
        return Err(io::Error::other("resource: image output file"));
    }
    let mut file = File::open(path)?;
    let mut sha = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > OUTPUT_BYTES {
            return Err(io::Error::other("resource: image output file"));
        }
        sha.update(&buffer[..count]);
    }
    if size != metadata.len() {
        return Err(io::Error::other("image output changed"));
    }
    let digest = format!("sha256:{}", hex(&sha.finalize()));
    let header = serde_json::to_vec(&serde_json::json!({"digest": digest, "size": size}))?;
    write_frame(host, kind, &header)?;
    file.seek(SeekFrom::Start(0))?;
    let sent = io::copy(&mut file, host)?;
    if sent != size {
        return Err(io::Error::other("image output changed"));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("writing to String cannot fail");
    }
    text
}
