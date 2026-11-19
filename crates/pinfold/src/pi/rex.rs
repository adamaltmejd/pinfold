//! The Rex relay on macOS: a host socket into the project box's own Rex
//! server. The owner holds it for one generation and closes it before
//! teardown.

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, watch};

use crate::core::r#box::Signals;
use crate::core::ownership;
use crate::core::process::{AsyncCommandGuard, Listener};
use crate::core::runtime::sockets::HostSocket;
use crate::core::runtime::{ExecContext, runtime};
use crate::init::REX_SOCKET;
use crate::pi::session::forward_diagnostics;
use crate::pi::session_wire as wire;

const HELPER_DEADLINE: Duration = Duration::from_secs(3);

/// The owner's answer to a start that asked for the relay.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    /// No runnable `rex` in the box, or the box is ending.
    Absent,
    RemoteControl,
    Failed,
    Relayed,
}

/// A start asks for the relay only in a Rex pane, with terminals.
pub(crate) fn wanted() -> bool {
    cfg!(target_os = "macos")
        && std::env::var_os("REX_SESSION").is_some()
        && io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && io::stderr().is_terminal()
}

fn leaf(key: &str) -> String {
    format!("{key}.rex")
}

/// Print a start's outcome on stderr. An absent `rex` prints nothing.
pub(crate) async fn report(
    status: Option<Status>,
    key: &str,
    signals: &mut Signals,
) -> io::Result<()> {
    let lines = match status {
        Some(Status::Relayed) => vec![
            format!(
                "pinfold start: rex: in the Rex app, add unix://{} once under Add Remote Host…",
                ownership::host_dir()?.join(leaf(key)).display()
            ),
            "pinfold start: rex: the host is reachable while this project's box runs.".into(),
        ],
        Some(Status::RemoteControl) => vec![
            "pinfold start: rex-remote-control: Remote Control is on or unreadable in the Rex app; no Rex host was set up.".into(),
        ],
        Some(Status::Failed) => {
            vec!["pinfold start: rex-relay-failed: the Rex host could not be set up.".into()]
        }
        Some(Status::Absent) | None => vec![],
    };
    for line in lines {
        forward_diagnostics(line.into_bytes(), signals).await?;
    }
    Ok(())
}

/// The owner's relay for its generation. Requests serialize on the lock.
#[derive(Clone)]
pub(crate) struct Rex(Arc<Mutex<State>>);

struct State {
    name: String,
    init: PathBuf,
    leaf: String,
    relay: Option<Relay>,
    closed: bool,
}

struct Relay {
    socket: HostSocket,
    stop: watch::Sender<bool>,
    accept: tokio::task::JoinHandle<()>,
    server: Child,
}

impl Rex {
    pub(crate) fn new(name: &str, init: PathBuf, key: &str) -> Self {
        Self(Arc::new(Mutex::new(State {
            name: name.to_owned(),
            init,
            leaf: leaf(key),
            relay: None,
            closed: false,
        })))
    }

    /// Check for `rex` in the box, then the app's Remote Control, then
    /// relay. A relay already up is kept while Remote Control stays off.
    pub(crate) async fn request(&self) -> Status {
        let mut state = self.0.lock().await;
        if state.closed {
            return Status::Absent;
        }
        if state.relay.is_none() && !state.present().await {
            return Status::Absent;
        }
        if !remote_control_off().await {
            state.shut().await;
            return Status::RemoteControl;
        }
        if state.relay.is_some() {
            return Status::Relayed;
        }
        match state.open() {
            Ok(relay) => {
                state.relay = Some(relay);
                Status::Relayed
            }
            Err(_) => Status::Failed,
        }
    }

    /// End the relay before teardown. Later requests set up nothing.
    pub(crate) async fn close(&self) {
        let mut state = self.0.lock().await;
        state.closed = true;
        state.shut().await;
    }
}

