//! The branch crossing (Disks, Crossing): `push` and `fetch` move git
//! branches between a host repository and a live box's as bundles. The
//! box's side runs in the box as the box user. Neither touches a working
//! tree, and nothing from the box runs on the host: the host only verifies
//! a bundle, fetches its objects with fsck, and writes refs under one
//! remote namespace.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use crate::core::r#box::RefusalReason;
use crate::core::plan::valid_name;
use crate::core::runtime::Runtime;
use crate::dirs;

/// A fetched bundle larger than this is refused as `limit`.
const LIMIT: u64 = 4 << 30;

/// A box listing or bundling more branches than this fails the fetch.
const BRANCHES: usize = 4096;

/// The box script's exit code for a GUEST_REPO that is not a repository,
/// which the host reports as `spec`.
const NOT_A_REPO: i32 = 64;

/// The prefix of a fetch's quarantine under REPO's objects dir.
const QUARANTINE: &str = "pinfold-incoming-";

/// The bytes of a box command's stderr kept for a `failed` detail.
const STDERR_KEPT: u64 = 16 << 10;

/// `$1` is GUEST_REPO, `$2` the remote name. The bundle arrives on stdin and
/// is fetched into `refs/remotes/$2/*` only.
const PUSH_SCRIPT: &str = r#"
git -C "$1" rev-parse --git-dir >/dev/null || exit 64
dir=$(git -C "$1" rev-parse --absolute-git-dir) || exit
bundle=$(mktemp "$dir/pinfold-push.XXXXXX") || exit
trap 'rm -f "$bundle"' EXIT
cat >"$bundle" &&
  git -C "$1" fetch -q --atomic --no-tags --no-write-fetch-head "$bundle" "+refs/heads/*:refs/remotes/$2/*"
"#;

/// `$1` is GUEST_REPO; stdin holds the commits the host already has, one
/// per line, read whole before any output. Prints every branch as
/// `OID REF`, an empty line, then a bundle of what the host lacks, or
/// nothing when it lacks nothing. A branch whose tip the host holds is left
/// out of the bundle by git, so the listing names it.
const FETCH_SCRIPT: &str = r#"
repo=$1
git -C "$repo" rev-parse --git-dir >/dev/null || exit 64
held=$(git -C "$repo" cat-file --batch-check='%(objectname) %(objecttype)' |
  sed -n 's/^\([0-9a-f]*\) commit$/^\1/p') || exit
git -C "$repo" for-each-ref --format='%(objectname) %(refname)' refs/heads/ || exit
echo
revs() { echo --branches; printf '%s\n' $held; }
[ -n "$(revs | git -C "$repo" rev-list -n1 --stdin)" ] || exit 0
revs | exec git -C "$repo" bundle create -q - --stdin
"#;

pub enum CrossingError {
    Refused(RefusalReason, String),
    Failed(String),
}

impl From<io::Error> for CrossingError {
    fn from(error: io::Error) -> CrossingError {
        CrossingError::Failed(error.to_string())
    }
}

/// The refs a crossing set on the receiving side, by full name.
pub type Refs = BTreeMap<String, String>;

/// Send REPO's branches into the box's GUEST_REPO as
/// `refs/remotes/REMOTE/*`.
pub fn push(
    runtime: &dyn Runtime,
    init: &Path,
    name: &str,
    repo: &Path,
    guest_repo: &str,
    remote: &str,
) -> Result<Refs, CrossingError> {
    check_args(repo, guest_repo, remote)?;
    if git(repo, &["for-each-ref", "--count=1", "refs/heads/"])?
        .stdout
        .is_empty()
    {
        return Err(spec(format!("{} has no branches", repo.display())));
    }
    let bundle = Bundle::new()?;
    let path = bundle
        .path
        .to_str()
        .ok_or_else(|| io::Error::other(format!("{} is not UTF-8", bundle.path.display())))?;
    succeeded(git(repo, &["bundle", "create", "-q", path, "--branches"])?)?;
    let heads = bundle_heads(repo, path)?;
    let mut command = runtime.exec_command(
        name,
        init,
        false,
        None,
        &script(PUSH_SCRIPT, [guest_repo, remote]),
    );
    command
        .stdin(File::open(&bundle.path)?)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stderr = kept_stderr(&mut child);
    let status = child.wait()?;
    box_status(status.code(), &stderr.join(), guest_repo)?;
    Ok(heads
        .into_iter()
        .map(|(branch, oid)| (format!("refs/remotes/{remote}/{branch}"), oid))
        .collect())
}

