# Git remote for disk projects

2026-11-19. Issue #296, step 6 of `2026-11-12-project-box-plan.md`
(decisions 9, 16, 17 and 26). Evidence before it:
`2026-11-12-git-remote-helper-probe.md`. This records what was built and
what the real binary did on Linux and macOS. No test was added: the suite
is suspended.

## Built

- `git-remote-pinfold`, a link to the pinfold executable, is the remote
  helper. It offers `fetch` (with `list`) and `push`, never `connect`.
  It runs core crossing's fetch at Git's `list` and answers only with
  checked branches; Git's `fetch` moves the quarantine in. Git writes
  the refs.
- Core crossing keeps every check on the box's bytes and loses its own
  ref transaction (`update_refs`). `fetch` returns the checked branches
  and the live quarantine; `push` takes the sources to bundle and the
  refspecs into the box's `refs/remotes/host/*`.
- The helper refuses a URL given on the command line, a URL naming
  another project, a `GIT_DIR` other than the project's `.git`, and a
  configured fetch refspec outside `refs/remotes/NAME/`. It moves Git's
  protocol to private, close-on-exec descriptors; fd 0 becomes
  `/dev/null` and fd 1 stderr.
- First start: when the project disk has no `.git`, the owner creates
  the box repository, runs `git remote add box pinfold::ROOT` on the host
  unless `remote.box.url` is set, pushes every host branch and checks
  out the host's branch. Later owners do nothing.
- Deleted: `pinfold push`, `pinfold fetch`, `pinfold box fetch`, their
  JSON and text reports, the per-owner push and fast-forward. The exit
  summary and scratch handling were already gone on main, as was
  guarantee 46.

Design choices the issue left open:

- **URL.** `pinfold::ROOT`, the canonical project root. The helper
  requires ROOT to be the project Git runs in, so the URL documents the
  project and a copied config is refused.
- **Who writes the remote.** pinfold, at the first start only, and never
  over an existing `remote.box.url`. A project whose disk predates this
  change gets no remote; `git remote add box pinfold::ROOT` adds it.
- **Helper on PATH.** The README's install steps add the link. pinfold
  writes nothing into PATH directories.
- **Connectivity.** The quarantined `git fetch` of the bundle stays, not
  the probe's `rev-list` replacement.
- **One listing per helper process,** since the stale-quarantine sweep
  keys on the pid.
- **`box fetch` deleted.** The helper calls core crossing directly.
  Keeping `box fetch` would keep a second ref writer, `update_refs`,
  that nothing else uses. A `box up` caller with a disk has no crossing
  now.
- **Copy-out** is `pinfold disk export`, unchanged. A project disk's
  name is `project.disks.project` in `pinfold config --json`.
- **Push** sends only what Git names. The first start alone sends every
  branch, with one glob refspec so argv does not grow with the branch
  count.

## Setup

Both platforms used temporary projects and private XDG roots with a fresh
`maintenance` stamp, `GIT_CONFIG_GLOBAL=/dev/null`,
`GIT_CONFIG_NOSYSTEM=1`, the calling environment's `GIT_CONFIG_*` and
`PINFOLD_PROFILE` unset, and the built binary first on PATH as `pinfold`
and `git-remote-pinfold`. The project was a fresh repository with
branches `main` and `side`, configured `kind = "disk"`. `T` below is the
temporary root.

- **macOS:** 27.2, Apple `container` 1.7.1, git 2.56.1 from Homebrew, the
  debug build of this branch. Image `localhost/pinfold/profile-default`
  already in the store.
- **Linux:** the dev VM, Debian 13, kernel 6.12.105, git 2.47.3,
  Firecracker. A musl debug build of this branch. `pinfold build`
  built `profile-default` in 2 min 27 s under the private roots.

**Hostile box.** A box's `PATH` has no writable directory, so a test
image stood in for a compromised box: `image-hostile296`, the default
profile image plus `/usr/local/bin/git`, a shim. While
`~/hostile/active` (or `REPO/.hostile/active`, for a transient box,
which has no home disk) exists, the shim prints `listing` for the fetch
script's branch listing, answers the "anything new" probe with a line,
and for `bundle create` sleeps 600 s (`stall`), streams N zero bytes
(`zeros`) or prints `bundle`. Otherwise it runs `/usr/bin/git`. The box
wrote these files itself, through `pinfold -- sh -c 'cat >…'`. The
hostile bundles were made on the host in a scratch repository: a commit
with no author email, a tree with a `.git/hooks/post-checkout` entry,
and a hand-written v2 bundle whose pack lacks the head's tree.

