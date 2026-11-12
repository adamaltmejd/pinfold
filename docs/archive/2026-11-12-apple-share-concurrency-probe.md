# Apple writable share: concurrent git and editor probe

Date: 2026-11-12. Follows `2026-11-11-apple-share-probe.md`. Diagnostic
only; no guarantee is published here. Scripts and raw JSONL logs:
`2026-11-12-apple-share-concurrency-probe/`.

The question: a host directory shared writable into an Apple `container`
box, with git and editors used on both sides of the same checkout. Which
interleavings lose or damage data, and does any guest-side step without
capabilities prevent it.

## Environment

- macOS 27.2.1, build 26B131, arm64. Host uid:gid 501:20.
- Apple `container` 1.7.1, apiserver 1.7.1.
- Guest kernel 6.18.15 aarch64. Share reports `virtiofs on /work type virtiofs (rw,relatime)`.
- Host git 2.56.1. Guest git 2.47.3. Python 3.15.2 drives the trials.
- Image `localhost/pinfold/profile-default:latest` (ae6f1273f183).
- Box: `container run -d --user 501:20 -v /tmp/pfs.EUq1:/work --entrypoint sleep IMAGE infinity`.
  Case 7 adds a second box that mounts the same directory at its host
  path (`-v /private/tmp/pfs.EUq1:/private/tmp/pfs.EUq1`).
- Every guest step is one `container exec … sh -c`. An exec starts in about
  40 ms.
- Both sides run git with `GIT_CONFIG_GLOBAL=/dev/null` and fixed author
  names, so the host's signing config stays out. The guest adds
  `safe.directory=*` through `GIT_CONFIG_*`.
- Each trial uses a fresh repository under the temp dir: `git init -b main`
  and two commits `c0`, `c1` touching files `a` and `b`.

The container service was stopped at the start; it was started with
`container system start --disable-kernel-install` and stopped again at the
end.

Log handling: `logs/c1-base.jsonl` holds two appended runs (delays 0 to 2 s,
then 0.75 and 0.9 s), as does `logs/c8-ls.jsonl` (0 s, then 0.25 and
0.5 s). In `logs/c8-wrap.jsonl` the `guest_commit_rc` field was re-parsed
after the run, because the wrapper's stderr line followed `commit_rc=0` and
broke the first parse; all 20 are 0. A first guest-loop run of case 4 was
discarded: its loop had a shell syntax error and never ran.

## Words used below

- Prime: the guest runs `git status --porcelain; git log -1` in the repository.
  The prior probe found the stale window counts from the guest's last lookup.
- History loss: the host's last commit is no longer an ancestor of `main`.
- Content loss: the commit at the tip of `main` lacks a file or line the
  host had committed before the guest acted.
- Delay: time from the end of the host's command to the start of the guest
  exec.

## Case 1: host commit, then guest commit

`c1.py base 10`. Guest primes. Host runs
`echo host >> a; echo h > hfile; git add a hfile; git commit -qm host`, 21 to
26 ms after the prime. After the delay the guest runs
`echo g > gfile; git add gfile; git commit -qm guest`. The host then checks
ancestry, `git ls-tree`, `git fsck --full`, `git reflog main` and
`git status`.

| Delay (s) | Trials | History loss | Content loss | Guest commit exit 0 |
|---|---|---|---|---|
| 0 | 10 | 10 | 10 | 10 |
| 0.25 | 10 | 10 | 10 | 10 |
| 0.5 | 10 | 10 | 10 | 10 |
| 0.75 | 10 | 10 | 10 | 10 |
| 0.9 | 10 | 0 | 2 | 10 |
| 1.0 | 10 | 0 | 0 | 10 |
| 1.5 | 10 | 0 | 0 | 10 |
| 2.0 | 10 | 0 | 0 | 10 |

