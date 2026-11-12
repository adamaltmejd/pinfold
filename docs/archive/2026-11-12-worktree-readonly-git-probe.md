# Linked worktrees with a read-only `.git` in an Apple share

Date: 2026-11-12. Probe 3 of the project box plan. The plan is
`docs/archive/2026-11-12-project-box-plan.md` on PR #288, not yet on main.
This probe extends `2026-11-12-readonly-git-mount-probe.md` to linked Git
worktrees. Diagnostic only; nothing in pinfold was built or changed.

Scripts and raw logs are in `2026-11-12-worktree-readonly-git-probe/`.
Each case has a script (`c1.sh` ... `c6.sh`, `c2.py`) and a log of the
same name under `logs/`. `logs/env.log` and `logs/c4-owner.log` were
written by hand, not by a script. Every number below is a line in those
logs unless the text says it is not logged.

## Environment

From `logs/env.log`: macOS 27.2.1, Apple `container` 1.7.1, image
`localhost/pinfold/profile-default:latest` (ae6f1273f183, already in the
store), Git 2.56.1 on the host and 2.47.3 in the box.

Not logged: the command that started the container service, and the
host's uid:gid. `env.log` records only that the service was not running
before and after the probe. The boxes ran as `--user 501:20`, which is in
every logged `container run` line. That 501:20 is the host's own uid:gid
is not logged; uid 501 appears as a file owner in `logs/c4-owner.log`
and gid 20 appears nowhere else.

`T` is a temporary directory under `/private/tmp`. `R=$T/R` is a main
checkout with one commit, `.gitignore` holding `wt/`, and a linked
worktree `R/wt/x` on branch `x` made by `git worktree add`. Its `.git`
file reads `gitdir: R/.git/worktrees/x`. Every share is mounted at its
host path:

```
container run -d --name NAME --user 501:20 -v R:R -v R/.git:R/.git:ro \
  [EXTRA] --entrypoint sleep IMAGE infinity
```

How the cases were run:

- There is no driver script. Each case script was run by hand with `T`
  set, and all six cases used the same `T`.
- The reset between cases was done by hand: `R`, `$T/side` and
  `$T/marker.log` were removed. No script does this (`fresh()` in
  `lib.sh` only runs `mkdir -p` and `git init`) and no log shows it.
- Inference from the logs that the reset happened: each case log has a
  different first commit hash, and `c5b`'s marker log holds only its own
  two lines. Case 4's "marker.log absent at start" depends on this
  manual step.
- The helpers `g` and `h` print one exit code per call. For commands
  joined by `;` that code is the last command's only. Where the text
  below says a chain printed correct output, the code is not claimed
  for the chain.

Host Git runs with `GIT_CONFIG_GLOBAL=/dev/null` and
`GIT_CONFIG_NOSYSTEM=1`, so the operator's own config and hooks stay
out (`lib.sh` line 7). The box sets `GIT_CONFIG_GLOBAL=/dev/null` but
not `GIT_CONFIG_NOSYSTEM`, so the box's system config was not excluded.
The box also sets `safe.directory=*` through `GIT_CONFIG_*`, as in the
earlier probes.

## Case 1: Git in the worktree (`c1.sh`)

Observed:

- Mounts: `R` is `rw`, `R/.git` is `ro`.
- `git rev-parse --show-toplevel --git-dir --git-common-dir` prints
  `R/wt/x`, `R/.git/worktrees/x`, `R/.git`.
- `git status`, `git log --oneline` and `git worktree list`, run as one
  chain, printed correct output. The logged rc 0 is the last command's.
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
- On the host afterwards: one commit and ` M a`. `git fsck` printed
  nothing, and no new file was in `R/.git/worktrees/x`. The logged rc
  for that chain is the last command's (`ls`), not `git fsck`'s.

## Case 2: host commits in the worktree while the box reads (`c2.sh`, `c2.py`)

Twenty trials. In each, the box appended a line to `a`. The host checked
it saw the line and ran `git commit -qam`. The box then ran
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

One run, one read per step. The "1.5 s" reads are a `sleep 1.5` after
the box's previous read finished. They have no timestamp. The time since
the host's action is that 1.5 s plus the duration of the previous read.

Observed:

- After the host's `git worktree add -q wt/y -b y`, the box's first read
  already showed `wt/y` in `git worktree list` and in `ls wt`.
  `git -C wt/y status` and `log` were correct. The read after the
  `sleep 1.5` was the same.
- The box wrote an untracked file in `wt/y`. The host's
  `git worktree remove wt/y` refused: "contains modified or untracked
  files, use --force to delete it", exit 128. `--force` removed it.
