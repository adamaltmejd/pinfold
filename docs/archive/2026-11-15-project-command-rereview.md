# Project-command re-review, 2026-11-15

Baseline: `eb46372`. Source and specification fixes: `b800a03`.
All three supplied findings are valid. No finding was skipped as false.
The additional Apple raw foreground-handoff case below still fails.

1. Main saved and restored termios even when stdout was redirected and
   execution never entered raw mode. Main now drains a snapshot owned by
   successful raw setup. The guard and main share that snapshot, restore
   once, check foreground ownership before writing, and prevent raw setup
   after cancellation. If the incoming terminal is already raw, no changed
   state is owned. Foreground checking and setting are separate syscalls.
2. The unbiased select could choose shutdown while the exec handle was
   ready. Exec is now the first branch in a biased select. The source
   defect is confirmed; the baseline scheduling case did not reproduce
   the wrong status. The fixed real binary retained 23 and 127 with a
   completed exec worker and a public down request pending.
3. The component walk used O_RDONLY. It now uses O_PATH in the Linux
   guests on both runtimes, with O_DIRECTORY, O_NOFOLLOW and descriptor
   entry. The host-only Darwin compilation uses O_SEARCH. The ordinary
   directory-population helper is unchanged.

`cases.py BIN IMAGE baseline|fixed` drives a private PTY and disposable Git
project. With terminal stdin and piped stdout, CMD announces readiness,
the host changes stty to -echo, and public box exec releases CMD's FIFO.
On macOS and Linux, baseline returned 0 but overwrote the later setting;
fixed returned 0 and retained it. Baseline execute-only cwd returned 125
with Permission denied; fixed returned 0 and printed ENTERED. A symlink
planted inside the guest to /home/pi still refused as working-directory
125; its replacement by a plain directory succeeded. Exact commands and
output: [mac-cases.txt](2026-11-15-project-command-rereview/mac-cases.txt)
and [linux-cases.txt](2026-11-15-project-command-rereview/linux-cases.txt).

`completed-exec.py BIN IMAGE baseline|fixed` uses Linux ptrace to stop only
the owner's main thread. It releases CMD, observes the selected exec
worker change from socket wait to futex with exec-input gone, sends public
box down, observes its reply wait, then resumes main. Both baseline and
fixed preserved 23 and 127; down returned 0. These are external scheduling
observations, not direct inspection of Tokio's ready queue. Apple did not
run this Linux-specific observer. See
[linux-scheduling.txt](2026-11-15-project-command-rereview/linux-scheduling.txt).

The existing `../2026-11-15-project-command-review/tty-review.py` was run
against each fixed binary. Foreground SIGTERM returned 143 with sane,
restored settings. Linux blocked-output cancellation did too. Forced TTY
with redirected stdout, zero-size input and guest ioctl refusal passed
on both; Linux raw-setup refusal dispatched no marker and preserved
public box exec's exit 1. See
[terminal.txt](2026-11-15-project-command-rereview/terminal.txt).

Follow-up ticket: Apple foreground handoff, guarantee 39.
`foreground-case.py BIN IMAGE` lets CMD announce readiness, hands the
terminal to a helper group, installs canonical -echo settings there, and
signals only host pinfold. Linux returned 143 and left the helper's full
settings unchanged, with SIGTTOU default. Apple returned 143 but removed
OPOST, ICANON and ISIG. This is not Darwin PENDIN. See
[mac-foreground.txt](2026-11-15-project-command-rereview/mac-foreground.txt).
Pinfold's own restore skips this background group. The independent native
`container exec -t` client still controls the same terminal; its help has
no raw-mode suppression option. Installed-version source confirms
[ProcessIO creates a raw console and resets it on close](https://github.com/apple/container/blob/1.7.1/Sources/Services/ContainerAPIService/Client/ProcessIO.swift).
[Terminal.reset unconditionally writes its saved termios](https://github.com/apple/containerization/blob/0.54.0/Sources/ContainerizationOS/Terminal.swift).
Proposed refactor: give native exec a private host PTY, with
pinfold owning the caller's raw mode, byte relay and resize forwarding.
This changes the Apple transport and needs design review under AGENTS.md's
stop-before-refactor rule. It was not bolted onto these fixes. Whole-runtime
Apple foreground preservation is not established.

Both platforms passed cargo fmt --check and strict all-targets Clippy;
Linux used x86_64-unknown-linux-musl. Commands and retained completion
lines: [checks.txt](2026-11-15-project-command-rereview/checks.txt).
The first Linux lease expired before output retention; a fresh three-hour
lease repeated the cases and checks. No expired-lease result is relied on.
The initial Apple extra-terminal run lacked sandbox access and refused
image lookup; the authorized rerun passed. An earlier input-release
observer blocked on Apple; the retained FIFO observer replaced it.
Disposable disks were removed. No Apple image was created. Linux retained
the prepared `localhost/pinfold/image-probe-commands:latest` image and the
active lease. Free space was 53 GiB on macOS Data and 25 GiB on Linux.
No suite test, dependency, push, PR or release was added.
