//! The branch crossing (Disks, Crossing): `push` moves host branches and
//! tags into a live box's repository, and `fetch` the box's branches and
//! tags out, as bundles. The box's side runs in the box as the box user. Neither
//! touches a working tree, and nothing from the box runs on the host: the
//! host only verifies a bundle and indexes its objects with fsck into a
//! quarantine. Git, through the remote helper, writes the refs.

use crate::core::process::output_cancellable;
use crate::core::process::{ExitWait, drain, fork_guard};

use nix::fcntl::{Flock, FlockArg};
use nix::poll::{PollFd, PollFlags};
use std::collections::{BTreeMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::core::r#box::RefusalReason;
use crate::core::plan::valid_name;
use crate::core::runtime::{ExecContext, Runtime};
use crate::dirs;
use crate::formatting::terminal_text;

/// A fetched bundle larger than this is refused as `limit`.
const LIMIT: u64 = 4 << 30;

/// A box listing more refs than this fails the fetch.
const REFS: usize = 4096;

/// The box script's exit code for a GUEST_REPO that is not a repository,
/// which the host reports as `spec`.
const NOT_A_REPO: i32 = 64;

/// The push script's exit code for an unforced tag the box already holds
/// at another object; it only selects the failure's reason.
const TAG_EXISTS: i32 = 65;

/// Why a push that would move an existing box tag failed whole.
pub(crate) const TAG_EXISTS_DETAIL: &str = "a pushed tag already exists in the box with a different target; nothing was pushed (use --force to replace box tags)";

/// The prefix of a fetch's quarantine under REPO's objects dir.
const QUARANTINE: &str = "pinfold-incoming-";

/// The bytes of a box command's stderr kept for a `failed` detail.
const STDERR_KEPT: u64 = 16 << 10;

/// `$1` is GUEST_REPO, `$2` the host's object format, the rest refspecs
/// of full ref names: forced ones into `refs/remotes/host/*` or
/// `refs/tags/*`, unforced ones into `refs/tags/*`. The bundle arrives on
/// stdin. When GUEST_REPO has no `.git`, it first creates the repository,
/// on Git's default unborn branch. Two pushes racing here may both init; a
/// reinit keeps refs and objects. When an unforced tag already exists in
/// the box at another object, nothing changes and the script exits
/// [`TAG_EXISTS`]. A tag created after that check makes the atomic fetch
/// reject it, and with it every ref.
const PUSH_SCRIPT: &str = r#"
repo=$1 format=$2; shift 2
if [ ! -e "$repo/.git" ] && [ ! -L "$repo/.git" ]; then
  git init -q --object-format="$format" "$repo" || exit
fi
dir=$(git -C "$repo" rev-parse --absolute-git-dir) || exit 64
bundle=$(mktemp "$dir/pinfold-push.XXXXXX") || exit
trap 'rm -f "$bundle"' EXIT
cat >"$bundle" || exit
for spec do
  case $spec in
  +*) ;;
  *)
    if old=$(git -C "$repo" show-ref --verify --hash "${spec#*:}" 2>/dev/null) &&
      new=$(git -C "$repo" bundle list-heads "$bundle" "${spec%%:*}") &&
      [ "$old" != "${new%% *}" ]; then
      exit 65
    fi ;;
  esac
done
# Only the ref transaction may decide the exit status.
git -C "$repo" fetch -q --atomic --no-tags --no-write-fetch-head --no-prune --no-prune-tags \
  --no-recurse-submodules --no-auto-maintenance --no-write-commit-graph "$bundle" "$@"
"#;

/// `$1` is GUEST_REPO; stdin holds the commits the host already has, one
/// per line, read whole before any output. Prints every branch, and every
/// tag at a commit or at a tag of a commit whose name is printable ASCII
/// and that is not a symbolic ref, as `OID REF`, an empty line,
/// then a bundle of what the host lacks, or nothing when it lacks nothing.
/// A branch or lightweight tag whose commit the host holds is left out of
/// the bundle by git, so the listing names it; an annotated tag's object
/// is always bundled.
const FETCH_SCRIPT: &str = r#"
repo=$1
git -C "$repo" rev-parse --git-dir >/dev/null || exit 64
held=$(git -C "$repo" cat-file --batch-check='%(objectname) %(objecttype)' |
  sed -n 's/^\([0-9a-f]*\) commit$/^\1/p') || exit
