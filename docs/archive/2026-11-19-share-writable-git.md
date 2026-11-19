# Share with a writable .git: host commits against a committing box

Date: 2026-11-19. Issue #334, "Before the code". Follows
`2026-11-19-shared-git-probe.md`. Scripts and raw logs:
`2026-11-19-share-writable-git/`.

## Probe: host commits with GIT_OPTIONAL_LOCKS=0, box commits in a loop

The question: does a host that commits with optional locks off damage
commits while a box commits in the same shared repository.

Environment (`logs/env.log`): macOS 27.2.1 (26B131), arm64, Apple
`container` 1.7.1, host git 2.56.1, guest git 2.47.3, guest kernel 6.18.15,
share `virtiofs /work virtiofs rw,relatime`. Image
`localhost/pinfold/profile-default:latest` `9b5df3f99e34`, the same as the
earlier probe. The box is raw `container run` with a writable share, as
before: `container run -d --name pfs-hc1119 --user 501:20 -v ROOT:/work
--entrypoint sleep IMAGE infinity`. `run.sh` starts it, runs `hc.py 3 30`,
and removes the box and the temp directory. `lib.py` is the 2026-11-12 file.

Shape, as case 4: three fresh repositories (`c0`, `c1` touching `a` and
`b`), 30 host iterations each of
`echo N >> mine; git status --porcelain; git add mine && git commit` with
`GIT_OPTIONAL_LOCKS=0` exported and no global config. The box runs one
`container exec` loop per repository: append to `theirs`, `git status
--porcelain`, `git add theirs && git commit`, sleep 0.1 s, until the host
creates a stop file. The box loop starts 1 s before the host. A status line
is anomalous unless it names `mine` or `theirs` with a code other than `D`.

Results (`logs/hc.jsonl`, summary lines):

| Repo | Host anomalous | Host refusals | Host commits exit 0 | Host commits on `main` | Box iterations | Box anomalous | Box commits exit 0 | Box commits on `main` | Commits on `main` | Final tree on `main` |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 30/30 | 5 | 25 | 1 | 14 | 7 | 13 | 1 | 2 | `mine theirs` |
| 2 | 29/30 | 2 | 28 | 0 | 14 | 7 | 9 | 1 | 1 | `theirs` |
| 3 | 26/30 | 6 | 24 | 1 | 13 | 5 | 12 | 1 | 2 | `mine theirs` |

Host anomalies count a refused commit or an odd status line. Anomalies
carry forward, as in case 4. Host status lines over all iterations:
`?? a` and `?? b` 82 times each, `D  mine` 9, `D  a` and `D  b` 3 each.
Box status lines: `?? a` and `?? b` 19 each, one `D  a`, `D  b`,
`D  theirs`. The host's first anomalous iterations were 0, 1 and 0; the
box's 8, 8 and 9. In repository 2 the host's iteration 1 status was
`D  a`, `D  b`, `D  mine`, `M  theirs`, `?? a`, `?? b`, `?? mine` and its
commit exited 0.

Host refusals: 13 `index.lock: File exists`, and 2 `cannot lock ref 'HEAD':
is at X but expected Y`. One box refusal named `index.lock`; five box
refusals in repository 2 logged no stderr.

At the end, in all three repositories: `main` had one root commit and one or
two commits in total, against 38, 37 and 36 commits that exited 0 plus
the two setup commits. `a` and `b`
were gone from `main` and untracked on the host. In repository 2 `mine` was
gone from `main` too. The reflog of `main` held 33 to 37 entries. Nothing was
staged and `fsck` exited 0 with dangling commits and blobs.

Result: with optional locks off on the host, concurrent host `git add`
and `git commit` still damage commits. Commits exit 0 and are then lost
from the branch, and commits record tracked files as deleted. The reflog
held 35, 37 and 33 entries against 40, 39 and 38 commits in total, so some
lost commits remain only as dangling objects. `GIT_OPTIONAL_LOCKS=0`
covers host read commands such as `status` and `diff`; it does not make
host commits safe against a committing box. The spec's Not protected text
says so.

Not tested in this case:

- A box committing every 0.1 s always lands inside the known window of
  about 1 s after a host commit (case 1 of the earlier probe), so this
  case cannot separate that window from host index writes.
- Host commits paced the way a person commits, seconds or minutes apart.
- Host commits with optional locks on, against a committing box.
- Box loop paces other than 0.1 s, or a box that commits once.

## Change

Commit `6b0a740`. A share mounts the project folder writable, `.git`
included. Removed: the generated read-only Git attachment and its private
mount marker, its last position among Apple mounts, the session
snapshot's generated Git mount, the owner's per-grant protection-layout
recheck (`project.protection`), and every admission check that existed
for protected `.git`: the `.git` file or symlink refusal, destination and
source overlap with `.git`, overlapping-share exposure, the symlink and
multiply-linked-file walks, and the destination inode comparison.
`core/mounts.rs` went from 350 to 190 lines.

