# Apple writable share: shared .git with real host tools

Date: 2026-11-19. Follows `2026-11-12-apple-share-concurrency-probe.md`.
Diagnostic only; no guarantee is published here. Scripts and raw logs:
`2026-11-19-shared-git-probe/`.

The question: with a host directory and its `.git` shared writable into an
Apple `container` box, how often do the 2026-11-12 failures (cases 1 and 4)
occur with the host tools used on this Mac.

## Environment

`logs/env.log`, from `env.sh`.

- macOS 27.2.1, build 26B131, arm64. Host uid:gid 501:20.
- Apple `container` 1.7.1, apiserver 1.7.1. The container service was
  already running and was left running.
- Guest kernel 6.18.15 aarch64. Share `virtiofs /work virtiofs rw,relatime`.
- Host git 2.56.1. Guest git 2.47.3. Python 3.15.2. starship 1.27.0.
  Zed 1.29.2.
- Image `localhost/pinfold/profile-default:latest`, now `9b5df3f99e34`
  (2026-11-12: `ae6f1273f183`).
- Box: `container run -d --name pfs-probe1119 --user 501:20 -v /tmp/pfs.9Ffk:/work --entrypoint sleep IMAGE infinity`.
  Every guest step is one `container exec … sh -c`.
- `lib.py` is the 2026-11-12 file. `c1.py` adds three variants and
  `c4.py` adds observer modes; both are described in their docstrings.
  Fresh temp repositories as before: `git init -b main`, commits `c0`, `c1`
  touching `a` and `b`.
- The committing side and the 2026-11-12 `hostpoll` loop run git with
  `GIT_CONFIG_GLOBAL=/dev/null`. The new host observers run with the
  user's environment and global git config, which sets `core.whitespace`,
  `core.autocrlf input`, `status.showuntrackedfiles all` and a pager, and
  no fsmonitor, untracked cache or index setting. starship reads the user's
  config (`STARSHIP_CONFIG`), which enables `git_branch` and `git_status`.

`run.sh` holds the runs in order. `tally.py` prints the tables below.

## Host-only control: which observers replace the index

`ctl.sh`, no box. A fresh repository per command, 20 runs of
`touch a; CMD`, counting runs after which `.git/index` had a new inode
(`logs/ctl.log`).

| Command | Index replaced |
|---|---|
| `git status --porcelain` | 20/20 |
| `starship prompt` | 0/20 |
| `git --no-optional-locks status --porcelain` | 0/20 |
| `GIT_OPTIONAL_LOCKS=0 git status --porcelain` | 0/20 |
| `git log -1` | 0/20 |
| `git diff --stat` | 2/20 |

With `STARSHIP_LOG=trace`, one `starship prompt` in a repository with a
modified file printed `main` and `!`. The trace lists one git process:
`GIT_OPTIONAL_LOCKS="0" git … -c core.fsmonitor= … for-each-ref … refs/heads/main`.
No `git status` process appears in the trace.

## Case 1 baseline and agent-shaped variants

Guest primes (`git status; git log -1`), host commits `hfile` and a line in
`a`, and after the delay the guest commits `gfile`. Columns as in
2026-11-12. "Commit exec at" is the time from the end of the host commit to
the launch of the guest's commit exec.

| Variant | Delay (s) | Trials | History loss | Content loss | Guest commit exit 0 | Commit exec at (s) |
|---|---|---|---|---|---|---|
| base | 0 | 10 | 10 | 10 | 10 | 0.00 |
| base | 0.5 | 10 | 10 | 10 | 10 | 0.50 |
| base | 1.0 | 10 | 0 | 0 | 10 | 1.00 |
| base | 2.0 | 10 | 0 | 0 | 10 | 2.00 |
| status: `git status --porcelain` first, same exec | 0 | 10 | 10 | 10 | 10 | 0.00 |
| status | 0.5 | 10 | 10 | 10 | 10 | 0.50 |
| status | 1.0 | 10 | 0 | 0 | 10 | 1.00 |
| status | 2.0 | 10 | 0 | 0 | 10 | 2.00 |
| agent: a `git status --porcelain` exec, then a commit exec | 0 | 10 | 10 | 10 | 10 | 0.05 to 0.08 |
| agent | 0.5 | 10 | 10 | 10 | 10 | 0.58 to 0.61 |
| agent | 1.0 | 10 | 0 | 0 | 10 | 1.08 to 1.12 |
| agent | 2.0 | 10 | 0 | 0 | 10 | 2.07 to 2.11 |
| waitvis: execs of `git log -1 --format=%s` until it prints `host`, then a commit exec | 0 | 20 | 0 | 0 | 20 | 0.96 to 1.04 |
| waitvis1: the same wait (`sleep 0.05` between tries) inside the commit exec | 0 | 20 | 0 | 0 | 20 | 0.00 |

Loss trials look as on 2026-11-12: `main` is one parentless `guest` commit
holding only `gfile`. In the `agent` variant the guest's status exec at
delays 0 and 0.5 s printed `?? a`, `?? b`, `?? hfile`.

`waitvis` needed 13 to 17 `git log -1` execs before `host` appeared.
`waitvis1` looped 15 or 16 times. No trial hit the cap (100 execs,
200 tries). All 40 wait trials were clean: four commits in a line, all files,
clean host status, `fsck` exit 0.