In every history-loss trial, `main` became one parentless commit `guest`
whose tree held only `gfile`. Host `git status` then showed `?? a`, `?? b`,
`?? hfile`. `git fsck --full` exited 0 with no output. `git reflog main`
still listed `host`, `c1` and `c0`, so the old history was reachable only
through the reflog.

The two content-loss trials at 0.9 s kept history: the guest commit had
`host` as its parent but its tree held only `gfile`. The ref was already
fresh for the guest; the index was not.

`c1.py observe 10` repeats the sweep with
`git status --porcelain; git log --oneline -3` in the same exec before the
commit. Counts were identical: 30 of 30 lost at 0 to 0.5 s, 0 of 30 at
1.0 to 2.0 s. Inside the window the guest printed:

```
?? a
?? b
?? hfile
fatal: your current branch 'main' does not have any commits yet
```

From 1.0 s the guest log showed `host`, `c1`, `c0`.

## Case 2: guest commit, then host commit

`c2.py 10`. Guest primes and commits `gfile`; after the delay the host
commits as in case 1. 60 trials at 0, 0.25, 0.5, 1.0, 1.5 and 2 s: 60 clean.
Both commits on `main`, four commits in a line, all four files, clean host
status, `fsck` exit 0.

## Case 3: concurrent commits

`c3.py 3`. Guest primes. Then the guest commit (`gfile`) and the host
commit (`hfile`) start together, with the host start offset from the guest
exec launch by -0.1 to +0.15 s. 30 trials.

| Host start offset (s) | Trials | Outcome |
|---|---|---|
| -0.1 to +0.02 | 15 | Both exit 0. `main` is a parentless `guest` with only `gfile`. Host commit dropped (reflog only). |
| +0.04 | 1 | Same as above. |
| +0.04 | 2 | Host fails: `Unable to create '…/.git/index.lock': File exists.` Guest commit on `c1`, tree `a b gfile`. No loss; `hfile` untracked. |
| +0.06 | 1 | Guest fails: `cannot lock ref 'HEAD': unable to resolve reference 'refs/heads/main'`. Host exits 0, but its commit's tree is empty: it records `a` and `b` as deleted and lacks `hfile`. Host status `?? a ?? b ?? gfile ?? hfile`. |
| +0.06 to +0.15 | 11 | Clean: four commits in a line, all files. |

No `index.lock` or `main.lock` was left behind. `fsck` exited 0 in all 30.
With a 40 ms exec start, offsets up to about +0.02 s mean the host commit
finished before the guest git ran, so those 15 trials are case 1 at delay 0.
Only the +0.04 and +0.06 rows overlapped.

`c3.py 10` then sampled the overlap band densely, 50 trials
(`logs/c3-dense.jsonl`). "Refused" means one commit exited non-zero
(`index.lock: File exists`, `cannot lock ref 'HEAD'`, or exit 1) and the
other side's commit and files were all on `main`.

| Offset (s) | Clean | Refused, nothing lost | Content loss, history kept | Host commit dropped from `main` |
|---|---|---|---|---|
| +0.03 | 0 | 3 | 3 | 4 |
| +0.04 | 0 | 6 | 3 | 1 |
| +0.05 | 6 | 3 | 1 | 0 |
| +0.06 | 8 | 1 | 1 | 0 |
| +0.07 | 10 | 0 | 0 | 0 |

The content-loss trials (8) had a commit that exited 0 whose tree lacked
`a`, `b` or the committer's own file; in two of them the tree on `main` was
empty. `fsck` exited 0 in all 50 and no lock file was left.

## Case 4: a side that only refreshes the index

`c4.py MODE 3 30`. One side loops `git status --porcelain; sleep 0.1`. In the
plain poll mode that is all it does. The forced modes also `touch a` before each status, so that every
status refreshes stat data and replaces `.git/index`. Host only, 20
iterations of the forced loop: the index inode changed 20 of 20 times with
`GIT_OPTIONAL_LOCKS=1`, 0 of 20 with `GIT_OPTIONAL_LOCKS=0`
(`logs/c4-index-rewrite.log`). The other side runs 30 iterations of
`echo N >> mine; git status --porcelain; git add mine && git commit`.
Three repositories per mode, 90 iterations. Anomalous means a commit
refused, or a status line other than `M mine` or `?? mine`.

