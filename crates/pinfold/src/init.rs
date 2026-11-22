//! `pinfold init`: PID 1 in a box.
//!
//! It reports readiness on stdout, has the kernel reap every child
//! reparented to it, and exits on SIGTERM. PID 1 has no default signal
//! actions, so waiting for the blocked signal is what makes a stop possible.
//! When the transport carries the proxy socket in, it also relays
//! `127.0.0.1:3128` to that socket, because clients only know how to reach a
//! proxy over TCP. On Linux, `init boot` is PID 1 of a Firecracker box
//! instead (`init/firecracker.rs`).

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{SigHandler, SigSet, Signal};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "init/builder.rs"]
mod builder;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[path = "init/exec_wire.rs"]
pub(crate) mod exec_wire;
#[path = "init/export.rs"]
pub(crate) mod export;
#[cfg(target_os = "linux")]
#[path = "init/firecracker.rs"]
pub(crate) mod firecracker;
#[path = "formatting.rs"]
pub(crate) mod formatting;
#[cfg(target_os = "linux")]
#[path = "init/session.rs"]
mod session;
#[cfg(target_os = "linux")]
#[path = "init/session_io.rs"]
mod session_io;

/// The exec service hands box values to `init exec` under these names, so a
/// box name equal to [`EXEC_METADATA`] still arrives. Hex keeps every byte.
pub const BOX_ENV_PREFIX: &str = "PINFOLD_BOX_ENV_";

/// The abstract socket on which Apple's PID 1 serves the box's environment.
/// It has no file, and PID 1 holds the name from before ready.
#[cfg(target_os = "linux")]
const ENV_SOCKET: &[u8] = b"pinfold-env";

/// The guest Rex server's socket. pinfold fixes it; guest output never names it.
pub const REX_SOCKET: &str = "/home/pinfold/.local/state/rex/run/server.sock";

/// Private exec metadata is removed before restoring caller-named variables.
pub const EXEC_METADATA: &str = "PINFOLD_EXEC_METADATA";

#[derive(Default)]
pub struct ExecMetadata {
    pub agent: Option<String>,
    pub project_directory: Option<(PathBuf, PathBuf)>,
    pub window: Option<(u16, u16)>,
    pub term: Option<Vec<u8>>,
    pub colorterm: Option<Vec<u8>>,
    /// Deliver all of the command's output, with no deadline after it exits.
    pub whole_output: bool,
}

impl ExecMetadata {
    // Shared with the host; the standalone Apple init only decodes.
    #[allow(dead_code)]
    pub fn encode(&self) -> String {
        let optional = |value: Option<&[u8]>| {
            value.map_or_else(
                || "-".to_string(),
                |value| value.iter().map(|byte| format!("{byte:02x}")).collect(),
            )
        };
        let window = self
            .window
            .map_or_else(|| "-".to_string(), |(rows, cols)| format!("{rows},{cols}"));
        format!(
            "{window}:{}:{}:{}:{}:{}:{}",
            optional(self.agent.as_deref().map(str::as_bytes)),
            optional(self.term.as_deref()),
            optional(self.colorterm.as_deref()),
            optional(
                self.project_directory
                    .as_ref()
                    .map(|(root, _)| root.as_os_str().as_bytes())
            ),
            optional(
                self.project_directory
                    .as_ref()
                    .map(|(_, relative)| relative.as_os_str().as_bytes())
            ),
            if self.whole_output { "1" } else { "-" }
        )
    }