# A symbolic tag's last field starts with its target's name, so it never
# matches; the C locale makes the ASCII test byte-wise.
tags=$(git -C "$repo" for-each-ref --format='%(objectname) %(refname) %(symref)%(objecttype)%(*objecttype)' refs/tags/) || exit
tags=$(printf '%s\n' "$tags" | LC_ALL=C sed -n '/[^ -~]/d; / \(tag\)\{0,1\}commit$/p')
git -C "$repo" for-each-ref --format='%(objectname) %(refname)' refs/heads/ || exit
[ -z "$tags" ] || printf '%s\n' "$tags" | sed 's/ [a-z]*$//'
echo
# Full names, unchecked for ambiguity: `bundle create` drops a name a tag
# also matches.
revs() {
  git -C "$repo" for-each-ref --format='%(refname)' refs/heads/
  [ -z "$tags" ] || printf '%s\n' "$tags" | sed 's/^[^ ]* \([^ ]*\) .*$/\1/'
  printf '%s\n' $held
}
[ -n "$(revs | git -C "$repo" rev-list -n1 --stdin)" ] ||
  printf '%s\n' "$tags" | grep -q ' tagcommit$' || exit 0
revs | exec git -C "$repo" -c core.warnAmbiguousRefs=false bundle create -q - --stdin
"#;

/// `$1` is GUEST_REPO. Prints, in the fetch listing's format, the refs a
/// push writes, then an empty line and nothing after it.
const PUSHED_SCRIPT: &str = r#"
repo=$1
git -C "$repo" rev-parse --git-dir >/dev/null || exit 64
git -C "$repo" for-each-ref --format='%(objectname) %(refname)' refs/remotes/host/ refs/tags/ || exit
echo
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

/// Full ref names and their object ids.
pub type Refs = BTreeMap<String, String>;

/// Bundle REPO's SOURCES and fetch them in the box with REFSPECS, whose
/// destinations the caller puts under `refs/remotes/host/` or `refs/tags/`.
/// Cancellation reaps each host helper before return.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_cancellable(
    runtime: &'static dyn Runtime,
    init: &Path,
    name: &str,
    repo: &Path,
    guest_repo: &str,
    sources: &[String],
    refspecs: &[String],
    cancel: &AtomicBool,
) -> Result<(), CrossingError> {
    check_guest_repo(guest_repo)?;
    let format = object_format(object_width(repo, cancel)?);
    if sources.is_empty() {
        return Err(spec(format!(
            "{} has no branches",
            terminal_text(&repo.to_string_lossy())
        )));
    }
    let bundle = Bundle::new()?;
    let path = bundle.path.as_str();
    let sources: String = sources.iter().map(|source| format!("{source}\n")).collect();
    // Full names, unchecked for ambiguity: `bundle create` drops a name a
    // tag also matches.
    succeeded(git_input(
        repo,
        &[
            "-c",
            "core.warnAmbiguousRefs=false",
            "bundle",
            "create",
            "-q",
            path,
            "--stdin",
        ],
        sources.as_bytes(),
        &[],
        cancel,
    )?)?;
    // The box's output is its own text; a push reports only its outcome.
    let null = || File::options().write(true).open("/dev/null");
    let mut child = BoxChild::spawn(
        runtime,
        init,
        name,
        guest_repo,
        script(
            PUSH_SCRIPT,
            [guest_repo, format]
                .into_iter()
                .chain(refspecs.iter().map(String::as_str)),
        ),
        [
            Some(File::open(&bundle.path)?),
            Some(null()?),
            Some(null()?),
        ],
    )?;
    let mut wait = ExitWait::new();
    while !child.exited() {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::from(io::ErrorKind::Interrupted).into());
        }
        wait.poll(&mut [])?;
    }
    match child.finish()?.code() {
        Some(TAG_EXISTS) => Err(CrossingError::Failed(TAG_EXISTS_DETAIL.into())),
        code => box_status(code, "", guest_repo),
    }
}

/// What the box holds where a push writes, its `refs/remotes/host/*` and
/// `refs/tags/*`, read through the fetch listing's checks. It carries no
/// objects and never feeds a fetch; a lying box can only make a push skip
/// or attempt a ref.
pub(crate) fn pushed_cancellable(
    runtime: &'static dyn Runtime,
    init: &Path,
    name: &str,
    repo: &Path,
    guest_repo: &str,
    cancel: &AtomicBool,
) -> Result<Refs, CrossingError> {
    check_guest_repo(guest_repo)?;
    let width = object_width(repo, cancel)?;
    let child = BoxChild::spawn(
        runtime,
        init,
        name,
        guest_repo,
        script(PUSHED_SCRIPT, [guest_repo]),
        [Some(File::open("/dev/null")?), None, None],
    )?;
    // No bundle: a byte after the listing fails it, and nothing is written.
    let listed = receive(
        child,
        b"",
        None,
        width,
        &["refs/remotes/host/", "refs/tags/"],
        guest_repo,
        cancel,
    )?;
    let mut refs = Refs::new();
    for (name, oid) in listed {
        if !valid_ref(repo, &name, cancel)? || refs.insert(name, oid).is_some() {
            return Err(malformed_listing());
        }
    }
    // Keep only ids a host branch or tag points at, so pinfold looks up no
    // box-chosen id and host Git has nothing to fetch for any it sees.
    let tips = succeeded(git(
        repo,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            "refs/heads/",
            "refs/tags/",
        ],
        cancel,
    )?)?;
    let tips = String::from_utf8_lossy(&tips.stdout);
    let tips: HashSet<&str> = tips.lines().collect();
    refs.retain(|_, oid| tips.contains(oid.as_str()));
    Ok(refs)
}
/// REPO's object id length in hex, which every id the box sends has.
fn object_width(repo: &Path, cancel: &AtomicBool) -> Result<usize, CrossingError> {
    let format = git(repo, &["rev-parse", "--show-object-format"], cancel)?;
    if !format.status.success() {
        return Err(spec(format!(
            "{} is not a git repository: {}",
            terminal_text(&repo.to_string_lossy()),
            terminal_text(String::from_utf8_lossy(&format.stderr).trim())
        )));
    }
    match String::from_utf8_lossy(&format.stdout).trim() {
        "sha1" => Ok(40),
        "sha256" => Ok(64),
        format => Err(CrossingError::Failed(format!(
            "unknown object format {format:?}"
        ))),
    }
}