In the plain poll mode the host loop compared the index inode before and after each of its own
`git status` runs. It replaced the index 17, 18 and 17 times in the three
repositories, prompted by the guest's edits to `mine`. The `index_inodes_seen`
field in the logs is sampled once per iteration and also counts the
committing side's own index writes. It does not measure the loop.

| Mode | Anomalous iterations | Final tree on `main` | Commits landed |
|---|---|---|---|
| Host plain poll, guest commits | 86/90 (4 `index.lock` refusals) | `mine` only (3/3 repos) | 86/90 |
| Host forced loop, guest commits | 87/90 | `mine` only (3/3) | 90/90 |
| Host forced loop with `GIT_OPTIONAL_LOCKS=0`, guest commits | 0/90 | `a b mine` (3/3) | 90/90 |
| Host forced loop, guest uses `gitwrap.sh` | 89/90 (4 `index.lock` refusals) | `mine` only (3/3) | 86/90 |
| Guest forced loop, host commits | 12/90 (7 `index.lock` refusals, 5 follow-on `MM mine`) | `a b mine` (3/3) | all 30 lines of `mine` present; refused commits folded into the next |

With either host loop, the guest's status showed lines such as `?? a`, `?? b`,
`D  mine`, `?? mine` or `D  a`, `D  b`. Its commits exited 0. The commits
recorded `a` and `b` as deleted, and the host status afterwards showed
`?? a ?? b`. History stayed linear with one root. `fsck` exited 0 in every
repository of every mode.

With the guest loop, the host saw only `index.lock` refusals from the guest's
`git status` holding the lock. No file or line was lost.

## Case 5: host ref updates other than commit

`c5.py`. Setups: `checkout-b` (host on `main`, runs `git checkout -b feat`);
`switch` (host on `feat` with one extra commit, runs `git switch main`);
`rebase` (host on `feat` branched from `c0`, runs `git rebase main`);
`stash` (host has `a` modified, runs `git stash`). The guest primes with
status, log, `git branch`, `git stash list`, `cat a b` and
`ls -la . .git .git/refs/heads`.

Observe, 5 trials each, guest reading branch, HEAD, status and stash count
at 0.04, 0.3, 0.6, 1.0 and 1.5 s:

| Op | 0.04 to 0.6 s | 1.0 and 1.5 s |
|---|---|---|
| checkout -b | `fatal: not a git repository (or any parent up to mount point /)` (5/5) | correct (5/5) |
| switch | same (5/5) | correct (5/5) |
| rebase | same (5/5) | correct (5/5) |
| stash | branch, HEAD and stash count correct; status `D  a`, `D  b`, `?? a`, `?? b` (5/5) | correct, status clean (5/5) |

Damage, 10 trials each, guest at once runs `git status`, adds `gfile` and
commits:

| Op | Guest commit | Refs moved | Result |
|---|---|---|---|
| checkout -b | fails, not a git repository (10/10) | none | `gfile` untracked; no other change |
| switch | fails, same (10/10) | none | same |
| rebase | fails, same (10/10) | none | same |
| stash | exits 0 (10/10) | `refs/heads/main` only | commit on the host's tip, tree deletes `a` and `b`; host status `?? a ?? b`; stash entry kept |

`fsck` exited 0 in all 40.

The same damage run with the guest acting 0.9 s after the op
(`C5_DELAY=0.9`, `logs/c5-damage-0.9.jsonl`), 10 trials each:

