//! `git-remote-pinfold`: the Git remote of a disk project (Git). Git runs
//! it as `git-remote-pinfold REMOTE ROOT` for a remote whose URL is
//! `pinfold::ROOT`. It offers `fetch` and `push`, never `connect`: every
//! byte from the box passes Crossing's checks at `list`, into a quarantine
//! that `fetch` moves into the repository. Git then writes the refs.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::OwnedFd;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use nix::fcntl::{FcntlArg, FdFlag, fcntl};

use crate::core::crossing::{self, CrossingError, Fetched};
use crate::core::plan::valid_name;
use crate::formatting::terminal_text;
use crate::pi::launch;

pub fn run(args: &[String]) -> io::Result<i32> {
    let [remote, address] = args else {
        return Err(refused(
            "Git runs this helper for a remote whose URL is pinfold::ROOT",
        ));
    };
    // A URL given on the command line arrives as REMOTE; only a configured
    // remote has refspecs the helper can check.
    if !valid_name(remote) {
        return Err(refused(&format!(
            "remote-url: run `git fetch box` with the configured remote, not {}",
            terminal_text(remote)
        )));
    }
    let (input, mut output) = protocol()?;
    let mut input = BufReader::new(input);
    let guard = |root: &Path| check(root, remote, address);
    let mut fetched: Option<Fetched> = None;
    let mut listed = false;
    loop {
        let line = read_line(&mut input)?;
        match line.as_str() {
            "" => return Ok(0),
            "capabilities" => send(&mut output, "fetch\npush\n\n")?,
            "list" => {
                // The quarantine sweep keys on the pid: one listing a process.
                if listed {
                    return Err(refused("Git listed the box twice"));
                }
                listed = true;
                let remote = remote.clone();
                let landing = crossed(launch::cross(
                    guard,
                    move |runtime, init, name, root, guest, cancel| {
                        crossing::fetch_cancellable(
                            runtime, init, name, guest, root, &remote, cancel,
                        )
                    },
                )?)?;
                let mut listing: String = landing
                    .refs
                    .iter()
                    .map(|(branch, oid)| format!("{oid} refs/heads/{branch}\n"))
                    .collect();
                listing.push('\n');
                fetched = Some(landing);
                send(&mut output, &listing)?;
            }
            "list for-push" => send(&mut output, "\n")?,
            line if line.starts_with("fetch ") => {
                while !read_line(&mut input)?.is_empty() {}
                if let Some(fetched) = &fetched {
                    fetched.land()?;
                }
                send(&mut output, "\n")?;
            }
            line if line.starts_with("push ") => {
                let mut batch = vec![line.to_string()];
                loop {
                    let line = read_line(&mut input)?;
                    if line.is_empty() {
                        break;
                    }
                    batch.push(line);
                }
                let report = push(&batch, guard)?;
                send(&mut output, &report)?;
            }
            line => {
                return Err(refused(&format!(
                    "Git sent an unexpected command: {}",
                    terminal_text(line)
                )));
            }
        }
    }
}

