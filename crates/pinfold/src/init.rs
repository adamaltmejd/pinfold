//! `pinfold init`: PID 1 in a box.
//!
//! It reports readiness on stdout, has the kernel reap every child
//! reparented to it, and exits on SIGTERM. PID 1 has no default signal
//! actions, so waiting for the blocked signal is what makes a stop possible.
//! When the transport carries the proxy socket in, it also relays
//! `127.0.0.1:3128` to that socket, because clients only know how to reach a
//! proxy over TCP.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::Duration;

use nix::sys::signal::{SigHandler, SigSet, Signal};

/// Guest values travel under names the host runtime does not interpret.
/// Hex preserves arbitrary environment bytes through runtime JSON encoding.
pub const BOX_ENV_PREFIX: &str = "PINFOLD_BOX_ENV_";

const RELAY_LISTEN: &str = "127.0.0.1:3128";

/// Run as PID 1 until SIGTERM. `args` is the optional guest path of the
/// carried proxy socket; without it the box has no egress and no relay.
pub fn run(args: &[String]) -> ! {
    match args.first().map(String::as_str) {
        Some("exec") => exec(&args[1..]),
        Some("setup") => exit_with("setup", setup(&args[1..])),
        Some("receive") => exit_with("receive", receive(&args[1..])),
        _ => {}
    }

    // Blocked before the relay thread exists, which inherits the mask, so
    // both signals stay pending for `wait` below. A blocked signal reaches
    // PID 1 even with no handler installed.
    let mut terminate = SigSet::empty();
    terminate.add(Signal::SIGTERM);
    terminate.add(Signal::SIGINT);
    terminate.thread_block().expect("block SIGTERM and SIGINT");
    // With SIGCHLD ignored the kernel reaps every exited child, the ones
    // reparented to PID 1 included. init forks nothing, so no wait of its
    // own can lose a status to this.
    // Safety: SIG_IGN runs no code.
    unsafe { nix::sys::signal::signal(Signal::SIGCHLD, SigHandler::SigIgn) }
        .expect("ignore SIGCHLD");

    if let Some(target) = args.first() {
        let target = PathBuf::from(target);
        // Ready is reported after the relay listens, so a caller that sees
        // ready can reach the proxy at once.
        let listener = TcpListener::bind(RELAY_LISTEN).unwrap_or_else(|error| {
            eprintln!("pinfold init: bind {RELAY_LISTEN}: {error}");
            process::exit(1)
        });
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let target = target.clone();
                thread::spawn(move || relay(client, &target));
            }
        });
    }

    println!("ready");
    io::stdout().flush().expect("write ready");

    let _ = terminate.wait();
    process::exit(0);
}

/// `init exec -- ARGV...`: raise this process's `oom_score_adj` to 1000 and
/// become ARGV. Every process pinfold starts in a box other than init runs
/// through here, so the kernel's OOM killer takes one of them before init.
fn exec(args: &[String]) -> ! {
    let (program, args) = match args {
        [separator, program, args @ ..] if separator == "--" => (program, args),
        _ => {
            eprintln!("pinfold init exec: usage: pinfold init exec -- argv...");
            process::exit(1);
        }
    };
    // Raising a process's own OOM score needs no privilege; a rootless
    // container cannot lower it again.
    if let Err(error) = fs::write("/proc/self/oom_score_adj", "1000") {
        eprintln!("pinfold init exec: oom_score_adj: {error}");
        process::exit(1);
    }
    let mut command = process::Command::new(program);
    command.args(args);
    let mut restored = Vec::new();
    for (key, value) in std::env::vars_os() {
        let Some(name) = key
            .to_str()
            .and_then(|key| key.strip_prefix(BOX_ENV_PREFIX))
        else {
            continue;
        };
        command.env_remove(&key);
        let value = value.as_bytes();
        let decoded: Option<Vec<u8>> = value
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                std::str::from_utf8(pair)
                    .ok()
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            })
            .collect();
        let Some(decoded) = decoded.filter(|_| value.len().is_multiple_of(2)) else {
            eprintln!("pinfold init exec: invalid encoded box environment for {name}");
            process::exit(1);
        };
        restored.push((name.to_string(), OsString::from_vec(decoded)));
    }
    // Restore after removing every transport name: a guest variable may
    // itself start with the prefix and must survive that removal pass.
    command.envs(restored);
    let error = command.exec();
    eprintln!("pinfold init exec: {program}: {error}");
    // The shell's convention for a command that could not be run.
    process::exit(if error.kind() == io::ErrorKind::NotFound {
        127
    } else {
        126
    });
}

