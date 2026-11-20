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
            "remote-url: use a configured remote, not a URL: `git remote add box {}`, then `git push box BRANCH` or `git fetch box`",
            shell_word(remote)
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
                // Git reads a listing's object ids as SHA-1 unless told.
                let mut listing = format!(":object-format {}\n", object_format()?);
                for (branch, oid) in &landing.refs {
                    listing.push_str(&format!("{oid} refs/heads/{branch}\n"));
                }
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
        // Git names a pushed ref by its full name, so a stash, a tag or a
        // commit expression mapped to a branch is not a host branch.
        if !source.starts_with("refs/heads/")
            || !crossing::valid_ref(repo, source, &cancel)?
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
        match &pushed {
            Err(error) => eprintln!("pinfold: {}", describe(error)),
            Ok(true) => {
                let branch = shell_word(&accepted[0]["refs/heads/".len()..]);
                eprintln!(
                    "pinfold: the box repository is new; in the box run: git switch -c {branch} host/{branch}"
                );
            }
            Ok(false) => {}
        }
        for destination in accepted {
            report.push_str(&match &pushed {
                Ok(_) => format!("ok {destination}\n"),
                Err(_) => format!("error {destination} the box refused the push\n"),
            });
        }
    }
    report.push('\n');
    Ok(report)
}

/// ROOT's remote URL as one shell word for a printed command.
pub(crate) fn pinfold_url(root: &Path) -> String {
    shell_word(&format!("pinfold::{}", root.to_string_lossy()))
}

/// TEXT as one POSIX shell word for a printed command: unquoted when every
/// character is plain, else single-quoted. Control characters are escaped.
fn shell_word(text: &str) -> String {
    let text = terminal_text(text);
    let plain = |ch: char| ch.is_ascii_alphanumeric() || "/._+:@%=,-".contains(ch);
    if !text.is_empty() && text.chars().all(plain) {
        text
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// Refuse a URL that names another project, a repository other than ROOT's
/// own `.git`, and a configured fetch refspec that writes outside
/// `refs/remotes/REMOTE/`. A refspec given on the command line never
/// reaches the helper.
fn check(root: &Path, remote: &str, address: &str) -> io::Result<()> {
    if Path::new(address) != root {
        return Err(refused(&format!(
            "remote-url: {remote}'s URL names {}, not this project; run `git remote set-url {remote} {}`",
            terminal_text(address),
            pinfold_url(root)
        )));
    }
    let git_dir = std::env::var_os("GIT_DIR").map(std::fs::canonicalize);
    if !matches!(&git_dir, Some(Ok(dir)) if *dir == root.join(".git")) {
        return Err(refused(
            "remote-repository: run Git in the project's main checkout",
        ));
    }
    // Each moves the refs, objects or configuration Git and the checks
    // below use away from ROOT's `.git`. Git sets none for a helper.
    for name in [
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_NAMESPACE",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
        "GIT_GRAFT_FILE",
    ] {
        if std::env::var_os(name).is_some() {
            return Err(refused(&format!(
                "remote-repository: unset {name} to cross with the project's repository"
            )));
        }
    }
    let namespace = format!("refs/remotes/{remote}/");
    // Only the configuration names the remote: Git's legacy
    // `.git/remotes` and `.git/branches` files carry refspecs unchecked here.
    let Some(_) = config(root, &["--get", &format!("remote.{remote}.url")])? else {
        return Err(refused(&format!(
            "remote-url: set remote.{remote}.url; run `git remote add {remote} {}`",
            pinfold_url(root)
        )));
    };
    // Git writes through a symbolic ref to its target.
    let mut command = crossing::git_command(root);
    command
        .args(["for-each-ref", "--format=%(refname) %(symref)", &namespace])
        .stdin(std::process::Stdio::null());
    let refs = crossing::output_cancellable(&mut command, None, &AtomicBool::new(false))?;
    if !refs.status.success() {
        return Err(refused(
            "remote-repository: Git could not list the remote's refs",
        ));
    }
    for line in String::from_utf8_lossy(&refs.stdout).lines() {
        if let Some((name, target)) = line.split_once(' ')
            && !target.is_empty()
        {
            return Err(refused(&format!(
                "remote-refspec: {} is a symbolic ref to {}; delete it",
                terminal_text(name),
                terminal_text(target)
            )));
        }
    }
    let configured = config(root, &["--get-all", &format!("remote.{remote}.fetch")])?;
    for refspec in configured.as_deref().unwrap_or_default().lines() {
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

/// The repository's object format, as `git rev-parse` names it.
fn object_format() -> io::Result<String> {
    let mut command = crossing::git_command(Path::new("."));
    command
        .args(["rev-parse", "--show-object-format"])
        .stdin(std::process::Stdio::null());
    let output = crossing::output_cancellable(&mut command, None, &AtomicBool::new(false))?;
    let format = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || format.is_empty() {
        return Err(refused(
            "remote-repository: Git could not name the object format",
        ));
    }
    Ok(format)
}

/// `git config ARGS` in ROOT: the value, `None` when unset, and a refusal
/// when Git cannot read the configuration.
fn config(root: &Path, args: &[&str]) -> io::Result<Option<String>> {
    let mut command = crossing::git_command(root);
    command
        .arg("config")
        .args(args)
        .stdin(std::process::Stdio::null());
    let output = crossing::output_cancellable(&mut command, None, &AtomicBool::new(false))?;
    match output.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned())),
        Some(1) => Ok(None),
        _ => Err(refused(&format!(
            "remote-repository: Git could not read the configuration: {}",
            terminal_text(String::from_utf8_lossy(&output.stderr).trim())
        ))),
    }
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
