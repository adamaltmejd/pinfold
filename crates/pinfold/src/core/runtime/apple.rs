//! The Apple `container` runtime adapter.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use serde::Deserialize;
use tokio::process::{Child, Command};

use crate::core::plan::{Env, Plan};
use crate::core::proxy::PROXY_URL;
use crate::core::runtime::{BoxInfo, BuildRequest, ImageInfo, Runtime, guest_path};

/// Where Apple `container` forwards `SSH_AUTH_SOCK` inside the box.
pub const GUEST_PROXY_SOCKET: &str = "/var/host-services/ssh-auth.sock";

/// Apple `container`.
pub struct Apple;

impl Runtime for Apple {
    fn up(&self, plan: &Plan, init: &Path, proxy_socket: Option<&Path>) -> io::Result<Child> {
        let argv = up_argv(plan, init, proxy_socket);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let mut command = Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Values go through the child's environment, so argv holds only names
        // and `ps` cannot read a secret. A `from` entry names a host variable
        // the box sees under the entry's key; the two need not match.
        for (name, value) in &plan.env {
            match value {
                Env::Exact(value) => {
                    command.env(name, value);
                }
                Env::From { from } => {
                    if let Some(value) = std::env::var_os(from) {
                        command.env(name, value);
                    }
                }
            }
        }
        if let Some(socket) = proxy_socket {
            command.env("SSH_AUTH_SOCK", socket);
        }
        // Always applied: Node's fetch reads the proxy variables only with
        // this set.
        command.env("NODE_USE_ENV_PROXY", "1");
        if plan.egress.is_some() {
            command.env("HTTPS_PROXY", PROXY_URL);
            command.env("http_proxy", PROXY_URL);
        }
        command.spawn()
    }

    fn make_proxy_connectable(&self, name: &str) -> io::Result<()> {
        let argv = chmod_proxy_argv(name);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let status = std::process::Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "container exec chmod {GUEST_PROXY_SOCKET}: {status}"
            )))
        }
    }

    fn down(&self, name: &str) -> io::Result<()> {
        let argv = down_argv(name);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let status = std::process::Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "container rm -f {name}: {status}"
            )))
        }
    }

    fn exec(
        &self,
        name: &str,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus> {
        let argv = exec_argv(name, tty, workdir, argv);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let mut command = std::process::Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if !tty {
            // Without a TTY the runtime's exec gets its own process group, so
            // a terminal signal reaches pinfold and not the exec.
            command.process_group(0);
        }
        command.status()
    }

    fn list(&self) -> io::Result<Vec<BoxInfo>> {
        let output = std::process::Command::new("container")
            .args(["list", "--all", "--format", "json"])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "container list: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_list(&output.stdout)
    }

    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        let output = std::process::Command::new("container")
            .args(["image", "list", "--format", "json"])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "container image list: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_images(&output.stdout)
    }

    fn remove_image(&self, reference: &str) -> io::Result<()> {
        // `image delete` also collects the layers no image references.
        let output = std::process::Command::new("container")
            .args(["image", "delete", reference])
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "container image delete {reference}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn build(&self, request: &BuildRequest) -> io::Result<()> {
        let argv = build_argv(request);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let status = std::process::Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            // The build prints the tags on stdout; pinfold prints the ref
            // itself, so the caller's stdout holds only the ref.
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("container build: {status}")))
        }
    }

    fn image_digest(&self, reference: &str) -> io::Result<Option<String>> {
        // A floating tag must be pulled for its digest to be current and
        // present to inspect. `scratch` and other non-registry references
        // cannot be pulled; they simply have no digest.
        let _ = std::process::Command::new("container")
            .args(["image", "pull", reference])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status();
        let output = std::process::Command::new("container")
            .args(["image", "inspect", reference])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        parse_digest(&output.stdout)
    }
}

/// One `container list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedContainer {
    id: String,
    #[serde(default)]
    configuration: ListedConfiguration,
}