/// The box's checked branches and tags, and every host tag at its own id,
/// their objects held in a quarantine inside REPO's object store until
/// [`Fetched::land`] moves them in.
pub(crate) struct Fetched {
    pub(crate) refs: Refs,
    /// Each new annotated tag's commit, which Git needs to follow it.
    pub(crate) peeled: Refs,
    quarantine: Quarantine,
}

impl Fetched {
    pub(crate) fn land(&self) -> io::Result<()> {
        self.quarantine.migrate()
    }
}

/// Have the box bundle GUEST_REPO's branches, less what REPO's
/// `refs/remotes/REMOTE/*` hold, and check every byte the box sends.
pub(crate) fn fetch_cancellable(
    runtime: &'static dyn Runtime,
    init: &Path,
    name: &str,
    guest_repo: &str,
    repo: &Path,
    remote: &str,
    cancel: &AtomicBool,
) -> Result<Fetched, CrossingError> {
    if !valid_name(remote) {
        return Err(spec(format!(
            "remote name {remote:?} must start alphanumeric and hold only [A-Za-z0-9._-]"
        )));
    }
    check_guest_repo(guest_repo)?;
    let width = object_width(repo, cancel)?;
    let namespace = format!("refs/remotes/{remote}/");
    let held = String::from_utf8_lossy(
        &succeeded(git(
            repo,
            &["for-each-ref", "--format=%(objectname)", &namespace],
            cancel,
        )?)?
        .stdout,
    )
    .into_owned();
    let child = BoxChild::spawn(
        runtime,
        init,
        name,
        guest_repo,
        script(FETCH_SCRIPT, [guest_repo]),
        [None, None, None],
    )?;
    let bundle = Bundle::new()?;
    let listed = receive(
        child,
        held.as_bytes(),
        Some(&bundle.path),
        width,
        &["refs/heads/", "refs/tags/"],
        guest_repo,
        cancel,
    )?;
    // The host's own tags, read after the box's stream so a slow box widens
    // no window. A box tag the host has, by name ignoring case, is never
    // listed: loose refs are files, and on a case-insensitive file system
    // Git would write the host's. Nor is a non-ASCII one, and while any host
    // tag's name is non-ASCII no box tag is, as Unicode normalization could
    // match the two. Listing every host tag back at its own id leaves Git
    // none to move or prune.
    let host_tags = host_tags(repo, cancel)?;
    let host_ascii = host_tags.keys().all(|name| name.is_ascii());
    let host_names: HashSet<String> = host_tags
        .keys()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let kept_out = |name: &str| {
        name.starts_with("refs/tags/")
            && (!host_ascii || !name.is_ascii() || host_names.contains(&name.to_ascii_lowercase()))
    };
    let mut refs = Refs::new();
    let mut names = HashSet::new();
    for (name, oid) in listed {
        if !names.insert(name.clone()) {
            return Err(malformed_listing());
        }
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            if !valid_ref(repo, &format!("{namespace}{branch}"), cancel)? {
                return Err(CrossingError::Failed(format!(
                    "the box named a malformed branch: {branch:?}"
                )));
            }
        } else if !host_tags.contains_key(&name) && !valid_ref(repo, &name, cancel)? {
            // Left out; a bundle head naming it fails below.
            continue;
        }
        refs.insert(name, oid);
    }

    // The box's listing and bundle are untrusted. Every branch name passes
    // check-ref-format under the namespace, and every new tag name as
    // itself, before git sees it in a refspec or a ref update. A bundled
    // head lands at an object the bundle carries, which index-pack checks
    // with fsck, or one the host already holds connected: fetch's own
    // connectivity check covers a head whose objects are already here. A
    // listed branch or lightweight tag the bundle leaves out lands only at
    // a commit this remote's refs already reach. Every object the box sends
    // lands in a quarantine first, so a rejected bundle leaves none of its
    // bytes in REPO.
    let quarantine = Quarantine::new(repo, cancel)?;
    // Objects the box's pack itself carries, not bases `--fix-thin` copied
    // in from REPO: a head among them came from the box.
    let mut sent = HashSet::new();
    let mut heads = Refs::new();
    let path = bundle.path.as_str();
    let bundle_len = fs::metadata(path)?.len();
    if bundle_len > 0 {
        let (filtered, pack_start) = header(path)?;
        if filtered {
            return Err(CrossingError::Failed(
                "the box sent a filtered bundle".into(),
            ));
        }
        // Quarantined: in a partial clone, verify fetches a missing
        // prerequisite from the promisor remote into the object directory.
        succeeded(git_input(
            repo,
            &["bundle", "verify", "-q", path],
            b"",
            &quarantine.env(),
            cancel,
        )?)?;
        // The listing is the box's whole claim; the bundle's heads repeat a
        // subset of it exactly.
        heads = bundle_heads(repo, path, width, &quarantine, cancel)?;
        if heads.iter().any(|(name, oid)| refs.get(name) != Some(oid)) {
            return Err(CrossingError::Failed(
                "the bundle's heads differ from the box's listing".into(),
            ));
        }
        let wanted: String = heads.keys().map(|name| format!("{name}\n")).collect();
        // Older host git (Ubuntu 24.04's 2.43) ignores fetch.fsckObjects
        // when it fetches from a bundle, so pinfold indexes the bundle's
        // pack with fsck itself. The fetch then finds every object here and
        // only checks connectivity. Revisit when every supported host's git
        // fscks a bundle fetch.
        let mut pack = File::open(path)?;
        pack.seek(SeekFrom::Start(pack_start))?;
        let mut index = git_command(repo);
        index
            .args(["index-pack", "--stdin", "--fix-thin", "--fsck-objects"])
            .envs(quarantine.env())
            .stdin(pack);
        let indexed = succeeded(output_cancellable(&mut index, None, cancel)?)?;
        sent = sent_objects(
            repo,
            &quarantine,
            &indexed.stdout,
            bundle_len - pack_start,
            width,
            cancel,
        )?;
        // fsck checks each object but not its links, and fetch checks only
        // the heads' history: an unreachable object naming a missing one
        // would migrate with the pack. Every object the box sent must be
        // connected. Not `index-pack --strict`, which refuses a pack whose
        // trees name objects a partial clone's promisor holds, and not
        // `--missing=allow-promisor`, which passes a tag naming a missing
        // object. Git's own fetch checks a partial clone this way.
        let objects: String = sent.iter().map(|oid| format!("{oid}\n")).collect();
        succeeded(git_input(
            repo,
            &[
                "rev-list",
                "--objects",
                "--exclude-promisor-objects",
                "--quiet",
                "--stdin",
                "--not",
                "--all",
            ],
            objects.as_bytes(),
            &quarantine.env(),
            cancel,
        )?)?;
        // A fetch that finds a head missing unbundles the pack again before
        // its connectivity check fails, so it runs quarantined too. Its auto
        // maintenance would see only the quarantine. It names no
        // destination, and prune is pinned off: `fetch.pruneTags` would
        // otherwise add a tag refspec that writes and deletes REPO's tags.
        succeeded(git_input(
            repo,
            &[
                "fetch",
                "-q",
                "--no-auto-maintenance",
                "--no-prune",
                "--no-prune-tags",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-write-fetch-head",
                "--stdin",
                path,
            ],
            wanted.as_bytes(),
            &quarantine.env(),
            cancel,
        )?)?;
    }
    // Each commit a ref lands at is a bundle head the box's pack carries,
    // whose connectivity the quarantined fetch checked, or one this
    // remote's refs already reach; an annotated tag is a bundle head the
    // pack carries, peeling on the host to such a commit. An object REPO
    // holds otherwise is the host's, never the box's to name. A bundle
    // head that fails fails the fetch; a tag outside the bundle that fails
    // is left out.
    let reached = |commit: &str| -> Result<bool, CrossingError> {
        let reached = git_input(
            repo,
            &[
                "rev-list",
                "-n1",
                commit,
                "--not",
                &format!("--glob={namespace}*"),
            ],
            b"",
            &quarantine.env(),
            cancel,
        )?;
        Ok(reached.status.success() && reached.stdout.is_empty())
    };
    let tips: String = refs.values().map(|oid| format!("{oid}\n")).collect();
    let types = batch_check(
        repo,
        "%(objecttype)",
        &tips,
        refs.len(),
        &quarantine,
        cancel,
    )?;
    let mut checked = Refs::new();
    let mut annotated = Vec::new();
    for ((name, oid), kind) in refs.iter().zip(types) {
        let head = heads.contains_key(name);
        let carried = head && sent.contains(oid);
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            if kind != "commit" {
                return Err(CrossingError::Failed(format!(
                    "branch {branch:?} names {oid}, which is not a commit"
                )));
            }
            if !carried && !reached(oid)? {
                return Err(CrossingError::Failed(format!(
                    "branch {branch:?} names {oid}, which is neither a verified bundle head nor reachable from {namespace}*"
                )));
            }
        } else if !head && kept_out(name) {
            continue;
        } else if kind == "tag" && carried {
            annotated.push((name, oid));
            continue;
        } else if !(kind == "commit" && (carried || reached(oid)?)) {
            if head {
                return Err(bad_tag(name, oid));
            }
            continue;
        }
        checked.insert(name.clone(), oid.clone());
    }
    let tags: String = annotated
        .iter()
        .map(|(_, oid)| format!("{oid}^{{}}\n"))
        .collect();
    let commits = batch_check(
        repo,
        "%(objectname) %(objecttype)",
        &tags,
        annotated.len(),
        &quarantine,
        cancel,
    )?;
    let mut peeled = Refs::new();
    for ((name, oid), line) in annotated.into_iter().zip(commits) {
        let commit = line
            .strip_suffix(" commit")
            .filter(|commit| object_id(commit, width));
        match commit {
            Some(commit) if sent.contains(commit) || reached(commit)? => {
                checked.insert(name.clone(), oid.clone());
                peeled.insert(name.clone(), commit.to_owned());
            }
            _ => return Err(bad_tag(name, oid)),
        }
    }
    checked.retain(|name, _| !kept_out(name));
    peeled.retain(|name, _| checked.contains_key(name));
    checked.extend(host_tags);

    Ok(Fetched {
        refs: checked,
        peeled,
        quarantine,
    })
}

