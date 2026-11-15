# Runtime sockets and Apple terminal transport

Baseline `809fc36`; implementation `551341e`. Implementation is paused at the Apple blocked-output
acceptance failure below. The brief's completion gate has not passed.

## Implementation

Runtime sockets use canonical `/tmp/pinfold-<uid>` and
`r-<generation-prefix>/{generation,p.sock,v/}`. Linux uses only the three
vsock sockets in `v/`. The shutdown socket retains its hashed host path.
Checked directory descriptors retain identity. Components are opened without
following links; type, UID, exact modes and full generation are checked.
Publication uses exclusive rename from a full-generation staging directory.
Cleanup checks all entries before removing matching published or staged
directories. An incomplete or missing generation marker cannot authorize
deletion. Socket length checks use the actual platform address structure.

Socket identity now reaches preparation, proxy startup, VM startup, BOOT,
readiness, environment delivery, exec, stat and unmount. Failed starts,
down and pruning remove matching socket state after runtime removal.
The VMM receives only `v/`. Its final umask makes new sockets 0600.
State, configuration, images and records retain their XDG locations.
The separate image-builder paths were inspected, not rebuilt or exercised.

Apple TTY exec uses a private host PTY, a private controlling session and
foreground group, and piped native stderr. The relay handles caller input,
PTY output and native stderr with nonblocking I/O and cancellation. It
sizes before spawn and forwards SIGWINCH. Cancellation kills the native
host client, closes its master, then reaps it. Caller flags restore once;
restoration failure preserves observed nonzero status and fails success.
The project owner cancels and joins its TTY worker before returning.
Non-TTY execution retains its previous worker lifetime.

Deleted state-derived proxy/vsock path helpers and direct caller-terminal
inheritance in Apple TTY exec. Moved Linux's existing SIGWINCH self-pipe
into the shared terminal module. Updated README and the authorized spec
sentences and guarantee rows. Added no dependency or suite test.

Detached ownership, sessions, `c.sock`, joining, start/stop, comparison,
wire protocol and all owner-dependent section 13/14 wording remain for
the next PR. One foreground command still owns each box.

## Reproduction and observations

All commands use fixture configuration and isolated state with a fresh
maintenance stamp, no update check, the real cache, two CPUs and 1G memory.
No real credential or login was used. The initial mandated memory-registry
lookup preceded reading the brief's directory restriction; no auth file
was read. Previous archives and excluded paths were not modified.

Run these from the archive directory, substituting the local binary and
prepared image. Existing drivers are referenced rather than duplicated.

```sh
python3 lifecycle.py "$BIN" "$IMAGE"
python3 ../2026-11-15-project-command-rereview/foreground-case.py "$BIN" "$IMAGE"
python3 ../2026-11-15-project-command-review/tty-review.py "$BIN" "$IMAGE" fixed
python3 terminal-cases.py "$BIN" "$IMAGE" remaining
python3 ../2026-11-15-project-command-rereview/cases.py "$BIN" "$IMAGE" fixed
cc socket-capacity.c -o /tmp/pfrt-capacity
/tmp/pfrt-capacity
```

[mac.txt](2026-11-16-runtime-transport/mac.txt) and
[linux.txt](2026-11-16-runtime-transport/linux.txt) retain output excerpts.
Both hosts passed a 218/210-byte state root, environment delivery, exec,
stat, stop, persistent restart, owner SIGKILL, unsafe-path refusal, prune
and corrected restart. Symlink targets retained their bytes. A live
witness's socket identity and exec survived cleanup. Public stdin EOF
ended its owner. Linux's jail held only `v.sock`, `v.sock_1000` and
`v.sock_3128`; its shutdown socket was outside. Attachment 15 refused;
14 ran. The unchanged PR 1 driver passed argv, cwd, statuses and separate
binary streams on both hosts.

Both hosts preserved the foreground helper's complete termios after
client SIGTERM, exit 143, with SIGTTOU default. Ordinary cancellation,
already-raw input, redirected forced-TTY exec, zero size, host raw-setup
refusal and guest ioctl refusal passed, with working setup controls and
no refused CMD marker. Darwin refusal used a process-local sandbox denying
only its SDK's three terminal-setting ioctls; Linux used the existing
seccomp driver. First-instruction dimensions and TERM/COLORTERM, live
resize and redirected-output host-stty preservation also passed.

Linux's full `terminal-cases.py` passed blocked-output cancellation.
Apple's full case failed. After closing the exec master before reaping,
the exec relay ended, but teardown still waited for `container run`.
[blocked.txt](2026-11-16-runtime-transport/blocked.txt) retains the native
stack: `ProgressBar.restoreCursorAtExit` reaches `NSFileHandle.write` and
blocks on inherited caller stderr, despite `--progress none`.
Reproduce with `terminal-cases.py "$BIN" "$IMAGE" diagnose`; the diagnostic
observer samples the remaining direct native child into the scratch root.
This case has not passed and must not be called verified cancellation.

The brief says to stop if the plan cannot meet its requirements. Extending
the fix to pipe the box-owning native client's stderr, retain bounded
startup diagnostics in memory, and close that pipe during cancellation
needs the operator's scope decision. No such extension was implemented.

Initial harness corrections: SIGKILL must not wait on pipes held by the
surviving Apple client; remove the native orphan before testing its unsafe
socket record. The first Linux start exposed v.sock mode 0700 under umask
077; final pre-exec umask 0177 fixed it. Only that reaped fixture's socket
was corrected manually for cleanup. Final Linux cases used fresh starts.
Darwin's blocked-output observer uses kqueue, not Linux pidfds or a fixed
queue-size threshold. No timing retry supplied a passing acceptance claim.

## Checks, limits and caller changes

[checks.txt](2026-11-16-runtime-transport/checks.txt) records the build and
strict Clippy gates. Linux used the static x64 musl binary on the active
trial lease, in a separate checkout preserving the existing dirty clone.
The lease was retained. No image was created or removed. Existing images:
Mac `localhost/pinfold/profile-default:latest`; Linux
`localhost/pinfold/image-probe-commands:latest`. Disposable disks were
removed, including failed-harness leftovers.

Unverified: Apple blocked-output completion, publication-boundary crashes,
short-prefix collision injection and restoration-syscall failure injection.
The C observer measured capacities 104 on Mac and 108 on Linux; the
binary's capacity comparison was inspected, not forced past a fixed path.
No release, push or PR was made.

Caller changes: new refusal/diagnostic `runtime-path`; internal runtime
socket paths leave XDG state; checked socket modes become exact; Apple's
TTY native client receives a private terminal. CLI flags, successful JSON
shapes, public exit-code rules and single-owner lifetime retain their
contracts. The blocked-output completion requirement remains unmet.
