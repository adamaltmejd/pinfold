//! Host release checks and replacement of the installed executable.

use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Seek, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};

use crate::{cli, core, dirs};

const REPOSITORY: &str = "https://api.github.com/repos/adamaltmejd/pinfold";
const DOWNLOADS: &str = "https://github.com/adamaltmejd/pinfold/releases/download";
const DAY: u64 = 24 * 60 * 60;
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

#[derive(Default, Deserialize, Serialize)]
struct Check {
    checked_at: u64,
    profile: String,
}

fn version(value: &str) -> io::Result<[u64; 3]> {
    let parts = value
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>();
    parts
        .ok()
        .and_then(|parts| parts.try_into().ok())
        .ok_or_else(|| io::Error::other(format!("invalid stable release version: {value}")))
}

fn latest(timeout: u64) -> io::Result<String> {
    let bytes = core::download::bytes(
        &format!("{REPOSITORY}/releases/latest"),
        Duration::from_secs(timeout),
        1024 * 1024,
    )?;
    let release: Release = serde_json::from_slice(&bytes)?;
    let value = release
        .tag_name
        .strip_prefix('v')
        .ok_or_else(|| io::Error::other("release tag must start with v"))?;
    Ok(value.to_string())
}

fn development(path: &Path) -> bool {
    path.components().any(|part| part.as_os_str() == "target")
        && path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "debug" || name == "release")
}

/// Checks are bounded, silent on failure, and confined to interactive launches.
pub fn notice(verb: &str, args: &[String]) {
    if !matches!(verb, "pi" | "attach")
        || std::env::var_os("PINFOLD_NO_UPDATE_CHECK").as_deref() == Some("1".as_ref())
        || !io::stdin().is_terminal()
        || !io::stdout().is_terminal()
        || !io::stderr().is_terminal()
        || args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return;
    }
    let _ = notice_due(false);
}

fn notice_due(record_only: bool) -> io::Result<()> {
    if development(&std::env::current_exe()?) {
        return Ok(());
    }
    let cache = dirs::cache_dir()?;
    fs::create_dir_all(&cache)?;
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(cache.join("update-check"))?;
    let Ok(mut file) = Flock::lock(file, FlockArg::LockExclusiveNonblock) else {
        return Ok(());
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut previous: Check = serde_json::from_slice(&bytes).unwrap_or_default();
    if record_only && !previous.profile.is_empty() {
        return Ok(());
    }
    let profile = core::profile::builtin_hash()?;
    let profile_changed = !previous.profile.is_empty() && previous.profile != profile;
    let now = core::now();
    let due = !record_only && now.saturating_sub(previous.checked_at) >= DAY;
    if previous.profile == profile && !due {
        return Ok(());
    }
    previous.profile = profile;
    if due {
        previous.checked_at = now;
    }
    // Claim the day before the request, including when offline.
    file.rewind()?;
    file.set_len(0)?;
    file.write_all(&serde_json::to_vec(&previous)?)?;
    if profile_changed && !record_only {
        eprintln!(
            "pinfold: bundled-profiles-changed: bundled profiles changed; saved project settings and user profiles were kept. To inspect a bundled profile, run `pinfold profile new fresh-profile --from NAME --builtin` (NAME: default, documents or full)."
        );
    }
    if !due {
        return Ok(());
    }
    let release = latest(1)?;
    if version(&release)? > version(VERSION)? {
        eprintln!("pinfold {release} is available (installed: {VERSION}). Run `pinfold update`.");
    }
    Ok(())
}

/// Lock the executable itself, so different XDG roots still coordinate.
/// The host owner retains this lock for the lifetime of its box.
pub fn working() -> io::Result<Flock<File>> {
    let (path, lock) = executable_lock(FlockArg::LockSharedNonblock)?;
    loaded_version(&path)?;
    Ok(lock)
}

fn loaded_version(path: &Path) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        use std::os::unix::process::CommandExt;
        // macOS reports the executable's path, not its loaded inode. A process
        // delayed until after replacement must not start with its old pins.
        let output = Command::new(path)
            .arg0("pinfold")
            .arg("--version")
            .output()?;
        if !output.status.success() || output.stdout != format!("pinfold {VERSION}\n").as_bytes() {
            return Err(io::Error::other("executable-replaced: rerun the command"));
        }
    }
    Ok(())
}

