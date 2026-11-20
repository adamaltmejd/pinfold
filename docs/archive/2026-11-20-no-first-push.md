# A new project disk starts empty

2026-11-20. Owner decision: drop the automatic first-start synchronization
of disk projects (`2026-11-19-git-remote.md`, "First start"). That start
created the box repository, pushed every host branch in as `host/*`,
checked out the host's branch and added the host remote `box`. It moved
all branches into a box and edited the host's Git configuration without
being asked. No test was added: the suite is suspended.

## Built

- A start runs no host Git beyond admission's verification, pushes nothing
  and writes no host Git configuration. A new project disk starts empty,
  Git project or not.
- Code enters a disk by a clone or `git init` in the box, or by
  `git push box BRANCH` after `git remote add box pinfold::ROOT`.
- The first push into a box path without `.git` creates the repository
  (`git init` with the host's object format, the first pushed branch as
  its unborn branch). After the fetch into `refs/remotes/host/*`, it
  fast-forwards that branch with `git merge --ff-only`. The old first
  start did the same. A failed checkout keeps the refs, and the push
  reports success. The checked-out branch is the first refspec Git sends
  (with `--all`, Git's order decides), not the host's HEAD. A creating
  push that fails after `git init` leaves an unborn branch, and a later
  push lands refs without a checkout.
- Fetch from a box path without a repository refuses as `spec`, as
  before. The refusal now ends with a note that a push creates the
  repository.
- The helper's bare-URL refusal now names `git remote add box URL`.
- Wording. The first-run `disk` line is the same in Git and non-Git
  folders. The Linux statement says a new disk starts empty. The bare
  `pinfold` intro for a Git disk project prints the remote-add, push and
  fetch lines. The `--help` "Disk projects" block names the remote-add
  step. The README, the spec and the delegate skill are updated to match.

Deleted:

- `launch::first_push`, its `INIT_SCRIPT` and `CHECKOUT_SCRIPT`, `in_box`
  and `launch::git`: 133 lines.
- The owner's synchronization worker, its cancel flag and select loop in
  `session.rs`. The `ended_if_ready` check after it stays.
- `Box.created` (the disks an `up` created): only the push read it.
  Core's own `claimed.created` (disk filling) stays.
- The `sync-failed` diagnostic. The spec's First start paragraph shrinks
  from 24 lines to 5.
- The race guard "project crossing cannot enter between core readiness and
  completed initial synchronization". There is no synchronization left to
  guard. A bootstrapping owner holds the project's launch lock until it
  admits the first client. Crossing's `borrow` takes that lock before it
  asks an owner for a lease, so it still waits for admission.

Kept:

- The `unfilled` marker. Core disk filling and the helper's "no project
  disk yet" check use it.
- `repository` in the configuration snapshot. It drives `config-changed`
  on a gained or lost root `.git`, the share line's wording and the intro.

### The bare URL

`git push pinfold::ROOT BRANCH` stays refused, and so does the fetch form.
Git passes a command-line URL as the remote name. The helper refuses that
before either direction, and row 10 lists it as a control. A bare fetch
could not keep its checks, which key on `refs/remotes/NAME/*`. Allowing
push only would split `check` with a special case. So the intro prints
`git remote add box pinfold::ROOT` and then `box`-remote commands.

## Verification

### Checks

- `cargo fmt --check`: clean.
- `cargo clippy --all-targets --locked -- -D warnings`: clean.
- The same with `--target x86_64-unknown-linux-musl` is clean. That needs
  `RUSTC_WRAPPER=` and `CC_x86_64_unknown_linux_musl` set to a
  `zig cc -target x86_64-linux-musl` wrapper that drops cc-rs's
  `--target=` argument.

### Setup

Debug binary on macOS (Apple runtime), existing
`localhost/pinfold/profile-default` image (none built or removed). A
fresh temp dir `$T` held:

- `bin/pinfold` and `bin/git-remote-pinfold`, links to the debug binary.
- `bin/git`, a wrapper that logs each call (time, pid, ppid, cwd, argv)
  and execs `/usr/bin/git`.
- A wrapper that sets the following, prepends `$T/bin` to PATH, and runs
  in the project folder:
  - throwaway `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME`
  - `PINFOLD_PROFILE=default`
  - `VISUAL=true` and `EDITOR=true`
  - `GIT_CONFIG_GLOBAL=/dev/null` and a fixed author

Test repos have `main` (a.txt) and `dev` (b.txt), and an untracked `.env`.
`expect` answered `disk` and `y` to the first-run prompts.

### (a) First start of a Git disk project

`pinfold -- sh -c 'echo "in box: PWD=$PWD"; ls -la; git status'`:

```
Profile: default
Files for the box in $T/gitproj:
  share  the box edits this folder directly, including .git
  disk   the box works on an isolated disk; a new disk starts empty
Choose [share/disk] (share)
Save these settings and run sh? [Y/n]
in box: PWD=$T/gitproj
total 8
drwxr-xr-x 2  501 dialout 4096 Nov 20 10:21 .
drwxr-xr-x 4 root root    4096 Nov 20 10:21 ..
fatal: not a git repository (or any parent up to mount point $T)
```

- The project directory was empty.
- The host `.git/config` was byte-identical (`cmp`) before and after.
- The git log held two calls, `git -C ROOT rev-parse --show-toplevel`: one
  from the client and one from the owner (cwd `/`). That is
  `Context::verify`, admission's verification and the only
  `show-toplevel` call in the source. The owner's call is the positive control that the owner
  inherits the logging PATH.
- The same held for a first `pinfold start` (gitproj2): the config was
  identical, and the log had the same two calls.

### (d) Fetch from a disk that never held a repository (no owner)

`git push pinfold::ROOT main` and `git fetch pinfold::ROOT` both exit 128:

```
pinfold remote: remote-url: use a configured remote, not a URL: `git remote add box pinfold::ROOT`, then `git push box BRANCH` or `git fetch box`
fatal: remote helper 'pinfold' aborted session
```

After `git remote add box pinfold::ROOT`, `git fetch box` exits 128.
This is the final build, re-run on a fresh project:

```
pinfold remote: spec: ROOT is not a git repository in the box: fatal: not a git repository (or any parent up to mount point $T)\nStopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).; a push creates one where `.git` is absent
```

The guest's Git discovery stops at the disk's mount point. An ancestor
directory in the guest cannot take the push's or fetch's repository.

### (b) Explicit push into the empty disk

No owner, so each crossing used a transient box. `git push box main` exits
0 (`* [new branch] main -> main`). The box then showed:

```
branch: main
refs/heads/main 1b872c5
refs/remotes/host/main 1b872c5
## main
.git
a.txt
```

The working tree is checked out clean at the host's `main`. `.env` and
`b.txt` are absent. A later `git push box dev` exits 0 and adds
`refs/remotes/host/dev e1a7aa2`. The checked-out `main` is unchanged.

Under a running owner (gitproj2, after `pinfold start`),
`git push box dev main` exits 0. The box was on `dev` (the first pushed
branch), clean, with `host/dev` and `host/main`. `pinfold stop` exits 0.

Into a disk holding stray files (gitproj3), the box had first written
`a.txt` ("stray") and `notes.txt`, with no repository. `git push box main`
exits 0. The box then had `refs/remotes/host/main`, an unborn `main`
("No commits yet"), and `a.txt` and `notes.txt` untracked with their
contents kept. The fast-forward refused to overwrite them, and the refs
landed.

With a sha256 host repository on branch `trunk`, `git push box trunk`
exits 0. In the box, `git rev-parse --show-object-format` printed
`sha256`, and the branch was `trunk`, clean.

### (c) A box commit comes back

In the box: `git switch -c work`, then commit `c.txt`. On the host,
`git fetch box` exits 0 (`* [new branch] work -> box/work`).
`box/work` holds `c.txt`, authored `pi`. `box/main` and `box/dev` are
present: Git had already set them when the pushes succeeded. The host
config gained only the user's own `[remote "box"]` lines.

### (e) A non-Git folder on a disk

The first run showed the non-Git share line and the same disk line. The
box's `ls -A` printed nothing. It wrote `box.txt`. A second run listed
only `box.txt`, and the host folder held only `host.txt`.

### (f) A share project

First run with `share`, then `pinfold start`, `pinfold -- true` (exit 0),
`pinfold -- sh -c 'ls -A'` (`.env .git a.txt`), and `pinfold stop`
(exit 0). The host `.git/config` was byte-identical. Commands warned
`git-optional-locks` as before.

### (g) Texts as printed

Bare `pinfold`, configured Git disk project:

```
Run a command    pinfold -- CMD
Keep it running  pinfold start
Add Git remote   git remote add box pinfold::ROOT
Send a branch    git push box BRANCH
Bring work back  git fetch box
Change settings  pinfold config edit
All commands     pinfold --help
```

A non-Git disk project and a share print the same lines without the three
Git lines.

`pinfold --help`:

```
Disk projects
  git push box    Send a host branch into the box as host/*
  git fetch box   Bring guest branches back as box/*
                  Once first: git remote add box pinfold::ROOT
                  (needs git-remote-pinfold on PATH)
  disk export     Copy a stopped disk's files into a new directory
```

The Linux statement, `Files for the box in ROOT: an isolated disk; a new
disk starts empty.`, was not run: this host is macOS.

### Not run

- Linux: clippy only, no runtime.
- A host branch that two tags shadow is not run. Tags do not cross, so
  the box has none, and the row no longer names it.
- A first `pinfold -- CMD` from a project subdirectory is not run. On an
  empty disk, that cwd is absent, and row 39 makes an absent cwd a setup
  failure. The old first start made a checkout before the command, so the
  directory could exist.

### Cleanup

No box was left running. Every disk this run created was removed by name
with `pinfold disk rm` in the throwaway state root, and the temp dir was
deleted.