#[derive(Default, Deserialize)]
struct ListedConfiguration {
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

fn parse_list(json: &[u8]) -> io::Result<Vec<BoxInfo>> {
    let containers: Vec<ListedContainer> = serde_json::from_slice(json).map_err(|error| {
        io::Error::other(format!("container list returned invalid JSON: {error}"))
    })?;
    Ok(containers
        .into_iter()
        .map(|container| BoxInfo {
            id: container.id,
            labels: container.configuration.labels,
        })
        .collect())
}

/// One `container image list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedImage {
    id: String,
    configuration: ListedImageConfiguration,
    #[serde(default)]
    variants: Vec<ListedImageVariant>,
}

#[derive(Deserialize)]
struct ListedImageConfiguration {
    name: String,
    descriptor: Option<ListedImageDescriptor>,
}

#[derive(Deserialize)]
struct ListedImageDescriptor {
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ListedImageVariant {
    config: Option<ListedImageConfig>,
}

#[derive(Deserialize)]
struct ListedImageConfig {
    config: Option<ListedImageLabels>,
}

#[derive(Deserialize)]
struct ListedImageLabels {
    #[serde(default, rename = "Labels")]
    labels: BTreeMap<String, String>,
}

fn parse_images(json: &[u8]) -> io::Result<Vec<ImageInfo>> {
    let images: Vec<ListedImage> = serde_json::from_slice(json).map_err(|error| {
        io::Error::other(format!(
            "container image list returned invalid JSON: {error}"
        ))
    })?;
    Ok(images
        .into_iter()
        .map(|image| {
            let ListedImage {
                id,
                configuration,
                variants,
            } = image;
            // Build labels are OCI image config labels; a locally built
            // image also carries name annotations on its index descriptor.
            let mut labels = configuration
                .descriptor
                .map(|descriptor| descriptor.annotations)
                .unwrap_or_default();
            for variant in &variants {
                if let Some(config) = variant
                    .config
                    .as_ref()
                    .and_then(|config| config.config.as_ref())
                {
                    labels.extend(config.labels.clone());
                }
            }
            ImageInfo {
                id,
                reference: configuration.name,
                labels,
            }
        })
        .collect())
}

/// One `container image inspect` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct InspectedImage {
    configuration: InspectedConfiguration,
}

#[derive(Deserialize)]
struct InspectedConfiguration {
    descriptor: InspectedDescriptor,
}

#[derive(Deserialize)]
struct InspectedDescriptor {
    digest: String,
}

fn parse_digest(json: &[u8]) -> io::Result<Option<String>> {
    let images: Vec<InspectedImage> = serde_json::from_slice(json).map_err(|error| {
        io::Error::other(format!(
            "container image inspect returned invalid JSON: {error}"
        ))
    })?;
    Ok(images
        .into_iter()
        .next()
        .map(|image| image.configuration.descriptor.digest))
}