/// Bring the box's GUEST_REPO branches into REPO as `refs/remotes/REMOTE/*`.
pub fn fetch(
    runtime: &dyn Runtime,
    init: &Path,
    name: &str,
    guest_repo: &str,
    repo: &Path,
    remote: &str,
) -> Result<Refs, CrossingError> {
    check_args(repo, guest_repo, remote)?;
    let namespace = format!("refs/remotes/{remote}/");
    let held = String::from_utf8_lossy(
        &succeeded(git(
            repo,
            &["for-each-ref", "--format=%(objectname)", &namespace],
        )?)?
        .stdout,
    )
    .into_owned();
    let mut command =
        runtime.exec_command(name, init, false, None, &script(FETCH_SCRIPT, [guest_repo]));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stderr = kept_stderr(&mut child);
    let mut stdin = child.stdin.take().expect("stdin is piped");
    // A box that stops reading only fails its own fetch.
    std::thread::spawn(move || stdin.write_all(held.as_bytes()));
    let bundle = Bundle::new()?;
    // `receive` drops the pipe when it returns, so a box still writing
    // ends on its own. A box past the limit is killed outright.
    let received = receive(&mut child, &bundle.path);
    if let Err(limit @ CrossingError::Refused(RefusalReason::Limit, _)) = received {
        let _ = child.kill();
        let _ = child.wait();
        return Err(limit);
    }
    let status = child.wait()?;
    // The exit code says why a box command failed, even when it left its
    // listing short.
    box_status(status.code(), &stderr.join(), guest_repo)?;
    let listed = received?;
    for (branch, _) in &listed {
        check_branch(repo, &namespace, branch)?;
    }

    // The box's listing and bundle are untrusted. Every branch name passes
    // check-ref-format under the namespace before git sees it in a refspec
    // or a ref update. A bundled head lands at a commit the bundle carries,
    // which index-pack checks with fsck, or one the host already holds
    // connected: fetch's own connectivity check covers a head whose
    // objects are already here. A listed branch the bundle leaves out lands
    // only at a commit this remote's refs already reach. Every object the
    // box sends lands in a quarantine first, so a rejected bundle leaves
    // none of its bytes in REPO.
    let quarantine = Quarantine::new(repo)?;
    let mut refs = BTreeMap::new();
    let path = bundle
        .path
        .to_str()
        .ok_or_else(|| io::Error::other(format!("{} is not UTF-8", bundle.path.display())))?;
    if fs::metadata(&bundle.path)?.len() > 0 {
        let (filtered, pack_start) = header(&bundle.path)?;
        if filtered {
            return Err(CrossingError::Failed(
                "the box sent a filtered bundle".into(),
            ));
        }
        succeeded(git(repo, &["bundle", "verify", "-q", path])?)?;
        let heads = bundle_heads(repo, path)?;
        if heads.len() > BRANCHES {
            return Err(too_many());
        }
        for branch in heads.keys() {
            check_branch(repo, &namespace, branch)?;
        }
        let wanted: String = heads
            .keys()
            .map(|branch| format!("refs/heads/{branch}\n"))
            .collect();
        // Older host git (Ubuntu 24.04's 2.43) ignores fetch.fsckObjects
        // when it fetches from a bundle, so pinfold indexes the bundle's
        // pack with fsck itself. The fetch then finds every object here and
        // only checks connectivity. Revisit when every supported host's git
        // fscks a bundle fetch.
        let mut pack = File::open(&bundle.path)?;
        pack.seek(SeekFrom::Start(pack_start))?;
        succeeded(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["index-pack", "--stdin", "--fix-thin", "--fsck-objects"])
                .envs(quarantine.env())
                .stdin(pack)
                .output()?,
        )?;
        // A fetch that finds a head missing unbundles the pack again before
        // its connectivity check fails, so it runs quarantined too. Its auto
        // maintenance would see only the quarantine.
        succeeded(git_input(
            repo,
            &[
                "fetch",
                "-q",
                "--no-auto-maintenance",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-write-fetch-head",
                "--stdin",
                path,
            ],
            wanted.as_bytes(),
            &quarantine.env(),
        )?)?;
        refs.extend(heads);
    }
    for (branch, oid) in listed {
        if refs.contains_key(&branch) {
            continue;
        }
        let reached = git(
            repo,
            &[
                "rev-list",
                "-n1",
                &format!("{oid}^{{commit}}"),
                "--not",
                &format!("--glob={namespace}*"),
            ],
        )?;
        if !reached.status.success() || !reached.stdout.is_empty() {
            return Err(CrossingError::Failed(format!(
                "branch {branch:?} names {oid}, which the bundle does not carry and {namespace}* does not reach"
            )));
        }
        refs.insert(branch, oid);
    }

    // One transaction: every ref lands or none does. `-z` keeps each value
    // a single field, never a revision expression read from a name.
    let refs: Refs = refs
        .into_iter()
        .map(|(branch, oid)| (format!("{namespace}{branch}"), oid))
        .collect();
    let updates: String = refs
        .iter()
        .map(|(name, oid)| format!("update {name}\0{oid}\0\0"))
        .collect();
    if !refs.is_empty() {
        update_refs(repo, &quarantine, &updates)?;
    }
    Ok(refs)
}

