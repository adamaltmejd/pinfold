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

The signal skips destructors where the old panic unwound. Every report
prints after its work or only reads between lines, except `clean`, which
prints its plan before removing (both old and new behavior abandon the
removal), and `box up`. Its `down` line is printed only after `hold`'s
teardown succeeded, when `Claim::finish` has already removed the
ownership record, so `Claim`'s drop has nothing left to remove; `refused`
and a signal's `down` before start hold no box.

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