    fn decode(value: &str) -> io::Result<Self> {
        let invalid = || io::Error::other("invalid execution metadata");
        let fields: [&str; 7] = value
            .split(':')
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| invalid())?;
        let window = if fields[0] == "-" {
            None
        } else {
            let (rows, cols) = fields[0].split_once(',').ok_or_else(invalid)?;
            Some((
                rows.parse().map_err(|_| invalid())?,
                cols.parse().map_err(|_| invalid())?,
            ))
        };
        let optional = |value: &str| {
            if value == "-" {
                Ok(None)
            } else {
                decode_hex(value.as_bytes()).map(Some)
            }
        };
        let project_directory = match (optional(fields[4])?, optional(fields[5])?) {
            (None, None) => None,
            (Some(root), Some(relative)) => Some((
                PathBuf::from(OsString::from_vec(root)),
                PathBuf::from(OsString::from_vec(relative)),
            )),
            _ => return Err(invalid()),
        };
        if window.is_some_and(|(rows, cols)| rows == 0 || cols == 0) {
            return Err(invalid());
        }
        let whole_output = match fields[6] {
            "1" => true,
            "-" => false,
            _ => return Err(invalid()),
        };
        Ok(Self {
            project_directory,
            agent: optional(fields[1])?
                .map(String::from_utf8)
                .transpose()
                .map_err(|_| invalid())?,
            window,
            term: optional(fields[2])?,
            colorterm: optional(fields[3])?,
            whole_output,
        })
    }
}

fn decode_hex(value: &[u8]) -> io::Result<Vec<u8>> {
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
    decoded
        .filter(|_| value.len().is_multiple_of(2))
        .ok_or_else(|| io::Error::other("invalid encoded environment"))
}

const RELAY_LISTEN: &str = "127.0.0.1:3128";

/// Run as PID 1 until SIGTERM. `args` is the optional guest path of the
/// carried proxy socket; without it the box has no egress and no relay.
pub fn run(args: &[String]) -> ! {
    match args.first().map(String::as_str) {
        Some("exec") => exec(&args[1..]),
        #[cfg(target_os = "linux")]
        Some("session-exec") => exit_with("session-exec", session_exec()),
        Some("setup") => exit_with("setup", setup(&args[1..])),
        Some("proxy-socket") => exit_with("proxy-socket", proxy_socket(&args[1..])),
        Some("receive") => exit_with("receive", receive(&args[1..])),
        Some("send") => exit_with("send", export::run(&args[1..])),
        Some("rex-dial") => exit_with("rex-dial", rex_dial()),
        #[cfg(target_os = "linux")]
        Some("boot") => firecracker::boot(),
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        Some("builder") => builder::boot(),
        _ => {}
    }

    #[cfg(target_os = "linux")]
    if let Err(error) = nix::sys::prctl::set_no_new_privs() {
        eprintln!("pinfold init: no_new_privs: {error}");
        process::exit(1);
    }

    let user_namespaces =
        fs::read_to_string("/proc/sys/user/max_user_namespaces").unwrap_or_else(|error| {
            eprintln!("pinfold init: read user.max_user_namespaces: {error}");
            process::exit(1)
        });
    if user_namespaces.trim() != "0" {
        eprintln!("pinfold init: user.max_user_namespaces must be 0");
        process::exit(1)
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

    // The host sends the box's environment on stdin before anything runs in
    // the box; nothing is passed to the runtime. Ready means it is served.
    #[cfg(target_os = "linux")]
    if let Err(error) = serve_env() {
        eprintln!("pinfold init: box environment: {error}");
        process::exit(1)
    }

    if let Some(target) = args.first() {
        let target = PathBuf::from(target);
        // Ready is reported after the relay listens, so a caller that sees
        // ready can reach the proxy at once.
        let listener = TcpListener::bind(RELAY_LISTEN).unwrap_or_else(|error| {
            eprintln!("pinfold init: bind {RELAY_LISTEN}: {error}");
            process::exit(1)
        });
        serve_relay(listener, move || UnixStream::connect(&target)).expect("start the relay");
    }

    println!("ready");
    io::stdout().flush().expect("write ready");

    let _ = terminate.wait();
    process::exit(0);
}

/// Read the ENV frame from stdin and serve it to each connection on
/// [`ENV_SOCKET`].
#[cfg(target_os = "linux")]
fn serve_env() -> io::Result<()> {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener};
    let (kind, payload) = exec_wire::read_frame(&mut io::stdin().lock())?;
    if kind != exec_wire::ENV {
        return Err(io::Error::other(format!("expected ENV, got frame {kind}")));
    }
    firecracker::decode_env(&payload)?;
    let listener = UnixListener::bind_addr(&SocketAddr::from_abstract_name(ENV_SOCKET)?)?;
    let payload = std::sync::Arc::new(payload);
    thread::Builder::new().spawn(move || {
        loop {
            let Ok((mut client, _)) = listener.accept() else {
                back_off();
                continue;
            };
            // A thread per client: one that never reads stalls only itself.
            let payload = payload.clone();
            let _ = thread::Builder::new()
                .spawn(move || exec_wire::write_frame(&mut client, exec_wire::ENV, &payload));
        }
    })?;
    Ok(())
}

