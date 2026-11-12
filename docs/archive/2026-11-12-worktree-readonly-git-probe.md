# Linked worktrees with a read-only `.git` in an Apple share

Date: 2026-11-12. Probe 3 of the project box plan. It extends
`2026-11-12-readonly-git-mount-probe.md` to linked Git worktrees.
Diagnostic only; nothing in pinfold was built or changed.

Scripts and raw logs are in `2026-11-12-worktree-readonly-git-probe/`.
Each case has a script (`c1.sh` ... `c6.sh`, `c2.py`) and a log of the
same name under `logs/`. Every number below is a line in those logs.

## Environment

From `logs/env.log`: macOS 27.2.1, Apple `container` 1.7.1, image
`localhost/pinfold/profile-default:latest` (ae6f1273f183, already in the
store), Git 2.56.1 on the host and 2.47.3 in the box, host uid:gid
501:20. The container service was stopped at the start; it was started
with `container system start --disable-kernel-install` and stopped at
the end.

`T` is a temporary directory under `/private/tmp`. `R=$T/R` is a main
checkout with one commit, `.gitignore` holding `wt/`, and a linked
worktree `R/wt/x` on branch `x` made by `git worktree add`. Its `.git`
file reads `gitdir: R/.git/worktrees/x`. Each case starts from a fresh
`R`. Every share is mounted at its host path:

```
container run -d --name NAME --user 501:20 -v R:R -v R/.git:R/.git:ro \
  [EXTRA] --entrypoint sleep IMAGE infinity
```

Host and box Git run with `GIT_CONFIG_GLOBAL=/dev/null`, so the
operator's own config and hooks stay out. The box sets
`safe.directory=*` through `GIT_CONFIG_*`, as in the earlier probes.

## Case 1: Git in the worktree (`c1.sh`)

Observed:

- Mounts: `R` is `rw`, `R/.git` is `ro`.
- `git rev-parse --show-toplevel --git-dir --git-common-dir` prints
  `R/wt/x`, `R/.git/worktrees/x`, `R/.git`.
- `git status`, `git log --oneline`, `git worktree list` exit 0 and are
  correct.
- The box appended to `a` and touched `.gitignore`, so the index was
  stale. `git diff` showed the change. `git status --short` printed
  only ` M a` and exited 0, with no warning, both with and without
  `GIT_OPTIONAL_LOCKS=0`. No `index.lock` was left in
  `R/.git/worktrees/x`.
- `git update-index --refresh`, `git add a`, `git commit -qam box` and
  `git checkout -q -b other` fail: "Unable to create
  'R/.git/worktrees/x/index.lock': Read-only file system", exit 128.
- `git stash` fails: "error: could not write index", exit 1.
- `touch R/.git/worktrees/x/y` and a write to its `HEAD` fail with
  "Read-only file system" (exit 1 and 2).
- On the host afterwards: one commit, ` M a`, `git fsck` silent, no new
  file in `R/.git/worktrees/x`.

## Case 2: host commits in the worktree while the box reads (`c2.sh`, `c2.py`)

Twenty trials. In each, the box appended a line to `a`; the host checked
it saw the line and ran `git commit -qam`; the box then ran
`git log -1 --format=%H` and `git status --porcelain` at once, and again
1.5 s after the host commit. A read is right when HEAD is the host's new
commit and status is empty.

Observed (`logs/c2.log`, `logs/c2.jsonl`):

- The host saw the box's edit in 20 of 20 and committed in 20 of 20.
- The read at once was wrong in 19 of 20. All 19 said "your current
  branch 'x' does not have any commits yet" and listed `.gitignore` and
  `a` as untracked. Those reads finished within 0.1 s of the host
  commit.
- The read at 1.5 s was wrong in 0 of 20.
- The host ended with 21 commits on `x`, a clean status and a silent
  `git fsck`.

## Case 3: host adds and removes a worktree while the box runs (`c3.sh`)

One run, one read per step.

Observed:

- After the host's `git worktree add -q wt/y -b y`, the box's first read
  already showed `wt/y` in `git worktree list` and in `ls wt`, and
  `git -C wt/y status` and `log` were correct. Same at 1.5 s.
- The box wrote an untracked file in `wt/y`. The host's
  `git worktree remove wt/y` refused: "contains modified or untracked
  files, use --force to delete it", exit 128. `--force` removed it.
- The box's read at once after the removal: `git worktree list` and
  `ls wt` no longer showed `y`, but `git -C wt/y` still entered the
  directory and failed with "error opening 'R/wt/y/.git': No such file
  or directory", exit 128. At 1.5 s it failed with "cannot change to
  'wt/y': No such file or directory".
- On the host: branch `y` remains, `git fsck` silent.

Not observed: what happened to a box process whose cwd was in `wt/y`.
The script started one, but `pgrep` is not in the image, so the check
printed nothing useful.

## Case 4: the box repoints the worktree's `.git` file (`c4.sh`)

The `.git` file of `R/wt/x` is an ordinary file in the writable share.
Every script the box planted only appends one line naming itself and
`uname -s` to `$T/marker.log`. `$T` is not shared, so the box's own runs
could not write it ("Permission denied"); every marker line is from the
host.

Variant A: the box made `R/wt/evilA` with `HEAD`, `index`, `gitdir`,
`commondir` pointing at `R/.git`, and its own `config` setting
`core.fsmonitor` to its script. It rewrote `R/wt/x/.git` to point there.
Observed: host Git in `wt/x` used `R/wt/evilA` as git dir and `R/.git`
as common dir; `git config core.fsmonitor` found nothing (exit 1); no
marker was written. Git reads config and hooks from the common dir, which
the box cannot write.