Kept, because each still serves a share without the attachment: the
exact-path `safe.directory` (Apple mount tops are root-owned in the
guest), the `*` destination refusal, the plain-directory share source,
protected-path overlap in both directions, init and forwarded-socket
overlap, the plain-directory walk of destinations nested in a share,
worktree topology reads, and the snapshot's `repository` comparison.

Added: `pinfold -- CMD` on a share prints `git-optional-locks` and one
line on stderr when the caller's environment does not hold
`GIT_OPTIONAL_LOCKS=0`. The bounded wait for the host's HEAD in #334 was
dropped by the owner before it was built.

## Checks

On arm64 macOS: `cargo fmt --check`, `cargo clippy --all-targets --locked
-- -D warnings`, and the same Clippy for `--target
x86_64-unknown-linux-musl` with a `zig cc -target x86_64-linux-musl`
wrapper as ring's C compiler. All passed. On the dev VM, `cargo build
--locked --target x86_64-unknown-linux-musl` of `6b0a740`.

## Verification on macOS

`verify-mac.sh` with the debug binary of `6b0a740`, Apple `container`
1.7.1, private XDG config, state and cache roots, temp Git projects,
`PINFOLD_CPUS=2 PINFOLD_MEMORY=1G`, no global Git config. The caller's
shell set `PINFOLD_PROFILE=full`, so the existing
`localhost/pinfold/profile-full:latest` ran and every command printed
`image-outdated`. Output: `logs/verify-mac.log`.

| Case | Observed |
|---|---|
| S1: box edits lines 1 and 20 of `a`, `printf 'y\nn\n' \| git add -p a`, `git commit` | exit 0. Host `HEAD` is `box1`, its parent the prior host commit; `HEAD:a` has line 1 changed and line 20 not; the working tree has both; host status ` M a`; `fsck` exit 0 |
| S1: launch record | two mounts: the share (`kind: share`) and the profile files (`readonly`); no `.git` mount |
| S1, S2: warning | unset: `pinfold: git-optional-locks: …` on stderr; `GIT_OPTIONAL_LOCKS=0`: none; `=1`: printed |
| S3: kept box; host commit, 3 s, box `git commit -qam`, 5 times | 5/5 box commits exit 0 with the host commit as parent; 12 commits on `main`; host status clean; `fsck` exit 0; no warning |
| S4: `git worktree add wt -b side`, box commit from `wt` | exit 0. `side` is `wtbox` with the starting commit as parent; `main` unchanged |
| S5: disk project | box `git log -1` printed the host's `c0`; no warning; the box commit stayed off the host branch; `pinfold fetch` brought it as `pinfold/main` |

Cleanup removed the three disks under the private state root and the
roots. The first run's script parsed `disk list --json` wrongly and left
its three disks; they were removed by recreating that run's state path
and `disk rm`. No `pfv` or `pfs` box or directory remains.

## Verification on Linux

`verify-linux.sh` on the dev VM (Debian 13, x86_64, Firecracker), private
XDG roots, a saved profile `pfl` `FROM debian:trixie-slim` with git and a
system Git identity. A first run without that identity failed the box
commit with Git's identity error; the profile was the cause. Output of
the second run: `logs/verify-linux.log`.

| Case | Observed |
|---|---|
| L1: disk project, box `git log -1` and commit | exit 0, printed `c0`, no warning; `pinfold fetch` brought `lbox` as `pinfold/main` |
| L2: project configuration set to `share` | `spec: share mounts are unsupported on Linux`, exit 125 |
| L3: `box up` spec with a share | `refused`, reason `spec`, `share mounts are unsupported on Linux`, exit 1 |

Cleanup removed both disks, the private roots, the bundle, source and
build, and `/tmp/pinfold-1000`, which was created during these runs. No
Firecracker process remained.

## Not run

- The old binary as a control for S1 (its read-only `.git` refused box
  commits by design).
- A box commit within a second of a host commit, and host Git running
  while the box commits, through the real binary: the probe above and the
  earlier probe cover them.
- Several boxes on one share; editors and GUI Git clients.
- The gained-`.git` `config-changed` refusal, and the kept admission
  refusals after the `prepare` rewrite (plain-directory share source,
  nested destination through a symlink, `*` destination).

## Host-only control: what GIT_OPTIONAL_LOCKS=0 covers

The spec draft said the variable keeps host `git diff` from rewriting the
index; the earlier probe measured it only with `git status`. `ctl.sh`, no
box: a fresh repository per command, 20 runs of `sleep 1.1; touch a; CMD`,
counting runs after which `.git/index` had a new inode. The sleep keeps
the index entry from being racily clean. `logs/ctl.log`.

| Command | Index replaced |
|---|---|
| `git diff --stat` | 20/20 |
| `GIT_OPTIONAL_LOCKS=0 git diff --stat` | 20/20 |
| `git --no-optional-locks diff --stat` | 20/20 |
| `git diff` | 20/20 |
| `GIT_OPTIONAL_LOCKS=0 git diff` | 20/20 |
| `git status --porcelain` | 20/20 |
| `GIT_OPTIONAL_LOCKS=0 git status --porcelain` | 0/20 |

The variable covers `git status` only. The spec, README and warning were
narrowed to say so. The reworded warning through the rebuilt binary, a
share project with the variable unset:
`pinfold: git-optional-locks: GIT_OPTIONAL_LOCKS=0 is not set, so host
`git status` rewrites the index and can make box commits record files as
deleted.`, exit 0. Its home disk and roots were removed.

## Review fixes (2026-11-19)

Six findings from the review of PR #342 at `3e1db10`.

1. A signal received while the `git-optional-locks` warning is written is
   no longer discarded. The client leaves its lease and returns 128+n
   before CMD is dispatched. A failed write without a signal stays
   nonfatal.
2. The owner again checks a share's root `.git` marker without following
   it, against its snapshot's `repository`, on every Probe and immediately
   before every Join or Keep grant. A marker that appeared, disappeared or
   is not a plain directory refuses as `config-changed` (`repository`).
   This needs no generated mount. Spec and README say so.
3. Guarantee 48 is path confinement. It and Not protected now say a share
   file hard-linked to a file outside the share, Git objects from a local
   clone included, can be changed in place through the share.
4. `run.sh` names its box per run and removes only a box it created.
   `logs/env.log` and `logs/hc.jsonl` came from the earlier version, whose
   box was `pfs-hc1119`.
5. The warning's reason now reads: host `git status` can refresh the
   index when optional locks are enabled. The spec says disk projects and
   `start` omit this warning. The trigger stays literal: the warning prints
   unless `GIT_OPTIONAL_LOCKS` is exactly `0`. A first version silenced
   every Git false value; the coordinator withdrew that part of the
   finding, and it was reverted.
6. `core::directory_entries` no longer passes the parent descriptor to its
   callback; its `borrow_raw` is gone.

Checks: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D
warnings`, and the same Clippy for `x86_64-unknown-linux-musl` with zig as
ring's C compiler. All passed.