/// The box's environment, from Apple's PID 1.
#[cfg(target_os = "linux")]
fn box_env() -> io::Result<Vec<(OsString, OsString)>> {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::SocketAddr;
    let mut server = UnixStream::connect_addr(&SocketAddr::from_abstract_name(ENV_SOCKET)?)?;
    match exec_wire::read_frame(&mut server)? {
        (exec_wire::ENV, payload) => firecracker::decode_env(&payload),
        (kind, _) => Err(io::Error::other(format!("expected ENV, got frame {kind}"))),
    }
}

#[cfg(not(target_os = "linux"))]
fn box_env() -> io::Result<Vec<(OsString, OsString)>> {
    Err(io::Error::other(
        "the box environment requires the guest kernel",
    ))
}

/// `init exec [--box-env] -- ARGV...`: raise this process's `oom_score_adj`
/// to 1000 and become ARGV. With `--box-env` (Apple), the box's environment
/// comes from PID 1; otherwise from the exec service's transport names.
fn exec(args: &[String]) -> ! {
    let (fetch, args) = match args {
        [flag, args @ ..] if flag == "--box-env" => (true, args),
        args => (false, args),
    };
    let (program, args) = match args {
        [separator, program, args @ ..] if separator == "--" => (program, args),
        _ => {
            eprintln!("pinfold init exec: usage: pinfold init exec [--box-env] -- argv...");
            process::exit(1);
        }
    };
    let metadata = std::env::var(EXEC_METADATA)
        .map_err(io::Error::other)
        .and_then(|value| ExecMetadata::decode(&value));
    let metadata = match metadata {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!("pinfold init exec: execution metadata: {error}");
            process::exit(125);
        }
    };
    let project = metadata.project_directory.is_some();
    let setup_code = if project { 125 } else { 1 };
    // Apple's runtime starts exec independently of PID 1.
    #[cfg(target_os = "linux")]
    if let Err(error) = nix::sys::prctl::set_no_new_privs() {
        eprintln!("pinfold init exec: no_new_privs: {error}");
        process::exit(setup_code);
    }
    // Raising a process's own OOM score needs no privilege. Linux work can
    // lower it again only to the floor inherited from the exec service.
    if let Err(error) = fs::write("/proc/self/oom_score_adj", "1000") {
        eprintln!("pinfold init exec: oom_score_adj: {error}");
        process::exit(setup_code);
    }
    let _directory = if let Some((root, relative)) = &metadata.project_directory {
        match open_project_directory(root, relative).and_then(|directory| {
            nix::unistd::fchdir(&directory)?;
            Ok(directory)
        }) {
            Ok(directory) => Some(directory),
            Err(error) => {
                // A new project disk starts empty (#372).
                let remedy = if error.kind() == io::ErrorKind::NotFound {
                    ". If this is a disk project, run from the project root, or push \
                     the branch into the box through a pinfold Git remote (`pinfold \
                     --help`, Disk projects) and check it out there"
                } else {
                    ""
                };
                eprintln!(
                    "pinfold init exec: working-directory: {} below {}: {error}{remedy}",
                    formatting::terminal_text(&relative.to_string_lossy()),
                    formatting::terminal_text(&root.to_string_lossy())
                );
                process::exit(125);
            }
        }
    } else {
        None
    };
    if let Some((rows, cols)) = metadata.window {
        let size = nix::libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: TIOCSWINSZ reads the supplied winsize for the inherited TTY.
        if unsafe { nix::libc::ioctl(0, nix::libc::TIOCSWINSZ, &size) } < 0 {
            eprintln!(
                "pinfold init exec: terminal size: {}",
                io::Error::last_os_error()
            );
            process::exit(125);
        }
    }
    let mut command = process::Command::new(program);
    command.args(args).env_remove(EXEC_METADATA);
    let mut restored = Vec::new();
    // With `--box-env` the inherited environment is the image's, applied as is.
    for (key, value) in std::env::vars_os().filter(|_| !fetch) {
        let Some(name) = key
            .to_str()
            .and_then(|key| key.strip_prefix(BOX_ENV_PREFIX))
        else {
            continue;
        };
        command.env_remove(&key);
        let value = value.as_bytes();
        let decoded = match decode_hex(value) {
            Ok(decoded) => decoded,
            Err(error) => {
                eprintln!(
                    "pinfold init exec: encoded box environment for {}: {error}",
                    formatting::terminal_text(name)
                );
                process::exit(setup_code);
            }
        };
        restored.push((OsString::from(name), OsString::from_vec(decoded)));
    }
    if fetch {
        match box_env() {
            Ok(vars) => restored.extend(vars),
            Err(error) => {
                eprintln!("pinfold init exec: box environment: {error}");
                process::exit(setup_code);
            }
        }
    }
    // Restore after removing every transport name: a guest variable may
    // itself start with the prefix and must survive that removal pass.
    command.envs(restored);
    for (name, value) in [("TERM", metadata.term), ("COLORTERM", metadata.colorterm)] {
        if let Some(value) = value {
            command.env(name, OsString::from_vec(value));
        }
    }
    if let Some(agent) = metadata.agent {
        command.env("HERDR_AGENT", agent);
    }
    let error = command.exec();
    let program = formatting::terminal_text(program);
    if project && error.kind() == io::ErrorKind::NotFound {
        eprintln!(
            "pinfold: could not start {program:?} in the project image. Check that the command and its interpreter are installed in the image selected by `pinfold config`."
        );
    } else {
        eprintln!("pinfold init exec: {program}: {error}");
    }
    // Public exec keeps the shell's executable lookup codes.
    process::exit(if project {
        125
    } else if error.kind() == io::ErrorKind::NotFound {
        127
    } else {
        126
    });
}

