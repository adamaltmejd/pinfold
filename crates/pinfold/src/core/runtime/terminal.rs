//! Host terminal setup shared by the execution transports.

use std::io::{self, IsTerminal};
use std::sync::Mutex;

use nix::libc;

pub(super) fn window() -> io::Result<Option<(u16, u16)>> {
    if !io::stdout().is_terminal() {
        return Ok(None);
    }
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ fills the winsize on the stack.
    if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((size.ws_row != 0 && size.ws_col != 0).then_some((size.ws_row, size.ws_col)))
}

// A CLI invocation owns one interactive exec. Main and the exec worker share
// its snapshot so cancellation cannot restore twice or admit a late mutation.
struct State {
    saved: Option<libc::termios>,
    ended: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    saved: None,
    ended: false,
});

fn foreground() -> io::Result<bool> {
    // SAFETY: tcgetpgrp reads stdin's foreground group; getpgrp has no arguments.
    let group = unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) };
    if group < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(group == unsafe { libc::getpgrp() })
}

pub(crate) fn restore() -> io::Result<()> {
    let mut state = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.ended = true;
    let Some(saved) = state.saved else {
        return Ok(());
    };
    if foreground()? {
        // SAFETY: restores only this execution's changed settings on its TTY.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    state.saved = None;
    Ok(())
}

pub(super) struct Raw;

impl Raw {
    pub(super) fn enter() -> io::Result<Option<Self>> {
        if !io::stdin().is_terminal() {
            return Ok(None);
        }
        let mut state = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.ended {
            return Err(io::Error::other("terminal execution has ended"));
        }
        if !foreground()? {
            return Err(io::Error::other(
                "terminal execution is not in the foreground",
            ));
        }
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: tcgetattr fills the termios on the stack.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = saved;
        // SAFETY: cfmakeraw updates our copy; tcsetattr reads it.
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
        if (raw.c_iflag, raw.c_oflag, raw.c_cflag, raw.c_lflag, raw.c_cc)
            == (
                saved.c_iflag,
                saved.c_oflag,
                saved.c_cflag,
                saved.c_lflag,
                saved.c_cc,
            )
        {
            return Ok(None);
        }
        // SAFETY: tcsetattr reads our termios copy.
        unsafe {
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        state.saved = Some(saved);
        Ok(Some(Self))
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = restore();
    }
}
