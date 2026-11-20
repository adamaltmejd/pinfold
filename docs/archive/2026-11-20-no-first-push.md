# A new project disk starts empty

2026-11-20. Owner decision: drop the automatic first-start synchronization
of disk projects (`2026-11-19-git-remote.md`, "First start"). That start
created the box repository, pushed every host branch in as `host/*`,
checked out the host's branch and added the host remote `box`. It moved
all branches into a box and edited the host's Git configuration without
being asked. No test was added: the suite is suspended. PR #362.

## Built

- A start runs no host Git beyond admission's verification, pushes nothing
  and writes no host Git configuration. A new project disk starts empty,
  Git project or not.
- Code enters a disk by a clone or `git init` in the box, or by
  `git push box BRANCH` after `git remote add box pinfold::ROOT`.
- A push into a box path without `.git` first runs `git init` there with
  the host's object format. HEAD names Git's default branch, unborn
  (`master` in the default image). It then lands `refs/remotes/host/*`
  and checks nothing out. Crossing never touches a working tree.
  `git switch -c BRANCH host/BRANCH` in the box checks a branch out. That
  command is static text in `--help`, the README and the delegate skill;
  a push prints nothing extra.
- A nonzero exit from the box's push script fails the push, as on main.
  64 maps to `spec` (not a repository), and anything else to "the box's
  git failed".
- Fetch from a box path without a repository refuses as `spec`, as
  before. The refusal now ends with a note that a push creates the
  repository.
- Printed remote-add commands quote the URL as one shell word. This
  covers the intro line and the helper's three `remote-url` refusals,
  including the bare-URL one, which now names `git remote add box URL`.
  - A plain URL prints unquoted. Any other prints single-quoted, with
    `'\''` for a quote.
  - A URL holding a control character prints as the placeholder
    `pinfold::ROOT`, since it cannot be pasted.
  - The function, `url_word`, lives in `remote.rs`: `formatting.rs` is
    also compiled into `pinfold-init`.
- Wording.
  - The first-run `disk` line is the same in Git and non-Git folders.
  - The Linux statement says a new disk starts empty.
  - The bare `pinfold` intro for a Git disk project prints the
    remote-add, push and fetch lines.
  - The `--help` "Disk projects" block names the remote-add step and the
    in-box switch command.

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
  completed initial synchronization", with nothing left to guard.
  - A bootstrapping owner holds the project's launch lock until it admits
    the first client.
  - Crossing's `borrow` takes that lock before it asks an owner for a
    lease, so it still waits for admission.
  - The stale comment on the `Starting` status now says the owner
    publishes its generation before it admits its first client.

Kept:

- The `unfilled` marker. Core disk filling and the helper's "no project
  disk yet" check use it.
- `repository` in the configuration snapshot. It drives `config-changed`
  on a gained or lost root `.git`, the share line's wording and the intro.

### The bare URL

`git push pinfold::ROOT BRANCH` stays refused, and so does the fetch form.
Git passes a command-line URL as the remote name, and the helper refuses
that before either direction. Row 10 lists the refusal as a control. A
bare fetch could not keep its checks, which key on `refs/remotes/NAME/*`,
and allowing only push would split `check` with a special case. So the
intro prints `git remote add box pinfold::ROOT` and then `box`-remote
commands.

### Concurrent first pushes

Nothing serializes pushes in the box. Two first pushes may both run
`git init`.

- A second `git init` on an existing repository leaves its refs, objects,
  HEAD and config. Inside a box (Git 2.47.3), a reinit left HEAD, refs
  and config byte-identical.
- In 50 simultaneous pairs of `git init` on one path, every pair had one
  init fail and every resulting repository was usable. "Usable" was
  checked by reading the config, fetching a ref in and running fsck. This
  held on the host (2.54.0) and in a box (2.47.3).
- The code does not rely on that failure: whichever init finishes, HEAD
  is Git's default branch, unborn, and each push lands only its own
  refs.

### Designs tried and dropped

These were on intermediate commits of the PR, none released:

- **Checkout on the creating push.** The push used `--initial-branch` set
  to the first pushed branch, then `git merge --ff-only`. Review found
  three problems:
  - Concurrent first pushes could merge into whatever HEAD named.
  - A merge into an unborn branch can overwrite an ignored guest file
    the commit tracks.
  - A failed checkout still reported success.