/// Apple's native pipes carry the same command frames as Linux's exec socket.
#[cfg(target_os = "linux")]
fn session_exec() -> io::Result<()> {
    use std::fs::File;
    nix::sys::prctl::set_no_new_privs()?;
    let stdin = io::stdin().as_fd().try_clone_to_owned()?;
    let stdout = io::stdout().as_fd().try_clone_to_owned()?;
    let departure = stdin.try_clone()?;
    let (mut connection, bridge) = UnixStream::pair()?;
    let mut incoming = bridge.try_clone()?;
    thread::Builder::new().spawn(move || {
        let _ = exec_wire::copy_stream(&mut File::from(stdin), &mut incoming);
        let _ = incoming.shutdown(Shutdown::Write);
    })?;
    let (drained, drain) = std::sync::mpsc::channel();
    thread::Builder::new().spawn(move || {
        let _ = exec_wire::copy_stream(&mut &bridge, &mut File::from(stdout));
        let _ = drained.send(());
    })?;
    let (kind, payload) = exec_wire::read_frame(&mut connection)?;
    if kind != exec_wire::EXEC {
        return Err(io::Error::other("expected command frame"));
    }
    let result = firecracker::exec(connection, &payload, None, Some((departure.as_fd(), false)));
    // Whole output waits for every relayed byte to reach the native pipe.
    if result.as_ref().is_ok_and(|whole_output| *whole_output) {
        let _ = drain.recv();
    } else {
        let _ = drain.recv_timeout(Duration::from_secs(1));
    }
    result.map(drop)
}

