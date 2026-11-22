//! Exclude process creation while descriptors temporarily lack CLOEXEC.

use nix::poll::{PollFd, PollFlags, poll};
use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{
    Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio,
};
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

/// Cleanup gets a fresh cancellation context even when its caller is cancelled.
pub(crate) fn with_deadline<T>(timeout: Duration, work: impl FnOnce(&AtomicBool) -> T) -> T {
    let cancel = Arc::new(AtomicBool::new(false));
    let (finished, waiting) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        let worker_cancel = &cancel;
        scope.spawn(move || {
            if matches!(
                waiting.recv_timeout(timeout),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                worker_cancel.store(true, Ordering::Release);
            }
        });
        // Drop before the scope joins, including while unwinding.
        let finished = finished;
        let result = with_cancellation(cancel.clone(), || work(&cancel));
        drop(finished);
        result
    })
}

pub(crate) fn is_cancelled() -> bool {
    CANCELLATION.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Acquire))
    })
}

pub(crate) fn fork_guard() -> MutexGuard<'static, ()> {
    FORK.lock().unwrap_or_else(|error| error.into_inner())
}

/// Snapshot inherited descriptors while fork is excluded. Keep the guard
/// through spawn: later descriptors must already be atomically CLOEXEC.
pub(crate) fn open_descriptors(_guard: &MutexGuard<'_, ()>) -> io::Result<Vec<RawFd>> {
    #[cfg(target_os = "linux")]
    let directory = "/proc/self/fd";
    #[cfg(target_os = "macos")]
    let directory = "/dev/fd";
    std::fs::read_dir(directory)?
        .map(|entry| {
            entry?
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
                .ok_or_else(|| io::Error::other("invalid descriptor entry"))
        })
        .collect()
}

/// Resolve the pure descriptor-query syscall wrapper before forking.
pub(crate) fn prepare_fork() -> io::Result<()> {
    #[cfg(target_os = "macos")]
    if unsafe {
        nix::libc::proc_pidinfo(
            nix::libc::getpid(),
            nix::libc::PROC_PIDLISTFDS,
            0,
            std::ptr::null_mut(),
            0,
        )
    } <= 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Close every inherited descriptor from `first` in a fork-only helper.
/// The helper is single-threaded; this performs no userspace allocation.
/// `first` is at most six, leaving room for fixed private helper endpoints.
pub(crate) unsafe fn close_fds_from(first: RawFd) -> bool {
    if !(0..=6).contains(&first) {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        unsafe { nix::libc::syscall(nix::libc::SYS_close_range, first as u32, u32::MAX, 0u32) == 0 }
    }
    #[cfg(target_os = "macos")]
    {
        let mut entries = [nix::libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        }; 256];
        loop {
            // proc_pidinfo is a thin __proc_info syscall wrapper. Query the
            // child, not the parent: concurrent CLOEXEC opens also reach fork.
            let bytes = unsafe {
                nix::libc::proc_pidinfo(
                    nix::libc::getpid(),
                    nix::libc::PROC_PIDLISTFDS,
                    0,
                    entries.as_mut_ptr().cast(),
                    std::mem::size_of_val(&entries) as _,
                )
            };
            if bytes <= 0
                || !(bytes as usize).is_multiple_of(std::mem::size_of::<nix::libc::proc_fdinfo>())
            {
                return false;
            }
            let count = bytes as usize / std::mem::size_of::<nix::libc::proc_fdinfo>();
            if count > entries.len() {
                return false;
            }
            for entry in &entries[..count] {
                if entry.proc_fd >= first {
                    unsafe { nix::libc::close(entry.proc_fd) };
                }
            }
            // A full batch may have omitted descriptors. Closing its entries
            // moves the remaining ones into the next query, including sparse fds.
            if count < entries.len() {
                return true;
            }
        }
    }
}

/// A private pipe to a blocking stdio copier. Only this pipe's flags change;
/// a shell may share the caller's open file description with other clients.
pub(crate) struct StdioRelay {
    endpoint: Option<OwnedFd>,
    pid: Option<nix::libc::pid_t>,
}

