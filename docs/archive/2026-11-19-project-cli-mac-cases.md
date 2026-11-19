# Project CLI re-review fixes and Mac cases, 2026-11-19

Base 339606e on `codex/project-cli-usability`; fixes in e14422b. A
re-review of 339606e listed five findings. Each was fixed. The base binary
was the control in every case below.

## Fixes

1. The bare `pinfold` overview uses the locator's recorded mount when the
   owner probe answers `config-changed`.
2. A setup prompt sets ICANON on a noncanonical terminal and restores the
   caller's settings when the prompt returns. Handled signals (INT, TERM,
   HUP) return through the prompt, so they restore too.
3. Image builds pass the context to the runtime as an absolute path.
4. A failed relay of a confirmed build's stderr is an error: setup exit
   125, or 128+n after a signal, and the command does not start.
5. `image build` tracks `--json` by option position, also when the
   invocation is malformed. An option's value is never the flag.

## Checks

On arm64 macOS: `cargo fmt --check`, `cargo clippy --all-targets --locked
-- -D warnings`, and the same Clippy for `--target
x86_64-unknown-linux-musl` with a `zig cc -target x86_64-linux-musl`
wrapper as ring's C compiler. All passed. On the dev VM, `cargo build
--locked --target x86_64-unknown-linux-musl` for base and fixed.

## Method

Debug binaries, private XDG config, state and cache roots, a temporary Git
project per case, `PINFOLD_CPUS=2 PINFOLD_MEMORY=1G`. A saved user profile
(`pfmac` on macOS, `pfl` on Linux) had a small Containerfile. Its resolver
stage takes `PINFOLD_AGENT_RESOLVE`, so every setup build reran it (about
12 s on macOS). The macOS profile was `FROM` the local default image. The
Linux profile was `FROM debian:trixie-slim` plus git.

Interactive cases ran through a Python `pty.fork` driver. The child ran
`sh -c 'trap : INT; stty ARGS; BIN …; stty -a'`, so the settings before
and after are recorded. The driver waited for each prompt, sent input
(optionally one byte at a time) and recorded the exit status. For finding
4, the child's stderr was a second PTY, and the driver closed its master
after `profile published` appeared. The project then had a project
Containerfile `FROM` the profile image, so `pinfold build` wrote one more
stderr line (`building project`) after the close.

## macOS (Apple container 1.7.1, real PTYs)

| Case | Base 339606e | Fixed |
|---|---|---|
| F1: share box kept by `start`, config edited to `disk`, bare `pinfold`, same `XDG_CACHE_HOME` | `Box files Shared…` | same |
| F1: as above, different `XDG_CACHE_HOME` | no `Box files` line; only `Files Stored on a private project disk` | `Box files Shared…`, then the disk next-launch line |
| F1 control: share config, both caches | — | `Box files Shared…` |
| F2: `-icanon min 0 time 0`, `default\n` one byte per 0.3 s at the profile prompt | after `d`, exit 125 | profile accepted; settings prompt; `n` declined, exit 125; settings after = before |
| F2: `-icanon min 0 time 0`, `pfmac\n` after 1.5 s, `y\n` after 1.5 s | `ran-42`, exit 0 | `ran-42`, exit 0 |
| F2: `-icanon min 4 time 0`, `PINFOLD_PROFILE=pfmac`, `y\n` at the settings prompt | no progress for 35 s; killed | `ran-42`, exit 0; `min = 4` restored |
| F2: `-icanon min 0 time 0`, Ctrl-C at the profile prompt | exit 130 | exit 130; `-icanon min = 0` restored |
| F3: `image build pfmacctx --containerfile Containerfile --context --json`, directory `--json` present | `Error: Unknown option '--json'`, build failed, exit 1 | `Built …:latest`; `cat /pf-marker` in the image printed the context file |
| F4: stderr PTY closed after `profile published` | project image built; command ran (marker file on the share), exit 0 | project image built; command did not run (no marker), exit 125 |
| F4 control: stderr PTY closed after `building profile`; no later stderr writes | exit 0, command ran | exit 0, command ran |
| F5: `image build demo --context --json` | JSON refusal on stdout | `pinfold image build: spec: image build needs --containerfile PATH` on stderr, nothing on stdout, exit 1 |
| F5: `--context . --bogus --json`; `--context . --json --json`; `image build --json` | JSON refusals | JSON refusals, exit 1 |

Cases not run on the Mac before, all on the fixed binary:

| Case | Observed |
|---|---|
| First launch, canonical: `pfmac`, `n` at the settings prompt | exit 125, no project config |
| First launch: Ctrl-C at the profile prompt | `pinfold: operation interrupted`, exit 130, no project config |
| First launch under `stty tostop`: `pfmac`, Enter, Enter at `Build it now?`, `sh -c 'echo ran-$((6*7))'` | `pinfold: building profile pfmac`, `ran-42` at 21.8 s, exit 0; config saved |
| Configured project, image removed, `stty tostop`: Enter at `Build it now?`, Ctrl-C 5 s after `building` | exit 130; command did not run; no box, no `pinfold` or `container build` process, no image |
| Overview with a running share box after the switch to disk | the F1 rows above; `stop` then exit 0 and the overview said `stopped` |

## Linux dev VM (Debian 13, x86_64, Firecracker)

Findings 2, 4 and 5 apply on Linux. Finding 1 needs a share and finding
3 an Apple build; neither was run here.

| Case | Base 339606e | Fixed |
|---|---|---|
| F2: `-icanon min 0 time 0`, `pfl\n` one byte per 0.4 s | after `p`, exit 125 | profile accepted; `n` declined, exit 125; settings restored |
| F2: `-icanon min 0 time 0`, whole lines after 1.5 s pauses through the build prompt (`n`) | exit 125 after the build prompt | same |
| F2: `-icanon min 4 time 0`, `PINFOLD_PROFILE=pfl`, `y\n` | no progress for 20 s; killed | build prompt; `n`, exit 125; `min = 4` restored |
| F2: `-icanon min 0 time 0`, Ctrl-C at the profile prompt | exit 130 | exit 130, settings restored |
| F4: stderr PTY closed after `profile published` and `building project` | `ran-42`, exit 0 at 76.7 s | exit 125 at 48.8 s, command did not run |
| F5: `image build demo --context --json` | JSON refusal on stdout | text refusal on stderr, exit 1 |
| F5 control: `--context . --bogus --json` | JSON refusal | JSON refusal |

The whole-line min 0 rows show that the zero-byte read needs a pause
inside a line: a pause before the first byte did not trigger it on either
platform.

## Not run

The finding 2 restore after SIGQUIT, SIGTSTP or SIGKILL. Those signals are
not handled by the client; a kill during a prompt leaves the terminal in
canonical mode. Finding 4 with a signal during a failed relay.

## Cleanup

macOS: `stop` removed the running box. `disk rm --all` under the private
roots removed the four home volumes. The `profile-pfmac`,
`project-accept-*` and `image-pfmacctx` tags were removed. The default
profile image and the user's volumes and boxes were untouched. The
temporary directories were deleted. Lock files in the shared
`/tmp/pinfold-501` cannot be told apart from the user's and were left.

Linux: `disk rm --all` removed the fixture's home and project disks. The
private roots holding the images, the sources, binaries and driver were
deleted. `/tmp/pinfold-1000` held only lock files created in this window;
they and the directory were removed. No firecracker or pinfold process
remained. Three `pgrep` wait loops from an earlier session were left
alone.