/// Answer each `push SRC:DST` line: a branch DST lands in the box's
/// `refs/remotes/host/*`, every one in one transaction. The host reads
/// nothing from the box but whether it succeeded.
fn push(batch: &[String], check: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<String> {
    let cancel = AtomicBool::new(false);
    let repo = Path::new(".");
    let (mut sources, mut refspecs, mut accepted) = (Vec::new(), Vec::new(), Vec::new());
    let mut report = String::new();
    for line in batch {
        let spec = &line["push ".len()..];
        let spec = spec.strip_prefix('+').unwrap_or(spec);
        let (source, destination) = spec.split_once(':').unwrap_or(("", spec));
        let target = format!(
            "refs/remotes/host/{}",
            destination.strip_prefix("refs/heads/").unwrap_or_default()
        );
        if source.is_empty()
            || !destination.starts_with("refs/heads/")
            || !crossing::valid_ref(repo, &target, &cancel)?
        {
            report.push_str(&format!(
                "error {destination} only branches can be pushed\n"
            ));
            continue;
        }
        sources.push(source.to_string());
        refspecs.push(format!("+{source}:{target}"));
        accepted.push(destination.to_string());
    }
    if !accepted.is_empty() {
        let pushed = launch::cross(check, move |runtime, init, name, root, guest, cancel| {
            crossing::push_cancellable(
                runtime, init, name, root, guest, &sources, &refspecs, cancel,
            )
        })?;
        if let Err(error) = &pushed {
            eprintln!("pinfold: {}", describe(error));
        }
        for destination in accepted {
            report.push_str(&match &pushed {
                Ok(()) => format!("ok {destination}\n"),
                Err(_) => format!("error {destination} the box refused the push\n"),
            });
        }
    }
    report.push('\n');
    Ok(report)
}

/// Refuse a URL that names another project, a repository other than ROOT's
/// own `.git`, and a configured fetch refspec that writes outside
/// `refs/remotes/REMOTE/`. A refspec given on the command line never
/// reaches the helper.
fn check(root: &Path, remote: &str, address: &str) -> io::Result<()> {
    if Path::new(address) != root {
        return Err(refused(&format!(
            "remote-url: {remote}'s URL names {}, not this project; run `git remote set-url {remote} pinfold::{}`",
            terminal_text(address),
            terminal_text(&root.to_string_lossy())
        )));
    }
    let git_dir = std::env::var_os("GIT_DIR").map(std::fs::canonicalize);
    if !matches!(&git_dir, Some(Ok(dir)) if *dir == root.join(".git")) {
        return Err(refused(
            "remote-repository: run Git in the project's main checkout",
        ));
    }
    let mut command = crossing::git_command(root);
    command
        .args(["config", "--get-all", &format!("remote.{remote}.fetch")])
        .stdin(std::process::Stdio::null());
    let configured = crossing::output_cancellable(&mut command, None, &AtomicBool::new(false))?;
    let namespace = format!("refs/remotes/{remote}/");
    for refspec in String::from_utf8_lossy(&configured.stdout).lines() {
        let destination = refspec
            .strip_prefix('+')
            .unwrap_or(refspec)
            .split_once(':')
            .map_or("", |(_, destination)| destination);
        if !refspec.starts_with('^')
            && !destination.is_empty()
            && !destination.starts_with(&namespace)
        {
            return Err(refused(&format!(
                "remote-refspec: remote.{remote}.fetch writes outside {namespace}: {}",
                terminal_text(refspec)
            )));
        }
    }
    Ok(())
}

/// The fds Git speaks to the helper on. Standard input becomes
/// `/dev/null` and standard output stderr, so nothing pinfold or a child
/// prints reaches the protocol.
fn protocol() -> io::Result<(File, File)> {
    let input = File::from(cloexec(nix::unistd::dup(io::stdin())?)?);
    let output = File::from(cloexec(nix::unistd::dup(io::stdout())?)?);
    nix::unistd::dup2_stdin(File::open("/dev/null")?)?;
    nix::unistd::dup2_stdout(io::stderr())?;
    Ok((input, output))
}

fn cloexec(fd: OwnedFd) -> io::Result<OwnedFd> {
    fcntl(&fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
    Ok(fd)
}

/// One protocol line without its newline; empty at the end of input.
fn read_line(input: &mut impl BufRead) -> io::Result<String> {
    let mut line = Vec::new();
    input.read_until(b'\n', &mut line)?;
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    String::from_utf8(line).map_err(|_| refused("Git sent a line that is not UTF-8"))
}

fn send(output: &mut File, text: &str) -> io::Result<()> {
    output.write_all(text.as_bytes())?;
    output.flush()
}

/// A crossing's outcome, its refusal or failure as an error to report.
fn crossed<T>(result: Result<T, CrossingError>) -> io::Result<T> {
    result.map_err(|error| io::Error::other(describe(&error)))
}

fn describe(error: &CrossingError) -> String {
    match error {
        CrossingError::Refused(reason, detail) => format!(
            "{}: {}",
            serde_json::json!(reason).as_str().unwrap_or("refused"),
            terminal_text(detail)
        ),
        CrossingError::Failed(detail) => terminal_text(detail),
    }
}

fn refused(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail.to_string())
}