fn exit_with(verb: &str, result: io::Result<()>) -> ! {
    match result {
        Ok(()) => process::exit(0),
        Err(error) => {
            eprintln!("pinfold init {verb}: {error}");
            process::exit(1)
        }
    }
}

/// `init setup [--own UID:GID DIR]…`, run as root with only `CAP_CHOWN` on
/// Apple: give each new disk's root to the box user.
fn setup(args: &[String]) -> io::Result<()> {
    let mut args = args.iter();
    while let Some(option) = args.next() {
        let (Some(owner), Some(dir)) = (args.next(), args.next()) else {
            return Err(io::Error::other(
                "usage: pinfold init setup [--own UID:GID DIR]...",
            ));
        };
        let owner = (option == "--own")
            .then(|| owner.split_once(':'))
            .flatten()
            .and_then(|(uid, gid)| Some((uid.parse().ok()?, gid.parse().ok()?)));
        let Some((uid, gid)) = owner else {
            return Err(io::Error::other(
                "usage: pinfold init setup [--own UID:GID DIR]...",
            ));
        };
        nix::unistd::chown(
            dir.as_str(),
            Some(nix::unistd::Uid::from_raw(uid)),
            Some(nix::unistd::Gid::from_raw(gid)),
        )?;
    }
    Ok(())
}

/// One entry of a `from` copy stream: a tag byte, the relative path as a
/// u32 length and bytes, then the payload: nothing for a directory, a u64
/// length and the bytes for a file, a u32 length and the target for a
/// symlink. [`STREAM_END`] ends the stream.
pub const STREAM_DIR: u8 = b'd';
pub const STREAM_FILE: u8 = b'f';
pub const STREAM_EXECUTABLE: u8 = b'x';
pub const STREAM_SYMLINK: u8 = b'l';
pub const STREAM_END: u8 = 0;

/// `init receive DIR`: write the copy stream on stdin below DIR, as the
/// box user. Directories and executables get 755, other files 644.
fn receive(args: &[String]) -> io::Result<()> {
    use std::io::Read;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let [root] = args else {
        return Err(io::Error::other("usage: pinfold init receive DIR"));
    };
    let root = Path::new(root);
    let mut input = io::BufReader::new(io::stdin().lock());
    fn read_bytes(input: &mut impl Read, length: usize) -> io::Result<Vec<u8>> {
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    loop {
        let tag = read_bytes(&mut input, 1)?[0];
        if tag == STREAM_END {
            return Ok(());
        }
        let length = u32::from_be_bytes(read_bytes(&mut input, 4)?.try_into().unwrap()) as usize;
        let relative = PathBuf::from(OsString::from_vec(read_bytes(&mut input, length)?));
        if !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return Err(io::Error::other(format!(
                "stream path {} is not plain",
                relative.display()
            )));
        }
        let path = root.join(&relative);
        match tag {
            STREAM_DIR => {
                fs::DirBuilder::new().mode(0o755).create(&path)?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
            }
            STREAM_FILE | STREAM_EXECUTABLE => {
                let mode = if tag == STREAM_EXECUTABLE {
                    0o755
                } else {
                    0o644
                };
                let size = u64::from_be_bytes(read_bytes(&mut input, 8)?.try_into().unwrap());
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .open(&path)?;
                let copied = io::copy(&mut (&mut input).take(size), &mut file)?;
                if copied != size {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                file.set_permissions(fs::Permissions::from_mode(mode))?;
            }
            STREAM_SYMLINK => {
                let length =
                    u32::from_be_bytes(read_bytes(&mut input, 4)?.try_into().unwrap()) as usize;
                let target = OsString::from_vec(read_bytes(&mut input, length)?);
                std::os::unix::fs::symlink(target, &path)?;
            }
            _ => return Err(io::Error::other(format!("unknown stream tag {tag}"))),
        }
    }
}

/// Guest upload EOF preserves the response path. Proxy EOF completes the
/// exchange and interrupts a guest that kept its upload half open.
fn relay(mut client: TcpStream, target: &Path) -> io::Result<()> {
    let mut server = UnixStream::connect(target)?;
    client.set_write_timeout(Some(Duration::from_secs(300)))?;
    server.set_write_timeout(Some(Duration::from_secs(300)))?;
    let mut client_reader = client.try_clone()?;
    let mut server_writer = server.try_clone()?;
    let up = thread::spawn(move || {
        if io::copy(&mut client_reader, &mut server_writer).is_err() {
            let _ = client_reader.shutdown(Shutdown::Both);
            let _ = server_writer.shutdown(Shutdown::Both);
        } else {
            let _ = server_writer.shutdown(Shutdown::Write);
        }
    });
    let _ = io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Both);
    let _ = server.shutdown(Shutdown::Both);
    let _ = up.join();
    Ok(())
}