/// The objects at offsets inside the box's own SENT_LEN pack bytes, read
/// from the index `index-pack` reported in INDEXED; `--fix-thin` appends
/// REPO's bases after them.
fn sent_objects(
    repo: &Path,
    quarantine: &Quarantine,
    indexed: &[u8],
    sent_len: u64,
    width: usize,
    cancel: &AtomicBool,
) -> Result<HashSet<String>, CrossingError> {
    let indexed = String::from_utf8_lossy(indexed);
    let hash = indexed
        .split_once('\t')
        .map(|(_, hash)| hash.trim())
        .filter(|hash| object_id(hash, width))
        .ok_or_else(|| CrossingError::Failed(format!("index-pack printed {indexed:?}")))?;
    let mut show = git_command(repo);
    show.args([
        "show-index",
        &format!("--object-format={}", object_format(width)),
    ])
    .stdin(File::open(
        quarantine.dir.join("pack").join(format!("pack-{hash}.idx")),
    )?);
    let shown = succeeded(output_cancellable(&mut show, None, cancel)?)?;
    // The pack's trailer is one hash; the box's objects all start before it.
    let end = sent_len.saturating_sub(hash.len() as u64 / 2);
    let mut sent = HashSet::new();
    for line in String::from_utf8_lossy(&shown.stdout).lines() {
        let mut fields = line.split(' ');
        if let (Some(Ok(offset)), Some(oid)) = (fields.next().map(str::parse::<u64>), fields.next())
            && offset < end
        {
            sent.insert(oid.to_owned());
        }
    }
    Ok(sent)
}