### Mac cases

`review-mac.sh` and `sigterm.py`, run once with the fixed binary and once
with a base build of `3e1db10` as the control. Private XDG roots, temp
projects, the caller's `PINFOLD_PROFILE=full`. Logs:
`logs/review-mac-fixed.log`, `logs/review-mac-base.log`.

Fix 1. A Git share project kept by `pinfold start`, so a join writes
nothing to stderr before the warning. Three joins with a read stderr put
the warning at 0.46 to 0.51 s. Then three joins of `touch ran-N` with
stderr a pipe filled to 65,536 bytes and never read, SIGTERM 0.5 s after
the latest measured warning time, inside the write's one-second deadline.

| Binary | Exit | CMD ran |
|---|---|---|
| fixed | 143, 143, 143 | no, no, no |
| base | 1, 1, 1 | no, no, no |

The base did not run CMD either; it exited 1 at some later setup step,
which was not traced. This run shows the fix's exit status, not the base
dispatching CMD.

Fix 5. The rows for the Git-false version in `logs/review-mac-fixed.log`
are superseded. After the revert, `values-mac.sh` with the rebuilt binary,
joins of a kept share box (`logs/values-mac.log`): unset, `1` and `false`
showed the warning; `0` was silent. All exited 0. The base build showed the
same. Its home disk and roots were removed.

Fix 2. A non-Git share project kept by `pinfold start`, then on the host:

| Step | fixed | base |
|---|---|---|
| `.git` a symlink to a missing path; `-- true`, then `start` | `config-changed`, `repository`, exit 125 (both) | `worktree: .git is not a directory`, exit 125 (both) |
| `.git` replaced by `git init`; `-- true`, then `start` | `config-changed`, `repository`, exit 125 (both) | `config-changed`, `repository`, exit 125 (both) |
| `stop`, then `-- sh -c 'echo after-stop'` | stop exit 0; the next owner runs it, exit 0 | same |

The symlink case reaches the owner's Probe before client discovery, so it
shows the owner check. The window between a client's snapshot and its Join
was not forced; the same check runs there.

Cleanup removed the four home disks and the private roots. No box, owner
process or `/tmp/pfr.*` directory remained.
