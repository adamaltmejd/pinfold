//! Host terminal setup shared by the execution transports.

use std::io::{self, IsTerminal};

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

pub(crate) struct Saved(Option<libc::termios>);

impl Saved {
    pub(crate) fn save() -> io::Result<Option<Self>> {
        if !io::stdin().is_terminal() {
            return Ok(None);
        }
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: tcgetattr fills the termios on the stack.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(Self(Some(saved))))
    }

    pub(crate) fn restore(mut self) -> io::Result<()> {
        self.restore_saved()
    }

    fn restore_saved(&mut self) -> io::Result<()> {
        if let Some(saved) = &self.0 {
            // SAFETY: restores the attributes read from stdin.
            if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved) } < 0 {
                return Err(io::Error::last_os_error());
            }
            self.0 = None;
        }
        Ok(())
    }
}

impl Drop for Saved {
    fn drop(&mut self) {
        let _ = self.restore_saved();
    }
}

pub(super) struct Raw {
    _saved: Saved,
}

impl Raw {
    pub(super) fn enter() -> io::Result<Option<Self>> {
        let Some(saved) = Saved::save()? else {
            return Ok(None);
        };
        let mut raw = saved.0.expect("the terminal was saved");
        // SAFETY: cfmakeraw updates our copy; tcsetattr reads it.
        unsafe {
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Some(Self { _saved: saved }))
    }
}
