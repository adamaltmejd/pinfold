//! The host's half of the guest protocol (`init/firecracker.rs`): requests
//! over the VM's vsock socket, and the exec client.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use crate::init::firecracker::{
    CONTROL_PORT, ENV, ENV_SET, EXEC, EXEC_PORT, EXIT, FAILED, RESIZE, STAT, STDERR, STDIN,
    STDIN_END, STDOUT, UNMOUNT, read_frame, write_frame,
};

/// The VM's vsock socket in a box's state dir. A host listener for port
/// `N` is this path with `_N`.
pub fn socket(state_dir: &Path) -> PathBuf {
    state_dir.join("vsock").join("v.sock")
}

/// A host-initiated connection to the guest's `port`.
pub fn connect(state_dir: &Path, port: u32) -> io::Result<UnixStream> {
    let mut stream = UnixStream::connect(socket(state_dir))?;
    // Box work runs as the exec service's user and can stop it from
    // answering, so every request has a deadline; `exec` lifts it after
    // the handshake.
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(format!("CONNECT {port}\n").as_bytes())?;
    // One byte at a time: what follows the line is the guest's.
    let mut line = Vec::new();
    let mut byte = [0];
    while line.len() < 64 {
        match stream.read(&mut byte)? {
            0 => break,
            _ if byte[0] == b'\n' => break,
            _ => line.push(byte[0]),
        }
    }
    if !line.starts_with(b"OK ") {
        return Err(io::Error::other(format!(
            "the box does not answer on vsock port {port}"
        )));
    }
    Ok(stream)
}

/// Read the answer to a request: the payload of a frame of `kind`, or the
/// guest's FAILED text as the error.
fn answer(stream: &mut UnixStream, kind: u8) -> io::Result<Vec<u8>> {
    let (got, payload) = read_frame(stream)?;
    match got {
        _ if got == kind => Ok(payload),
        FAILED => Err(io::Error::other(
            String::from_utf8_lossy(&payload).into_owned(),
        )),
        _ => Err(io::Error::other(format!("the box answered frame {got}"))),
    }
}

pub(super) fn put(payload: &mut Vec<u8>, bytes: &[u8]) {
    payload.extend((bytes.len() as u32).to_be_bytes());
    payload.extend(bytes);
}

fn put_pairs(payload: &mut Vec<u8>, pairs: &[(OsString, OsString)]) {
    payload.extend((pairs.len() as u32).to_be_bytes());
    for (name, value) in pairs {
        put(payload, name.as_bytes());
        put(payload, value.as_bytes());
    }
}

/// Give the exec service the box's environment, once.
pub fn set_env(state_dir: &Path, vars: &[(OsString, OsString)]) -> io::Result<()> {
    let mut stream = connect(state_dir, EXEC_PORT)?;
    let mut payload = Vec::new();
    put_pairs(&mut payload, vars);
    write_frame(&mut stream, ENV, &payload)?;
    answer(&mut stream, ENV_SET).map(drop)
}

/// The work cgroup's memory current and peak, pids current and max, and
/// OOM kills.
pub fn stat(state_dir: &Path) -> io::Result<[Option<u64>; 5]> {
    let mut stream = connect(state_dir, EXEC_PORT)?;
    write_frame(&mut stream, STAT, &[])?;
    let payload = answer(&mut stream, STAT)?;
    let mut values = [None; 5];
    let (fields @ [_, _, _, _, _], []) = payload.as_chunks::<9>() else {
        return Err(io::Error::other("the box's STAT has the wrong length"));
    };
    for (value, field) in values.iter_mut().zip(fields) {
        if field[0] != 0 {
            *value = Some(u64::from_be_bytes(field[1..].try_into().unwrap()));
        }
    }
    Ok(values)
}

/// Ask PID 1 to kill the work cgroup, sync and unmount every attachment.
pub fn unmount(state_dir: &Path) -> io::Result<()> {
    let mut stream = connect(state_dir, CONTROL_PORT)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    write_frame(&mut stream, UNMOUNT, &[])?;
    answer(&mut stream, crate::init::firecracker::CLEAN).map(drop)
}