| Op | Clean | Guest refused, not a git repository | Commit deletes `a` and `b` |
|---|---|---|---|
| checkout -b | 0 | 10 | 0 |
| switch | 2 | 8 | 0 |
| rebase | 1 | 9 | 0 |
| stash | 3 | 0 | 7 |

## Case 6: atomic editor saves

`c6.py`. File `f` holds `l1`. One side reads `f`, then the other saves it
atomically (`printf 'l1\nX\n' > f.tmp && mv -f f.tmp f`). After the delay
the first side edits `f` to add its own line. Sequential last-writer-wins
predicts both lines. The host content is read at once and again 1.5 s later.

Host saves, guest edits (10 trials at 0 s, 5 at 0.5 s, 5 at 1.5 s per method):

| Guest method | 0 and 0.5 s | 1.5 s |
|---|---|---|
| Guest `cat f` before editing | `No such file or directory` (60/60) | `l1 host` (20/20) |
| `echo guest >> f` | exit 2, `cannot create f: Directory nonexistent`; host keeps `l1 host` (15/15) | both lines (5/5) |
| `sed -i '$a guest' f` | exit 2, `can't read f`; host keeps `l1 host` (15/15) | both lines (5/5) |
| `c=$(cat f); printf … > f.tmp && mv f.tmp f` | exit 0; host file is the 7 bytes `\nguest\n` (15/15) | both lines (5/5) |
| `c=$(cat f); printf … > f` | exit 2, `cannot create f: Directory nonexistent`; host file is 0 bytes (15/15) | both lines (5/5) |

Guest saves, host edits (10 trials at 0 s per method: append, in-place
rewrite, temp and rename, `perl -0pi`): 40 of 40 ended with `l1 guest host`
on both sides.

## Case 7: linked worktree at ./wt

A host `git worktree add ./wt -b wtb` writes absolute host paths:
`wt/.git` holds `gitdir: /private/tmp/pfs.EUq1/…/.git/worktrees/wt`
(`logs/c7-paths.log`). In the `/work` box:

- In `wt`: `fatal: not a git repository: /private/tmp/pfs.EUq1/…/.git/worktrees/wt`.
- In the root: `git status` lists `?? wt/`. `git worktree list` marks the
  worktree `prunable`.
- `git worktree prune` in the guest exits 0 and prints
  `Removing worktrees/wt: gitdir file points to non-existent location`.
  The host then lists no worktree, `.git/worktrees` is gone, and host
  `git status` in `wt` fails with `gitfile does not point to a valid repository`.

With host `git worktree add --relative-paths`, host git sets
`extensions.relativeWorktrees=true`. Guest git 2.47.3 then refuses the whole
repository, root and worktree: `fatal: unknown repository extension found: relativeworktrees`.

To measure concurrency anyway, `c7.py 10` ran in the second box, which sees
the share at its host path. This setup is not what a pinfold box would use.
`wt/` was added to `.git/info/exclude`. Guest primes both checkouts, then
one side commits and the other commits at once.

| Order | Trials | Clean |
|---|---|---|
| Host commits in main, guest commits in wt | 10 | 10 |
| Host commits in wt, guest commits in main | 10 | 10 |

Each branch held its own commit on `c1`. Both statuses clean, `fsck` exit 0.

## Case 8: mitigations against case 1

`c1.py VARIANT 20 … 0`: case 1 at delay 0. Loss-free means no history loss,
no content loss, `gfile` committed, clean host status and `fsck` exit 0.