/// Prepare the ref transaction against the quarantine, so a ref git
/// refuses, such as `x` beside `x/y`, fails it before any pack moves into
/// REPO; then migrate and commit. Closing stdin before `commit` aborts it.
fn update_refs(repo: &Path, quarantine: &Quarantine, updates: &str) -> Result<(), CrossingError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["update-ref", "-z", "--stdin"])
        .envs(quarantine.env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    // A git that exits early is read as not prepared, with its stderr.
    let _ = stdin.write_all(format!("start\0{updates}prepare\0").as_bytes());
    let mut replies = String::new();
    for _ in 0..2 {
        stdout.read_line(&mut replies)?;
    }
    if replies != "start: ok\nprepare: ok\n" {
        drop(stdin);
        return Err(host_failed(&child.wait_with_output()?));
    }
    let migrated = quarantine.migrate();
    if migrated.is_ok() {
        let _ = stdin.write_all(b"commit\0");
    }
    drop(stdin);
    let output = child.wait_with_output()?;
    migrated?;
    succeeded(output)?;
    Ok(())
}

fn check_args(repo: &Path, guest_repo: &str, remote: &str) -> Result<(), CrossingError> {
    if !valid_name(remote) {
        return Err(spec(format!(
            "remote name {remote:?} must start alphanumeric and hold only [A-Za-z0-9._-]"
        )));
    }
    if !guest_repo.starts_with('/') {
        return Err(spec(format!(
            "guest repository {guest_repo:?} is not absolute"
        )));
    }
    let found = git(repo, &["rev-parse", "--git-dir"])?;
    if !found.status.success() {
        return Err(spec(format!(
            "{} is not a git repository: {}",
            repo.display(),
            String::from_utf8_lossy(&found.stderr).trim()
        )));
    }
    Ok(())
}

/// Fail unless `NAMESPACE + branch` is a well-formed ref name: no `:`,
/// space, `..`, `@{` or other text git would read as more than one name.
fn check_branch(repo: &Path, namespace: &str, branch: &str) -> Result<(), CrossingError> {
    let name = format!("{namespace}{branch}");
    if git(repo, &["check-ref-format", &name])?.status.success() {
        Ok(())
    } else {
        Err(CrossingError::Failed(format!(
            "the box named a malformed branch: {branch:?}"
        )))
    }
}

fn too_many() -> CrossingError {
    CrossingError::Failed(format!("the box has more than {BRANCHES} branches"))
}

fn spec(detail: String) -> CrossingError {
    CrossingError::Refused(RefusalReason::Spec, detail)
}

/// `sh -c SCRIPT sh ARGS…`: the arguments stay positional, never parsed as
/// shell.
fn script<'a>(script: &'a str, args: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    ["sh", "-c", script, "sh"]
        .into_iter()
        .chain(args)
        .map(str::to_owned)
        .collect()
}

/// Map the box command's exit code: 0 passes, [`NOT_A_REPO`] is `spec`, and
/// anything else, a signal included, is `failed` with its stderr.
fn box_status(code: Option<i32>, stderr: &str, guest_repo: &str) -> Result<(), CrossingError> {
    match code {
        Some(0) => Ok(()),
        Some(NOT_A_REPO) => Err(spec(format!(
            "{guest_repo} is not a git repository in the box: {}",
            stderr.trim()
        ))),
        _ => Err(CrossingError::Failed(format!(
            "the box's git failed: {}",
            stderr.trim()
        ))),
    }
}