/// The `container run` argv for a box, as data.
///
/// `init` is a host path; it and its directory are mounted read-only at the
/// same path, so it is also the path PID 1 runs. When `proxy_socket` is set,
/// the box carries the socket in over `--ssh` and init relays to the guest
/// path as its argument.
pub fn up_argv(plan: &Plan, init: &Path, proxy_socket: Option<&Path>) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        "container".into(),
        "run".into(),
        "-i".into(),
        "--progress".into(),
        "none".into(),
        "--name".into(),
        plan.name.clone().into(),
        "--network".into(),
        "none".into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--read-only".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--user".into(),
        user(plan),
    ];
    if proxy_socket.is_some() {
        // Off-label: the runtime treats the socket as an SSH agent and
        // forwards it to GUEST_PROXY_SOCKET.
        argv.push("--ssh".into());
    }
    if let Some(cpus) = plan.cpus {
        argv.push("--cpus".into());
        argv.push(cpus.to_string().into());
    }
    if let Some(memory) = &plan.memory {
        argv.push("--memory".into());
        argv.push(memory.into());
    }
    for (key, value) in &plan.labels {
        argv.push("--label".into());
        argv.push(format!("{key}={value}").into());
    }
    for name in plan.env.keys() {
        // Names only: `container` reads the value from our environment.
        argv.push("--env".into());
        argv.push(name.into());
    }
    // Always applied: Node's fetch reads the proxy variables only with this
    // set.
    argv.push("--env".into());
    argv.push("NODE_USE_ENV_PROXY".into());
    if plan.egress.is_some() {
        argv.push("--env".into());
        argv.push("HTTPS_PROXY".into());
        argv.push("--env".into());
        argv.push("http_proxy".into());
    }
    for mount in &plan.mounts {
        argv.push("--mount".into());
        argv.push(bind(&mount.host, &mount.guest, mount.readonly));
    }

    // The init binary comes from the host and runs as PID 1. box::up refuses
    // an init without a directory, so the parent is a real directory here.
    let init_dir = init.parent().unwrap_or(Path::new("/"));
    argv.push("--mount".into());
    argv.push(bind(init_dir, &guest_path(init_dir), true));
    argv.push("--entrypoint".into());
    argv.push(guest_path(init).into_os_string());
    argv.push(
        plan.image
            .clone()
            .expect("box up resolves the profile's image before the runtime runs")
            .into(),
    );
    argv.push("init".into());
    if proxy_socket.is_some() {
        argv.push(GUEST_PROXY_SOCKET.into());
    }
    argv
}

/// The one transient root exec that makes the forwarded socket connectable
/// by the box user, as data.
pub fn chmod_proxy_argv(name: &str) -> Vec<OsString> {
    vec![
        "container".into(),
        "exec".into(),
        "--user".into(),
        "0:0".into(),
        name.into(),
        "chmod".into(),
        "666".into(),
        GUEST_PROXY_SOCKET.into(),
    ]
}

/// The `container build` argv for one build, as data.
pub fn build_argv(request: &BuildRequest) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        "container".into(),
        "build".into(),
        "--file".into(),
        request.containerfile.into(),
    ];
    for tag in request.tags {
        argv.push("--tag".into());
        argv.push(tag.into());
    }
    for (key, value) in request.labels {
        argv.push("--label".into());
        argv.push(format!("{key}={value}").into());
    }
    argv.push(request.context.into());
    argv
}

/// The `container rm` argv that stops and removes a box, as data.
pub fn down_argv(name: &str) -> Vec<OsString> {
    vec!["container".into(), "rm".into(), "-f".into(), name.into()]
}

/// The `container exec` argv, as data.
///
/// The process inherits the run's user, so box-created files stay the host
/// user's. With a TTY, keep the host's terminal identity: the runtime
/// otherwise reports `TERM=xterm`.
pub fn exec_argv(name: &str, tty: bool, workdir: Option<&Path>, argv: &[String]) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["container".into(), "exec".into(), "-i".into()];
    if tty {
        args.push("-t".into());
        for variable in ["TERM", "COLORTERM"] {
            if let Ok(value) = std::env::var(variable) {
                args.push("--env".into());
                args.push(format!("{variable}={value}").into());
            }
        }
    }
    if let Some(dir) = workdir {
        args.push("--workdir".into());
        args.push(dir.as_os_str().to_os_string());
    }
    args.push(name.into());
    for arg in argv {
        args.push(OsString::from(arg));
    }
    args
}

fn bind(host: &Path, guest: &Path, readonly: bool) -> OsString {
    let mut value = OsString::from("type=bind,source=");
    value.push(host);
    value.push(",target=");
    value.push(guest);
    if readonly {
        value.push(",readonly");
    }
    value
}

fn user(plan: &Plan) -> OsString {
    match plan.user {
        Some(user) => {
            let (uid, gid) = (user.uid, user.gid);
            format!("{uid}:{gid}").into()
        }
        None => {
            let (uid, gid) = (nix::unistd::getuid(), nix::unistd::getgid());
            format!("{uid}:{gid}").into()
        }
    }
}