"State" is a diff of `git count-objects -v`, `git for-each-ref`, every
file under `.git/objects` and any `pinfold-incoming-*` directory,
`git fsck` and its exit code, `.git/config` and `.git/hooks`, before and
after the command.

## Observed

Both platforms unless marked. Paths are shortened to `T/`.

### Honest crossing

- **First start** (`pinfold -- sh -c 'git log --oneline --all
  --decorate; git status --short --branch; git config --list
  --show-origin'`): `(HEAD -> main, host/side, host/main) host one`,
  `## main`, and only `core.*` keys in the box repository's
  `.git/config`. On the host, `git config --get-regexp '^remote\.'`
  printed `remote.box.url pinfold::T/host` and `remote.box.fetch
  +refs/heads/*:refs/remotes/box/*`.
- **Transient fetch** (no owner): after two box commits, `git fetch box`
  printed `* [new branch] feature -> box/feature` and `main -> box/main`,
  exit 0. One new pack; no quarantine and no box left.
- **Lease fetch** (after `pinfold start`): `df746eb..3814349 main ->
  box/main` (macOS), `205d420..4918932` (Linux), exit 0.
- **Push:** `git push box main` and `git push box side:renamed` exited 0;
  the box then held `refs/remotes/host/main`, `host/renamed` and
  `host/side`. `git push box main:refs/tags/v1` and `git push box
  :refs/heads/side` (macOS) printed `! [remote rejected] … (only
  branches can be pushed)`, exit 1. `git push --atomic box main`:
  `fatal: helper pinfold does not support --atomic`, exit 128 (macOS);
  `git push --dry-run box main`: `fatal: helper pinfold does not support
  dry-run`, exit 128 (Linux). `git ls-remote box` printed the box's
  `main` and left no quarantine (Linux).
  After the push, the host's `refs/remotes/box/main` and `box/renamed`
  pointed at the host's own commit `f530490` until the next fetch.
- **Later owner:** after `pinfold stop` and a new host commit, a new
  owner's box still had `refs/remotes/host/main` at the old commit; the
  host's `main` had moved.
- **Existing remote:** a second project whose host repository
  already had `remote.box.url https://example.invalid/box.git` kept that
  URL after its first start.
- **Concurrent fetches,** both with a running owner and with none: both
  exited 0. One printed the update; the other printed `pinfold: waiting
  for the project box to start` and nothing else. `git fsck` afterwards
  reported only dangling commits from earlier cases.

### Refused before the box is reached

Each changed no state.

- A configured `remote.box.fetch +refs/heads/*:refs/heads/*`: `pinfold
  remote: remote-refspec: remote.box.fetch writes outside
  refs/remotes/box/: +refs/heads/*:refs/heads/*`, exit 128.
- `git -c 'remote.box.fetch=+refs/heads/*:refs/heads/box-*' fetch box`:
  the same message for that refspec, exit 128 (macOS).
- `git fetch pinfold::T/host main:other`: `remote-url: run \`git fetch
  box\` with the configured remote, not pinfold::T/host`, then Git's
  `fatal: remote helper 'pinfold' aborted session`, exit 128.
- A remote `other` at `pinfold::/elsewhere`: `remote-url: other's URL
  names /elsewhere, not this project; run \`git remote set-url other
  pinfold::T/host\``, exit 128.
- The helper run by hand with `GIT_DIR` of another repository: it
  answered `capabilities`, then refused `list` with `remote-repository:
  run Git in the project's main checkout`, exit 1.
- A share project (macOS) with a `box` remote: `git fetch box` and `git
  push box main` printed `share-no-crossing: edits in this shared
  project are already visible on the host and in the box`, with no
  owner and with a running share owner.
- Linux, the project's configuration switched to `kind = "share"`:
  `spec: share mounts are unsupported on Linux`, exit 128.

