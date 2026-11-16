//! Exclude process creation while descriptors temporarily lack CLOEXEC.

use nix::poll::{PollFd, PollFlags, poll};
use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

static FORK: Mutex<()> = Mutex::new(());

thread_local! {
    static CANCELLATION: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Apply cancellation to captured subprocesses in one blocking operation.
/// Nested scopes and unwinding restore the preceding thread-local context.
pub(crate) fn with_cancellation<T>(cancel: Arc<AtomicBool>, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<AtomicBool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CANCELLATION.with(|active| {
                active.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(CANCELLATION.with(|active| active.replace(Some(cancel))));
    work()
}

pub(crate) fn fork_guard() -> MutexGuard<'static, ()> {
    FORK.lock().unwrap_or_else(|error| error.into_inner())
}

pub(crate) fn pipe() -> io::Result<(io::PipeReader, io::PipeWriter)> {
    let _guard = fork_guard();
    io::pipe()
}

pub(crate) struct Listener {
    socket: tokio::io::unix::AsyncFd<std::os::unix::net::UnixListener>,
}

impl Listener {
    pub(crate) fn from_std(socket: std::os::unix::net::UnixListener) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            // SAFETY: the owned socket keeps the registered descriptor stable.
            socket: unsafe { tokio::io::unix::AsyncFd::register(socket) }?,
        })
    }

    pub(crate) async fn accept(
        &self,
    ) -> io::Result<(tokio::net::UnixStream, std::os::unix::net::SocketAddr)> {
        loop {
            let mut ready = self.socket.readable().await?;
            match ready.try_io(|socket| {
                let _guard = fork_guard();
                socket.get_ref().accept()
            }) {
                Ok(accepted) => {
                    let (socket, address) = accepted?;
                    socket.set_nonblocking(true)?;
                    return Ok((tokio::net::UnixStream::from_std(socket)?, address));
                }
                Err(_) => continue,
            }
        }
    }
}

pub(crate) trait CommandGuard {
    fn spawn_guarded(&mut self) -> io::Result<Child>;
    fn output_guarded(&mut self) -> io::Result<Output>;
    fn status_guarded(&mut self) -> io::Result<ExitStatus>;
}

pub(crate) trait AsyncCommandGuard {
    fn spawn_guarded(&mut self) -> io::Result<tokio::process::Child>;
}

impl AsyncCommandGuard for tokio::process::Command {
    fn spawn_guarded(&mut self) -> io::Result<tokio::process::Child> {
        let _guard = fork_guard();
        self.spawn()
    }
}

impl CommandGuard for Command {
    fn spawn_guarded(&mut self) -> io::Result<Child> {
        let _guard = fork_guard();
        self.spawn()
    }

    fn output_guarded(&mut self) -> io::Result<Output> {
        if let Some(cancel) = CANCELLATION.with(|active| active.borrow().clone()) {
            return output_cancellable(self, None, &cancel);
        }
        self.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        self.spawn_guarded()?.wait_with_output()
    }

    fn status_guarded(&mut self) -> io::Result<ExitStatus> {
        self.spawn_guarded()?.wait()
    }
}

/// Capture a helper without leaving a child or pipe-drain worker.
/// `None` preserves configured stdin; `Some` supplies bytes through a pipe.
pub(crate) fn output_cancellable(
    command: &mut Command,
    input: Option<&[u8]>,
    cancel: &AtomicBool,
) -> io::Result<Output> {
    output_with_limit(command, input, cancel, None)
}

/// Drain helper stdout without retaining bytes that the caller discards.
pub(crate) fn output_discard_cancellable(
    command: &mut Command,
    input: Option<&[u8]>,
    cancel: &AtomicBool,
) -> io::Result<Output> {
    output_with_limit(command, input, cancel, Some(0))
}

fn output_with_limit(
    command: &mut Command,
    input: Option<&[u8]>,
    cancel: &AtomicBool,
    stdout_limit: Option<usize>,
) -> io::Result<Output> {
    if cancel.load(Ordering::Acquire) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = GroupChild {
        child: command.spawn_guarded()?,
        reaped: false,
    };
    let mut stdin = child.child.stdin.take();
    let mut stdout = child.child.stdout.take();
    let mut stderr = child.child.stderr.take();
    if let Some(pipe) = stdin.as_ref() {
        super::pipe::nonblocking(pipe, "crossing")?;
    }
    if let Some(pipe) = stdout.as_ref() {
        super::pipe::nonblocking(pipe, "crossing")?;
    }
    if let Some(pipe) = stderr.as_ref() {
        super::pipe::nonblocking(pipe, "crossing")?;
    }
    let mut input = input.unwrap_or_default();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut exited = None;
    let mut input_failed = false;
    loop {
        if exited.is_none()
            && let Some(status) = child.child.try_wait()?
        {
            child.reaped = true;
            exited = Some((status, Instant::now()));
            input_failed |= !input.is_empty();
            stdin.take();
        }
        if cancel.load(Ordering::Acquire) && exited.is_none() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if input.is_empty() {
            stdin.take();
        }
        if let Some(pipe) = stdin.as_mut() {
            match pipe.write(input) {
                Ok(written) => input = &input[written..],
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                    input_failed = true;
                    stdin.take();
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        drain(&mut stdout, &mut out, stdout_limit)?;
        drain(&mut stderr, &mut err, Some(16 * 1024))?;
        if let Some((status, time)) = exited
            && ((stdout.is_none() && stderr.is_none()) || time.elapsed() >= Duration::from_secs(1))
        {
            if input_failed {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            return Ok(Output {
                status,
                stdout: out,
                stderr: err,
            });
        }
        let mut ready = Vec::new();
        if let Some(pipe) = stdin.as_ref() {
            ready.push(PollFd::new(pipe.as_fd(), PollFlags::POLLOUT));
        }
        if let Some(pipe) = stdout.as_ref() {
            ready.push(PollFd::new(pipe.as_fd(), PollFlags::POLLIN));
        }
        if let Some(pipe) = stderr.as_ref() {
            ready.push(PollFd::new(pipe.as_fd(), PollFlags::POLLIN));
        }
        match poll(&mut ready, 100u16) {
            Ok(_) | Err(nix::errno::Errno::EINTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

pub(crate) fn drain<R: Read>(
    pipe: &mut Option<R>,
    kept: &mut Vec<u8>,
    limit: Option<usize>,
) -> io::Result<()> {
    let Some(reader) = pipe.as_mut() else {
        return Ok(());
    };
    let mut bytes = [0u8; 8192];
    for _ in 0..8 {
        match reader.read(&mut bytes) {
            Ok(0) => {
                pipe.take();
                return Ok(());
            }
            Ok(read) => {
                let retain = limit.map_or(read, |limit| read.min(limit.saturating_sub(kept.len())));
                kept.extend_from_slice(&bytes[..retain]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The process-group id is used only while the direct child is unreaped.
pub(crate) struct GroupChild {
    pub(crate) child: Child,
    pub(crate) reaped: bool,
}

impl Drop for GroupChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(self.child.id() as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