impl State {
    /// `argv` in this generation's box, through init. stdio is null
    /// unless the caller pipes it.
    fn command(&self, argv: &[&str]) -> io::Result<Command> {
        let argv: Vec<String> = argv.iter().map(|word| word.to_string()).collect();
        let mut command = Command::from(runtime().exec_command(
            &self.name,
            &self.init,
            &ExecContext::default(),
            &argv,
        )?);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        Ok(command)
    }

    async fn present(&self) -> bool {
        let Ok(mut child) = self
            .command(&["rex", "--autostart=false", "version"])
            .and_then(|mut command| command.spawn_guarded())
        else {
            return false;
        };
        match tokio::time::timeout(HELPER_DEADLINE, child.wait()).await {
            Ok(Ok(status)) => status.success(),
            _ => {
                let _ = child.kill().await;
                false
            }
        }
    }

    fn open(&self) -> io::Result<Relay> {
        let (socket, listener) = HostSocket::bind(&self.leaf)?;
        let listen = format!("unix://{REX_SOCKET}");
        let init = self.init.to_string_lossy().into_owned();
        let opened = Listener::from_std(listener).and_then(|listener| {
            let dial = self.command(&[&init, "init", "rex-dial"])?;
            let server = self
                .command(&["rex", "server", "run", "--listen", &listen])?
                .spawn_guarded()?;
            Ok((listener, dial, server))
        });
        let (listener, dial, server) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let _ = socket.remove();
                return Err(error);
            }
        };
        let (stop, stopped) = watch::channel(false);
        let accept = tokio::spawn(serve(listener, dial, stopped));
        Ok(Relay {
            socket,
            stop,
            accept,
            server,
        })
    }

    async fn shut(&mut self) {
        let Some(mut relay) = self.relay.take() else {
            return;
        };
        let _ = relay.stop.send(true);
        let _ = (&mut relay.accept).await;
        let _ = relay.server.kill().await;
        let _ = relay.socket.remove();
    }
}

/// `defaults` reads the app's preference; only an explicit 0 is off.
async fn remote_control_off() -> bool {
    let mut command = Command::new("/usr/bin/defaults");
    command
        .args(["read", "com.superlogical.rex", "rex.client.remoteControl"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(child) = command.spawn_guarded() else {
        return false;
    };
    match tokio::time::timeout(HELPER_DEADLINE, child.wait_with_output()).await {
        Ok(Ok(output)) => output.status.success() && output.stdout.trim_ascii() == b"0",
        _ => false,
    }
}

/// Each connection gets its own `init rex-dial` in the box until stop.
async fn serve(listener: Listener, mut dial: Command, mut stopped: watch::Receiver<bool>) {
    dial.stdin(Stdio::piped()).stdout(Stdio::piped());
    let connection_stop = stopped.clone();
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = stopped.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((client, _)) if wire::peer(&client).is_ok() => {
                    if let Ok(child) = dial.spawn_guarded() {
                        connections.spawn(relay(client, child, connection_stop.clone()));
                    }
                }
                Ok(_) => {}
                // An accept error such as EMFILE would otherwise repeat at once.
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    while connections.join_next().await.is_some() {}
}

/// The owner copies bytes itself, so a dead owner leaves no bridge.
async fn relay(
    mut client: tokio::net::UnixStream,
    mut child: Child,
    mut stopped: watch::Receiver<bool>,
) {
    if let (Some(mut input), Some(mut output)) = (child.stdin.take(), child.stdout.take()) {
        let (mut reader, mut writer) = client.split();
        let upload = async move {
            let _ = tokio::io::copy(&mut reader, &mut input).await;
            drop(input);
            std::future::pending::<()>().await
        };
        let download = async move {
            let _ = tokio::io::copy(&mut output, &mut writer).await;
            let _ = writer.shutdown().await;
        };
        tokio::select! {
            _ = stopped.changed() => {}
            _ = download => {}
            _ = upload => {}
        }
    }
    let _ = child.kill().await;
}
