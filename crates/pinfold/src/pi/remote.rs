//! `git-remote-pinfold`: the Git remote of a disk project (Git). Git runs
//! it as `git-remote-pinfold REMOTE ROOT` for a remote whose URL is
//! `pinfold::ROOT`. It offers `fetch` and `push`, never `connect`: every
//! byte from the box passes Crossing's checks at `list`, into a quarantine
//! that `fetch` moves into the repository. Git then writes the refs.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use nix::fcntl::{FcntlArg, FdFlag, fcntl};

use crate::core::crossing::{self, CrossingError, Fetched, Refs};
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
            url_word(remote)
        )));
    }
    let (input, mut output) = protocol()?;
    let watched = input.try_clone()?;
    let git = watched.as_fd();
    let mut input = BufReader::new(input);
    let guard = |root: &Path| check(root, remote, address);
    let mut fetched: Option<Fetched> = None;
    let mut listed = false;
    // The box's `refs/remotes/host/*`, from `list for-push`.
    let mut held = Refs::new();
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
                let landing = launch::cross(
                    guard,
                    git,
                    move |runtime, init, name, root, guest, cancel| {
                        crossing::fetch_cancellable(
                            runtime, init, name, guest, root, &remote, cancel,
                        )
                    },
                )?
                .map_err(|error| io::Error::other(describe(&error)))?;
                // Git reads a listing's object ids as SHA-1 unless told.
                let mut listing = format!(":object-format {}\n", object_format()?);
                for (name, oid) in &landing.refs {
                    listing.push_str(&format!("{oid} {name}\n"));
                    // Git follows an annotated tag only by its commit.
                    if let Some(commit) = landing.peeled.get(name) {
                        listing.push_str(&format!("{commit} {name}^{{}}\n"));
                    }
                }
                listing.push('\n');
                fetched = Some(landing);
                send(&mut output, &listing)?;
            }
            "list for-push" => {
                // Only the push plan reads this; `fetched` stays empty, so
                // nothing listed here can land. Any failure but a signal
                // lists nothing and the push goes ahead: the push crosses
                // again and reports whatever still stands in its way. Git
                // sees only the tags: it would move the host's `box/BRANCH`
                // for a branch it calls up to date.
                let pushed =
                    match launch::cross(guard, git, |runtime, init, name, root, guest, cancel| {
                        crossing::pushed_cancellable(runtime, init, name, root, guest, cancel)
                    }) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                            return Err(error);
                        }
                        crossed => crossed.ok().and_then(Result::ok).unwrap_or_default(),
                    };
                let tags: Refs;
                (held, tags) = pushed
                    .into_iter()
                    .partition(|(name, _)| name.starts_with("refs/remotes/host/"));
                let mut listing = format!(":object-format {}\n", object_format()?);
                for (name, oid) in &tags {
                    listing.push_str(&format!("{oid} {name}\n"));
                }
                listing.push('\n');
                send(&mut output, &listing)?;
            }
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
                let report = push(&batch, &held, guard, git)?;
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