### Hostile box

Each `git fetch box` exited 128, printed the line below, and changed no
state.

| Case | Box sent | `pinfold remote:` |
|---|---|---|
| symref | `OID refs/heads/main`, `@refs/heads/main HEAD` | `the box's branch listing is malformed` |
| tag | `OID refs/tags/v1` | the same |
| long line | a 5011-byte name | the same |
| names | `a..b`, `x y`, `x:y`, `@{1}` | `the box named a malformed branch: "a..b"` and so on |
| control bytes | `a` ESC `[31mb` CR | `the box named a malformed branch: "a\u{1b}[31mb\r"` |
| branch limit | 4097 listed branches | `the box has more than 4096 branches` |
| reach rule | `stolen` at a host-only commit, no bundle | `branch "stolen" names 2a67…, which the bundle does not carry and refs/remotes/box/* does not reach` |
| fsck | the email-less commit | `git on the host failed: error: object 3708ee83…: missingEmail: invalid author/committer line - missing email\nfatal: fsck error in packed object` |
| `.git` entry | the hook tree | `… hasDotgit: contains '.git'\nfatal: fsck error in packed object` |
| missing tree | the hand-written bundle | `git on the host failed: fatal: bad tree object 0ee5b16c…\nerror: T/xdg/state/pinfold/crossing/PID.bundle did not send all necessary objects` |
| filtered | a v3 header with `@filter=blob:none` | `the box sent a filtered bundle` |
| size limit (Linux) | 4294967400 zero bytes | `limit: the bundle is past 4294967296 bytes`, after 48 s; no `head -c` left in the box |

Control: `alias` listed at the box's `main`, no bundle: `* [new branch]
alias -> box/alias`, exit 0.

### Not protected, as specified

- `git fetch box main:other` wrote `refs/heads/other` at the box's
  commit, exit 0 (macOS).
- A box branch `refs/heads/HEAD` at an older commit landed as
  `refs/remotes/box/HEAD`; `git rev-parse box` then gave that commit,
  not `box/main`.
- Partial update (macOS): with `box/x` on the host, the box replaced `x`
  by `x/y` and added `z`. `git fetch box`: `! [new branch] x/y -> box/x/y
  (unable to update local ref)` and `* [new branch] z -> box/z`, exit 1,
  `box/z` written. `git fetch --atomic box`: `error: cannot lock ref
  'refs/remotes/box/x/y': 'refs/remotes/box/x' exists`, exit 254.
- `git fetch --prune box` after the box deleted branches: `- [deleted]
  (none) -> box/HEAD`, `box/feature`, `box/renamed` (pushed earlier, see
  Push), exit 0.
- Git's auto maintenance ran after fetches on macOS: a snapshot taken
  right after showed `maintenance.lock`, `.tmp-*-pack-*` files and a
  transient `fsck` error (`packfile … index not opened`), gone in later
  snapshots.

### Kills

- **SIGKILL during the box's stream, running owner:** with the shim
  stalled, `kill -9` of the helper. Git: `error: git-remote-pinfold died
  of signal 9`, exit 128. No ref moved, no quarantine. Left:
  `T/xdg/state/pinfold/crossing/PID.bundle`. On macOS the box's stalled
  `sleep 600` was still running after the lease ended. The owner kept
  running, and the next fetch succeeded.
- **SIGKILL after `list`:** the helper was driven by hand with
  `capabilities` and `list` on a FIFO, then killed once it had answered.
  Left: `.git/objects/pinfold-incoming-PID` holding a pack, and no ref.
  The next `git fetch box` succeeded and removed that directory.
- **SIGKILL with a transient box:** Git exited 128 as above. The
  transient box ended with the helper (macOS: no container left; Linux:
  no Firecracker process). `pinfold disk list` showed the project disk
  `unclean`, attached to the dead box. The next `git fetch box` started
  a transient box on that disk. On macOS it succeeded and the disk read
  `clean`. On Linux the test's stall marker was still on the disk, so
  the new box showed the disk attached to it; SIGINT to the helper then
  ended the fetch (`interrupted by SIGINT`, exit 128, no state change)
  and the disk read `clean`.
- **SIGINT** (macOS, a fetch stalled against a transient box): `pinfold
  remote: interrupted by SIGINT`, exit 128; the box was torn down and
  the disk was clean.

### Copy-out and removed commands

- `pinfold disk export pi-…-project T/export-running` while the owner
  ran: `disk-in-use: export requires an unattached read-only disk`,
  exit 1, no destination. After `pinfold stop`: `Exported … (untrusted
  files)`, exit 0, with the box's `.git`, `b` and `h`.
- `pinfold push` and `pinfold fetch`: `pinfold: unknown command "push"`
  (`"fetch"`), exit 1. `pinfold box fetch x --repo /x --into .`:
  `pinfold box: unknown box verb "fetch"` and the box usage, exit 1.

## Findings

- **Bundle files leak.** A killed helper leaves
  `state/pinfold/crossing/PID.bundle`. `Bundle::new` removes only its own
  pid's file, so nothing removes another's. This predates the helper
  (`pinfold fetch` had the same path). A ticket candidate.
- **A box command outlives its lease.** On macOS the stalled fetch
  script kept running in a kept box after the helper died. It ends with
  the box.
- **Auto maintenance** runs after each fetch now, as the probe inferred.
- **A failed first start is final.** Linux, a new project with
  `.git/config.lock` held on the host: `pinfold: sync-failed: the host's
  \`box\` remote could not be added\n`, CMD ran, and the box held an
  empty repository (`.git` only, no commits, no refs). After the lock
  was removed, a second start did nothing: still no remote and no refs.
  Recovery is by hand: `git remote add box pinfold::ROOT`, `git push box
  BRANCH`, and a checkout of `host/BRANCH` in the box.