| Variant | What changes | History loss | Content loss | Loss-free |
|---|---|---|---|---|
| (a) `git status` first | guest runs it in the same exec before add | 20/20 | 20/20 | 0/20 |
| (a) `git update-index --refresh` first | exits 0 | 20/20 | 20/20 | 0/20 |
| (b) `ls -la .git .git/refs/heads` first | output discarded | 0/20 | 0/20 | 20/20 |
| (b) same, at 0.25 and 0.5 s | | 0/20 | 0/20 | 20/20 |
| (b) same, host runs `git pack-refs --all` after its commit | | 20/20 | 0/20 | 0/20 |
| (c) guest git through `gitwrap.sh` | waits 1.1 s when `.git/logs/HEAD` changed | 0/20 | 0/20 | 20/20 |
| (d) `core.trustctime=false`, `core.checkStat=minimal`, `core.fsmonitor=false`, `core.untrackedCache=false` | repo config before prime | 20/20 | 20/20 | 0/20 |
| (e) `git pack-refs --all` once, then a second host commit | steady state, loose ref replaced | 20/20 | 20/20 | 0/20 |
| (e) host `git pack-refs --all` after each commit | | 20/20 | 20/20 | 0/20 |
| (e) `git init --ref-format=reftable` | guest git 2.47.3 opens it | 20/20 | 20/20 | 0/20 |
| (f) host `git status` loop with `GIT_OPTIONAL_LOCKS=0` during the trial | host commits succeed (0 setup failures) | 20/20 | 20/20 | 0/20 |
| Control: guest `sleep 1.1` first | | 0/20 | 0/20 | 20/20 |
| Control: 1.2 s between prime and host commit | | 0/20 | 0/20 | 20/20 |

Notes on the rows:

- (b) with host `pack-refs`: the loose `refs/heads/main` was deleted on the
  host. The listing does not heal a removed name (as in the prior probe), and
  the guest made a parentless commit with the full tree `a b gfile hfile`.
- (c) printed `gitwrap: host change seen, waiting` in all 20 trials. In case 4
  the same wrapper did not change the outcome (89/90 anomalous iterations),
  because a host index refresh does not touch `.git/logs/HEAD`. These trials
  are sequential: none puts a host change between the wrapper's check and
  git's read.
- (e) reftable: afterwards on the host, `git symbolic-ref HEAD` fails with
  `ref HEAD is not a symbolic ref`, `git show-ref` prints nothing, and
  `git reflog --all` holds only the guest's initial commit. `fsck` reports
  the host commit as a dangling commit. `tables.list` names only the
  guest's new table. The previous table file is still in `.git/reftable`
  but unlisted (`logs/c8-reftable-inspect.log`).
- (f) `GIT_OPTIONAL_LOCKS=0` on the host loop: against case 1 it changed
  nothing, since the host commit itself replaces the ref and index. In the
  forced case 4 loop it removed every anomaly (0/90) and the loop never
  replaced the index. It was not run with the plain poll loop.
- (b) and (c) passed only in these sequential case 1 trials.

## Not tested

- Linux or podman shares.
- A guest wrapper whose check races a host change during the guest command;
  all case 8 trials are sequential.
- (b) against host deletions other than `pack-refs`: `git branch -d`,
  `git gc`, removed working-tree files.
- (b) under the host index-refresh loops of case 4.
- (e) with `pack-refs` once followed by a single host commit (the first
  commit after packing creates a new loose ref rather than replacing one).
- Refs in nested directories (`refs/heads/feature/x`), which `ls` of
  `.git/refs/heads` does not list.
- Case 1 in a linked worktree on the same branch, and any case 7 run in a
  `/work` box (it cannot open the worktree).
- Real editors (VS Code, JetBrains, vim) and their own git integrations.
  The host loop stands in for an editor's background `git status`; whether a
  given editor sets `GIT_OPTIONAL_LOCKS=0` was not checked.
- Agents' own file tools (read, then write) in the guest; case 6 used shell
  and `sed`.
- Merges, cherry-picks, `git am`, `git gc --auto`, hooks, submodules, LFS.
- Guest git newer than 2.47.3, which may accept `relativeWorktrees`.
- Case 5 between 0 and 0.9 s other than the observe timeline.
- Long runs (hours) and several boxes on one share at once.
- Why a failed guest `> f` (`Directory nonexistent`) left the host file
  truncated to 0 bytes.