/// Read the fetch script's branch listing, then copy the bundle after it to
/// `bundle`, refusing one past [`LIMIT`].
fn receive(child: &mut Child, bundle: &Path) -> Result<Vec<(String, String)>, CrossingError> {
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut listed = Vec::new();
    loop {
        let mut line = Vec::new();
        // A ref name is far below 4096 bytes; a longer line is not a listing.
        (&mut stdout).take(4096).read_until(b'\n', &mut line)?;
        let malformed = || CrossingError::Failed("the box's branch listing is malformed".into());
        let line = line.strip_suffix(b"\n").ok_or_else(malformed)?;
        if line.is_empty() {
            break;
        }
        if listed.len() == BRANCHES {
            return Err(too_many());
        }
        let line = std::str::from_utf8(line).map_err(|_| malformed())?;
        let (oid, name) = line.split_once(' ').ok_or_else(malformed)?;
        let branch = name.strip_prefix("refs/heads/").ok_or_else(malformed)?;
        if !object_id(oid) {
            return Err(malformed());
        }
        listed.push((branch.to_owned(), oid.to_owned()));
    }
    let mut file = File::create(bundle)?;
    let copied = io::copy(&mut (&mut stdout).take(LIMIT + 1), &mut file)?;
    if copied > LIMIT {
        return Err(CrossingError::Refused(
            RefusalReason::Limit,
            format!("the bundle is past {LIMIT} bytes"),
        ));
    }
    file.flush()?;
    Ok(listed)
}

/// Whether a bundle's header names a filter, and the header's length,
/// where the pack starts. Fetching a filtered bundle fails its
/// connectivity check but leaves a promisor pack in REPO first.
fn header(bundle: &Path) -> Result<(bool, u64), CrossingError> {
    let mut reader = BufReader::new(File::open(bundle)?);
    let (mut filtered, mut len) = (false, 0);
    let mut line = Vec::new();
    // The signature line, capabilities (`@…`), prerequisites and heads,
    // then an empty line. A prerequisite line carries its commit's subject,
    // which has no length limit, so only each line's first 8 bytes are kept,
    // enough to see `@filter` or the empty line, and the rest is skipped.
    // The bundle itself is already under LIMIT.
    loop {
        line.clear();
        (&mut reader).take(8).read_until(b'\n', &mut line)?;
        if line.is_empty() {
            return Err(CrossingError::Failed(
                "the bundle's header is malformed".into(),
            ));
        }
        filtered |= line.starts_with(b"@filter");
        len += line.len() as u64;
        if line == b"\n" {
            return Ok((filtered, len));
        }
        if !line.ends_with(b"\n") {
            len += reader.skip_until(b'\n')? as u64;
        }
    }
}

/// A bundle's branches, as git on the host reads them. Their names are
/// the bundle's bytes, unchecked.
fn bundle_heads(repo: &Path, path: &str) -> Result<Refs, CrossingError> {
    let listed = succeeded(git(repo, &["bundle", "list-heads", path])?)?;
    let mut heads = Refs::new();
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let Some((oid, name)) = line.split_once(' ') else {
            continue;
        };
        let Some(branch) = name.strip_prefix("refs/heads/") else {
            continue;
        };
        if !object_id(oid) {
            return Err(CrossingError::Failed(format!(
                "the bundle names {oid:?} for {name:?}"
            )));
        }
        heads.insert(branch.to_owned(), oid.to_owned());
    }
    Ok(heads)
}

/// Whether `oid` is a full sha1 or sha256 object name.
fn object_id(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The first [`STDERR_KEPT`] bytes of a child's stderr, read on a thread
/// that drains the rest so the child never blocks on it.
struct KeptStderr(Option<std::thread::JoinHandle<Vec<u8>>>);

fn kept_stderr(child: &mut Child) -> KeptStderr {
    KeptStderr(child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let _ = (&mut stderr).take(STDERR_KEPT).read_to_end(&mut kept);
            let _ = io::copy(&mut stderr, &mut io::sink());
            kept
        })
    }))
}