- The box's first read after the removal showed `y` gone from
  `git worktree list` and `ls wt`. But `git -C wt/y` still entered the
  directory and failed with "error opening 'R/wt/y/.git': No such file
  or directory", exit 128. After the `sleep 1.5` it failed with "cannot
  change to 'wt/y': No such file or directory".
- On the host: branch `y` remains, `git fsck` printed nothing.

Not observed: what happened to a box process whose cwd was in `wt/y`.
The script started one, but `pgrep` is not in the image, so the check
printed nothing useful.

## Case 4: the box repoints the worktree's `.git` file (`c4.sh`)

The `.git` file of `R/wt/x` is an ordinary file in the writable share.
Every script the box planted only appends one line naming itself and
`uname -s` to `$T/marker.log`. `$T` is not shared, so the box's own runs
could not write it ("Permission denied"). Every marker line is from the
host.

Variant A: the box made `R/wt/evilA` with `HEAD`, `index`, `gitdir`,
`commondir` pointing at `R/.git`, and its own `config` setting
`core.fsmonitor` to its script. It rewrote `R/wt/x/.git` to point there.

Observed: host Git in `wt/x` used `R/wt/evilA` as git dir and `R/.git`
as common dir. `git config core.fsmonitor` found nothing (exit 1). No
marker was written. Variant A planted no hook and ran no commit, so
hooks were not tested in it.

Inference, not observed: Git read config from the common dir `R/.git`,
which the box cannot write, and ignored `evilA/config`. Git can read a
per-worktree config from the worktree's own git dir when
`extensions.worktreeConfig` is on. That would give the box a config path
in this layout. It was not tried.

Variant B: the box copied `R/.git` to `R/wt/evilB`, reading the
read-only mount. It set `HEAD` to `refs/heads/x` and copied the
worktree's index. It set `core.fsmonitor` and `core.hooksPath` in
`evilB/config` to its own scripts (`fsm.sh`, `h/pre-commit`,
`h/post-commit`). It rewrote `R/wt/x/.git` to `gitdir: R/wt/evilB`.

Observed:

- The box's Git now worked read-write: `git commit -qam box-in-evil`
  exited 0 and made commit 2882f87 in `evilB`.
- On the host, `git status --short` in `wt/x` used `evilB` as git dir
  and common dir. It ran the box's `fsm.sh` twice ("fsmonitor-B ran on
  Darwin", args `2 <token>` and `1 <token>`).
- The host's `git commit -qam host-in-evil` exited 0. It ran the box's
  scripts on Darwin in this order: `fsm.sh` twice, `pre-commit`,
  `fsm.sh` twice, `post-commit`. With the two lines from `git status`,
  the marker log ends with 8 lines, all "on Darwin".
- The host commit bb30bc0 appeared in `git -C wt/x log --oneline -1`,
  while `git log --oneline --all` in `R` showed only c0. Inference: the
  commit went into `evilB`, not into `R/.git`.
- The files the box made show owner 501 on the host
  (`logs/c4-owner.log`), and host Git raised no ownership error. The
  evidence is weak. The listing was run by hand after `c4.sh`, and it is
  cut to four entries (`head -5`). `COMMIT_EDITMSG` was last written by
  the host, so its owner says nothing about the box. The group is 0, not
  20.
- `git worktree list` in `R` did not notice the repointing: it showed
  `wt/x` at c0 on `[x]`. `git worktree prune --dry-run -v` printed
  nothing.
- `git worktree repair` in `R` printed "repair: .git file incorrect:
  R/wt/x", exited 0, and rewrote `R/wt/x/.git` to point back at
  `R/.git/worktrees/x`.
- A second `git worktree repair`, run from inside `wt/x`, was silent. It
  ran after the first had already fixed the file, so it tested nothing.
  Repair run from inside a repointed worktree was not tried.

## Case 5: the worktree's `.git` file mounted read-only on its own (`c5.sh`, `c5b.sh`)

Extra mount: `-v R/wt/x/.git:R/wt/x/.git:ro`.

Observed in `c5.sh`:

- `container` accepted the single-file mount. The box ran, and mountinfo
  listed `R/wt/x/.git ro,relatime virtiofs rw`.
- The reads of case 1 worked: toplevel `R/wt/x`, git dir
  `R/.git/worktrees/x`, status, log and diff correct.
- Writing or appending to `.git` failed: "Read-only file system",
  exit 2. `mv .git .git.old`, `mv a .git` and `mv -T` of a symlink onto
  `.git` failed: "Device or resource busy", exit 1. `.git` was intact.
- `mv wt/x wt/x2` exited 0. `mv wt wt2` then exited 0. `mv R R2` failed:
  "Permission denied", exit 1.
- The script's last box step (make `wt/z`) failed because `wt` had been
  renamed. It shows nothing.
