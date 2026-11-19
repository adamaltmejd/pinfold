# Project CLI review fixes, 2026-11-19

Base 910a557 on `codex/project-cli-usability`; fixes in 643d5c9. An
independent review listed seven findings. Each was fixed. The base binary
was the control for the cases it could show.

## Checks

On arm64 macOS: `cargo fmt --check`, `cargo clippy --all-targets --locked
-- -D warnings`, and the same Clippy for `--target
x86_64-unknown-linux-musl` with a Zig `cc` wrapper as ring's C compiler.
All passed. On the Linux dev VM (Debian 13, x86_64, user in `kvm`):
`cargo build --locked --target x86_64-unknown-linux-musl`, fmt, native
Clippy and musl Clippy. All passed.

## macOS

The sandbox refused Apple `container` ("Operation not permitted") and
pseudo-terminal allocation (`openpty: Operation not permitted`). No box,
image build or interactive prompt ran on macOS. The cases below used a
logging shell stand-in named `container`, first on `PATH`, with private XDG
roots and a temporary Git project. The project was configured with
`VISUAL=true pinfold config edit`.

| Case | Base 910a557 | Fixed |
|---|---|---|
| `doctor --json`, stand-in fails `--version` (preflight) | exit 0; `image inspect` called once; `config.image_built` false | exit 0; no `image inspect`; `image_built` and `image_error` null |
| `doctor --json`, healthy stand-in, image absent | `image inspect` of the profile image twice | once; same findings, `image_built` false |
| `config` and `config --json` | — | one `image inspect` each; human report says "missing" |
| `image build pftest --containerfile Containerfile --context --json`, directory `--json` present | trailing `--json` stripped, refused JSON "--context needs a directory", exit 1 | context `--json` passed to the build, human "build failed", exit 1 (stand-in build fails) |
| `--context . --bogus --json` | — | JSON refusal, exit 1 |
| `--context . --json --json` | — | JSON refusal naming the second `--json`, exit 1 |
| `image rm --json` / `image rm` | — | JSON / text refusal "needs an image name", exit 1 |

A stand-in build received the context as a bare last argument, so a
context named `--json` would reach `container build` looking like an
option. This is unchanged from main and was not fixed here.

Not run on macOS: findings 1–4 (running owner, consent prompt, setup build,
prompt reads) and the non-UTF-8 directory case (APFS refuses such names).

## Linux dev VM

Base and fixed musl debug binaries. Each case used private XDG roots
(`PINFOLD_CPUS=2`, `PINFOLD_MEMORY=1G`) and a fresh Git project.
Interactive cases ran through a Python `pty.fork` driver. It ran `stty
ARGS` in the child, then the binary, sent the given input on each prompt
and recorded the transcript and exit status.

| Case | Base 910a557 | Fixed |
|---|---|---|
| `stty -icanon min 1`; `default\ny\nn\n` written at once at the profile prompt | one read took all three lines; refused profile name `"default\ny\nn"`, exit 125, no config | three prompts each took one line; build declined, exit 125, config saved, command not run |
| `-icanon`; `default\n`, then `n\n`, one byte per 0.2 s | `expected a complete line of terminal input` after `d`, exit 125 | profile accepted; setup declined, exit 125, no config |
| `-icanon`; Ctrl-C at the profile prompt | — | exit 130, no config root |
| canonical; Ctrl-D at the profile prompt | — | exit 125, no config root |
| `stty tostop`; Enter at all three prompts | after 3 min the client was sleeping and `pf-base build` was in state `T` (stopped); killed by hand | — |
| `stty tostop`; confirm the build, Ctrl-C after `pinfold: building` | — | exit 130; no firecracker or build process left; no image |
| `stty tostop`; Enter at all three prompts, `sh -c 'echo ran-$((6*7))'` | — | build output relayed, `ran-42`, exit 0, 159 s with a cold cache |
| `doctor --json` in a non-Git directory named `bad\377` | panic at cli.rs:1205, exit 101 | `path-not-utf8: working directory … is not valid UTF-8`, exit 1 |

The setup build's stderr is now a pipe, not the terminal, so the build
sees no terminal. Its output above was still complete.

The branch's recorded cases on the fixed binary:

- In a fresh project, bare `pinfold` showed the disk preview and left the
  XDG roots absent. `pinfold --` exited 125. A redirected `-- nonsense`
  exited 125 with no-config. `config` exited 1 with no-config. `doctor`
  led with the no-config finding. `typo` printed the help pointer and
  `box exec abc` printed leaf usage, both exit 1.
- After the confirmed build above, `start` exited 0. Bare `pinfold` showed
  `Box files Stored on a private project disk…`, then "The next launch
  uses these settings." and the configured settings. With `PATH=/nonexistent
  PINFOLD_PROFILE=full` it showed the full override and the saved default
  under that label. `PINFOLD_PROFILE=full pinfold -- sh -c 'echo
  UNEXPECTED'` exited 125 with the profile conflict. The command did not
  run.
- Separate stdout and stderr passed through. `exit 23` returned 23. After
  a host commit, `push` printed one host-to-box ref and `push --json`
  printed the pushed event. After a guest commit, `fetch` printed two
  box-to-host refs. `stop` exited 0 and bare `pinfold` then said stopped.
- With the file edited to `share`, `push` exited 1 with nothing on stdout
  and the spec refusal as text on stderr.
- `image build pfrfix` from the profile image printed `Built …`. With
  `--json` it printed the built event. With `--context --json` (directory
  `--json`) it built and printed human output. `image rm pfrfix --json`
  printed the removed event.

Linux refuses shares, so finding 1's share-to-disk mismatch was not shown.
The overview on Linux shows the live line and the next-launch label. A
`doctor` preflight failure on Linux still probes images (by spec), so that
skip was exercised only with the macOS stand-in.

## Cleanup

On the VM: `disk rm --all` removed the fixture's home and project disks.
The private roots holding the images were deleted, along with
`/tmp/pinfold-1000` (lock files from these runs only), the checkout,
binaries and driver. No firecracker or pinfold process remained. Three
older `pgrep` wait loops belonging to an earlier session were left alone.
On macOS, only temporary directories and the stand-in were created, and
they were removed.
