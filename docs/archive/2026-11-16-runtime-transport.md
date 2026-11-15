# Runtime sockets and Apple terminal transport

Baseline `809fc36`; initial implementation `551341e`, initial record
`0b8b235`. The operator approved the native-stderr extension recorded here.
The full Apple rerun and both platform Clippy gates pass.

## Implementation

Runtime sockets use canonical `/tmp/pinfold-<uid>` and
`r-<generation-prefix>/{generation,p.sock,v/}`. Linux uses only the three
vsock sockets in `v/`; shutdown retains its hashed host path. Retained
no-follow directory descriptors check identity, type, UID, exact modes
and full generation. Exclusive rename publishes a marked staging directory.
Cleanup checks entries before removing matching published or staged paths;
an incomplete marker cannot authorize deletion. Length checks use the
platform's actual address structure, including suffixes and terminator.

Socket identity reaches preparation, proxy startup, VM startup, BOOT,
readiness, environment delivery, exec, stat and unmount. Failed starts,
down and pruning remove matching socket state after runtime removal.
The VMM receives only `v/`; final umask 0177 creates its sockets as 0600.
VM files and records remain under XDG. The separate image-builder paths
were inspected, not rebuilt or exercised.

Apple TTY exec gives native exec a private PTY, controlling session and
foreground group. Pinfold relays caller input, private output and piped
native stderr, sizes before spawn and forwards SIGWINCH. Non-TTY exec
uses separate stdin/stdout/stderr pipes through the same cancellable relay.
Input EPIPE closes that pipe and preserves command completion; stdout
closure does not end input delivery. Cancellation kills the native host
client, closes relay endpoints and reaps it before restoring caller flags.
Completed-exec priority, foreground checks and restore-once remain intact.

The box-owning `container run` now also gets piped stderr. Readiness drains
it concurrently with stdout, retaining only the first 64 KiB in a fixed
capacity memory buffer. Startup failure escapes those bytes through the
shared `terminal_text` helper before surfacing them. Successful readiness
discards captured diagnostics; normal monitoring drains later stderr
without retaining it. Cancellation and teardown synchronously close the
reader before native reaping. No diagnostic bytes are written to a file.
No native Apple client inherits a caller-terminal descriptor.

Deleted state-derived socket helpers and direct Apple exec stdio
inheritance. Renamed the PTY relay to `transport` to share its pipe path;
Linux's SIGWINCH self-pipe moved into the shared terminal module. README,
Supervision and the authorized socket/guarantee clauses state this contract.
Added no dependency or suite test. Detached ownership, sessions, `c.sock`,
joining, start/stop, comparison, protocol and owner-dependent section 13/14
wording remain for the next PR. One foreground command owns each box.

## Reproduction and observations

Cases use fixture configuration, isolated state, a fresh maintenance stamp,
no update check, the real cache, two CPUs and 1G memory. No real credential
or login was used. The initial mandated memory-registry lookup preceded
reading the brief's directory restriction; no auth file was read. Excluded
repository paths and previous archives were not modified.

From this archive directory, use the local binary and prepared image:

```sh
python3 lifecycle.py "$BIN" "$IMAGE"
python3 ../2026-11-15-project-command-rereview/foreground-case.py "$BIN" "$IMAGE"
python3 ../2026-11-15-project-command-review/tty-review.py "$BIN" "$IMAGE" fixed
python3 terminal-cases.py "$BIN" "$IMAGE"
python3 ../2026-11-15-project-command-rereview/cases.py "$BIN" "$IMAGE" fixed
python3 native-startup.py "$BIN" "$IMAGE"
cc socket-capacity.c -o /tmp/pfrt-capacity
/tmp/pfrt-capacity
```

Before the extension, an unmodified `809fc36` binary was built from its
source and embedded profile files under `/tmp/pfrt-main/src`. Its blocked
PTY output case also hung: after CMD readiness and SIGTERM, kqueue observed
no client exit by the 60-second deadline. Thus the hang exists on main.
[blocked.txt](2026-11-16-runtime-transport/blocked.txt) retains that result
and the earlier transport implementation's native stack: despite
`--progress none`, `ProgressBar.restoreCursorAtExit` blocks in stderr write.
The first observer's `diagnose` mode accidentally consulted the command's
argv rather than its own mode, so this baseline used the full deadline;
the mode selection is corrected. No baseline sample is claimed.

[mac.txt](2026-11-16-runtime-transport/mac.txt) contains the complete final
Apple rerun's excerpts. Blocked-output cancellation now exits 143 before
draining and restores owned settings. Foreground handoff preserves the
helper's complete termios with SIGTTOU default. Ordinary cancellation,
already-raw input, redirected forced-TTY exec, zero size, host raw refusal,
guest ioctl refusal, first-instruction dimensions and TERM/COLORTERM,
live resize and redirected-output host-stty preservation pass. Refusals
retain working controls and dispatch no CMD marker. Darwin raw refusal
uses a process-local sandbox denying only its three SDK terminal-setting
ioctls; Linux uses the existing seccomp driver.

[linux.txt](2026-11-16-runtime-transport/linux.txt) retains the prior full
Linux run. Both hosts passed long state roots (Mac 218 bytes, Linux 210),
environment delivery, exec, stat, stop/unmount, persistent restart, owner
SIGKILL, unsafe-path refusal, prune and corrected restart. Symlink targets
retained their bytes. A live witness's socket identity and exec survived
cleanup. Public stdin EOF ended its owner. Linux's jail held only the three
vsock sockets, with shutdown outside. Attachment 15 refused; 14 ran. PR 1
argv/cwd/status/binary-stream cases passed on both, and were rerun on Apple.

The real native label parser supplies startup failure: an invalid label
containing newline/ESC and 96 KiB of fixture text. `native-startup.py`
captures the resulting user-facing failure only in memory. Its retained
output reports that the diagnostic surfaced, controls were escaped, the
64 KiB bound excluded its tail, and state held no diagnostic. Correcting
the same label starts and tears down successfully. It also verifies exit
23 despite 1 MiB unread stdin, and full input delivery after stdout closes.
Raw startup diagnostic contents are not retained in the evidence files.

Initial corrections: Apple SIGKILL must not wait on native-retained pipes;
remove its native orphan before testing unsafe socket cleanup. Linux's
first v.sock was 0700 under umask 077; final pre-exec umask 0177 fixed it.
Only that reaped fixture's socket was manually corrected for cleanup.
Final Linux starts used fresh directories. No timing retry supplied a pass.

## Checks, limits and caller changes

[checks.txt](2026-11-16-runtime-transport/checks.txt) records builds, fmt,
strict all-target Clippy on Mac and Linux musl, startup assertions and
cleanup. Linux's separate checkout preserved the pre-existing dirty clone.
The active lease is retained. No image was created or removed. Prepared
images remain `localhost/pinfold/profile-default:latest` on Mac and
`localhost/pinfold/image-probe-commands:latest` on Linux. Fixture disks,
including failed-run leftovers, were removed. No release, push or PR.

Unverified: publication-boundary crashes, shortened-prefix collision
injection and terminal-restoration syscall failure injection. The C
observer measured socket capacities 104/108 on Mac/Linux; the binary's
comparison was inspected, not forced beyond its fixed paths.

Caller changes: `runtime-path`; internal sockets leave XDG state; exact
socket modes; private Apple TTY transport and separate non-TTY pipes;
startup native diagnostics are bounded, escaped and surfaced with failure,
while post-ready native stderr is drained rather than forwarded. This last
change is the approved extension. Flags, success JSON shapes, exit-code
rules and the single-owner lifetime retain their contracts.