- The script's final host check also failed: `cd` to `R/wt/x` gave "No
  such file or directory" (`logs/c5.log` lines 55-57), for the same
  reason.

Observed in `c5b.sh`, which follows the rename:

- After `mv wt/x wt/x2` the box's mountinfo listed the mount at
  `R/wt/x2/.git`. In the box, `cat wt/x2/.git` gave "Permission denied"
  and a write gave "Read-only file system".
- On the host, `wt` held only `x2`, its `.git` file was unchanged, and
  `git worktree list` showed `R/wt/x` as "prunable".
- The box then made a gitdir `R/wt/evil` as in case 4 variant B (with
  `core.fsmonitor` only). It made a new directory `R/wt/x` with copies
  of `a` and `.gitignore`, and a `.git` file pointing at `R/wt/evil`.
- On the host, `git status --short` in `R/wt/x` used `R/wt/evil` and ran
  the box's `fsm.sh` twice on Darwin. `git worktree list` showed `wt/x`
  as a normal worktree again. `git worktree repair` printed ".git file
  incorrect" and rewrote the file.

## Case 6: a worktree outside the main root (`c6.sh`)

`$T/side` is a linked worktree of `R` on branch `side`. Mounts:
`-v T/side:T/side -v R/.git:R/.git:ro`. `R`'s working tree is not
shared.

Observed:

- Mountinfo listed `T/side` `rw` and `R/.git` `ro`.
- `git rev-parse` printed `T/side`, `R/.git/worktrees/side`, `R/.git`.
  `git status`, `git log` and `git diff` printed correct output. The
  logged rc 0 is the last command's in each chain.
- `git worktree list` in the box marked `R/wt/x` "prunable", because
  the box cannot see `R/wt/x`.
- `git add a` and `git commit` failed: "Unable to create
  'R/.git/worktrees/side/index.lock': Read-only file system", exit 128.

Not tried here: rewriting `T/side/.git`. It is in the writable share,
as in case 4, so the same result is expected. That is untested.

## Not tried

- Linux, and a box with privilege (remount, umount, mount over).
- Host Git with the operator's real global config, and the box's system
  config.
- Other host-executed config keys (`core.pager`, `core.sshCommand`,
  filters, aliases), and `extensions.worktreeConfig`.
- Hooks under variant A of case 4.
- `git worktree repair` run from inside a repointed worktree.
- Rewriting the `.git` file of the sibling worktree in case 6.
- A nested repository the box creates elsewhere in a main-checkout-only
  share (a new `R/sub/.git` directory), and submodules.
- Timing for case 3 beyond one read at once and one after a 1.5 s sleep.
- What happens to a box process whose cwd is removed by the host.

The operator's tool harness refused no command during the run. This is
not logged.

## What this means for Q12

Q12 is the open question on worktrees in the project box plan. It has
two options. Option (a): refuse to share a linked worktree, and share
only a main checkout. Option (b): share the main root and let the box
work in worktrees beneath it, read-only for Git like the rest.

Reads through a worktree work in both layouts. Git writes into the
read-only `R/.git` fail, as in the earlier probe.

The stale window after a host commit was measured in both probes, by
different methods:

- This probe: a linked worktree, 20 trials, a read at once and a read at
  1.5 s in every trial. 19 of 20 reads at once were wrong, and 0 of 20
  at 1.5 s.
- The earlier probe: a plain checkout, 20 trials, a read at once in
  each. 17 of 20 were wrong. It made one delayed read, at 1.2 s after
  the last trial, and that read was correct.

The methods differ, so the numbers do not show that the window is the
same. Both show a wrong read right after a host commit and a correct one
after about a second.

A worktree's `.git` file sits in the writable share. In these runs the
box repointed it at a gitdir it wrote, then committed there itself. Host
Git in that worktree then ran the box's fsmonitor and hooks on macOS.

A read-only mount of the `.git` file alone stopped a direct rewrite. It
did not stop the box renaming the worktree's directory and making a new
one at the old path.

`git worktree list` in `R` did not notice the repointing in case 4. In
case 5b it marked `R/wt/x` "prunable" right after the rename. It showed
the worktree as normal again once the box made the new `wt/x`.
`git worktree repair` noticed both changes.

So under option (b) a read-only `R/.git` does not stop the box from
choosing what host Git runs in a worktree beneath the root. Case 6 (a
sibling worktree) never had its `.git` file rewritten. It is expected to
behave like case 4, because the file is equally writable, but that is
untested.

A main checkout's `.git` is a mounted directory at the share's top, and
the box could not rename `R` itself. Whether a main-checkout-only share
has a similar path, through a nested repository the box creates, was not
tried.