fn executable_lock(mode: FlockArg) -> io::Result<(PathBuf, Flock<File>)> {
    let path = fs::canonicalize(std::env::current_exe()?)?;
    let file = File::open(&path)?;
    let lock = Flock::lock(file, mode).map_err(|(_, error)| {
        if error == nix::errno::Errno::EAGAIN {
            io::Error::other(
                "update-busy: another pinfold command is active; close boxes and retry",
            )
        } else {
            io::Error::from(error)
        }
    })?;
    let opened = lock.metadata()?;
    let installed = fs::metadata(&path)?;
    if (opened.dev(), opened.ino()) != (installed.dev(), installed.ino()) {
        return Err(io::Error::other("executable-replaced: rerun the command"));
    }
    Ok((path, lock))
}

fn target() -> io::Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        _ => Err(io::Error::other("no release binary for this host")),
    }
}

pub fn run(args: &[String]) -> io::Result<i32> {
    let check_only = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err(cli::usage("update", "update takes only --check")),
    };
    let release = latest(15)?;
    if version(&release)? <= version(VERSION)? {
        println!("pinfold {VERSION} is up to date.");
        return Ok(0);
    }
    if check_only {
        println!("pinfold {release} is available (installed: {VERSION}). Run `pinfold update`.");
        return Ok(0);
    }
    let path = fs::canonicalize(std::env::current_exe()?)?;
    if development(&path) {
        return Err(io::Error::other(
            "development-build: install a release binary outside the Cargo target directory first",
        ));
    }
    // Fail promptly for an active box; recheck after downloading as well.
    drop(installation_lock(&path)?);
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("executable has no parent"))?;
    let staging = parent.join(format!(".pinfold-update-{}", std::process::id()));
    fs::create_dir(&staging).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot stage update beside {}: {error}", path.display()),
        )
    })?;
    let result = install(&release, &path, &staging);
    let _ = fs::remove_dir_all(&staging);
    result?;
    println!(
        "Updated pinfold {VERSION} to {release} at {}.",
        path.display()
    );
    Ok(0)
}

fn install(release: &str, path: &Path, staging: &Path) -> io::Result<()> {
    let name = format!("pinfold-{release}-{}", target()?);
    let base = format!("{DOWNLOADS}/v{release}");
    let sums = staging.join("SHA256SUMS");
    let binary = staging.join("pinfold");
    eprintln!("Downloading pinfold {release}…");
    core::download::file(
        &format!("{base}/SHA256SUMS"),
        &sums,
        Duration::from_secs(300),
        1024 * 1024,
        None,
    )?;
    core::download::file(
        &format!("{base}/{name}"),
        &binary,
        Duration::from_secs(300),
        256 * 1024 * 1024,
        None,
    )?;
    let sums = fs::read_to_string(sums)?;
    let mut checksums = sums.lines().filter_map(|line| {
        let (digest, filename) = line.split_once(char::is_whitespace)?;
        (filename.trim_start().trim_start_matches('*') == name).then_some(digest)
    });
    let expected = checksums
        .next()
        .ok_or_else(|| io::Error::other("checksum-mismatch: asset absent from SHA256SUMS"))?;
    if checksums.next().is_some() || expected != core::download::sha256_file(&binary)? {
        return Err(io::Error::other(
            "checksum-mismatch: downloaded binary differs from SHA256SUMS",
        ));
    }
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
    File::open(&binary)?.sync_all()?;
    let _lock = installation_lock(path)?;
    // Preserve a missing old baseline before replacement. Notice bookkeeping
    // must not make a verified installation fail.
    let _ = notice_due(true);
    fs::rename(binary, path)
}

fn installation_lock(path: &Path) -> io::Result<Flock<File>> {
    let (installed, lock) = executable_lock(FlockArg::LockExclusiveNonblock)?;
    if installed != path {
        return Err(io::Error::other("executable-replaced: rerun the update"));
    }
    loaded_version(path)?;
    // Owners from releases before executable locking still hold their pid file.
    for entry in dirs::entries(&dirs::boxes_dir()?)? {
        if entry.file_type()?.is_dir() && core::clean::owner_alive(&entry.path()) {
            return Err(io::Error::other(
                "running-boxes: close pinfold boxes before updating",
            ));
        }
    }
    Ok(lock)
}
