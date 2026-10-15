//! Temporary diagnostics. Never prints command arguments or environment values.

use std::panic::Location;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct Span {
    label: &'static str,
    began: Instant,
    caller: &'static Location<'static>,
    finished: bool,
}

impl Span {
    #[track_caller]
    pub fn new(label: &'static str) -> Self {
        let span = Self {
            label,
            began: Instant::now(),
            caller: Location::caller(),
            finished: false,
        };
        span.emit("START");
        span
    }

    pub fn finish(mut self, event: &'static str) {
        self.emit(event);
        self.finished = true;
    }

    fn emit(&self, event: &'static str) {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let thread = std::thread::current();
        eprintln!(
            "TIMING {} {} wall_ms={} elapsed_ms={} caller={}:{} test={}",
            self.label,
            event,
            wall,
            self.began.elapsed().as_millis(),
            self.caller.file(),
            self.caller.line(),
            thread.name().unwrap_or("unnamed")
        );
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if !self.finished {
            self.emit("END");
        }
    }
}