Variant B: the box copied `R/.git` to `R/wt/evilB` (reading the
read-only mount), set `HEAD` to `refs/heads/x`, copied the worktree's
index, and set `core.fsmonitor` and `core.hooksPath` in
`evilB/config` to its own scripts (`fsm.sh`, `h/pre-commit`,
`h/post-commit`). It rewrote `R/wt/x/.git` to `gitdir: R/wt/evilB`.

Observed:

- The box's Git now worked read-write: `git commit -qam box-in-evil`
  exited 0 and made commit 2882f87 in `evilB`.
- On the host, `git status --short` in `wt/x` used `evilB` as git dir
  and common dir and ran the box's `fsm.sh` twice ("fsmonitor-B ran on
  Darwin", args `2 <token>` and `1 <token>`).
- The host's `git commit -qam host-in-evil` exited 0 and ran the box's
  scripts on Darwin, in this order: `fsm.sh` twice, `pre-commit`,
  `fsm.sh` twice, `post-commit`. The marker log ends with 8 lines, all "on Darwin".
- The host commit bb30bc0 went into `evilB`. `git log --oneline --all`
  in `R` still showed only c0.
- The files the box made show owner 501 on the host
  (`logs/c4-owner.log`); host Git raised no ownership error.
- `git worktree list` in `R` did not notice: it showed `wt/x` at c0 on
  `[x]`. `git worktree prune --dry-run -v` printed nothing.
- `git worktree repair` in `R` printed "repair: .git file incorrect:
  R/wt/x", exited 0, and rewrote `R/wt/x/.git` to point back at
  `R/.git/worktrees/x`. Run again from inside `wt/x`, it was silent.

## Case 5: the worktree's `.git` file mounted read-only on its own (`c5.sh`, `c5b.sh`)

Extra mount: `-v R/wt/x/.git:R/wt/x/.git:ro`.

Observed in `c5.sh`:

- `container` accepted the single-file mount; the box ran and
  mountinfo listed `R/wt/x/.git ro,relatime virtiofs rw`.
- The reads of case 1 worked: toplevel `R/wt/x`, git dir
  `R/.git/worktrees/x`, status, log and diff correct.
- Writing or appending to `.git` failed: "Read-only file system",
  exit 2. `mv .git .git.old`, `mv a .git` and `mv -T` of a symlink onto
  `.git` failed: "Device or resource busy", exit 1. `.git` was intact.
- `mv wt/x wt/x2` exited 0. `mv wt wt2` then exited 0. `mv R R2` failed:
  "Permission denied", exit 1.
- The script's last box step (make `wt/z`) failed because `wt` had been
  renamed; it shows nothing.

Observed in `c5b.sh`, which follows the rename:

- After `mv wt/x wt/x2` the box's mountinfo listed the mount at
  `R/wt/x2/.git`. In the box, `cat wt/x2/.git` gave "Permission denied"
  and a write gave "Read-only file system".
- On the host, `wt` held only `x2`, its `.git` file was unchanged, and
  `git worktree list` showed `R/wt/x` as "prunable".
- The box then made a gitdir `R/wt/evil` as in case 4 variant B (with
  `core.fsmonitor` only), a new directory `R/wt/x` with copies of `a`
  and `.gitignore`, and a `.git` file pointing at `R/wt/evil`.
- On the host, `git status --short` in `R/wt/x` used `R/wt/evil` and ran
  the box's `fsm.sh` twice on Darwin. `git worktree list` showed `wt/x`
  as a normal worktree again. `git worktree repair` printed ".git file
  incorrect" and rewrote the file.

## Case 6: a worktree outside the main root (`c6.sh`)

`$T/side` is a linked worktree of `R` on branch `side`. Mounts:
`-v T/side:T/side -v R/.git:R/.git:ro`; `R`'s working tree is not
shared.

Observed:

- Mountinfo listed `T/side` `rw` and `R/.git` `ro`.
- `git rev-parse` printed `T/side`, `R/.git/worktrees/side`, `R/.git`.
  `git status`, `git log` and `git diff` exited 0 and were correct.
- `git worktree list` in the box marked `R/wt/x` "prunable", because
  the box cannot see `R/wt/x`.
- `git add a` and `git commit` failed: "Unable to create
  'R/.git/worktrees/side/index.lock': Read-only file system", exit 128.

Not tried here: rewriting `T/side/.git`. It is in the writable share,
as in case 4.

## Not tried

- Linux, and a box with privilege (remount, umount, mount over).
- Host Git with the operator's real global config.
- Other host-executed config keys (`core.pager`, `core.sshCommand`,
  filters, aliases), and `extensions.worktreeConfig`.
- A nested repository the box creates elsewhere in a main-checkout-only
  share (a new `R/sub/.git` directory), and submodules.
- Timing for case 3 beyond one read at once and one at 1.5 s.
- What happens to a box process whose cwd is removed by the host.

No command was blocked by the safety check.

## What this means for Q12

Reads through a worktree work in both layouts, and Git writes into the
read-only `R/.git` fail, as in the earlier probe. The stale window after
a host commit is the same as before: 19 of 20 wrong at once, 0 of 20 at
1.5 s. But a worktree's `.git` file sits in the writable share. In these
runs the box repointed it at a gitdir it wrote, then committed there
itself, and host Git in that worktree ran the box's fsmonitor and hooks
on macOS. A read-only mount of the `.git` file alone stopped a direct
rewrite but not a rename of its directory followed by a new one at the
old path. `git worktree list` did not notice either change;
`git worktree repair` did. So under option (b), and for any linked
worktree shared on its own (case 6 has the same writable `.git` file),
a read-only `R/.git` does not stop the box from choosing what host Git
runs in a worktree. A main checkout's `.git` is a mounted directory at
the share's top, and the box could not rename `R` itself; whether a
main-checkout-only share has a similar path (a nested repository the
box creates) was not tried.