/// `init rex-dial`: copy stdio to and from the guest Rex server. The
/// server's end of the stream ends the command. Plain read loops: with
/// std's `io::copy` here, Rex handshakes stalled until the client timed out.
fn rex_dial() -> io::Result<()> {
    use std::fs::File;
    let server = UnixStream::connect(REX_SOCKET)?;
    let mut upload = server.try_clone()?;
    let mut stdin = File::from(io::stdin().as_fd().try_clone_to_owned()?);
    thread::Builder::new().spawn(move || {
        // EOF half-closes; an error also ends the server's direction.
        let how = match exec_wire::copy_stream(&mut stdin, &mut upload) {
            Ok(()) => Shutdown::Write,
            Err(_) => Shutdown::Both,
        };
        let _ = upload.shutdown(how);
    })?;
    let mut stdout = File::from(io::stdout().as_fd().try_clone_to_owned()?);
    exec_wire::copy_stream(&mut &server, &mut stdout)
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

const SETUP_USAGE: &str = "usage: pinfold init setup [--own UID:GID DIR]... [--user UID:GID [--fill ROOT]... [--mkdir ROOT RELATIVE]...]";

fn proxy_socket(args: &[String]) -> io::Result<()> {
    if !args.is_empty() {
        return Err(io::Error::other("usage: pinfold init proxy-socket"));
    }
    #[cfg(target_os = "linux")]
    {
        use nix::fcntl::OFlag;
        use nix::sys::stat::{Mode, SFlag};

        std::env::set_current_dir("/")?;
        nix::sys::prctl::set_no_new_privs()?;
        // Image symlinks can alias declared destinations. Inspect kernel
        // mount data before opening any part of the fixed socket directory.
        let mounts = fs::read_to_string("/proc/self/mountinfo")?;
        if mounts.lines().any(|line| {
            let Some((mount, filesystem)) = line.split_once(" - ") else {
                return true;
            };
            match mount.split_ascii_whitespace().nth(4) {
                Some("/var" | "/var/host-services") => {
                    filesystem.split_ascii_whitespace().next() == Some("virtiofs")
                        && mount
                            .split_ascii_whitespace()
                            .nth(5)
                            .is_some_and(|options| options.split(',').any(|option| option == "rw"))
                }
                // Apple forwards the socket itself as a tmpfs bind.
                Some("/var/host-services/ssh-auth.sock") => {
                    filesystem.split_ascii_whitespace().next() != Some("tmpfs")
                }
                _ => false,
            }
        }) {
            return Err(io::Error::other("forwarded proxy path is an attachment"));
        }
        let flags = OFlag::O_PATH | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
        let mut directory = nix::fcntl::open("/", flags | OFlag::O_DIRECTORY, Mode::empty())?;
        for name in ["var", "host-services"] {
            directory =
                nix::fcntl::openat(&directory, name, flags | OFlag::O_DIRECTORY, Mode::empty())?;
        }
        let socket = nix::fcntl::openat(&directory, "ssh-auth.sock", flags, Mode::empty())?;
        let metadata = nix::sys::stat::fstat(&socket)?;
        if SFlag::from_bits_truncate(metadata.st_mode) & SFlag::S_IFMT != SFlag::S_IFSOCK
            || metadata.st_uid != 0
        {
            return Err(io::Error::other(
                "forwarded proxy is not a root-owned socket",
            ));
        }
        // The proc fd link names the pinned inode, not its mutable original path.
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            format!("/proc/self/fd/{}", socket.as_raw_fd()),
            fs::Permissions::from_mode(0o666),
        )?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    Err(io::Error::other(
        "proxy socket setup requires the guest kernel",
    ))
}

/// Own only fresh disk roots, then drop privileges before preparing paths.
fn setup(args: &[String]) -> io::Result<()> {
    let mut args = args.iter();
    let mut ownership = Vec::new();
    let mut directories = Vec::new();
    let mut fills = Vec::new();
    let mut user = None;
    let owner = |text: &str| -> io::Result<(u32, u32)> {
        text.split_once(':')
            .and_then(|(uid, gid)| Some((uid.parse().ok()?, gid.parse().ok()?)))
            .ok_or_else(|| io::Error::other(SETUP_USAGE))
    };
    while let Some(option) = args.next() {
        match option.as_str() {
            "--own" => {
                let (Some(user), Some(dir)) = (args.next(), args.next()) else {
                    return Err(io::Error::other(SETUP_USAGE));
                };
                ownership.push((owner(user)?, dir));
            }
            "--user" if user.is_none() => {
                user = Some(owner(
                    args.next().ok_or_else(|| io::Error::other(SETUP_USAGE))?,
                )?);
            }
            "--mkdir" => {
                let (Some(root), Some(relative)) = (args.next(), args.next()) else {
                    return Err(io::Error::other(SETUP_USAGE));
                };
                let relative = Path::new(relative);
                check_relative_directory(relative)?;
                directories.push((Path::new(root), relative));
            }
            "--fill" => {
                let root = args.next().ok_or_else(|| io::Error::other(SETUP_USAGE))?;
                fills.push(Path::new(root));
            }
            _ => return Err(io::Error::other(SETUP_USAGE)),
        }
    }
    if (!directories.is_empty() || !fills.is_empty()) && user.is_none() {
        return Err(io::Error::other(SETUP_USAGE));
    }
    for ((uid, gid), dir) in ownership {
        // Only fresh disk roots reach this ownership pass. Recovered data stays.
        let _ = fs::remove_dir(Path::new(dir).join("lost+found"));
        nix::unistd::chown(
            dir.as_str(),
            Some(nix::unistd::Uid::from_raw(uid)),
            Some(nix::unistd::Gid::from_raw(gid)),
        )?;
    }
    if let Some((uid, gid)) = user {
        drop_identity(uid, gid)?;
    }
    let mut input = io::stdin().lock();
    for root in fills {
        let root = open_directory(root)?;
        nix::unistd::fchdir(&root)?;
        receive_into(Path::new("."), &mut input, false)?;
    }
    for (root, relative) in directories {
        make_directories(open_directory(root)?, relative)?;
    }
    Ok(())
}

fn open_directory(path: &Path) -> io::Result<OwnedFd> {
    use nix::fcntl::OFlag;
    Ok(nix::fcntl::open(
        path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        nix::sys::stat::Mode::empty(),
    )?)
}

/// Hold every directory through a no-follow descriptor walk, then enter the
/// final inode. Project cwd never follows even an in-project symlink.
fn open_project_directory(root: &Path, relative: &Path) -> io::Result<OwnedFd> {
    use nix::fcntl::OFlag;
    #[cfg(target_os = "linux")]
    let search = OFlag::O_PATH;
    #[cfg(not(target_os = "linux"))]
    let search = OFlag::O_SEARCH;
    let flags = search | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let root = root
        .strip_prefix("/")
        .map_err(|_| io::Error::other("project root is not absolute"))?;
    let mut directory = nix::fcntl::open(Path::new("/"), flags, nix::sys::stat::Mode::empty())?;
    for part in root.components().chain(relative.components()) {
        let Component::Normal(name) = part else {
            return Err(io::Error::other(
                "project working directory is not plain and relative",
            ));
        };
        directory = nix::fcntl::openat(&directory, name, flags, nix::sys::stat::Mode::empty())?;
    }
    Ok(directory)
}

fn check_relative_directory(relative: &Path) -> io::Result<()> {
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::other(
            "directory preparation path must be plain and relative",
        ));
    }
    Ok(())
}

