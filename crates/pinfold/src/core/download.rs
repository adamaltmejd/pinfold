//! Bounded host downloads for pinned artifacts and verified updates.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use nix::poll::PollFlags;
use sha2::{Digest, Sha256};

use crate::core::pipe::{nonblocking, remaining, wait_ready};

pub fn bytes(url: &str, timeout: Duration, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    transfer(url, timeout, limit, None, &mut bytes)?;
    Ok(bytes)
}

pub fn file(
    url: &str,
    path: &Path,
    timeout: Duration,
    limit: u64,
    cancel: Option<&AtomicBool>,
) -> io::Result<()> {
    transfer(url, timeout, limit, cancel, &mut File::create(path)?)
}

fn transfer(
    url: &str,
    timeout: Duration,
    limit: u64,
    cancel: Option<&AtomicBool>,
    output: &mut impl Write,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--location",
            "--silent",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
        ])
        .arg(timeout.as_secs_f64().min(15.0).to_string())
        .arg("--max-time")
        .arg(timeout.as_secs_f64().to_string())
        .arg("--max-filesize")
        .arg(limit.to_string())
        .args(["--speed-limit", "1", "--speed-time", "30"])
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| {
        let mut source = child.stdout.take().expect("curl stdout is piped");
        nonblocking(&source, "download")?;
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            remaining(deadline, cancel, "download")?;
            match source.read(&mut buffer) {
                Ok(0) => {
                    if let Some(status) = child.try_wait()? {
                        return if status.success() {
                            Ok(())
                        } else {
                            Err(io::Error::other(format!("download failed: {status}")))
                        };
                    }
                    wait_ready(None, PollFlags::empty(), deadline, cancel, "download")?;
                }
                Ok(read) => {
                    total += read as u64;
                    if total > limit {
                        return Err(io::Error::other("download exceeds byte limit"));
                    }
                    output.write_all(&buffer[..read])?;
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait_ready(
                        Some(source.as_fd()),
                        PollFlags::POLLIN,
                        deadline,
                        cancel,
                        "download",
                    )?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
