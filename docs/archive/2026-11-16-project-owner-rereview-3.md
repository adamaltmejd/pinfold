# Detached project owner: review round 4

2026-11-16. Baseline `6ce7137`, branch `project-owner-276`. All three
FIXES-4 findings are valid, including the inferred terminal overwrite and
helper-descendant findings. No accepted round-3 wait or deadline was changed.

## Findings and changes

1. `Box::down(None)` could synthesize another audit terminal after a successful
   write followed by a failed `ownership.finish`. The atomic writer replaces
   the old file. `5701d83` retains a terminal-write-attempt flag across retries,
   set before the writer runs. A failed first write also consumes the attempt.
2. The Linux fork-only preflight probe inherited owner descriptors and blocked
   in its report read and final wait. `7fd71e5` leaves only null stdio and its
   two private pipes in the child. Report reads and pidfd readiness now observe
   startup cancellation. Error and cancellation paths kill and reap the probe.
   Actual VMM report, identity gate and start-receiver behavior is unchanged.
3. Captured helpers were reaped before pipe draining. Their guard then skipped
   group cleanup, leaving pipe-holding descendants alive. `a49f833` observes
   exit with `waitid(WNOWAIT)`, signals the group while its leader remains
   unreaped, then reaps. `ECHILD` disarms signaling because ownership is lost.
   The private child and common spawn method replace exposed child/reaped
   fields and repeated group setup. The five users are process capture,
   crossing receive/update-ref, Apple seed copy and artifact extraction.
   Existing status precedence and bounded draining remain unchanged.

These implement the existing terminal-once and helper-cleanup contracts.
No spec change, new dependency, public flag, JSON field, exit convention or
wire change. No suite test, release, push or PR. No plan departure.

## Real-binary cases

Retained recipes and output are in [the evidence directory](2026-11-16-project-owner-rereview-3/).
Use `D=docs/archive/2026-11-16-project-owner-rereview-3`, `BIN` as
`target/debug/pinfold` on Mac or
`target/x86_64-unknown-linux-musl/debug/pinfold` on Linux. Mac `IMAGE` is
`localhost/pinfold/profile-default:latest`; Linux `IMAGE` is
`localhost/pinfold/image-probe-owner-rereview:latest`.

```sh
# Both hosts
python3 "$D/helper-descendants.py" "$BIN" "$IMAGE" all
python3 "$D/crossing-control.py" "$BIN" "$IMAGE"
# Linux only
python3 "$D/probe.py" "$BIN" "$IMAGE"
python3 "$D/terminal-retry.py" "$BIN" "$IMAGE"
```

`terminal-retry.py`: a real route request stays pending while the fixture
makes its own audit log unwritable. At the post-terminal/pre-finish breakpoint,
the debugger replaces only this run's owner-record path with a marked directory.
Finish fails. The outer owner-cleanup breakpoint restores the exact record
before retry. Linux output records one writer call, an observed retry, and
unchanged terminal timestamp, bytes and inode. The successful command stays 0;
its diagnostics include audit-log and teardown-failed. Ordinary audit departure
has one writer call without teardown-failed. Both cases permit the next start.

`probe.py`: root GDB follows the namespace clone, but an exec wrapper runs
pinfold as the ordinary user. The driver checks the owner's UID. After child
FD isolation, it stops the actual probe with SIGSTOP and lists its descriptors:
0–2 are `/dev/null`; 3–4 are distinct pipes. No lock, lease, proxy or control
FD remains. Stop returns 0 in 0.07 seconds; the starter returns 125/stopped,
CMD is absent, and pidfds report probe and owner exit. Locator and claim are
absent. The next ordinary start returns PROBE-CONTROL.

`helper-descendants.py`: a PATH Git stand-in delegates every operation except
the chosen host symbolic-ref. That operation gets the real branch, forks a
child holding stdout/stderr, and exits 0. Its child remains in its process
group. Normal drain expiry kills the child and permits DRAIN-CONTROL/0.
Stop and starter SIGTERM during draining produce 125 and 143 respectively,
without CMD. Kernel exit observers confirm descendant and owner exit; each
case then runs REAL-GIT-CONTROL with real Git.

Linux GDB gates after the capture loop observes exit and checks the leader
is still a zombie. On Mac, a stopped owner lets the descendant fill its private
stderr pipe to EAGAIN (65,536 bytes). After resume, a further blocking write of
131,073 bytes completes. This forces more than the capture loop's 64 KiB drain
quantum, proving another iteration after leader exit. The leader is still a
zombie and the descendant alive before cancellation. Final Mac stop/signal
cases take 1.57/1.45 seconds; Linux 1.32/1.32 seconds. These include draining
and teardown, not only the cancellation check interval.

`crossing-control.py`: both hosts pass project push/fetch and public box fetch.
The guest HEAD equals the independently read host remote ref, and generation
stays unchanged. Every driver removes its own boxes and disks. Retained output
is `mac.txt` and `linux.txt`; final Linux completion markers are present.

## Checks, failed attempts and limits

`checks.txt` retains fmt, strict all-target clippy and locked build outcomes
on Mac and Linux musl. All pass. The six changed Rust source files have identical
SHA-256 digests across the hosts. Every Mac build checked Data-volume free space
first; the final build had 15 GiB available. Final free space is 22 GiB on Mac
and 25 GiB on Linux.

Initial checks found stale imports and a missing Linux-only trait import;
these were corrected. Two initial Linux helper-signal driver attempts failed:
GDB stopped on SIGPIPE, and late waitpid collection lost the detached starter's
status. The final driver passes SIGPIPE and starts the status waiter immediately
after detachment. It asserts the collected status 143. Those failed attempts
are not acceptance passes. `attempts.txt` retains diagnostic excerpts.

Only inspected: first terminal-write failure also consumes the attempt;
Mac's exact post-terminal finish-failure window; the probe's final wait after
report EOF; exceptional ECHILD disarming; artifact extraction and Apple seed-copy
failure paths after their guard migration. The exact terminal retry is exercised
on Linux through shared code. No escaped descendant that calls setsid is claimed.
No old crash matrix or accepted round-3 long-wait cases were rerun.

No credentials were used. All state/config roots are private scratch roots,
with fresh maintenance stamps and the real image cache. Existing images were
reused; no image was built, changed or removed. Both named images remain. The
VM lease remains active and was not released. Earlier archives and excluded
paths were untouched. Instruction files remain untracked.
