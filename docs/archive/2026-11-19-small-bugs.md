# Small bugs #347, #348, #349

2026-11-19. Three defects found while building #296
(`2026-11-19-git-remote.md`). Each was reproduced through the CLI on main
(`b5b2b63`) before any change. No test was added: the suite is suspended.

## Setup

Both platforms used temporary disk projects (`git init`, one empty
commit, branches `main` and `side`, `mounts = [{ host = ".", kind =
"disk" }]` written through `VISUAL=true pinfold config edit`), private
XDG roots with a fresh `maintenance` stamp, `GIT_CONFIG_GLOBAL=/dev/null`,
`GIT_CONFIG_NOSYSTEM=1`, `GIT_OPTIONAL_LOCKS=0`, `PINFOLD_PROFILE` unset,
and the debug build first on PATH as `pinfold` and `git-remote-pinfold`.
`C` below is `$XDG_STATE_HOME/pinfold/crossing`.

- **macOS:** 27.2, Apple `container` 1.7.1, the existing
  `localhost/pinfold/profile-default` image (a launch warns
  `image-outdated`; it does not affect these paths).
- **Linux:** the dev VM, Debian 13, kernel 6.12.105, Firecracker, a musl
  debug build. `pinfold build` built `profile-default` under the private
  roots.

**Stalling a box command.** The box planted a FIFO as its repository's
alternates file: `pinfold -- sh -c 'mkdir -p .git/objects/info && mkfifo
.git/objects/info/alternates'`. Every git command the fetch script runs
then blocks opening it, with no output. `pinfold -- sh -c 'exec
3>.git/objects/info/alternates; rm .git/objects/info/alternates; sleep 2'`
releases the blocked readers and removes it.

## #349: owner diagnostics print a literal `\n`

**Reproduced on macOS** (Linux ran only after the fix). A new project with `.git/config.lock`
held on the host: `touch .git/config.lock; pinfold -- true
2>err`. Exit 0. The last bytes of `err` under `od -c` were
`` ` \ n \n ``: the `sync-failed` line ended in the two characters `\n`
and then a real newline.

**Cause.** `forward_diagnostics` passed the owner's whole stderr buffer
through `terminal_text`, which escapes every control character, the
buffer's newlines included, then appended a newline.

**Fix.** `bootstrap_diagnostics`, the one caller that forwards the
owner's stderr, escapes each line and keeps the line breaks; one trailing
newline is not doubled. Other callers pass single messages, some with
error text that was never escaped (an `io::Error`, a socket path), so
they keep escaping the whole text, newlines included.

**After.** The same command on a fresh project name (a reused disk is no
longer a first start): the line ends `` B R A N C H ` \n ``, one real
newline, on macOS and Linux. No owner buffer of more than one line
occurred in these runs, so an interior break was not observed.

## #347: a killed helper leaves its bundle file

**Reproduced, both platforms.** `pinfold start`, plant the FIFO, then
`git fetch box &`, wait for `C/PID.bundle`, `kill -9` the
`git-remote-pinfold box` process. Git: `error: git-remote-pinfold died of
signal 9`, exit 128. `C` held `PID.bundle`. After removing the FIFO, the
next `git fetch box` printed `* [new branch] main -> box/main`, exit 0,
and `C` still held the dead pid's file.

**Cause.** `Bundle::new` removed only a file under its own pid.

**Fix.** `Bundle::new` removes every `N.bundle` whose process is gone
(`kill(N, 0)` is `ESRCH`, or N is its own pid), the same check the
quarantine sweep makes; both now share it. The sweep runs under no lock
of its own. A crossing lease holds its project's `project:ID` lock
shared, and its project's launch lock only for a transient box; against
a live box it releases the launch lock first, and guarantee 10 has two
fetches run at once. Both locks are per project, while `crossing/` is
shared by every project under one state root. So liveness is what
protects another crossing's file. Guarantee 10 and the Crossing section
name the bundle file.