/// The object format of an id `width` hex digits long.
fn object_format(width: usize) -> &'static str {
    if width == 64 { "sha256" } else { "sha1" }
}

fn check_guest_repo(guest_repo: &str) -> Result<(), CrossingError> {
    if !guest_repo.starts_with('/') {
        return Err(spec(format!(
            "guest repository {guest_repo:?} is not absolute"
        )));
    }
    Ok(())
}

/// Whether git takes NAME as one well-formed ref name.
pub(crate) fn valid_ref(repo: &Path, name: &str, cancel: &AtomicBool) -> io::Result<bool> {
    Ok(git(repo, &["check-ref-format", name], cancel)?
        .status
        .success())
}

fn bad_tag(name: &str, oid: &str) -> CrossingError {
    CrossingError::Failed(format!(
        "tag {name:?} names {oid}, which is neither a verified bundle head peeling to a commit nor a commit reachable from the remote's refs"
    ))
}

/// One `cat-file --batch-check=FORMAT` line for each of the COUNT lines
/// of INPUT, read through the quarantine.
fn batch_check(
    repo: &Path,
    format: &str,
    input: &str,
    count: usize,
    quarantine: &Quarantine,
    cancel: &AtomicBool,
) -> Result<Vec<String>, CrossingError> {
    let output = succeeded(git_input(
        repo,
        &["cat-file", &format!("--batch-check={format}")],
        input.as_bytes(),
        &quarantine.env(),
        cancel,
    )?)?;
    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    if lines.len() != count {
        return Err(CrossingError::Failed(
            "git on the host misread the box's tips".into(),
        ));
    }
    Ok(lines)
}

