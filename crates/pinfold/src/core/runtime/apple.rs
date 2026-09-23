//! The Apple `container` runtime adapter.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::Stdio;

use tokio::process::{Child, Command};

use crate::core::plan::{Env, Plan};
use crate::core::runtime::{Runtime, guest_path};

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
    for (name, value) in &plan.env {
        argv.push("--env".into());
        match value {
            // The value stays out of argv; `container` reads it from us.
            Env::Exact(value) => argv.push(format!("{name}={value}").into()),
            Env::From { .. } => argv.push(name.into()),
        }
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