- **A per-push hint.** The script exited 65 after creating the
  repository, and the helper printed `git switch -c BRANCH host/BRANCH`.
  Review found three problems:
  - Any unrelated exit 65 in the script would acknowledge refs as pushed.
  - Two concurrent creators could both print a hint naming a branch HEAD
    did not name.
  - The command fails for accepted names like `HEAD` or `-topic`.
- **`--initial-branch`** went with these. Git rejects names like `HEAD`
  as an initial branch, which would fail such a push.
- **Terminal escaping before quoting.** It changed what a pasted command
  says, for example for U+0085.

## Verification

### Checks

- `cargo fmt --check` is clean.
- `cargo clippy --all-targets --locked -- -D warnings` is clean.
- The same with `--target x86_64-unknown-linux-musl` is clean. That needs
  `RUSTC_WRAPPER=` and `CC_x86_64_unknown_linux_musl` set to a
  `zig cc -target x86_64-linux-musl` wrapper that drops cc-rs's
  `--target=` argument.

### Setup

Debug binary on macOS (Apple runtime), with the existing
`localhost/pinfold/profile-default` image; none was built or removed.
Each run used a fresh temp dir `$T` holding:

- `bin/pinfold` and `bin/git-remote-pinfold`, links to the debug binary.
- `bin/git`, a wrapper that logs each call (time, pid, ppid, cwd, argv)
  and execs `/usr/bin/git`.
- A wrapper that runs in the project folder, prepends `$T/bin` to PATH,
  and sets:
  - throwaway `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME`
  - `PINFOLD_PROFILE=default`
  - `VISUAL=true` and `EDITOR=true`
  - `GIT_CONFIG_GLOBAL=/dev/null` and a fixed author

Test repos have `main` (a.txt) and `dev` (b.txt), and an untracked `.env`.
`expect` answered `disk` and `y` to the first-run prompts. Each start also
printed an `image-outdated` warning for the default image, omitted below.

Some cases ran on an intermediate build, marked so. Each such result
does not depend on the dropped checkout or hint.

### First start of a Git disk project

`pinfold -- sh -c 'echo "in box: PWD=$PWD"; ls -la; git status'` (an
intermediate build; this path is unchanged since):

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
- The git log held two calls of `git -C ROOT rev-parse --show-toplevel`:
  one from the client and one from the owner (cwd `/`).
  - This is `Context::verify`, admission's verification, and the only
    `show-toplevel` call in the source.
  - The owner's call is the positive control that the owner inherits the
    logging PATH.
- The same held for a first `pinfold start`.

### First push into an empty disk (final build, no owner)

`git push box main` exits 0. The push's stderr held only Git's own lines
and the image warning:

```
To pinfold::$T/gitproj
 * [new branch]      main -> main
```

The box then showed:

```
HEAD -> refs/heads/master
refs/remotes/host/main d563d09
status:
## No commits yet on master
ls:
.git
```

In the box, `git switch -c main host/main` prints
`Switched to a new branch 'main'`. It exits 0 and leaves `## main` with
`a.txt`, and no `.env` or `b.txt`.

### Second push (final build)

`git push box dev` exits 0 and adds `refs/remotes/host/dev`. A sha256
listing of every box file plus `ls -A` was identical before and after.
The box stayed on `## main`.

### A failing in-box step is a failed push (final build)

With the box's `.git` replaced by a file holding `junk`, `git push box
main` (a new commit) exits 1:

```
pinfold: spec: $T/gitproj is not a git repository in the box; a push creates one where `.git` is absent
 ! [remote rejected] main -> main (the box refused the push)
```

With `.git/objects` made unwritable in the box, so its fetch fails, the
push exits 1:

```
pinfold: the box's git failed
 ! [remote rejected] main -> main (the box refused the push)
