//! The box lifecycle: one attached `container run` process owns one box.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Child;
use tokio::signal::unix::{SignalKind, signal};

use crate::core::plan::Plan;
use crate::core::runtime::{Runtime, runtime};
use crate::dirs;

/// A started box, owned by this process.
pub struct Box {
    name: String,
    state_dir: PathBuf,
    child: Child,
    runtime: &'static dyn Runtime,
}

/// What stopped the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    /// The owner's stdin reached EOF.
    StdinEof,
    /// SIGTERM or SIGINT arrived.
    Signal,
}

impl Box {
    /// Start a box, wait for init's `ready` line, and return it.
    pub async fn up(plan: &Plan, init: &Path) -> io::Result<Box> {
        if !init.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "init path must be absolute",
            ));
        }
        if init.parent().is_none_or(|parent| parent == Path::new("/")) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "init must live in a directory below /",
            ));
        }

        let runtime = runtime()?;
        let state_dir = dirs::state_dir().join("boxes").join(&plan.name);
        tokio::fs::create_dir_all(&state_dir).await?;
        tokio::fs::write(state_dir.join("pid"), std::process::id().to_string()).await?;

        let mut child = match runtime.up(plan, init) {
            Ok(child) => child,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&state_dir).await;
                return Err(error);
            }
        };
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("runtime up did not pipe the box's stdout"))?;
        let mut lines = BufReader::new(stdout).lines();
        let failure = loop {
            match lines.next_line().await {
                Ok(Some(line)) if line == "ready" => break None,
                Ok(Some(_)) => {}
                Ok(None) => break Some("box exited before ready".to_string()),
                Err(error) => break Some(format!("box output failed: {error}")),
            }
        };
        if let Some(failure) = failure {
            let _ = child.start_kill();
            let status = child.wait().await;
            let _ = tokio::fs::remove_dir_all(&state_dir).await;
            return Err(io::Error::other(match status {
                Ok(status) => format!("{failure}: {status}"),
                Err(error) => format!("{failure}: {error}"),
            }));
        }
        // Keep the pipe drained so a talkative box cannot block on it.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

        Ok(Box {
            name: plan.name.clone(),
            state_dir,
            child,
            runtime,
        })
    }

    /// Wait until stdin closes or a termination signal arrives, then stop and
    /// remove the box.
    pub async fn hold(&mut self) -> io::Result<Shutdown> {
        let reason = wait_for_shutdown().await?;
        self.down().await?;
        Ok(reason)
    }

    /// Stop and remove the box, then delete its state directory.
    pub async fn down(&mut self) -> io::Result<()> {
        let result = self.runtime.down(&self.name);
        let _ = self.child.wait().await;
        let _ = tokio::fs::remove_dir_all(&self.state_dir).await;
        result
    }
}

async fn wait_for_shutdown() -> io::Result<Shutdown> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut stdin = tokio::io::stdin();
    let mut buffer = [0u8; 4096];
    loop {
        tokio::select! {
            _ = terminate.recv() => return Ok(Shutdown::Signal),
            _ = interrupt.recv() => return Ok(Shutdown::Signal),
            read = stdin.read(&mut buffer) => {
                if read? == 0 {
                    return Ok(Shutdown::StdinEof);
                }
            },
        }
    }
}
