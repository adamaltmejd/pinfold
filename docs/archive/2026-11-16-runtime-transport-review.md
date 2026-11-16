# Runtime transport review fixes

Follow-up to `34bb76a` and the [initial record](2026-11-16-runtime-transport.md).
Socket fixes: `933352d`. Apple fixes: `8668cf2`.
All A–G claims were checked against the pre-fix source. None was rejected.
G3 adds the operator-authorized post-ready diagnostic guarantee. No suite
test, dependency, image, release, push or PR was added.

## Changes and real-binary cases

A. Cleanup now renames a published directory, without replacement, to
`.d-<full-generation>` before unlinking. A complete matching marker or a
full-generation staging/retiring name authorizes deletion. An absent or
short marker is accepted only for those full-generation names. Socket
cleanup checks type and UID, without requiring a mode left by a completed
start. An unverifiable shortened name is preserved. After runtime absence
is confirmed under the name lock, its dead claim is retired independently
of socket cleanup failure. Linux checks the recorded VMM has ended.

Linux GDB killed real owners at the instruction before marker creation,
after the empty marker was opened, after marker unlink before rmdir, and
just after the atomic bind helper returned. Each prune and corrected start
succeeded. A preserved `34bb76a` binary was killed after bind before chmod;
its observed socket was 0755. Fixed prune and restart recovered it. Another
case put an unverifiable record first in directory iteration order: prune
reported it, retired both dead claims, cleaned the later valid directory,
and preserved the first directory's bytes and modes. A malformed owner JSON
case likewise allowed later valid cleanup, while preserving the corrupt
record and directory. Other ownership readers still fail closed. Unknown
generation ownership suppresses record-less sweeping until corrected.

Apple long-root lifecycle also killed its real owner, shortened its marker,
then pruned and started a fresh generation while the old directory remained
byte for byte. Restoring the fixture's complete marker allowed later cleanup.
A separate live witness remained usable throughout.

B. Bind runs in a syscall-only fork child with retained-directory cwd and
umask 0177. It creates the socket at 0600; the bind/chmod gap is deleted.
Validation uses retained descriptors and no-follow relative metadata. Linux
vsock connects through an O_PATH socket descriptor. The long-lived VMM
thread isolates its cwd and pins it to the retained vsock directory before
cloning the mount namespace; the child mounts that inherited cwd. This
avoids resolving a checked pathname again. The image-builder path uses the
same bind and jail primitives. Host-parent metadata must show root ownership
and either sticky mode or no group/other write permission.

Linux GDB replaced the generation pathname with a symlink after validation
before vsock connect: the real guest still answered and the target remained
unchanged. Two jail barriers replaced the source name before namespace clone
and inspected the resulting VMM root: its vsock mount had the retained
inode. Restoring the fixture name allowed startup and teardown. A private
user/mount namespace gave the real CLI root-owned /tmp mode 0777: it refused
runtime-path; correcting only the sticky bit permitted the same command.
No shared mount was changed.

C. Only typed socket-path errors become runtime-path. A corrupt ordinary
owner JSON produced a generic failed event, left its fixture bytes intact,
and allowed the corrected control. Exercised on Linux.

D. The shutdown listener is bound before publishing the owner record.
GDB held a real Linux owner immediately after publication and before socket
directory creation. A concurrent box down connected at that barrier, then
completed with owner exit 0 without readiness. A fresh start succeeded.

E. Maintenance measures generation directories, not state directories.
A Linux clean --dry-run report increased by exactly the fixtures' 92 bytes.
Public prune swept record-less full-generation staging/retiring directories
with absent/short markers, and a published directory with a complete marker.
An unverifiable shortened directory remained unchanged. Per-record failures
are reported and independent records continue. Normal prune JSON is retained.

F. The private Apple PTY is opened CLOEXEC using posix_openpt, grantpt,
unlockpt and a CLOEXEC slave open. The slave is raw before native spawn.
Relay notices and exposed CLI/restoration errors use non-panicking writes.
Observed native status survives relay errors and cancellation, including
while joining the worker. EIO/EPIPE to a closed private slave drops pending
input and disables writes. Caller non-TTY EPIPE closes that native stream.
After native exit, output drains to EOF or a one-second deadline.

