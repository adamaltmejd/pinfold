# Report commands and a closed stdout (#359)

2026-11-20. `pinfold | head -1` panicked with `failed printing to stdout:
Broken pipe (os error 32)`: reports print with `println!`, which panics
on EPIPE because Rust's runtime ignores SIGPIPE. No test was added: the
suite is suspended.

## Mechanism

`cli.rs` defines a `println!` that shadows std's in `cli.rs`, and in
`main.rs` and `update.rs`, which import it. It writes the line to stdout.
On `BrokenPipe` it restores SIGPIPE's default disposition and raises it,
so the process dies of SIGPIPE as a Unix filter's writer does (141 from
a shell). Any other write error panics as std's did.

Restoring SIGPIPE at startup per verb was rejected. Report verbs are not
pure printers: bare `pinfold` probes the project owner's control socket,
`doctor` and `box list` query the runtime, `disk export`, `build` and
`image build` feed child pipes, and maintenance runs before most verbs.
The code handles EPIPE on sockets and child pipes (`ownership.rs`,
`process.rs`, `crossing.rs`, `apple.rs`, `exec_transport.rs`); a default
SIGPIPE would bypass those handlers. The shadowed `println!` reacts only
to a failed stdout write.

The signal skips destructors where the old panic unwound. The first
commit claimed every report prints after its work or only reads between
lines. That was wrong: `disk rm --all` and `box prune` print inside their
removal loops, and `clean` prints its plan before removing, so a signal
there cut the work short. See Review fixes. `box up`'s `down` line is
printed only after `hold`'s teardown succeeded, when `Claim::finish` has
already removed the ownership record, so `Claim`'s drop has nothing left
to remove; `refused` and a signal's `down` before start hold no box.

Calls from `pi/` into `cli.rs` and `update.rs` (`usage`, `exit_code`,
`working`, `installed`, `link_helper`, `helper_on_path`, `link_remedy`,
`notice`) reach no shadowed `println!`: `update.rs` prints to stdout only
in `update`'s own `run`, and `notice` uses `eprintln!`.

Per role, from `main.rs`'s dispatch:

- Bare `pinfold`, `--version`, `--help`, subcommand `--help`, `config`,
  `doctor`, `disk`, `clean`, `build`, `image`, `profile`,
  `apparmor-profile`, `update`, and `box stat|list|prune|inspect|events`:
  the shadowed `println!`.
- `box up`: its `ready` and `failed` lines already use `writeln!` with
  `?`; a failed `ready` tears the box down and exits 1, unchanged. Its
  `down` and `refused` lines use the shadowed `println!`.
- `box exec` and `pinfold -- CMD`: unchanged. The relay already closes
  the guest stream on EPIPE so the guest writer sees SIGPIPE, and the
  command's status passes through.
- `start`, `stop`, the detached owner, proxy and relays, the Git remote
  helper (`pi/` has no `println!`; the helper writes a dup of stdout with
  error handling) and guest `init`: unchanged. SIGPIPE stays ignored.

## Setup

macOS 27.2, Apple `container`, the existing
`localhost/pinfold/profile-default` image. A debug build of main
(`24a14f1`) kept as the baseline, and the patched debug build. A wrapper
script set fresh `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME`
under a new temp dir, `PINFOLD_PROFILE=default`, `VISUAL=true
EDITOR=true`. A plain folder, configured with `pinfold config edit`, was
the project. `GIT_OPTIONAL_LOCKS=0` for sessions.

Each command ran three ways, recording `${PIPESTATUS[0]}` and whether
stderr held panic text:

```sh
pinfold ARGS 2>err | head -1
pinfold ARGS 2>err | true
{ sleep 1; exec pinfold ARGS 2>err; } | true   # reader certainly closed
```

`head -1` and `| true` race the reader: a short report can finish before
it closes, so the third form is the decisive one.

## Results

Baseline (main), reader closed: exit 101 and a three-line panic for bare
`pinfold` (with and without a running box), `--version`, `--help`,
`config --help`, `box --help`, `image --help`, `build --help`, `config`,
`config --json`, `doctor`, `doctor --json`, `doctor --artifacts [--json]`,
`disk list`, `disk list --json` (with a disk), `box list --label …`,
`box stat BOX`, `box inspect RUN`, `box events RUN`, `clean --dry-run`
and `update --check`. The `head -1` form often exited 0 because the
report fitted in the pipe before `head` exited.

Patched, the same commands, reader closed and `| true`: exit 141, empty
stderr. `head -1`: 141 or 0 depending on the race, never a panic.
Commands that print nothing (`box prune` with no dead boxes, `disk list
--json` without disks) exit 0; usage errors (`box list` without a label,
an unknown verb, `apparmor-profile` on macOS) still exit 1.

`disk rm NAME` with the reader closed: exit 141, and `disk list` shows the
disk removed.

Unpiped, each command's stdout was byte-identical between baseline and
patched (`cmp`), with the same exit status: bare, `--version`, `--help`,
`config --help`, `config`, `config --json`, `doctor`, `doctor --json`,
`doctor --artifacts [--json]`, `disk list [--json]`, `box list --label`,
`box --help`, `clean --dry-run`, `update --check`.

