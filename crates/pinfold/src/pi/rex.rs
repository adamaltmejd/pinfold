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
use tokio::time::{Instant, sleep, sleep_until, timeout_at};

use crate::core::r#box::Signals;
use crate::core::ownership;
use crate::core::process::{AsyncCommandGuard, Listener};
use crate::core::runtime::runtime;
use crate::core::runtime::sockets::HostSocket;
use crate::init::REX_SOCKET;
use crate::pi::session::forward_diagnostics;
use crate::pi::session_wire as wire;

/// Bounds each helper: the `rex` probe, each preference read, the wait for
/// the guest server, and a connection's reply after the host's end.
const HELPER_DEADLINE: Duration = Duration::from_secs(5);

/// A start's wait for the owner's answer covers every helper bound.
pub(crate) const REPLY_DEADLINE: Duration = Duration::from_secs(30);

/// The owner's answer to a start that asked for the relay.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    /// No runnable `rex` in the box.
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

/// The socket is named for the project id, so Rex names the host after it.
fn leaf(id: &str) -> String {
    format!("{id}.rex")
}

/// Remove this project's socket. Only a caller that has established that
/// no owner is live, and none can start, may call this.
pub(crate) fn remove_left(id: &str) {
    if cfg!(target_os = "macos") {
        let _ = HostSocket::remove_left(&leaf(id));
    }
}

/// Print a start's outcome on stderr. An absent `rex` prints nothing.
pub(crate) async fn report(
    status: Option<Status>,
    id: &str,
    signals: &mut Signals,
) -> io::Result<()> {
    let lines = match status {
        // The address alone on its line, so it selects and copies cleanly.
        Some(Status::Relayed) => vec![
            "pinfold start: rex: add this address in the Rex app under Add Remote Host…".into(),
            format!(
                "unix://{}",
                ownership::host_dir()?.join(leaf(id)).display()
            ),
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
        forward_diagnostics(&line, signals).await?;
    }
    Ok(())
}

/// The owner's relay for its generation. Requests serialize on the lock.
#[derive(Clone)]
pub(crate) struct Rex {
    state: Arc<Mutex<State>>,
    closing: Arc<watch::Sender<bool>>,
}

struct State {
    name: String,
    init: PathBuf,
    leaf: String,
    relay: Option<Relay>,
}

struct Relay {
    socket: HostSocket,
    stop: watch::Sender<bool>,
    accept: tokio::task::JoinHandle<()>,
}

/// Every drop, a cancelled `shut` included, stops the accept task and
/// unlinks the socket.
impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        let _ = self.socket.remove();
    }
}

impl Rex {
    /// Called by a new owner while it holds the launch lock, so a socket a
    /// dead owner left is removed here.
    pub(crate) fn new(name: &str, init: PathBuf, id: &str) -> Self {
        remove_left(id);
        Self {
            state: Arc::new(Mutex::new(State {
                name: name.to_owned(),
                init,
                leaf: leaf(id),
                relay: None,
            })),
            closing: Arc::new(watch::channel(false).0),
        }
    }

    /// `None` once shutdown began: pending setup is dropped before it binds.
    pub(crate) async fn request(&self) -> Option<Status> {
        let mut closing = self.closing.subscribe();
        let work = async {
            let mut state = self.state.lock().await;
            let status = state.request().await;
            // Setup that finished as shutdown began is not answered.
            (!*self.closing.borrow()).then_some(status)
        };
        tokio::select! {
            biased;
            // The sender lives in `self`, so the wait never fails.
            _ = closing.wait_for(|closing| *closing) => None,
            status = work => status,
        }
    }

    /// The shutdown transition: refuse requests and cancel pending setup.
    pub(crate) fn begin_close(&self) {
        self.closing.send_replace(true);
    }

    /// End the relay before teardown.
    pub(crate) async fn close(&self) {
        self.begin_close();
        self.state.lock().await.shut().await;
    }
}

impl State {
    /// Check for `rex` in the box, then the app's Remote Control, then
    /// relay. A relay is kept while its server runs and Remote Control
    /// stays off.
    async fn request(&mut self) -> Status {
        if self
            .relay
            .as_ref()
            .is_some_and(|relay| relay.accept.is_finished())
        {
            self.shut().await;
        }
        if self.relay.is_none() {
            let present = match self
                .command(&["rex", "--autostart=false", "version"])
                .spawn_guarded()
            {
                Ok(mut child) => wait_success(&mut child, Instant::now() + HELPER_DEADLINE).await,
                Err(_) => false,
            };
            if !present {
                return Status::Absent;
            }
        }
        if !remote_control_off().await {
            self.shut().await;
            return Status::RemoteControl;
        }
        if self.relay.is_some() {
            return Status::Relayed;
        }
        self.open().await
    }

