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
use crate::core::runtime::{BoxInfo, Runtime, guest_path};

/// Apple `container`.
pub struct Apple;

impl Runtime for Apple {
    fn up(&self, plan: &Plan, init: &Path) -> io::Result<Child> {
        let argv = up_argv(plan, init);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let mut command = Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Exact values go through the child's environment, so argv holds only
        // names and `ps` cannot read a secret.
        for (name, value) in &plan.env {
            if let Env::Exact(value) = value {
                command.env(name, value);
            }
        }
        command.spawn()
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

/// The `container run` argv for a no-egress box, as data.
///
/// `init` is a host path; it and its directory are mounted read-only at the
/// same path, so it is also the path PID 1 runs.
pub fn up_argv(plan: &Plan, init: &Path) -> Vec<OsString> {
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
    argv.push(plan.image.clone().into());
    argv.push("init".into());
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