Sessions, identical on baseline and patched:

- `pinfold -- yes | head -1` and `pinfold -- yes | true`: exit 141 within
  a second, empty stderr.
- `pinfold -- sh -c 'yes | head -100000' | head -1`: exit 0 within a
  second, empty stderr.
- Then `pinfold -- sh -c 'ps -eo pid,args; pgrep yes'`: only init, the
  session helper, `sh` and `ps`; `pgrep` exits 1.

`pinfold start`, `pinfold -- true` and `pinfold stop` each exited 0.

Git remote helper: a throwaway repository (one empty commit) as a disk
project, `pinfold start`, the patched build linked as
`git-remote-pinfold` first on `PATH`, `git remote add box pinfold::ROOT`,
`git push box main`: `* [new branch] main -> main`, exit 0; the box's
`git branch -a` lists `remotes/host/main`. `pinfold stop` exited 0 and
the three disks made were removed by name.

`box prune` ran in the harness against the shared runtime, printed
nothing and removed nothing; the one box running beforehand was still
listed running afterwards.

Not run: `box up` with a closed stdout (no spec was written), Linux.
`cargo clippy` for `x86_64-unknown-linux-musl` (zig as the C compiler)
passed.

## Review fixes

Review of `8cb8a2b` found three faults. All three reproduced on that
build.

1. **An inherited blocked SIGPIPE.** `raise` returned, and the
   `unreachable!` after it panicked (exit 101). Now `printed` exits 141
   when the process survives `raise`.
2. **`box up` statuses.** The `refused` and `down` records died by signal
   (141), against the spec's 1 for a refusal and 0 or 1 for `down`.
3. **Loops cut short.** `disk rm --all` printed each result inside its
   removal loop, so the signal left later disks unattempted, against "attempts
   each ordinary removal". `box prune` has the same shape, and `clean`
   prints its plan before removing.

Fix: `main.rs` marks `box`, `disk`, `image`, `clean`, `build` and
`update` with `cli::finishes_without_reader()`. In those, the shadowed
`println!` drops a line whose write fails with `BrokenPipe` and the command
goes on, exiting with its own status. The marked commands are the process
interface (`box`, `disk` and `image`, whose exit statuses are specified)
and the commands that change state. The others are read-only reports
and keep the SIGPIPE death: bare `pinfold`, `--version`, `--help`,
subcommand `--help` (help returns before the mark), `config`, `doctor`,
`apparmor-profile` and `profile`. `box up`'s `failed` line moved from
`writeln!` with `?` to the shadowed `println!`, so a missing reader no
longer adds a "Broken pipe" diagnostic; it still exits 1. Its `ready`
line keeps `writeln!` with `?`: a failed `ready` tears the box down and
exits 1. `image rm` prints after its removal and needed no change
beyond the mark.

Commands whose closed-reader status changes from 141 to 0 (by the
mark; run below only for `box list`, `box up`, `disk rm --all`, `clean
--dry-run` and `update --check`):
`box list|stat|inspect|events|prune`, `disk list`, `disk rm` and
`disk export`, `image build|rm`, `clean` (including `--dry-run`),
`build`, and `update` (including `--check`). Whenever they would have
exited otherwise, they exit with that status: 1 for a refusal or a failed
removal, 3 for an absent box or run.

### Verification

Same setup as above, under a new temp dir. `prev` is `8cb8a2b`. Each
case ran with a reader certainly closed: `{ sleep 1; exec pf ARGS
2>err; } | true`, then `${PIPESTATUS[0]}`.

- `--version` with SIGPIPE blocked: a Perl wrapper ran
  `sigprocmask(SIG_BLOCK, SIGPIPE)` and then exec'd pinfold. Patched:
  exit 141, empty stderr. `prev`: exit 101, a panic at
  `entered unreachable code`. `--help` blocked, patched: 141, empty.
- Unblocked, patched: `--version`, `--help`, bare, `doctor` and `config
  --help` exited 141 with empty stderr. `box list --label …`,
  `clean --dry-run` and `update --check` exited 0 with empty stderr
  (`box list` on `prev`: 141). `box --help` exited 141.
- `box up < refuse.json` (an unknown spec field, refused as `spec`):
  patched, exit 1 with empty stderr; `prev`, exit 141. Unpiped, it
  prints the `refused` line and exits 1.
- `box up < up.json | head -1`: one box with three disks. `head` read
  `ready` and exited, and stdin EOF tore the box down. Patched: exit 0
  (`stdin-closed`), empty stderr. `disk list` then showed `bp359-a`,
  `-b` and `-c`.
- `disk rm --all`: on `prev`, exit 141; only `bp359-a` was removed and
  `-b` and `-c` remained. After the three disks were recreated with the
  same `box up`: patched, exit 0, empty stderr, and `disk list` was
  empty. No `bp359` box remained.
- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D
  warnings` on macOS, and the same for `x86_64-unknown-linux-musl` (zig
  as the C compiler): pass.

The session cases were not rerun: this change does not touch
`pinfold -- CMD`.