    /// `argv` in this generation's box, through init. stdio is null
    /// unless the caller pipes it.
    fn command(&self, argv: &[&str]) -> Command {
        let argv: Vec<String> = argv.iter().map(|word| word.to_string()).collect();
        let mut command = Command::from(runtime().exec_command(&self.name, &self.init, &argv));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }

    fn dial(&self) -> Command {
        let init = self.init.to_string_lossy().into_owned();
        self.command(&[&init, "init", "rex-dial"])
    }

    /// Start the guest server, wait until it accepts, read Remote Control
    /// again, and only then bind the host socket.
    async fn open(&mut self) -> Status {
        let listen = format!("unix://{REX_SOCKET}");
        let Ok(mut server) = self
            .command(&["rex", "server", "run", "--listen", &listen])
            .spawn_guarded()
        else {
            return Status::Failed;
        };
        let (socket, listener, dial) = match self.bind().await {
            Ok(bound) => bound,
            Err(status) => {
                let _ = server.kill().await;
                return status;
            }
        };
        let (stop, stopped) = watch::channel(false);
        let accept = tokio::spawn(serve(listener, dial, server, stopped));
        self.relay = Some(Relay {
            socket,
            stop,
            accept,
        });
        Status::Relayed
    }

    /// Wait for the started server, then bind the host socket.
    async fn bind(&self) -> Result<(HostSocket, Listener, Command), Status> {
        if !self.ready().await {
            return Err(Status::Failed);
        }
        if !remote_control_off().await {
            return Err(Status::RemoteControl);
        }
        let dial = self.dial();
        let (socket, listener) = HostSocket::bind(&self.leaf).map_err(|_| Status::Failed)?;
        let listener = Listener::from_std(listener).map_err(|_| {
            let _ = socket.remove();
            Status::Failed
        })?;
        Ok((socket, listener, dial))
    }

    /// A dial with closed stdin exits 0 only after connecting.
    async fn ready(&self) -> bool {
        let deadline = Instant::now() + HELPER_DEADLINE;
        while Instant::now() < deadline {
            if let Ok(mut child) = self.dial().spawn_guarded()
                && wait_success(&mut child, deadline).await
            {
                return true;
            }
            sleep(Duration::from_millis(100)).await;
        }
        false
    }

    async fn shut(&mut self) {
        let Some(mut relay) = self.relay.take() else {
            return;
        };
        let _ = relay.stop.send(true);
        let _ = (&mut relay.accept).await;
    }
}

async fn wait_success(child: &mut Child, deadline: Instant) -> bool {
    match timeout_at(deadline, child.wait()).await {
        Ok(Ok(status)) => status.success(),
        _ => {
            let _ = child.kill().await;
            false
        }
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

/// Each connection gets its own `init rex-dial` in the box. The relay ends
/// on stop or when the server exec exits; a later start sets it up again.
async fn serve(
    listener: Listener,
    mut dial: Command,
    mut server: Child,
    mut stopped: watch::Receiver<bool>,
) {
    dial.stdin(Stdio::piped()).stdout(Stdio::piped());
    let (end, ended) = watch::channel(false);
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = stopped.changed() => break,
            _ = server.wait() => break,
            accepted = listener.accept() => match accepted {
                Ok((client, _)) if wire::peer(&client).is_ok() => {
                    if let Ok(child) = dial.spawn_guarded() {
                        connections.spawn(relay(client, child, ended.clone()));
                    }
                }
                Ok(_) => {}
                // An accept error such as EMFILE would otherwise repeat at once.
                Err(_) => sleep(Duration::from_millis(100)).await,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    drop(listener);
    let _ = end.send(true);
    while connections.join_next().await.is_some() {}
    let _ = server.kill().await;
}

/// The owner copies bytes itself, so a dead owner leaves no bridge. The
/// host's end reaches the guest, whose reply may follow for a bounded time.
/// The guest's end ends the connection: `rex-dial` exits there.
async fn relay(client: tokio::net::UnixStream, mut child: Child, mut ended: watch::Receiver<bool>) {
    if let (Some(mut input), Some(mut output)) = (child.stdin.take(), child.stdout.take()) {
        let (mut reader, mut writer) = client.into_split();
        let upload = async move {
            let copied = tokio::io::copy(&mut reader, &mut input).await;
            drop(input);
            copied
        };
        let download = async move {
            let _ = tokio::io::copy(&mut output, &mut writer).await;
            let _ = writer.shutdown().await;
        };
        tokio::pin!(upload, download);
        let mut host_ended: Option<Instant> = None;
        loop {
            tokio::select! {
                _ = ended.changed() => break,
                _ = &mut download => break,
                copied = &mut upload, if host_ended.is_none() => match copied {
                    Ok(_) => host_ended = Some(Instant::now() + HELPER_DEADLINE),
                    Err(_) => break,
                },
                _ = sleep_until(host_ended.unwrap_or_else(Instant::now)), if host_ended.is_some() => break,
            }
        }
    }
    let _ = child.kill().await;
}