- **Owner diagnostics print a literal `\n`.** The detached owner's
  stderr reaches the client through `forward_diagnostics`, which escapes
  the whole buffer, its final newline included. Not new here.

## Not run

- Linux: the share refusals (Linux has no shares) and the `-c` refspec
  and partial-update cases.
- macOS: the 4 GiB limit and `push --dry-run`.
- 4096 real branches, a host repository past the first push's argv
  limits, sha256 repositories, `git clone pinfold::…`, `git pull box`
  with a hostile bundle, submodule recursion.
- Guarantee 10's host-hook scenario (merge and checkouts after a fetch):
  unchanged mechanism, not rerun.

## Review fixes

Review of PR #346 at 7a61874. Each finding was checked against the code
first. The same setup was used as above, rebuilt from the fixed tree:
Linux on the dev VM (git 2.47.3) and macOS (git 2.56.1). `T` is the
temporary root.

1. **The inner fetch wrote host tags. Confirmed.** In plain git, the
   inner fetch as crossing ran it, from a bundle that carried
   `refs/tags/boxtag`, gave these results:
   - with `fetch.prune` and `fetch.pruneTags`, it deleted the host's
     `hosttag` and wrote `boxtag`;
   - with `fetch.pruneTags` alone, it wrote `boxtag`;
   - with `--no-prune --no-prune-tags`, it changed neither.

   That fetch now pins both flags, and a bundle head that is not a
   branch fails earlier anyway (5).

   Linux: I drove the helper by hand with `GIT_CONFIG_COUNT` setting
   both options, then sent `capabilities` and `list` for an honest box
   commit. It answered `:object-format sha1`, the new commit and
   `refs/heads/main`. `hosttag` was still there, the state was
   unchanged, and the helper exited 0. A user's own
   `git -c fetch.prune=true -c fetch.pruneTags=true fetch box` then
   printed `- [deleted] (none) -> hosttag`. This is the outer fetch, now
   listed as Not protected.
2. **The first-start marker could be erased by the box. Confirmed.** The
   attempt is now recorded on the host, in
   `state/pinfold/disks/NAME.started`, before the object format or
   branch name is sent. Creating or removing the disk clears the record.

   On both platforms the box ran `rm -rf .git`, then the host created
   branch `newer`. The next start's box showed only `b` and `h`, and
   `git for-each-ref` answered `not a git repository`. Nothing was
   pushed.
