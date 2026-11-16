# Session departure review, #322, 2026-11-17

Implemented FIXES-1.md on `c303eff`. Source commits: `b239278` bounds the
Apple native bridge; `51c63bf` changes framed I/O, hangup and the spec.
No test, dependency, privilege or guest listener was added. Always applied
is byte-for-byte unchanged. No workload credential or login store was used.

## Changes

Apple project exec now watches native exit independently of stdout EOF.
One nonblocking bridge replaces its two blocking copies. Native stdout drains
to EOF or one second after native exit. Socket shutdown and cancellation
precede the worker join; an observed guest status wins later bridge failure.
Public Apple exec retains its existing native transport.

Both framed ends use nonblocking loops and bounded stream credits. Stdin
writes cannot block resize parsing. Closing a non-TTY consumer closes the
corresponding guest pipe reader, giving its writer SIGPIPE. One frame writer
handles guest output and EXIT. Child completion starts its one-second deadline
without acquiring an output mutex. Host credits rotate between streams and
alternate with queued input; continuous stdout cannot starve stderr or stdin.
Window-query errors propagate while an observed command status is retained.

A terminal departure closes the sole guest PTY master before TERM. The leader
stays unreaped through the two-second KILL deadline. PIDFD-open failure now
kills and reaps the already spawned group instead of dropping its Child.
The latter rollback is inspected, not fault-injected.

Removed duplicate copy_stream, wire re-exports and the dead public Apple
project-directory metadata/guard. Direct imports changed in the Linux VM,
image job and builder. README and ARCHITECTURE state the hangup limit and
one-second background-output rule. Public exec sends no signal on departure.

## Commands and results

Evidence: `2026-11-17-session-end-review/`, 4 scripts and selected outputs.
Each script uses isolated XDG roots, stamps maintenance before dispatch, uses
fixture values and removes its own box and two explicitly named disks.
The suite remains suspended. Commands were run through the real CLI.

Mac source: `RUSTC_WRAPPER= cargo build --locked`. Run each script with
`target/debug/pinfold localhost/pinfold/profile-default:latest`.
Linux source: archived main into `/tmp/pfr1.main`, candidate into
`/tmp/pfr1.candidate`; built with `cargo build --locked --target
x86_64-unknown-linux-musl`, sharing its disposable target cache. Run scripts
with that static binary and `localhost/pinfold/image-probe-session-review:latest`.
The existing `../2026-11-17-session-end/cases.py` was also rerun on both hosts.

1. `pipes.py`: finite stdout/stderr controls return 0. Closing either consumer
   after one `y` makes the candidate return 141 on both hosts. Main `7045e85`
   returned 141 on Mac but hung beyond six seconds on Linux, for both streams.
   Thus Linux parity with main is not established: this repairs its older
   contract violation. An unread stdout with a separately exiting leader keeps
   status 23 and returns after 1.00s Mac / 1.02s Linux. Reading the control gives
   exactly 1 MiB and status 23. Continuous stdout does not starve 1 MiB of stderr
   or a queued stdin marker. Later signals preserve observed 0 and 23.
2. `native.py` (Mac): a wrapper starts a real native client plus a descendant
   retaining stdout. Completed 0/23 survive that endpoint. Killing the native
   client while the descendant retains it returns project 125 in 1.05s. This is
   a host fault fixture; it does not replace the runtime or add a binary flag.
3. `window-fault.py` (Mac): the unarmed real ioctl resize control returns
   status 0. An interposer armed after guest readiness makes the real
   TIOCGWINSZ return EIO; the marker appears and project status is 125. Exact
   caller termios is restored in both. The binary is unchanged by the fixture.
4. `terminal.py`: a full-screen alternate-buffer/raw-input Perl program sees
   24x80, then 31x101, then 43x137 while a 384 KiB paste is blocked. Releasing
   its input yields exactly 393216 `x` bytes. Bash observes Ctrl-\\ as 131,
   Ctrl-Z stops a job, fg resumes it, Ctrl-C yields 130, and Ctrl-D exits 0.
   Caller termios matches after normal exit, client SIGTERM 143 and stop from
   another host process 125. A second session stays responsive until stop.
   Direct controlling-terminal closure ends Bash and its running/stopped jobs
   in 0.608s Mac / 0.340s Linux and saves the history marker, below the grace.
5. The prior departure matrix still passes: client INT/TERM/HUP/KILL, ignored
   TERM escalation, peer isolation, plain stdin EOF, normal background writers,
   completed-status races, public exec departure and the setsid limit. The final
   scheduling delta was rechecked with pipe controls on both hosts and the full
   Mac terminal matrix; the Linux terminal/departure matrices preceded that delta.

## Observer corrections and scope

Earlier fixture runs failed. The termios difference was Darwin's PENDIN state
bit, set when canonical mode is restored. A read-readiness query processes it
before the exact settings comparison; it changes no terminal setting. Apple's
[tty implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/tty.c)
sets PENDIN when enabling ICANON and clears it in ttnread/ttypend. The closure
case now runs the client directly as controlling-session leader; observer
parents are used only where post-exit settings must be read. History clearing
is a separate input line so it does not erase the marker. An early ioctl shim
recursed through dlsym and crashed; direct interposition passed the final app
fault/control case. Only final successful matrix output is retained.

The old mutex and blocked-write defects were verified by inspection, then
covered by the unread-output and blocked-paste cases. No claim of arbitrary
hostile guest containment or sustained-load throughput is made.

## Caller changes and cleanup

Linux now returns after delivering output for up to one second after command
exit, including public exec. Background writers no longer hold the client
indefinitely. Linux consumer closure now reaches the guest as SIGPIPE instead
of hanging. Terminal departure hangs up first, allowing Bash to save history
and notify job-control groups. A process leaving the command group, or for a
terminal session ignoring hangup after leaving it, may survive; stop ends all.
No box-spec field, JSON line, CLI flag or exit-code definition changed.

Fmt and strict clippy passed on Mac and Linux musl. Initial/final Mac disk:
56/50 GiB free. The fresh Debian 13 trial had 27 GiB free at completion. Its
small Bash/Perl/Git image built in 53 seconds; recipe, digest and command are
retained in checks.txt. All fixture boxes/disks were removed. The shared Mac
buildkit remains, with no Mac image created or removed. Linux evidence was
copied out before releasing this task's trial lease; no image remains there.

A separate public-Apple-framing ticket could delete
`core/runtime/apple/transport.rs`: its sole exported exec path serves public
Apple execution. That work must preserve public lifetime, terminal, stream and
status behavior. It was not moved in this change.