impl StdioRelay {
    pub(crate) fn new(caller: RawFd, input: bool) -> io::Result<Self> {
        Self::until(caller, input, Instant::now() + Duration::from_secs(5))
    }

    fn until(caller: RawFd, input: bool, deadline: Instant) -> io::Result<Self> {
        super::pipe::remaining(deadline, None, "stdio relay setup")?;
        let _guard = fork_guard();
        if unsafe { nix::libc::fcntl(caller, nix::libc::F_GETFL) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let (read, write) = io::pipe()?;
        let (mut ready, report) = io::pipe()?;
        let (parent, child) = if input {
            (OwnedFd::from(read), OwnedFd::from(write))
        } else {
            (OwnedFd::from(write), OwnedFd::from(read))
        };
        let source = if input { caller } else { child.as_raw_fd() };
        let target = if input { child.as_raw_fd() } else { caller };
        prepare_fork()?;
        let parent_pid = unsafe { nix::libc::getpid() };
        let pid = unsafe { nix::libc::fork() };
        if pid < 0 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            // The fork-only copier carries neither a lease nor containment
            // authority, and never calls an allocator or inherited lock.
            unsafe {
                let source = nix::libc::fcntl(source, nix::libc::F_DUPFD_CLOEXEC, 3);
                let target = nix::libc::fcntl(target, nix::libc::F_DUPFD_CLOEXEC, 3);
                let report = nix::libc::fcntl(report.as_raw_fd(), nix::libc::F_DUPFD_CLOEXEC, 3);
                if source < 0
                    || target < 0
                    || report < 0
                    || nix::libc::dup2(source, 0) < 0
                    || nix::libc::dup2(target, 1) < 0
                    || nix::libc::dup2(report, 2) < 0
                    || !close_fds_from(3)
                {
                    nix::libc::_exit(1);
                }
                prepare_stdio();
                loop {
                    if nix::libc::write(2, b"R".as_ptr().cast(), 1) == 1 {
                        break;
                    }
                    if nix::errno::Errno::last_raw() != nix::libc::EINTR {
                        nix::libc::_exit(1);
                    }
                }
                nix::libc::close(2);
                copy_stdio(parent_pid);
            }
        }
        drop(_guard);
        drop((child, report));
        let relay = Self {
            endpoint: Some(parent),
            pid: Some(pid),
        };
        super::pipe::nonblocking(&relay, "stdio relay")?;
        super::pipe::nonblocking(&ready, "stdio relay setup")?;
        let mut answer = [0];
        if super::pipe::read(&mut ready, &mut answer, deadline, None, "stdio relay setup")? != 1
            || answer != *b"R"
        {
            return Err(io::Error::other("stdio relay setup failed"));
        }
        Ok(relay)
    }

    pub(crate) fn close(&mut self) {
        self.endpoint.take();
    }

    pub(crate) fn finished(&mut self) -> io::Result<bool> {
        let Some(pid) = self.pid else { return Ok(true) };
        let mut status = 0;
        let result = loop {
            let result = unsafe { nix::libc::waitpid(pid, &mut status, nix::libc::WNOHANG) };
            if result >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                break result;
            }
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if result == 0 {
            return Ok(false);
        }
        self.pid.take();
        if nix::libc::WIFEXITED(status) && nix::libc::WEXITSTATUS(status) == 0 {
            Ok(true)
        } else {
            Err(io::Error::other("stdio relay failed"))
        }
    }
}

/// A full caller stderr must not retain an already completed command.
pub(crate) fn diagnostic(message: impl std::fmt::Display) {
    let deadline = Instant::now() + Duration::from_secs(1);
    let _ = (|| -> io::Result<()> {
        let mut relay = StdioRelay::until(2, false, deadline)?;
        super::pipe::write_all(
            &mut relay,
            // One record per line: interpolated text, a tool's stderr
            // included, keeps no line break of its own.
            format!(
                "pinfold: {}\n",
                crate::formatting::terminal_text(&message.to_string())
            )
            .as_bytes(),
            deadline,
            None,
            "diagnostic",
        )?;
        relay.close();
        let mut wait = ExitWait::new();
        while !relay.finished()? {
            super::pipe::remaining(deadline, None, "diagnostic")?;
            wait.poll(&mut [])?;
        }
        Ok(())
    })();
}