/// Answer each `push SRC:DST` line: a branch lands in the box's
/// `refs/remotes/host/*` and a tag in its `refs/tags/*`, every one in one
/// transaction. A tag the box holds at another object needs `+`, or the
/// whole push fails. A branch HELD already names at the host's tip does
/// not cross; when none crosses, no push runs in the box. The host reads
/// nothing from the box but its exit status.
fn push(
    batch: &[String],
    held: &Refs,
    check: impl FnOnce(&Path) -> io::Result<()>,
    git: BorrowedFd<'_>,
) -> io::Result<String> {
    let cancel = AtomicBool::new(false);
    let repo = Path::new(".");
    let (mut sources, mut refspecs, mut accepted) = (Vec::new(), Vec::new(), Vec::new());
    let mut report = String::new();
    for line in batch {
        let spec = &line["push ".len()..];
        let forced = spec.starts_with('+');
        let spec = spec.strip_prefix('+').unwrap_or(spec);
        let (source, destination) = spec.split_once(':').unwrap_or(("", spec));
        // Git names a pushed ref by its full name, so a stash or a commit
        // expression is neither a host branch nor a host tag.
        let target = if source.starts_with("refs/heads/")
            && let Some(branch) = destination.strip_prefix("refs/heads/")
        {
            Some(format!("refs/remotes/host/{branch}"))
        } else if source.starts_with("refs/tags/") && destination.starts_with("refs/tags/") {
            Some(destination.to_string())
        } else {
            None
        };
        let valid = match &target {
            Some(target) => {
                crossing::valid_ref(repo, source, &cancel)?
                    && crossing::valid_ref(repo, target, &cancel)?
            }
            None => false,
        };
        let Some(target) = target.filter(|_| valid) else {
            report.push_str(&format!(
                "error {destination} only a branch to a branch or a tag to a tag can be pushed\n"
            ));
            continue;
        };
        // A branch always moves `host/BRANCH`; a tag keeps Git's rule.
        let force = forced || target.starts_with("refs/remotes/");
        // HELD is non-empty only once `list for-push` passed CHECK.
        let current = match held.get(&target) {
            Some(id) => *id == rev_parse(source, "a pushed branch")?,
            None => false,
        };
        if !current {
            refspecs.push(format!("{}{source}:{target}", if force { "+" } else { "" }));
            sources.push(source.to_string());
        }
        accepted.push(destination.to_string());
    }
    if !accepted.is_empty() {
        let pushed = if refspecs.is_empty() {
            Ok(())
        } else {
            launch::cross(
                check,
                git,
                move |runtime, init, name, root, guest, cancel| {
                    crossing::push_cancellable(
                        runtime, init, name, root, guest, &sources, &refspecs, cancel,
                    )
                },
            )?
        };
        if let Err(error) = &pushed {
            eprintln!("pinfold: {}", describe(error));
        }
        let reason = match &pushed {
            Err(CrossingError::Failed(detail)) if detail == crossing::TAG_EXISTS_DETAIL => {
                crossing::TAG_EXISTS_DETAIL
            }
            _ => "the box refused the push",
        };
        for destination in accepted {
            report.push_str(&match &pushed {
                // Git updates the tracking ref for the reported name, the
                // box's `refs/remotes/host/BRANCH`, so the host's
                // `box/BRANCH` keeps naming the box's own branch.
                Ok(()) => match destination.strip_prefix("refs/heads/") {
                    Some(branch) => {
                        format!("ok {destination}\noption refname refs/remotes/host/{branch}\n")
                    }
                    None => format!("ok {destination}\n"),
                },
                Err(_) => format!("error {destination} {reason}\n"),
            });
        }
    }
    report.push('\n');
    Ok(report)
}

/// ROOT's remote URL as one shell word for a printed command.
pub(crate) fn pinfold_url(root: &Path) -> String {
    url_word(&format!("pinfold::{}", root.to_string_lossy()))
}

/// The name of ROOT's remote whose URL is `pinfold::ROOT` and whose name
/// the helper accepts; of several, `box` if it is one, else the first. A
/// configuration Git cannot read counts as none. An accepted name starts
/// alphanumeric, so it is no option and needs no shell quoting.
pub(crate) fn remote_name(root: &Path) -> Option<String> {
    let entries = config(root, &["-z", "--get-regexp", r"^remote\..*\.url$"]).ok()??;
    let names: Vec<&str> = entries
        .split('\0')
        .filter_map(|entry| {
            let (key, url) = entry.split_once('\n')?;
            let address = url.strip_prefix("pinfold::")?;
            (Path::new(address) == root).then_some(())?;
            let name = key.strip_prefix("remote.")?.strip_suffix(".url")?;
            valid_name(name).then_some(name)
        })
        .collect();
    names
        .iter()
        .find(|name| **name == "box")
        .or(names.first())
        .map(|name| name.to_string())
}

/// URL as one shell word; the placeholder `pinfold::ROOT` stands in for
/// one that cannot be.
fn url_word(url: &str) -> String {
    shell_word(url).unwrap_or_else(|| "pinfold::ROOT".to_string())
}

/// TEXT as one POSIX shell word: unquoted when every character is plain,
/// else single-quoted. Text holding a control character cannot be printed
/// as pasteable text: `None`.
pub(crate) fn shell_word(text: &str) -> Option<String> {
    let plain = |ch: char| ch.is_ascii_alphanumeric() || "/._+:@%=,-".contains(ch);
    if text.chars().any(char::is_control) {
        None
    } else if text.chars().all(plain) {
        Some(text.to_string())
    } else {
        Some(format!("'{}'", text.replace('\'', r"'\''")))
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
    rev_parse("--show-object-format", "the object format")
}

/// `git rev-parse ARG`'s one line; WHAT names it in a refusal.
fn rev_parse(arg: &str, what: &str) -> io::Result<String> {
    let mut command = crossing::git_command(Path::new("."));
    command
        .args(["rev-parse", arg])
        .stdin(std::process::Stdio::null());
    let output = crossing::output_cancellable(&mut command, None, &AtomicBool::new(false))?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || line.is_empty() {
        return Err(refused(&format!(
            "remote-repository: Git could not name {what}"
        )));
    }
    Ok(line)
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