3. **The helper trusted a URL from outside the configuration, and
   symbolic refs under the remote. Confirmed.** On Linux:
   - `.git/remotes/legacy` (`URL: pinfold::T/host`,
     `Pull: refs/heads/*:refs/heads/legacy-*`) was refused:
     `remote-url: set remote.legacy.url; …`, exit 128, state unchanged;
   - `refs/remotes/box/evil` as a symbolic ref to `refs/heads/main` was
     refused: `remote-refspec: refs/remotes/box/evil is a symbolic ref to
     refs/heads/main; delete it`, exit 128, state unchanged.

   A configuration that Git cannot read is now refused, with every exit
   code except 0 and 1. That path was not reproduced.
4. **Environment redirects were honoured. Confirmed for
   `GIT_CONFIG`,** which makes the refspec gate read another file. On
   Linux:
   - `GIT_OBJECT_DIRECTORY=…` gave `remote-repository: unset
     GIT_OBJECT_DIRECTORY …`, exit 128;
   - `GIT_CONFIG=/dev/null` was refused the same way.

   A linked worktree was refused before and still is: `worktree: .git
   is not a directory; run pinfold from the main checkout`. The spec has
   always refused a `GIT_DIR` other than ROOT's `.git`, so there was
   nothing to break.
5. **Bundle heads could reach host-only commits. Confirmed.** In plain
   git, the old sequence (`bundle verify`, `index-pack --fix-thin
   --fsck-objects`, the quarantined fetch) accepted a header naming a
   host-only commit over an empty pack, all with exit 0.

   Now the checks work as follows:
   - every bundle head must be a listed branch at its listed id;
   - every tip must be of type `commit`;
   - a tip passes only if its offset lies inside the box's own pack
     bytes, read from `git show-index`, or if `box/*` reaches it.

   On Linux each case below exited 128 with the state unchanged:
   - the stolen head over an empty pack: `branch "stolen" names 95c7…,
     which the box did not send and refs/remotes/box/* does not reach`;
   - an annotated tag object as a tip: `… is not a commit`;
   - a complete bundle whose head `extra` is not listed: `the bundle's
     heads differ from the box's branch listing`;
   - a branch listed twice: `listing is malformed`.

   All earlier hostile cases ran again with the same messages, and so
   did the `alias` control.
6. **A rollback could unlink a pack another fetch had adopted.
   Confirmed by reading the code, not reproduced.** The code dates from
   #194 (2026-10-24). Moves now run under an exclusive `flock` on
   `objects/pack`. Two concurrent fetches of the same commits succeeded,
   both against a running owner and against transient boxes, and
   `fsck --connectivity-only` passed. In both runs the second fetch
   waited for the first fetch's lease, so the race window itself was not
   hit.
7. **Pushes accepted other sources and echoed the box's stderr.
   Confirmed.** On both platforms these were rejected with `only
   branches can be pushed`:
   - `refs/stash:refs/heads/st`;
   - `hosttag:refs/heads/fromtag`;
   - `main~0:refs/heads/expr`.

   On Linux, with `refs/remotes/host/main.lock` held in the box, `git
   push box main` printed only `pinfold: the box's git failed` and
   `(the box refused the push)`. The push succeeded once the lock was
   gone.
8. **The recovery text was incomplete. Confirmed.** `sync-failed`, the
   README and the skill now say to check `git remote get-url box` (or
   add the remote) before `git push box BRANCH`.
9. **Object format. Confirmed, and incomplete as briefed.** The box
   repository now takes the host's format. A sha256 project then
   failed `git fetch box` with `fatal: object 5fbeba7a… not found`,
   because Git reads a helper's listing as SHA-1. The listing now starts
   with `:object-format FORMAT`.

   On both platforms, a sha256 host's box printed `sha256` and `git
   fetch box` landed `box sha`. The sha1 projects fetched as before.
10. **The first start needs the helper on PATH. Not confirmed.** The
    first start runs no `git-remote-pinfold`: `git remote add` does not
    invoke a helper. On both platforms, with no helper on PATH (`command
    -v` printed `none`), the first start wrote `remote.box.*` and
    checked out `host/main`. Nothing was changed.

Not run:
- a configuration that Git cannot read;
- the kill cases, which were not rerun;
- the 4 GiB limit, which was not rerun;
- the migrate race itself (see 6).

I removed everything I created on both machines.