/// REPO's tags and their ids. A name that is not UTF-8 could not be listed
/// back as itself, so Git could prune it: the fetch refuses.
fn host_tags(repo: &Path, cancel: &AtomicBool) -> Result<Refs, CrossingError> {
    let listed = succeeded(git(
        repo,
        &[
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            "refs/tags/",
        ],
        cancel,
    )?)?;
    let mut tags = Refs::new();
    for line in listed.stdout.split(|&byte| byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(line) = std::str::from_utf8(line) else {
            return Err(spec(
                "a host tag's name is not UTF-8, so a fetch from the box could delete it; rename it (`git for-each-ref refs/tags` shows it)".into(),
            ));
        };
        let (oid, name) = line
            .split_once(' ')
            .ok_or_else(|| CrossingError::Failed("git on the host misread its tags".into()))?;
        tags.insert(name.to_owned(), oid.to_owned());
    }
    Ok(tags)
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
    let detail = match stderr.trim() {
        "" => String::new(),
        stderr => format!(": {}", terminal_text(stderr)),
    };
    match code {
        Some(0) => Ok(()),
        Some(NOT_A_REPO) => Err(spec(format!(
            "{} is not a git repository in the box{detail}; a push creates one where `.git` is absent",
            terminal_text(guest_repo),
        ))),
        _ => Err(CrossingError::Failed(format!(
            "the box's git failed{detail}"
        ))),
    }
}

/// A crossing script run in the box as a project command in GUEST_REPO,
/// over the session transport from this process rather than from a child
/// that could outlive it. However the crossing ends, a signal or SIGKILL
/// included, the connection closes and the box ends the script's process
/// group (Process supervision).
struct BoxChild {
    stdin: Option<io::PipeWriter>,
    stdout: Option<io::PipeReader>,
    stderr: Option<io::PipeReader>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<ExitStatus>>>,
}

impl BoxChild {
    /// Each `None` in `stdio` is a pipe to this process.
    fn spawn(
        runtime: &'static dyn Runtime,
        init: &Path,
        name: &str,
        guest_repo: &str,
        argv: Vec<String>,
        stdio: [Option<File>; 3],
    ) -> io::Result<BoxChild> {
        let [stdin, stdout, stderr] = stdio;
        let (mut child, theirs) = {
            // The pipes must not leak into a concurrent spawn before CLOEXEC.
            let _guard = fork_guard();
            let mut child = BoxChild {
                stdin: None,
                stdout: None,
                stderr: None,
                stop: Arc::new(AtomicBool::new(false)),
                worker: None,
            };
            let stdin: OwnedFd = match stdin {
                Some(file) => file.into(),
                None => {
                    let (read, write) = io::pipe()?;
                    child.stdin = Some(write);
                    read.into()
                }
            };
            let output = |file: Option<File>, ours: &mut Option<io::PipeReader>| {
                io::Result::Ok(match file {
                    Some(file) => OwnedFd::from(file),
                    None => {
                        let (read, write) = io::pipe()?;
                        *ours = Some(read);
                        write.into()
                    }
                })
            };
            let theirs = [
                stdin,
                output(stdout, &mut child.stdout)?,
                output(stderr, &mut child.stderr)?,
            ];
            (child, theirs)
        };
        let (init, name, guest) = (
            init.to_path_buf(),
            name.to_string(),
            PathBuf::from(guest_repo),
        );
        let stop = child.stop.clone();
        child.worker = Some(std::thread::Builder::new().spawn(move || {
            runtime.exec(
                &name,
                &init,
                &ExecContext {
                    project_directory: Some((&guest, Path::new(""))),
                    cancel: Some(&stop),
                    stdio: Some(theirs.each_ref().map(AsFd::as_fd)),
                    ..ExecContext::default()
                },
                &argv,
            )
        })?);
        Ok(child)
    }

    /// Whether the command's status is in. The worker has then closed its
    /// ends of the pipes.
    fn exited(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// End the command if it still runs, and return its status.
    fn finish(&mut self) -> io::Result<ExitStatus> {
        self.stop.store(true, Ordering::Release);
        self.worker
            .take()
            .ok_or_else(|| io::Error::other("box command already finished"))?
            .join()
            .map_err(|_| io::Error::other("box command worker failed"))?
    }
}

impl Drop for BoxChild {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.finish();
        }
    }
}

struct Receiver {
    /// Where bytes after the listing go; with none, any such byte fails.
    file: Option<File>,
    /// The repository's object id length in hex.
    width: usize,
    /// The ref name prefixes a listed name may have.
    prefixes: &'static [&'static str],
    /// Full ref names and their object ids.
    listed: Vec<(String, String)>,
    line: Vec<u8>,
    listing: bool,
    copied: u64,
}

fn malformed_listing() -> CrossingError {
    CrossingError::Failed("the box's listing is malformed".into())
}