```

In both cases the host's `refs/remotes/box/main` stayed at the old commit,
and the box's refs were unchanged. Once the box was repaired, the same
push exits 0.

By the code, `push_cancellable` maps only exit 0 to success.

### Concurrent first pushes against a running box (intermediate build, with the hint)

`git push box main &` and `git push box dev &` ran against a box under
`pinfold start`, in 3 rounds, with `.git` moved aside between rounds.

- Both pushes exited 0 every round.
- One of the two printed `pinfold: waiting for the project box to start`
  first, so the crossings did not overlap here. The simultaneous case
  rests on the Git measurements above.
- Afterwards the box had both `host/` refs, HEAD unborn, an empty working
  tree, fsck ok and `core.repositoryformatversion` 0.

### Stray and ignored files (intermediate build, with the hint)

The host commit tracks `.gitignore` (`build/`) and, forced, `build/out`.
Before any repository existed, the box wrote:

- `a.txt` ("stray")
- `notes.txt`
- its own `.gitignore`
- `build/out` ("guest")
- `build/cache`

After `git push box main` (exit 0), every file's sha256 was
byte-identical. `git status --ignored` showed three untracked files and
`build/` ignored, and the only ref was `host/main`.

### Fetch from a disk that never held a repository (no owner)

`git push pinfold::ROOT main` and `git fetch pinfold::ROOT` both exit 128:

```
pinfold remote: remote-url: use a configured remote, not a URL: `git remote add box pinfold::ROOT`, then `git push box BRANCH` or `git fetch box`
fatal: remote helper 'pinfold' aborted session
```

After `git remote add box pinfold::ROOT`, `git fetch box` exits 128:

```
pinfold remote: spec: ROOT is not a git repository in the box: fatal: not a git repository (or any parent up to mount point $T)\nStopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).; a push creates one where `.git` is absent
```

The guest's Git discovery stops at the disk's mount point, so an
ancestor directory in the guest cannot take the push's or fetch's
repository.

### A box commit comes back (intermediate build)

In the box: `git switch -c work`, then commit `c.txt`. On the host,
`git fetch box` exits 0 (`* [new branch] work -> box/work`).

- `box/work` holds `c.txt`, authored `pi`.
- `box/main` and `box/dev` were present too: Git set them when the
  pushes succeeded.
- The host config gained only the user's own `[remote "box"]` lines.

### sha256 (intermediate build)

From a sha256 host repository, `git push box trunk` exits 0. In the box,
`git rev-parse --show-object-format` prints `sha256`.

### A non-Git folder on a disk, and a share

- **Non-Git folder on a disk.** The first run showed the non-Git share
  line and the same disk line. The box's `ls -A` printed nothing, and it
  wrote `box.txt`. A second run listed only `box.txt`; the host folder
  held only `host.txt`.
- **Share.**
  - The first run took `share`, then ran `pinfold start`, `pinfold --
    true` (exit 0), `pinfold -- sh -c 'ls -A'` (`.env .git a.txt`) and
    `pinfold stop` (exit 0).
  - The host `.git/config` was byte-identical, and commands warned
    `git-optional-locks` as before.

### A root with a space (final build)

Bare `pinfold` in `$T/sp ace` printed:

```
Run a command    pinfold -- CMD
Keep it running  pinfold start
Add Git remote   git remote add box 'pinfold::$T/sp ace'
Send a branch    git push box BRANCH
Bring work back  git fetch box
Change settings  pinfold config edit
All commands     pinfold --help
```

- Running that printed line through `sh -c` exits 0.
- `git push box main` exits 0.
- The bare-URL refusal prints:

```
pinfold remote: remote-url: use a configured remote, not a URL: `git remote add box 'pinfold::$T/sp ace'`, then `git push box BRANCH` or `git fetch box`
```

A non-Git disk project and a share print the intro without the three Git
lines.

`pinfold --help`:

```
Disk projects
  git push box    Send a host branch into the box as host/*
                  In the box: git switch -c BRANCH host/BRANCH
  git fetch box   Bring guest branches back as box/*
                  Once first: git remote add box pinfold::ROOT
                  (needs git-remote-pinfold on PATH)
  disk export     Copy a stopped disk's files into a new directory
```

### Not run

- Linux: clippy only, no runtime. The Linux statement, `Files for the box
  in ROOT: an isolated disk; a new disk starts empty.`, did not print on
  this macOS host.
- A URL holding a control character: the placeholder rule was read from
  the code, not run.
- A first `pinfold -- CMD` from a project subdirectory. On an empty disk
  that cwd is absent, and row 39 makes an absent cwd a setup failure. The
  old first start checked out before the command, so the directory could
  exist.

### Cleanup

No box was left running. Every disk the runs created was removed by name
with `pinfold disk rm` in each throwaway state root, and every temp dir
was deleted.