/// Run `argv` in the box and relay stdio until it exits: its exit code, or
/// `128+n` for signal `n`. The client stays in pinfold's process.
pub fn exec(
    state_dir: &Path,
    tty: bool,
    workdir: Option<&Path>,
    argv: &[String],
) -> io::Result<i32> {
    let mut connection = connect(state_dir, EXEC_PORT)?;
    // A command runs as long as it runs.
    connection.set_read_timeout(None)?;
    let (rows, cols) = if tty { window() } else { (0, 0) };
    // Keep the host's terminal identity on a TTY.
    let extra: Vec<(OsString, OsString)> = if tty {
        ["TERM", "COLORTERM"]
            .into_iter()
            .filter_map(|name| Some((name.into(), std::env::var_os(name)?)))
            .collect()
    } else {
        Vec::new()
    };
    let mut payload = Vec::new();
    payload.extend((argv.len() as u32).to_be_bytes());
    for arg in argv {
        put(&mut payload, arg.as_bytes());
    }
    put(
        &mut payload,
        workdir.map_or(&[][..], |dir| dir.as_os_str().as_bytes()),
    );
    payload.push(u8::from(tty));
    payload.extend(rows.to_be_bytes());
    payload.extend(cols.to_be_bytes());
    put_pairs(&mut payload, &extra);
    write_frame(&mut connection, EXEC, &payload)?;

    let raw = if tty { Raw::enter() } else { None };
    let winch = if tty { Some(Winch::install()?) } else { None };
    let (stop, stopping) = io::pipe()?;
    let input = {
        let connection = connection.try_clone()?;
        let winch = match &winch {
            Some(winch) => Some(winch.read.try_clone()?),
            None => None,
        };
        std::thread::Builder::new()
            .name("exec-input".into())
            .spawn(move || forward_input(connection, &stop, winch.as_ref()))?
    };
    let result = relay_output(&mut connection);
    // Ends the input thread's poll; it never blocks in a read.
    drop(stopping);
    let _ = input.join();
    drop((winch, raw));
    result
}

/// Copy the guest's output to pinfold's own until EXIT.
fn relay_output(connection: &mut UnixStream) -> io::Result<i32> {
    let (mut stdout, mut stderr) = (io::stdout(), io::stderr());
    loop {
        let (kind, payload) = read_frame(connection).map_err(|error| {
            io::Error::other(format!("the box ended the exec before it exited: {error}"))
        })?;
        // A closed host output never stops the relay: the exit code still
        // has to arrive.
        match kind {
            STDOUT => {
                let _ = stdout.write_all(&payload).and_then(|()| stdout.flush());
            }
            STDERR => {
                let _ = stderr.write_all(&payload).and_then(|()| stderr.flush());
            }
            EXIT => {
                let code: [u8; 4] = payload
                    .try_into()
                    .map_err(|_| io::Error::other("the box's EXIT has the wrong length"))?;
                return Ok(i32::from_be_bytes(code));
            }
            FAILED => {
                return Err(io::Error::other(
                    String::from_utf8_lossy(&payload).into_owned(),
                ));
            }
            _ => return Err(io::Error::other(format!("the box sent frame {kind}"))),
        }
    }
}