**After, both platforms.** Two projects `pa` and `pb` under one state
root, both started and kept. In `pa`: plant the FIFO, SIGKILL a first
fetch (`C`: the dead pid's file), start a second fetch, which stalls.
`C` then held only the live helper's file: its own `Bundle::new` had
removed the dead one. `pb`'s `git fetch box` succeeded with `pa`'s helper
live, and `C` still held the live file. After releasing the FIFO, `pa`'s
fetch printed `* [new branch] main -> box/main`, exit 0, and `C` was
empty. The first start's push had already removed the leftovers of the
reproduction runs before this.

## #348: a box command outlives its lease

**Reproduced, both platforms; not fixed here.**

Same steps as #347's reproduction. After the helper's SIGKILL, and still
after the next successful fetch, `pinfold -- ps -eo pid,pgid,args`
listed the stalled fetch script's process group (`sh -c`, `sh -c`, `git
… rev-list -n1 --stdin`). On the host the helper's child also survived:
`container exec -i … init exec --box-env -- sh -c …` on macOS,
`/proc/self/exe box exec NAME -- sh -c …` on Linux.

Linux, SIGTERM instead of SIGKILL: Git printed `pinfold remote:
interrupted by SIGTERM`, exit 128, and three seconds later the box still
ran the same three processes. From the code, cancellation kills only
the host child's process group.

Control, Linux, same kept box: `pinfold -- sleep 600 &`, `kill -9` of
that client; `sleep 600` was running before and gone three seconds
after.

**Affected commands** (read from the code, not run). Every caller of `Runtime::exec_command` runs a
public, non-project exec: crossing push and fetch, the first-start sync
(`launch.rs` `in_box`), Rex helpers and disk export. Only crossings can
outlive their box's holder: the first-start sync and Rex run in the
owner, whose death ends the box, and disk export and a transient
crossing box end with their process. Public `box exec` sends no signal
on departure by spec.

**Cause.** Project commands depart through the guest relay: when their
connection breaks, the helper sends TERM to the command's group and
KILL two seconds later (#325, spec Process supervision). That needs two
things crossing execs lack. The guest gates it on `project_directory`
in the exec metadata, which crossing execs do not set. And the broken
connection must be the client's own: crossing execs go through a child
process (`container exec` running plain `init exec` with no framing on
Apple, `pinfold box exec` on Linux) that outlives the helper. On Apple
there is no framed connection at all, so stdin EOF, which both crossing
scripts reach normally, cannot signal departure.

**Why not here.** A fix touches both runtimes' exec paths and crossing's
process handling. That is not a small fix, and a one-platform or
guest-only change would leave the spec claiming what one runtime does
not do.

**Proposal.** Run crossing execs through the session transport in the
helper's own process. `exec_transport::exec` and Apple's `project_exec`
take the command's three stdio descriptors instead of fds 0–2. Crossing
runs that on a thread with pipes in place of the `Command` it spawns
now. Crossing execs set `project_root` to the guest repository, which is
also their cwd, so the guest's existing departure applies. The helper's
death or cancellation then breaks its own vsock connection (Linux) or
the native client's stdin (Apple), and the guest ends the group: TERM,
then KILL after two seconds. Guarantee 10 or 47 gains the scenario
above. Roughly 100 to 200 lines across `exec_transport.rs`, `apple.rs`,
`firecracker/vsock.rs` and `crossing.rs`, and less code in crossing's
child handling.

**#322.** Not the same defect. #325 landed #322's first part: project
clients' groups get TERM and then KILL. What remains of #322 is owner-
enforced, cgroup-contained cleanup that a `setsid` or double fork cannot
escape, with new trusted controllers. That is a supervisor redesign and
an amendment to Always applied. If it lands, crossing leases could
register their execs with the owner the same way, which would also
close #348. The proposal above is the smaller path for #348 alone.

## Not run

- An owner diagnostic of more than one line (#349).
- The #348 SIGTERM variant on macOS.

I removed everything I created.

## Review round

- Both sweeps take only a canonical positive pid (decimal digits, no
  sign or leading zero, within `pid_t`, above 0). macOS: with
  `007.bundle`, `+5.bundle`, `0.bundle`, `abc.bundle`,
  `99999999999.bundle`, `pinfold-incoming-0099` and
  `pinfold-incoming-+7` planted, the #347 run above repeated with the
  same results, and every planted name survived.
- `process::diagnostic` escapes its whole message, so a record it writes
  is one line even when it carries a tool's stderr (`container volume
  rm` through disk cleanup). Two callers that escaped first no longer
  do. The owner's `eprintln!` sites in `pi/` already escape; core's
  remaining ones print host file and socket errors, not tool output.
  macOS: the #349 run still ends in one real newline.
- The Crossing section now says what the sweep does: a leftover goes once
  no process has its pid, and a reused pid keeps it until that process
  ends.
