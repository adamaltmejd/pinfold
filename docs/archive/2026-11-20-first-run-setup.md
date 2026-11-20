# First run asks share or disk; start runs setup; clearer intro

2026-11-20. Three owner-decided changes to the first-run experience:
interactive first run asks for the project mount on macOS, an interactive
`pinfold start` with no configuration runs the same setup, and bare
`pinfold` prints a shorter intro with next steps. No test was added: the
suite is suspended.

## Setup

macOS 27.2, Apple `container` 1.7.1, a debug build of this branch. One
throwaway root `/tmp/pf.jYcm` (short, for socket path length) held
`cfg`, `state` and `cache` as `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and
`XDG_CACHE_HOME`, a `bin` with the debug build linked as `pinfold` and
`git-remote-pinfold` first on `PATH`, and `proj`, a Git repository with
one committed file on `main`. A wrapper exported those, plus
`PINFOLD_PROFILE=default` and `PINFOLD_NO_UPDATE_CHECK=1`, and ran its
arguments in `proj`. Prompts were driven through a pty by `expect`. The
existing `localhost/pinfold/profile-default:latest` image was used; no
image was built or removed. A missing managed image came from a saved
profile `pftmp` (`pinfold profile new pftmp --from default` under the
temporary config root), whose image was never built.

## Checks

`cargo fmt --check` and `cargo clippy --all-targets --locked -- -D
warnings` pass. `cargo clippy -p pinfold --all-targets --locked --target
x86_64-unknown-linux-musl -- -D warnings` passes with
`CC_x86_64_unknown_linux_musl` set to a `zig cc -target x86_64-linux-musl`
wrapper that drops cc-rs's `--target=` flag.

## Cases

**Intro, no configuration.** `pinfold` exits 0 and leaves the three
temporary roots empty:

    Project   /private/tmp/pf.jYcm/proj
    Box       not set up

    The first run asks you to confirm, and lets you pick a disk instead:
    Profile   default
    Files     Shared: the box edits /private/tmp/pf.jYcm/proj

    Set up and run     pinfold -- CMD     (e.g. pinfold -- claude)
    Change settings    pinfold config edit
    All commands       pinfold --help

With `PINFOLD_PROFILE=pftmp` it adds, after Files,
`Image     not built yet. The first run offers to build it.` With `PATH`
reduced to the bin dir, `/usr/bin` and `/bin` (no `container`), the same
command prints no Image line and exits 0.

**First run via `-- true`, bad mount answer.** Answer `n`:
`pinfold: unknown files choice "n"; answer share or disk`, exit 125, no
configuration file.

**First run via `-- true`, declined.** The pty showed:

    Using default (bundled profile, selected by PINFOLD_PROFILE).
    Files: share this folder with the box, or use an isolated disk? [share/disk] (share)
    share
    The box can edit files in /private/tmp/pf.jYcm/proj, including its Git metadata.
    Save these settings and run true? [Y/n]
    n

Exit 125; the configuration and state roots stayed empty.

**Profile question.** With `PINFOLD_PROFILE` unset, `Choose a profile
(default, documents, adam, or a saved profile) [default]` came first;
Enter gave `Using default (bundled profile).`, then the Files question.
Declined: exit 125, no file.

**First run via `-- CMD`, share.** The command was `pinfold -- sh -c
'echo inside; ls /tmp/pf.jYcm/proj 2>/dev/null || pwd; ls'` (in place of `-- true`, to see the folder). Enter
at the Files question, `y` at the confirmation: it ran in the box (cwd
`/private/tmp/pf.jYcm/proj`, `f.txt` listed), exit 0. The file holds
`mounts = [{ host = ".", kind = "share" }]` and `profile = "default"`.

**Intro, configured share.** Stopped:

    Project   /private/tmp/pf.jYcm/proj
    Box       stopped
    Profile   default
    Files     Shared: the box edits /private/tmp/pf.jYcm/proj
    Settings  /tmp/pf.jYcm/cfg/pinfold/projects/proj-b18a7fb13f5b/pinfold.toml

    Run a command    pinfold -- CMD
    Keep it running  pinfold start
    Change settings  pinfold config edit
    All commands     pinfold --help

After a noninteractive `pinfold start` (exit 0) the Box lines read
`Box       running with profile default` and `Box files Shared: the box
edits /private/tmp/pf.jYcm/proj`, followed by `The next launch uses these
settings.`; `pinfold stop` exited 0. `pinfold config` still prints
`Image     localhost/pinfold/profile-default:latest (available locally)`
and its Files line uses the same text. With `PINFOLD_PROFILE=pftmp` the
intro adds the first line below; with `image =
"localhost/pf-nonexistent:1"` appended to the file it adds the second:

    Image     not built yet. Build it with `pinfold build`.
    Image     localhost/pf-nonexistent:1 is not available locally. Make it available before running.

**Noninteractive refusals.** With no configuration, `pinfold start
</dev/null` printed this and exited 125; `pinfold -- true </dev/null`
refused the same way. No file was written.

    pinfold start: no-config: /private/tmp/pf.jYcm/proj has no configuration. Run `pinfold -- pi` or `pinfold -- sh` in a terminal to set it up

**First run via `-- CMD`, disk.** The command was `pinfold -- git -C
/tmp/pf.jYcm/proj log --oneline --all`. Answer `disk`, then `y`: the
statement read

    The box uses an isolated project disk for /private/tmp/pf.jYcm/proj. Host files stay separate from edits in the box.

and the file holds `kind = "disk"`. The command itself exited 128
(`fatal: cannot change to '/tmp/pf.jYcm/proj'`): that host path does not
exist in the guest. The host gained remote `box` at
`pinfold::/private/tmp/pf.jYcm/proj`, and a following `pinfold -- git log
--oneline --all --decorate` printed `e4a7f1d (HEAD -> main, host/main)
file`, exit 0: the owner's first-start push ran. Setup only writes the file and
admits again; the push is the owner's existing first-start code, which
reads the saved mount.

**First run via `start`, disk.** The configuration file, the project disk
(`pinfold disk rm pi-proj-b18a7fb13f5b-project`) and the host remote `box`
were removed first. Answer `disk`, then `y` at `Save these settings and
start the box? [Y/n]`: exit 0, the file holds `kind = "disk"`, and the
remote `box` was added again. The intro then read
`Box files Isolated disk: the box keeps its own copy of
/private/tmp/pf.jYcm/proj. git fetch box and git push box move branches.`
`git fetch box` printed `* [new branch] main -> box/main`, exit 0.
`pinfold stop` exited 0 and the intro read `Box       stopped`.

**`start` build offer.** With no configuration and
`PINFOLD_PROFILE=pftmp`, `start` set up a share, then asked `The project
image localhost/pinfold/profile-pftmp:latest is missing. Build it now?
[Y/n]`; `n` exited 125 with the configuration saved. A second interactive
`start`, now configured, did not offer; it printed this and exited 125:

    pinfold start: image localhost/pinfold/profile-pftmp:latest is missing; run `pinfold build --profile pftmp`

**Not exercised.** Choosing `disk` from a linked worktree (setup checks
the worktree against the chosen mount before writing the file) was not
tried. The Linux path was compiled, not run.

## Cleanup

Both disks were removed by name with `pinfold disk rm`, `pinfold disk
list` was empty, no pinfold container remained in `container ls -a`, and
`container volume ls` was empty, as it was before the run. The temporary
root was deleted.

## Review fixes, rounds 1 and 2

Two review rounds first moved Git verification before the save, then
guarded it against a live owner with the launch lock and a cancellable
worker. Round 3 replaced both designs; their setup behavior is not
recorded here. Their wording changes stand and were observed in round
2, with a fresh root `/tmp/pf.mIop` laid out as above:

    Files     Isolated disk: the box has its own files, separate from /private/tmp/pf.mIop/gd.
    The box has its own files on an isolated disk, separate from /private/tmp/pf.mIop/gd.

Bare `pinfold` in the configured Git disk project `gd`, stopped:

    Project   /private/tmp/pf.mIop/gd
    Box       stopped
    Profile   default
    Files     Isolated disk: the box has its own files, separate from /private/tmp/pf.mIop/gd.
    Settings  /tmp/pf.mIop/cfg/pinfold/projects/gd-98cc5064af5d/pinfold.toml

    Run a command    pinfold -- CMD
    Keep it running  pinfold start
    Move branches    git fetch box / git push box
    Change settings  pinfold config edit
    All commands     pinfold --help

A configured share printed no `Move branches` line. A non-Git folder
configured as a disk (`EDITOR=true VISUAL=true pinfold config edit`, kind
changed to `disk` by hand) printed `Files     Isolated disk: the box has
its own files, separate from /private/tmp/pf.mIop/plain.` and no `Move
branches` line; `pinfold config` printed the same Files lines. Every disk
those rounds created was removed by name and their roots deleted.

## Review fixes, round 3

Setup never runs host Git. Before saving, it runs the chosen mount's
checks that need none (`Context::check_metadata`: the worktree check, and
for a share the `.git` and `commondir` reads); a refusal there saves
nothing. The disk's Git verification and the comparison with a live
owner run at the start that follows, after the save.

Same host, a debug build of the round 3 commit, a fresh root
`/tmp/pf.kG1O` laid out as above, with `VISUAL=true EDITOR=true` in the
wrapper. A `git` script first on `PATH` logged each call with whether any
project configuration file existed at that moment (`config-saved=`), and
with `PF_GIT_FAIL` set made `rev-parse` exit 1. Projects: `s`, `g`, `gd`,
`rf` and `w`, Git repositories with one committed file; `w/wt`, a linked
worktree of `w` beneath it; `bad`, a fresh repository whose
`.git/commondir` is `../../other`. `cargo fmt --check` and `cargo clippy
--all-targets --locked -- -D warnings` pass.

**(a) Share, `commondir` outside `.git`.** In `bad`, `pinfold -- true`,
`share`, `y`: `pinfold: worktree: .git is not a directory; run pinfold
from the main checkout`, exit 125, no Git call, no file.

**(b) Disk from a nested linked worktree.** In `w/wt`, `pinfold -- true`,
`disk`, `y` (the statement named the main root `w`): the same `worktree`
refusal, exit 125, no Git call, no file.

**(c) Disk, Git cannot verify.** In `rf` with `PF_GIT_FAIL=1`, `pinfold
start`, `disk`, `y`: `pinfold start: git-discovery: Git could not verify
the project root`, exit 125. The only Git call was `rev-parse
--show-toplevel` with `config-saved=yes`, and the file remains with
`kind = "disk"`. No other project had a file at that point.

**(d) Live share owner.** An interactive `pinfold start` in `s` with
`share` exited 0 and kept the box. With its file deleted, `pinfold --
true`, `disk`, `y` printed `pinfold: config-changed: the project mount
kind changed (project.mount). …`, exit 125; no Git call, and the saved
`kind = "disk"` file remained. With that file deleted, `share`, `y`
joined the box: exit 0, no Git call. `pinfold stop` exited 0.

**(e) Controls.** In `g`, `share`, `y`: `true` ran, exit 0, `kind =
"share"`. In `gd`, `disk`, `y`: exit 0, `kind = "disk"`; the log showed
admission's two `rev-parse --show-toplevel` calls, then the first-start
sync (`remote add box pinfold::/private/tmp/pf.kG1O/gd`, `bundle
create`).

**Cleanup.** The disks `config --json` named were removed by name
(`pi-s-058a83cde655-home`, `pi-g-6b7bc93d5e3e-home`,
`pi-gd-11a8036d5c54-home`, `pi-gd-11a8036d5c54-project`; `rf` had none).
`pinfold disk list` and `container volume ls` were empty and `container
ls -a` held only the runtime's builder. The root was deleted. No image
was built or removed.