impl AsRawFd for StdioRelay {
    fn as_raw_fd(&self) -> RawFd {
        self.endpoint.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }
}

impl AsFd for StdioRelay {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.endpoint.as_ref().expect("open stdio relay").as_fd()
    }
}

impl Write for StdioRelay {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count =
            unsafe { nix::libc::write(self.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        if count < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(count as usize)
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for StdioRelay {
    fn drop(&mut self) {
        self.close();
        if let Some(pid) = self.pid.take() {
            unsafe {
                nix::libc::kill(pid, nix::libc::SIGKILL);
                while nix::libc::waitpid(pid, std::ptr::null_mut(), 0) < 0 {
                    if nix::errno::Errno::last_raw() != nix::libc::EINTR {
                        break;
                    }
                }
            }
        }
    }
}

extern "C" fn stdio_tick(_: nix::libc::c_int) {}

unsafe fn prepare_stdio() {
    unsafe {
        let mut action: nix::libc::sigaction = std::mem::zeroed();
        nix::libc::sigemptyset(&mut action.sa_mask);
        action.sa_sigaction = nix::libc::SIG_DFL;
        for signal in 1..32 {
            nix::libc::sigaction(signal, &action, std::ptr::null_mut());
        }
        action.sa_sigaction = nix::libc::SIG_IGN;
        for signal in [nix::libc::SIGPIPE, nix::libc::SIGTTIN, nix::libc::SIGTTOU] {
            nix::libc::sigaction(signal, &action, std::ptr::null_mut());
        }
        action.sa_sigaction = stdio_tick as extern "C" fn(nix::libc::c_int) as usize;
        if nix::libc::sigaction(nix::libc::SIGALRM, &action, std::ptr::null_mut()) < 0 {
            nix::libc::_exit(1);
        }
        let mut mask = std::mem::zeroed();
        nix::libc::sigemptyset(&mut mask);
        nix::libc::sigprocmask(nix::libc::SIG_SETMASK, &mask, std::ptr::null_mut());
        // Interrupt a blocked caller read/write after abrupt client death.
        // Otherwise a copier could retain a shell endpoint indefinitely.
        let interval = nix::libc::timeval {
            tv_sec: 0,
            tv_usec: 100_000,
        };
        let timer = nix::libc::itimerval {
            it_interval: interval,
            it_value: interval,
        };
        if nix::libc::setitimer(nix::libc::ITIMER_REAL, &timer, std::ptr::null_mut()) < 0 {
            nix::libc::_exit(1);
        }
    }
}

unsafe fn copy_stdio(parent: nix::libc::pid_t) -> ! {
    let mut bytes = [0u8; 16384];
    loop {
        if unsafe { nix::libc::getppid() } != parent {
            unsafe { nix::libc::_exit(0) }
        }
        let read = unsafe { nix::libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) };
        if read == 0 {
            unsafe { nix::libc::_exit(0) }
        }
        if read < 0 {
            let error = nix::errno::Errno::last_raw();
            if error == nix::libc::EINTR {
                continue;
            }
            if error == nix::libc::EAGAIN {
                let mut poll = nix::libc::pollfd {
                    fd: 0,
                    events: nix::libc::POLLIN,
                    revents: 0,
                };
                unsafe { nix::libc::poll(&mut poll, 1, -1) };
                continue;
            }
            unsafe { nix::libc::_exit(1) }
        }
        let mut written = 0;
        while written < read as usize {
            if unsafe { nix::libc::getppid() } != parent {
                unsafe { nix::libc::_exit(0) }
            }
            let count = unsafe {
                nix::libc::write(
                    1,
                    bytes[written..read as usize].as_ptr().cast(),
                    read as usize - written,
                )
            };
            if count > 0 {
                written += count as usize;
                continue;
            }
            let error = nix::errno::Errno::last_raw();
            if error == nix::libc::EINTR {
                continue;
            }
            if error == nix::libc::EPIPE {
                unsafe { nix::libc::_exit(0) }
            }
            if error == nix::libc::EAGAIN {
                let mut poll = nix::libc::pollfd {
                    fd: 1,
                    events: nix::libc::POLLOUT,
                    revents: 0,
                };
                unsafe { nix::libc::poll(&mut poll, 1, -1) };
                continue;
            }
            unsafe { nix::libc::_exit(1) }
        }
    }
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
    if cancel.load(Ordering::Acquire) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = GroupChild::spawn(command)?;
    let mut stdin = child.take_stdin();
    let mut stdout = child.take_stdout();
    let mut stderr = child.take_stderr();
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
    let mut wait = ExitWait::new();
    loop {
        if exited.is_none() && child.exited()? {
            exited = Some(Instant::now());
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
        drain(&mut stdout, &mut out, None)?;
        drain(&mut stderr, &mut err, Some(16 * 1024))?;
        if let Some(time) = exited
            && ((stdout.is_none() && stderr.is_none()) || time.elapsed() >= Duration::from_secs(1))
        {
            if input_failed {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            return Ok(Output {
                status: child.finish()?,
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
        wait.poll(&mut ready)?;
    }
}

/// Polls a child's pipes. With none left, the exit their EOF announces is
/// usually microseconds away, so the wait starts at 1 ms and doubles to
/// 100 ms: a child that closed its pipes but runs on is not spun on, and
/// cancellation is still checked every 100 ms. Blocking on the exit itself
/// would never see cancellation.
pub(crate) struct ExitWait(u16);

impl ExitWait {
    pub(crate) fn new() -> Self {
        Self(1)
    }

    pub(crate) fn poll(&mut self, fds: &mut [PollFd]) -> io::Result<()> {
        let timeout = if fds.is_empty() {
            let timeout = self.0;
            self.0 = (timeout * 2).min(100);
            timeout
        } else {
            100
        };
        match poll(fds, timeout) {
            Ok(_) | Err(nix::errno::Errno::EINTR) => Ok(()),
            Err(error) => Err(error.into()),
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

/// Wait for `child` until `deadline` or cancellation; on either, kill and
/// reap it.
#[cfg(target_os = "linux")]
pub(crate) fn wait_child(
    mut child: Child,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
    operation: &str,
) -> io::Result<ExitStatus> {
    let result = (|| loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        super::pipe::wait_ready(None, PollFlags::empty(), deadline, cancel, operation)?;
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Retain the leader until group cleanup, so its process-group id stays pinned.
pub(crate) struct GroupChild {
    child: Option<Child>,
}

impl GroupChild {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        Ok(Self {
            child: Some(command.process_group(0).spawn_guarded()?),
        })
    }

    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.as_mut()?.stdin.take()
    }

    pub(crate) fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.as_mut()?.stdout.take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.as_mut()?.stderr.take()
    }

    /// Observe exit without reaping; descendants may still hold captured pipes.
    pub(crate) fn exited(&mut self) -> io::Result<bool> {
        let child = self
            .child
            .as_ref()
            .ok_or_else(|| io::Error::other("helper already reaped"))?;
        let mut info: nix::libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            nix::libc::waitid(
                nix::libc::P_PID,
                child.id() as nix::libc::id_t,
                &mut info,
                nix::libc::WEXITED | nix::libc::WNOHANG | nix::libc::WNOWAIT,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(false);
            }
            if error.raw_os_error() == Some(nix::libc::ECHILD) {
                // Another reaper invalidated the group identity. Never signal it.
                self.child.take();
            }
            return Err(error);
        }
        Ok(unsafe { info.si_pid() } != 0)
    }

    /// Signal the group while the leader is still ours, then reap it once.
    pub(crate) fn finish(&mut self) -> io::Result<ExitStatus> {
        let mut child = self
            .child
            .take()
            .ok_or_else(|| io::Error::other("helper already reaped"))?;
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(child.id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = child.kill();
        child.wait()
    }
}

impl Drop for GroupChild {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.finish();
        }
    }
}