/// Descriptor-relative traversal keeps a reused disk's symlinks out of setup.
fn make_directories(mut root: OwnedFd, relative: &Path) -> io::Result<()> {
    use nix::fcntl::OFlag;
    use nix::sys::stat::Mode;
    check_relative_directory(relative)?;
    for part in relative.components() {
        let name = part.as_os_str();
        match nix::sys::stat::mkdirat(&root, name, Mode::from_bits_truncate(0o755)) {
            Ok(()) | Err(nix::errno::Errno::EEXIST) => {}
            Err(error) => return Err(error.into()),
        }
        root = nix::fcntl::openat(
            &root,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;
    }
    Ok(())
}

fn drop_identity(uid: u32, gid: u32) -> io::Result<()> {
    // nix does not expose setgroups on Apple, where this module also compiles.
    if unsafe { nix::libc::setgroups(0, std::ptr::null()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    nix::unistd::setgid(nix::unistd::Gid::from_raw(gid))?;
    nix::unistd::setuid(nix::unistd::Uid::from_raw(uid))?;
    #[cfg(target_os = "linux")]
    {
        firecracker::set_capabilities(None)?;
        nix::sys::prctl::set_no_new_privs()?;
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

/// `init receive [--missing] DIR`: write the copy stream on stdin below
/// DIR, as the box user. With `--missing`, an entry already present is
/// left as it is: a profile's seeds.
fn receive(args: &[String]) -> io::Result<()> {
    let (missing, root) = match args {
        [flag, root] if flag == "--missing" => (true, root),
        [root] => (false, root),
        _ => {
            return Err(io::Error::other(
                "usage: pinfold init receive [--missing] DIR",
            ));
        }
    };
    receive_into(
        Path::new(root),
        &mut io::BufReader::new(io::stdin().lock()),
        missing,
    )
}

/// Write one copy stream from `input` below `root`, up to its
/// [`STREAM_END`]. Directories and executables get 755, other files 644.
/// With `missing`, an existing entry is skipped and keeps its mode.
fn receive_into(root: &Path, mut input: &mut impl io::Read, missing: bool) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    fn read_bytes(input: &mut impl Read, length: usize) -> io::Result<Vec<u8>> {
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    let present = |result: io::Result<()>| match result {
        Err(error) if missing && error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        result => result.map(|()| true),
    };
    loop {
        let tag = read_bytes(&mut input, 1)?[0];
        if tag == STREAM_END {
            // Before ready, so the host can make a finished fill durable.
            #[cfg(target_os = "linux")]
            nix::unistd::syncfs(fs::File::open(root)?)?;
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
                if present(fs::DirBuilder::new().mode(0o755).create(&path))? {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
                }
            }
            STREAM_FILE | STREAM_EXECUTABLE => {
                let mode = if tag == STREAM_EXECUTABLE {
                    0o755
                } else {
                    0o644
                };
                let size = u64::from_be_bytes(read_bytes(&mut input, 8)?.try_into().unwrap());
                let mut payload = (&mut input).take(size);
                let file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .open(&path);
                let mut file = match file {
                    Err(error) if missing && error.kind() == io::ErrorKind::AlreadyExists => {
                        // Present, whatever it is; drain its payload.
                        io::copy(&mut payload, &mut io::sink())?;
                        continue;
                    }
                    file => file?,
                };
                let copied = io::copy(&mut payload, &mut file)?;
                if copied != size {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                file.set_permissions(fs::Permissions::from_mode(mode))?;
            }
            STREAM_SYMLINK => {
                let length =
                    u32::from_be_bytes(read_bytes(&mut input, 4)?.try_into().unwrap()) as usize;
                let target = OsString::from_vec(read_bytes(&mut input, length)?);
                present(std::os::unix::fs::symlink(target, &path))?;
            }
            _ => return Err(io::Error::other(format!("unknown stream tag {tag}"))),
        }
    }
}

/// A blocking socket's writes, each failing with `TimedOut` after `timeout`
/// without progress. `SO_SNDTIMEO` is not enough: Linux's timer wheel
/// rounds a five-minute timeout up by up to half a minute, and a partial
/// write returns only at the timeout, restarting it for the rest.
pub(crate) struct Deadline<'a>(pub BorrowedFd<'a>, pub Duration);

impl Write for Deadline<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let began = Instant::now();
        let mut ready = [PollFd::new(self.0, PollFlags::POLLOUT)];
        // Polling only after a refused send keeps Linux at a blocking
        // send's throughput. Darwin ignores MSG_DONTWAIT on a blocking
        // socket, so there every send waits for POLLOUT first.
        let mut wait = cfg!(target_os = "macos");
        loop {
            if wait {
                let remaining = self
                    .1
                    .checked_sub(began.elapsed())
                    .ok_or(io::ErrorKind::TimedOut)?;
                let timeout = PollTimeout::try_from(remaining.max(Duration::from_millis(1)))
                    .expect("five-minute timeout fits poll");
                match poll(&mut ready, timeout) {
                    Ok(0) => return Err(io::ErrorKind::TimedOut.into()),
                    Err(nix::errno::Errno::EINTR) => continue,
                    Ok(_) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            wait = true;
            // Safety: the fd stays borrowed, and buf is readable for its length.
            let sent = unsafe {
                nix::libc::send(
                    self.0.as_raw_fd(),
                    buf.as_ptr().cast(),
                    buf.len(),
                    nix::libc::MSG_DONTWAIT | nix::libc::MSG_NOSIGNAL,
                )
            };
            if sent >= 0 {
                return Ok(sent as usize);
            }
            let error = io::Error::last_os_error();
            if !matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Err(error);
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Relay each client `listener` accepts to a new `connect()` stream, each
/// on its own thread.
fn serve_relay(
    listener: TcpListener,
    connect: impl Fn() -> io::Result<UnixStream> + Clone + Send + 'static,
) -> io::Result<()> {
    thread::Builder::new().spawn(move || {
        loop {
            let Ok((client, _)) = listener.accept() else {
                back_off();
                continue;
            };
            let connect = connect.clone();
            let _ = thread::Builder::new().spawn(move || relay(client, connect()?));
        }
    })?;
    Ok(())
}

/// Pause after an accept error, such as EMFILE, that would otherwise
/// repeat at once.
fn back_off() {
    thread::sleep(Duration::from_millis(100));
}

/// Guest upload EOF preserves the response path. Proxy EOF completes the
/// exchange and interrupts a guest that kept its upload half open.
fn relay(client: TcpStream, mut server: UnixStream) -> io::Result<()> {
    const WRITE_TIMEOUT: Duration = Duration::from_secs(300);
    let mut client_reader = client.try_clone()?;
    let server_writer = server.try_clone()?;
    let up = thread::spawn(move || {
        let mut writer = Deadline(server_writer.as_fd(), WRITE_TIMEOUT);
        if io::copy(&mut client_reader, &mut writer).is_err() {
            let _ = client_reader.shutdown(Shutdown::Both);
            let _ = server_writer.shutdown(Shutdown::Both);
        } else {
            let _ = server_writer.shutdown(Shutdown::Write);
        }
    });
    let _ = io::copy(&mut server, &mut Deadline(client.as_fd(), WRITE_TIMEOUT));
    let _ = client.shutdown(Shutdown::Both);
    let _ = server.shutdown(Shutdown::Both);
    let _ = up.join();
    Ok(())
}