Real Apple cases observed native exit 0 and 23 with kqueue while caller
output remained full, then signalled pinfold; both statuses survived.
Before this fix the same observed-0 case returned 143. A no-signal case
returned 23 despite undrained output. Forced-TTY immediate exits 0 and 23
also survived 1 MiB pending input. Closed stdout and stderr each returned
141 on both main809fc36 and the fixed binary. Full nonblocking diagnostic
stderr produced 125 without panic, followed by a successful control.
A real guest received CR, INTR and NUL bytes exactly. A libproc observer
confirmed a distinct private noncanonical/noecho/nosignals slave and a
CLOEXEC master. The native client subsequently adjusts some PTY flags;
initial raw setup and atomic descriptor creation were inspected in source.

G. Startup readiness observes child exit as well as stdout/stderr. Stdout
EOF or child exit starts a one-second failure bound. At ready, a dedicated
thread immediately drains stderr, even during synchronous finish_start.
Startup retains at most the first 64 KiB; after ready it retains the latest
64 KiB. Teardown closes and joins this drain before native reaping. Only
unexpected native exit prints the post-ready tail, escaped by terminal_text.
No native diagnostic bytes go to a file. No native Apple client holds a
caller-terminal descriptor. Supervision and README state these bounds.

Killing the real native run process at observed spawn before ready failed
startup and completed cleanup. A guest wrote 96 KiB through its console
stderr without blocking. Unexpected native death printed the escaped last
64 KiB including TAIL-END; requested down printed no NORMAL-TAIL. The real
native label parser's startup refusal still surfaced its diagnostic,
escaped newline/ESC, excluded the tail beyond 64 KiB, and wrote no diagnostic
to state files. Correcting the label reached ready and ended on stdin EOF.

## Reproduction and retained observations

Use fixture roots, a fresh maintenance stamp and the prepared images from
the initial record. Linux uses the static musl binary in the separate trial
checkout. The original dirty checkout is preserved and the lease retained.
Commands and key output are in [mac.txt](2026-11-16-runtime-transport-review/mac.txt),
[linux.txt](2026-11-16-runtime-transport-review/linux.txt) and
[checks.txt](2026-11-16-runtime-transport-review/checks.txt).

```sh
python3 crash-windows.py "$BIN" "$IMAGE"
python3 host-parent.py "$BIN"
cc fd-inspect.c -o /tmp/pfrt-review/fd-inspect
python3 apple-review.py "$BIN" "$IMAGE" "$MAIN809"
python3 lifecycle-review.py "$BIN" "$IMAGE"
```

The crash driver requires GDB and the preserved pre-fix binary at
/tmp/pfrt-review-baseline. Its generated debugger scripts use external
instruction barriers and SIGKILL, never sleeps or binary hooks. The full
Apple rerun also uses the initial record's foreground-case.py, tty-review.py,
terminal-cases.py, cases.py and native-startup.py commands. It passed all
terminal cases and long-root lifecycle/PR1 compatibility controls.
The prior record establishes that blocked-output cancellation hangs on
unmodified main809fc36; the fixed case exits 143 before output is drained.

Verification corrections: an initial Mac run lacked sandbox runtime access;
its foreground driver timed out before a handoff and cleanup failed. Its
isolated fixture was checked and cleaned, then the full run used native
runtime access. A first legacy GDB breakpoint was after chmod; moving it to
the actual post-bind/pre-chmod instruction exposed 0755. The first mounting
attempt through proc-fd after mount-namespace cloning returned EINVAL;
retained cwd before cloning was verified with the real VMM instead. These
failed observations supplied no pass.

Unverified: Apple instruction-level crash windows, initial PTY setup race,
and held-descendant stdout-EOF/stderr and post-exit endpoint windows. LLDB
could not launch here with Developer Mode disabled; the system setting was
left unchanged. Those exact windows were inspected, not exercised. The
post-exit deadline and pre-ready native death were exercised separately.
Image-builder changes were inspected, without rebuilding an image. Prefix
collision injection and terminal-restoration syscall failure injection
remain unverified. Socket capacity was measured in the prior record; its
fixed-path comparison remains inspected rather than overflow-injected.

Caller changes: restart can retire a dead unverifiable claim while preserving
its directory; prune reports independent errors and continues; generic
ownership failures keep generic handling. Apple completed status, SIGPIPE,
bounded output drain and unexpected-exit diagnostic tail now follow the
contracts above. No flags, success JSON shape or single-owner lifetime
changed. Fixture disks, including failed-run leftovers, were removed. No image was
created or removed; prepared images remain available. Final free space was
36 GiB on Mac Data and 24 GiB on Linux. Both fmt/strict Clippy gates passed.