impl Receiver {
    fn new(
        bundle: Option<&str>,
        width: usize,
        prefixes: &'static [&'static str],
    ) -> io::Result<Self> {
        Ok(Self {
            file: bundle.map(File::create).transpose()?,
            width,
            prefixes,
            listed: Vec::new(),
            line: Vec::new(),
            listing: true,
            copied: 0,
        })
    }

    fn feed(&mut self, mut bytes: &[u8]) -> Result<(), CrossingError> {
        while self.listing && !bytes.is_empty() {
            let take = bytes
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(bytes.len(), |index| index + 1);
            if self.line.len() + take > 4096 {
                return Err(malformed_listing());
            }
            self.line.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if !self.line.ends_with(b"\n") {
                if self.line.len() == 4096 {
                    return Err(malformed_listing());
                }
                continue;
            }
            let line = &self.line[..self.line.len() - 1];
            if line.is_empty() {
                self.listing = false;
            } else {
                if self.listed.len() == REFS {
                    return Err(CrossingError::Failed(format!(
                        "the box lists more than {REFS} refs"
                    )));
                }
                let line = std::str::from_utf8(line).map_err(|_| malformed_listing())?;
                let (oid, name) = line.split_once(' ').ok_or_else(malformed_listing)?;
                if !self.prefixes.iter().any(|prefix| name.starts_with(prefix))
                    || !object_id(oid, self.width)
                {
                    return Err(malformed_listing());
                }
                self.listed.push((name.to_owned(), oid.to_owned()));
            }
            self.line.clear();
        }
        if !self.listing && !bytes.is_empty() {
            let Some(file) = self.file.as_mut() else {
                return Err(malformed_listing());
            };
            self.copied += bytes.len() as u64;
            if self.copied > LIMIT {
                return Err(CrossingError::Refused(
                    RefusalReason::Limit,
                    format!("the bundle is past {LIMIT} bytes"),
                ));
            }
            file.write_all(bytes)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<(String, String)>, CrossingError> {
        if self.listing {
            return Err(malformed_listing());
        }
        if let Some(file) = self.file.as_mut() {
            file.flush()?;
        }
        Ok(self.listed)
    }
}

/// Read the ref listing and bounded bundle, reaping the helper on cancellation.
fn receive(
    mut child: BoxChild,
    mut input: &[u8],
    bundle: Option<&str>,
    width: usize,
    prefixes: &'static [&'static str],
    guest_repo: &str,
    cancel: &AtomicBool,
) -> Result<Vec<(String, String)>, CrossingError> {
    if cancel.load(Ordering::Acquire) {
        return Err(io::Error::from(io::ErrorKind::Interrupted).into());
    }
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let pipes = [
        stdin.as_ref().map(AsFd::as_fd),
        stdout.as_ref().map(AsFd::as_fd),
        stderr.as_ref().map(AsFd::as_fd),
    ];
    for pipe in pipes.into_iter().flatten() {
        super::pipe::nonblocking(&pipe, "crossing")?;
    }
    let mut receiver = Receiver::new(bundle, width, prefixes)?;
    let mut failed = None;
    let mut errors = Vec::new();
    let mut exited = None;
    let mut wait = ExitWait::new();
    loop {
        if exited.is_none() && child.exited() {
            exited = Some(Instant::now());
            stdin.take();
        }
        if cancel.load(Ordering::Acquire) && exited.is_none() {
            return Err(io::Error::from(io::ErrorKind::Interrupted).into());
        }
        if input.is_empty() {
            stdin.take();
        }
        if let Some(pipe) = stdin.as_mut() {
            match pipe.write(input) {
                Ok(written) => input = &input[written..],
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                    stdin.take();
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        if let Some(pipe) = stdout.as_mut() {
            let mut bytes = [0; 8192];
            for _ in 0..8 {
                match pipe.read(&mut bytes) {
                    Ok(0) => {
                        stdout.take();
                        break;
                    }
                    Ok(read) => {
                        if let Err(error) = receiver.feed(&bytes[..read]) {
                            if matches!(error, CrossingError::Refused(RefusalReason::Limit, _)) {
                                return Err(error);
                            }
                            failed = Some(error);
                            stdout.take();
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        drain(&mut stderr, &mut errors, Some(STDERR_KEPT as usize))?;
        if let Some(time) = exited
            && ((stdout.is_none() && stderr.is_none()) || time.elapsed() >= Duration::from_secs(1))
        {
            let status = child.finish()?;
            // A rejected listing closes the pipe, so the box's git then fails
            // on it; the rejection is the cause.
            if let Some(error) = failed {
                return Err(error);
            }
            box_status(status.code(), &String::from_utf8_lossy(&errors), guest_repo)?;
            return receiver.finish();
        }
        let mut fds = Vec::new();
        if let Some(pipe) = &stdin {
            fds.push(PollFd::new(pipe.as_fd(), PollFlags::POLLOUT));
        }
        if let Some(pipe) = &stdout {
            fds.push(PollFd::new(pipe.as_fd(), PollFlags::POLLIN));
        }
        if let Some(pipe) = &stderr {
            fds.push(PollFd::new(pipe.as_fd(), PollFlags::POLLIN));
        }
        wait.poll(&mut fds)?;
    }
}

/// Whether a bundle's header names a filter, and the header's length,
/// where the pack starts. Fetching a filtered bundle fails its
/// connectivity check but leaves a promisor pack in REPO first.
fn header(bundle: &str) -> Result<(bool, u64), CrossingError> {
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

/// A bundle's branches and tags by full name, as git on the host reads
/// them. Their names are the bundle's bytes, unchecked.
fn bundle_heads(
    repo: &Path,
    path: &str,
    width: usize,
    quarantine: &Quarantine,
    cancel: &AtomicBool,
) -> Result<Refs, CrossingError> {
    let listed = succeeded(git_input(
        repo,
        &["bundle", "list-heads", path],
        b"",
        &quarantine.env(),
        cancel,
    )?)?;
    let mut heads = Refs::new();
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let head = line.split_once(' ').filter(|(oid, name)| {
            object_id(oid, width)
                && (name.starts_with("refs/heads/") || name.starts_with("refs/tags/"))
        });
        let Some((oid, name)) = head else {
            return Err(CrossingError::Failed(format!(
                "the bundle names a head that is not a branch or tag: {}",
                terminal_text(line)
            )));
        };
        if heads.insert(name.to_owned(), oid.to_owned()).is_some() {
            return Err(CrossingError::Failed(format!(
                "the bundle names {name:?} twice"
            )));
        }
    }
    Ok(heads)
}

/// Whether `oid` is a full object name of WIDTH hex digits, 40 (sha1)
/// or 64 (sha256).
fn object_id(oid: &str, width: usize) -> bool {
    oid.len() == width && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git(repo: &Path, args: &[&str], cancel: &AtomicBool) -> io::Result<Output> {
    git_input(repo, args, b"", &[], cancel)
}

fn git_input(
    repo: &Path,
    args: &[&str],
    input: &[u8],
    env: &[(&str, &OsStr)],
    cancel: &AtomicBool,
) -> io::Result<Output> {
    let mut command = git_command(repo);
    command
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    output_cancellable(&mut command, Some(input), cancel)
}

pub(crate) fn git_command(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo);
    command
}

fn succeeded(output: Output) -> Result<Output, CrossingError> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(CrossingError::Failed(format!(
            "git on the host failed: {}",
            terminal_text(String::from_utf8_lossy(&output.stderr).trim())
        )))
    }
}

/// Whether a bundle or quarantine about to be created may remove the
/// leftover named PID: a canonical positive pid that no process has, or
/// the creating process's own, since it holds at most one of each at a
/// time. Any other name is left alone. A reused pid keeps its leftover
/// until that process ends.
fn crossing_gone(pid: &str) -> bool {
    let canonical = !pid.starts_with('0') && pid.bytes().all(|byte| byte.is_ascii_digit());
    let Some(pid) = pid.parse::<i32>().ok().filter(|_| canonical) else {
        return false;
    };
    pid as u32 == std::process::id()
        || nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
            == Err(nix::errno::Errno::ESRCH)
}

/// A bundle file under pinfold's state dir, removed on drop.
struct Bundle {
    path: String,
}

impl Bundle {
    fn new() -> io::Result<Bundle> {
        let dir = dirs::state_dir()?.join("crossing");
        fs::create_dir_all(&dir)?;
        // A killed crossing's bundle outlives it; remove those whose
        // process is gone, this one's included.
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".bundle"))
                .is_some_and(crossing_gone)
            {
                let _ = fs::remove_file(entry.path());
            }
        }
        let path = dir.join(format!("{}.bundle", std::process::id()));
        let path = path
            .to_str()
            .ok_or_else(|| {
                io::Error::other(format!(
                    "{} is not UTF-8",
                    terminal_text(&path.to_string_lossy())
                ))
            })?
            .to_string();
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Bundle { path })
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
    /// `QUARANTINE` + `PID`.
    dir: PathBuf,
    alternates: OsString,
}

impl Quarantine {
    fn new(repo: &Path, cancel: &AtomicBool) -> Result<Quarantine, CrossingError> {
        let mut objects = succeeded(git(
            repo,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
            cancel,
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
            if entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix(QUARANTINE))
                .is_some_and(crossing_gone)
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        let dir = objects.join(format!("{QUARANTINE}{}", std::process::id()));
        fs::create_dir(&dir)?;
        Ok(Quarantine {
            objects,
            dir,
            alternates: OsString::from_vec(alternates),
        })
    }

    fn env(&self) -> [(&'static str, &OsStr); 2] {
        [
            ("GIT_OBJECT_DIRECTORY", self.dir.as_os_str()),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES", &self.alternates),
        ]
    }

    /// Move the quarantine's packs into REPO, each index after its pack so
    /// no reader finds an index without one. Migrations into one object
    /// store run one at a time: two fetches can carry the same pack, and
    /// one's undo must never remove a file the other skipped as present.
    fn migrate(&self) -> io::Result<()> {
        let _lock = Flock::lock(
            File::open(self.objects.join("pack"))?,
            FlockArg::LockExclusive,
        )
        .map_err(|(_, errno)| io::Error::from(errno))?;
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