/// Forward stdin as STDIN frames and its end as STDIN_END, and each window
/// change as RESIZE, until `stop` reads end of file.
fn forward_input(
    mut connection: UnixStream,
    stop: &io::PipeReader,
    winch: Option<&io::PipeReader>,
) {
    let stdin = io::stdin();
    let mut open = true;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let mut fds = vec![PollFd::new(stop.as_fd(), PollFlags::POLLIN)];
        if let Some(winch) = winch {
            fds.push(PollFd::new(winch.as_fd(), PollFlags::POLLIN));
        }
        if open {
            fds.push(PollFd::new(stdin.as_fd(), PollFlags::POLLIN));
        }
        match poll(&mut fds, PollTimeout::NONE) {
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return,
        }
        let ready: Vec<bool> = fds
            .iter()
            .map(|fd| fd.revents().is_some_and(|events| !events.is_empty()))
            .collect();
        if ready[0] {
            return;
        }
        if let Some(winch) = winch.filter(|_| ready[1]) {
            let _ = (&*winch).read(&mut [0; 64]);
            let (rows, cols) = window();
            let mut payload = rows.to_be_bytes().to_vec();
            payload.extend(cols.to_be_bytes());
            if write_frame(&mut connection, RESIZE, &payload).is_err() {
                return;
            }
        }
        if open && *ready.last().unwrap() {
            let sent = match read_fd(stdin.as_fd(), &mut buffer) {
                Ok(0) | Err(_) => {
                    open = false;
                    write_frame(&mut connection, STDIN_END, &[])
                }
                Ok(size) => write_frame(&mut connection, STDIN, &buffer[..size]),
            };
            if sent.is_err() {
                return;
            }
        }
    }
}

fn read_fd(fd: BorrowedFd, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match nix::unistd::read(fd, buffer) {
            Err(nix::errno::Errno::EINTR) => continue,
            result => return result.map_err(io::Error::from),
        }
    }
}

/// The host terminal's rows and columns, 24x80 when stdout is none.
fn window() -> (u16, u16) {
    // SAFETY: TIOCGWINSZ fills a winsize on the stack.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let got = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
    if got < 0 || size.ws_row == 0 {
        return (24, 80);
    }
    (size.ws_row, size.ws_col)
}

/// The host terminal in raw mode, restored on drop.
struct Raw(libc::termios);

impl Raw {
    /// None when stdin is no terminal: `--tty` from a caller's own pty.
    fn enter() -> Option<Raw> {
        // SAFETY: tcgetattr and tcsetattr on stdin with termios on the stack.
        unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut saved) < 0 {
                return None;
            }
            let mut raw = saved;
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) < 0 {
                return None;
            }
            Some(Raw(saved))
        }
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        // SAFETY: restores the attributes `enter` read.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0) };
    }
}

/// The write end of the SIGWINCH self-pipe, -1 when none is installed.
static WINCH: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_winch(_: libc::c_int) {
    let fd = WINCH.load(Ordering::Relaxed);
    if fd >= 0 {
        // SAFETY: write is async-signal-safe; the pipe is non-blocking.
        unsafe { libc::write(fd, [0u8].as_ptr().cast(), 1) };
    }
}

/// A SIGWINCH handler that writes a byte to `read`'s pipe, restored on
/// drop.
struct Winch {
    read: io::PipeReader,
    _write: OwnedFd,
    previous: libc::sigaction,
}

impl Winch {
    fn install() -> io::Result<Winch> {
        let (read, write) = io::pipe()?;
        let write = OwnedFd::from(write);
        crate::core::pipe::nonblocking(&write, "window-size")?;
        WINCH.store(write.as_raw_fd(), Ordering::Relaxed);
        // SAFETY: installs a handler that makes only async-signal-safe calls.
        let previous = unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_winch as extern "C" fn(libc::c_int) as usize;
            action.sa_flags = libc::SA_RESTART;
            let mut previous: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(libc::SIGWINCH, &action, &mut previous) < 0 {
                WINCH.store(-1, Ordering::Relaxed);
                return Err(io::Error::last_os_error());
            }
            previous
        };
        Ok(Winch {
            read,
            _write: write,
            previous,
        })
    }
}

impl Drop for Winch {
    fn drop(&mut self) {
        // SAFETY: restores the action `install` replaced.
        unsafe { libc::sigaction(libc::SIGWINCH, &self.previous, std::ptr::null_mut()) };
        WINCH.store(-1, Ordering::Relaxed);
    }
}