impl KeptStderr {
    fn join(self) -> String {
        let bytes = self
            .0
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

fn git(repo: &Path, args: &[&str]) -> io::Result<Output> {
    git_input(repo, args, b"", &[])
}

fn git_input(
    repo: &Path,
    args: &[&str],
    input: &[u8],
    env: &[(&str, &OsStr)],
) -> io::Result<Output> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| io::Error::new(error.kind(), format!("git: {error}")))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let written = stdin.write_all(input);
    drop(stdin);
    let output = child.wait_with_output()?;
    written?;
    Ok(output)
}

fn succeeded(output: Output) -> Result<Output, CrossingError> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(host_failed(&output))
    }
}

fn host_failed(output: &Output) -> CrossingError {
    CrossingError::Failed(format!(
        "git on the host failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

/// A bundle file under pinfold's state dir, removed on drop.
struct Bundle {
    path: PathBuf,
}

impl Bundle {
    fn new() -> io::Result<Bundle> {
        let dir = dirs::state_dir()?.join("crossing");
        fs::create_dir_all(&dir)?;
        for attempt in 0u32.. {
            let path = dir.join(format!("{}-{attempt}.bundle", std::process::id()));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Bundle { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!("u32 attempts exhausted")
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// An object directory inside REPO's, removed on drop. A git command run
/// with [`Quarantine::env`] writes its objects here and reads REPO's
/// through an alternate, as receive-pack's own quarantine does.
struct Quarantine {
    objects: PathBuf,
    /// `QUARANTINE` + `PID-N`.
    dir: PathBuf,
    alternates: OsString,
}

impl Quarantine {
    fn new(repo: &Path) -> Result<Quarantine, CrossingError> {
        let mut objects = succeeded(git(
            repo,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
        )?)?
        .stdout;
        if objects.last() == Some(&b'\n') {
            objects.pop();
        }
        // git splits the alternates list on `:` unless the entry is C-quoted.
        let mut alternates = vec![b'"'];
        for &byte in &objects {
            if matches!(byte, b'"' | b'\\') {
                alternates.push(b'\\');
            }
            alternates.push(byte);
        }
        alternates.push(b'"');
        let objects = PathBuf::from(OsString::from_vec(objects));
        // A killed fetch's quarantine outlives it, and git never prunes
        // one; remove those whose process is gone.
        for entry in fs::read_dir(&objects)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(pid) = name
                .to_str()
                .and_then(|name| name.strip_prefix(QUARANTINE))
                .and_then(|rest| rest.split_once('-'))
                .and_then(|(pid, _)| pid.parse().ok())
            else {
                continue;
            };
            let gone = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
                == Err(nix::errno::Errno::ESRCH);
            if gone {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        for attempt in 0u32.. {
            let dir = objects.join(format!("{QUARANTINE}{}-{attempt}", std::process::id()));
            match fs::create_dir(&dir) {
                Ok(()) => {
                    return Ok(Quarantine {
                        objects,
                        dir,
                        alternates: OsString::from_vec(alternates),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        unreachable!("u32 attempts exhausted")
    }

    fn env(&self) -> [(&'static str, &OsStr); 2] {
        [
            ("GIT_OBJECT_DIRECTORY", self.dir.as_os_str()),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES", &self.alternates),
        ]
    }

    /// Move the quarantine's packs into REPO, each index after its pack so
    /// no reader finds an index without one.
    fn migrate(&self) -> io::Result<()> {
        let from = self.dir.join("pack");
        let entries = match fs::read_dir(&from) {
            Ok(entries) => entries,
            // A quarantine another fetch swept away lost its packs; committing
            // would name objects REPO lacks.
            Err(error) if error.kind() == io::ErrorKind::NotFound && self.dir.is_dir() => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let mut names = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            if name.as_bytes().starts_with(b"pack-") {
                names.push(name);
            }
        }
        names.sort_by_key(|name| name.as_bytes().ends_with(b".idx"));
        let mut moved = Vec::new();
        for name in names {
            let to = self.objects.join("pack").join(&name);
            // A pack's name is its content's hash, so one already there is
            // this one, and another git may have it open.
            if to.exists() {
                continue;
            }
            if let Err(error) = fs::rename(from.join(&name), &to) {
                // An index after its pack: undo in reverse.
                for path in moved.iter().rev() {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
            moved.push(to);
        }
        Ok(())
    }
}

impl Drop for Quarantine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
