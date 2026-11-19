# Setup prompts only on a canonical terminal, 2026-11-19

Base 17f8a72 on `codex/project-cli-usability`. e14422b made a setup
prompt set ICANON on a noncanonical terminal and restore the caller's
settings afterwards. A review found two faults. A restore from the
background on SIGTERM can stop pinfold with SIGTTOU or overwrite another
foreground job's settings. Switching to canonical mode lets the line
discipline edit bytes already queued for the guest command.

## Change

The mode switch is deleted. Setup I/O is interactive only when stdin,
stdout and stderr are terminals and `tcgetattr` on stdin reports ICANON.
pinfold never calls `tcsetattr` for a prompt. A noncanonical terminal gets
the redirected outcome: `no-config`, or the missing-image refusal naming
the build command. Prompts still read one byte at a time up to the
newline. nix's `term` feature stays for `ttyname` and `tcgetattr`.

## Checks

On arm64 macOS: `cargo fmt --check`, `cargo clippy --all-targets --locked
-- -D warnings`, and the same Clippy for `--target
x86_64-unknown-linux-musl` with a `zig cc -target x86_64-linux-musl`
wrapper as ring's C compiler. All passed. On the dev VM, `cargo build
--locked --target x86_64-unknown-linux-musl`. No base-binary control was
run.

## Method

Debug binaries; private XDG config, state, cache and data roots; a new Git
project per case; `PINFOLD_CPUS=2 PINFOLD_MEMORY=1G`; `PINFOLD_PROFILE`
unset. A Python `pty.fork` driver ran
`sh -c 'trap : INT; stty ARGS; stty -g >before; pinfold -- CMD; stty -g >after'`
on a fresh PTY, waited for each prompt, sent the answer and compared the
two `stty -g` strings. SIGTERM went to pinfold's pid while the profile
prompt was open. CMD was `sh -c 'echo ran-$((6*7))'`.

The queued-bytes case ran `pinfold -- sh -c 'head -c 7 | od -An -tx1'`
with stdout redirected to a host file, so execution was non-TTY. A guest
TTY has its own canonical line discipline that would apply the erase
itself, which says nothing about the host. The driver wrote
`y\nab\x7fc\n` in one write after `stty -icanon` had run. Redirected
stdout alone disables prompts, so this case shows the bytes reach the
guest unedited, not the ICANON gate. The `min` rows and the missing-image
row cover the gate.

macOS: profile `pft` with an explicit `localhost/pinfold/profile-default:latest`
image, and `pfb` with an unbuilt recipe `FROM` that image. Linux: profile
`pfl` with the recipe `FROM debian:trixie-slim`.

## macOS (Apple container, real PTYs)

| Case | Observed |
|---|---|
| Canonical: `pft`, `n` at the settings prompt | exit 125, no project config, stty unchanged |
| Canonical: Ctrl-C at the profile prompt | `pinfold: operation interrupted`, exit 130, no config, stty unchanged |
| Canonical: SIGTERM at the profile prompt | exit 143, no config, stty unchanged |
| Canonical: `pft`, Enter | config saved, `ran-42`, exit 0 |
| Canonical: `pfb`, Enter, `n` at `Build it now?` | exit 125 |
| `-icanon min 0 time 0`, no config | no prompt, `no-config`, exit 125, stty unchanged |
| `-icanon min 4 time 0`, no config | no prompt, `no-config`, exit 125, stty unchanged (`min=4` in both) |
| `-icanon min 0 time 0`, `pfb` project, image missing | no prompt, `image … is missing; run pinfold build --profile pfb`, exit 125, stty unchanged |
| `-icanon`, configured project, queued `y\nab\x7fc\n` | guest read `79 0a 61 62 7f 63 0a`, exit 0, stty unchanged |

After every TTY run of a configured project on macOS, including one with
no prompt, `stty -g` differed only by PENDIN (`lflag` 5cb to 200005cb).
This comes from the raw-mode restore of TTY execution, not from setup
prompts, and was not changed here. Linux showed no difference.

## Linux dev VM (Debian 13, x86_64, Firecracker)

| Case | Observed |
|---|---|
| Canonical: `pfl`, `n` | exit 125, no config, stty unchanged |
| Canonical: Ctrl-C at the profile prompt | exit 130, no config, stty unchanged |
| Canonical: SIGTERM at the profile prompt | exit 143, no config, stty unchanged |
| Canonical: `pfl`, Enter, Enter at `Build it now?` | `building profile pfl`, `ran-42`, exit 0, stty unchanged |
| `-icanon min 0 time 0` and `min 4 time 0`, no config | no prompt, `no-config`, exit 125, stty unchanged |
| `-icanon`, configured project, queued `y\nab\x7fc\n` | guest read `79 0a 61 62 7f 63 0a`, exit 0, stty unchanged |

The `pfl` image has no Git, so each run also printed
`sync-failed: … git: not found`. That warning is about the fixture.

## Not run

SIGTERM to a backgrounded pinfold during a prompt. With no `tcsetattr`
left, nothing is restored to stop it. The `no-config` diagnostic still
says "in a terminal", which is also true of a noncanonical one.

## Cleanup

macOS: `disk rm --all` under the private roots removed the one home
volume. No box was left running, and no image was built. The private roots
and projects were deleted. Lock files in the shared `/tmp/pinfold-501`
cannot be told apart from the user's and were left.

Linux: `disk rm --all` removed the home and project disks. The working
directory holding the sources, binary, private roots and image was deleted.
`/tmp/pinfold-1000` held only lock files created in this window, and was
removed. No firecracker process remained.