## Case 4 with host observers, guest commits

`c4.py MODE 3 30`. The observer loops on the host in the repository. The guest
runs 30 iterations of
`echo N >> mine; git status --porcelain; git add mine && git commit` in
each of three repositories. Anomalous: a failed commit, or a status line
other than `M mine` or `?? mine`. Damage carries forward within a
repository, so "first anomalous" gives the iteration of the first damage
per repository. In `status1` and `status5` the guest sleeps 0.7 s between
iterations, so the observer runs during the trial; other modes are
unpaced.

"Index replaced" is the observer's own count: runs after which
`.git/index` had a new inode, out of runs. It includes guest index writes
that landed during the observer's command.

| Mode | Observer | Anomalous | Refusals | First anomalous (per repo) | Commits landed | Final tree on `main` | Index replaced (per repo) |
|---|---|---|---|---|---|---|---|
| none | no observer | 0/90 | 0 | none | 90/90 | `a b mine` (3/3) | n/a |
| hostpoll | `git status --porcelain`, 0.1 s, no global config | 85/90 | 1 | 1, 1, 3 | 89/90 | `mine` (3/3) | 18/18, 20/20, 18/18 |
| starship | `starship prompt`, 0.1 s | 0/90 | 0 | none | 90/90 | `a b mine` (3/3) | 7/17, 8/16, 5/17 |
| nolocks | `git --no-optional-locks status --porcelain`, 0.1 s | 0/90 | 0 | none | 90/90 | `a b mine` (3/3) | 4/18, 3/19, 2/20 |
| ol0 | `GIT_OPTIONAL_LOCKS=0 git status --porcelain`, 0.1 s | 0/90 | 0 | none | 90/90 | `a b mine` (3/3) | 4/18, 4/19, 5/19 |
| status1 | `git status --porcelain`, 1 s | 84/90 | 0 | 2, 2, 2 | 90/90 | `mine` (3/3) | 24/24, 24/24, 24/24 |
| status5 | `git status --porcelain`, 5 s | 63/90 | 0 | 7, 7, 13 | 90/90 | `mine` (3/3) | 5/5, 5/5, 5/5 |
| log | `git log -1`, 0.1 s | 0/90 | 0 | none | 90/90 | `a b mine` (3/3) | 3/19, 4/17, 4/18 |
| diffstat | `git diff --stat`, 0.1 s | 86/90 | 0 | 1, 2, 1 | 90/90 | `mine` (3/3) | 16/18, 16/17, 16/18 |

In the damaging modes the first anomalous iteration showed `D  a`, `D  b`,
`D  mine`, `?? a`, `?? b` (in one `diffstat` repository only `?? a ?? b`).
The guest commit in that iteration exited 0 and recorded `a` and `b` as
deleted. Every later iteration showed `?? a ?? b`, some with `D  mine`.
History stayed linear with one root; all 30 lines of `mine` were on
`main`; host status afterwards was `?? a ?? b`; `fsck` exited 0. The one
`hostpoll` refusal was an `index.lock: File exists` on the guest.

In `status5` the observer ran 5 times per repository and replaced the
index each time; the first damage came at iteration 7, 7 and 13.

## Reverse direction: guest observer, host commits

The same shape with the roles swapped. `guestloop` is the 2026-11-12 forced
loop (`touch a; git status`); `guestpoll` is plain `git status --porcelain`
every 0.1 s.

| Mode | Anomalous | Refusals | Commits landed | Final tree on `main` | Lines of `mine` |
|---|---|---|---|---|---|
| guestloop | 5/90 | 3 | 87/90 | `a b mine` (3/3) | 30, 30, 30 |
| guestpoll | 6/90 | 5 | 85/90 | `a b mine` (3/3) | 30, 30, 30 |

Refusals were host `index.lock: File exists`. The other anomalies were
`MM mine` in the iteration after a refusal (3 in total). The refused
change was committed in the next iteration. No file or line was lost.

## Zed

Zed 1.29.2 was already running, so it was not opened on a temp repository
and no live Zed mode was run. Not measured; what was read:

- `strings` on `/Applications/Zed.app/Contents/MacOS/zed` (1.29.2) shows,
  adjacent, `-c`, `core.fsmonitor=false`, `log.showSignature=false`,
  `--no-optional-locks`, `--no-pager`, `core.hooksPath=/dev/null`.
- On Zed's `main` branch, `crates/git/src/repository.rs`
  `RealGitRepository::status` runs the git binary through
  `GitBinary::build_command(&git_status_args(…))`. The body of
  `build_command`, which adds `--no-optional-locks`, came from a page
  summary, not a verbatim read. The 1.29.2 tag was not read.

## Not tested

- A live Zed session on a shared repository.
- Other editors (VS Code, JetBrains) and their git integrations; GUI git
  clients; `git fetch` or `gc --auto` from host tools.
- Observers combined (prompt plus editor plus agent) on one repository.
- Case 1 under any host observer; case 4 at other guest paces.
- The waiting variants at delays other than 0, with a host change landing
  during the wait, or with host operations other than commit (case 5).
- Several boxes on one share, long runs, Linux or podman shares.
- Guest git newer than 2.47.3.
