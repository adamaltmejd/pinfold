# Issue 89 final implementation review

This is a targeted correctness, simplification and test review of the work
being proposed for main, not a new release or a whole-tree release audit.
Base: `4b8141cedd6b63e118b39fe6b10027e8075777a3`.
Implementation candidate: `d27e757` on `codex/issue-89-final`.
Issue #89 remains open. The stock runtime's storage contention is not fixed.

## Scope and leanness

Three independent reviews covered runtime inventory and process ownership;
acknowledged teardown and guarantee 9's tests; and runtime version admission,
doctor, documentation and Linux provisioning. The cancellation correction
received a further independent review.

The implementation retains:

- One shared bounded box-inventory helper, with buffered output, a
  20-second deadline, client process-group cleanup and direct-child reaping.
- A successful owner teardown acknowledgement that avoids a redundant
  inventory; disconnects and unsuccessful teardown retain recovery checks.
- Stable Podman 6.1.4 and Apple container 1.5.0 startup floors, using
  existing preflight queries and one shared numeric comparison.
- Explicit cancellation of a pending standalone `box list` through the
  existing signal, worker and cancellation primitives.

No dependency, runtime service, per-box store, image-preparation policy,
Podman fork or storage scheduler was added. Pinfold does not install or
upgrade runtimes. CI builds the unchanged pinned Podman release because
the hosted runner's packaged runtime is below the requested minimum.
Experimental API and mapped-storage implementations are absent from the
production diff. Investigation records are preserved as dated evidence.

The reviews found no justified deletion from the remaining mechanisms.
The unreaped process-group leader and WNOWAIT observer prevent PID-reuse
hazards; removing them would weaken cleanup. The repeated post-stall image
identity assertion was retained as preservation evidence, not cut merely
to reduce line count. No controls or guarantees were removed.

Relative to main, Rust production source adds 301 lines and removes 81
(net +220); the existing lifecycle test adds 254 and removes 8
(net +246). Cargo manifests and lockfile are unchanged. The largest Rust
production file is `cli.rs`, 1,125 lines. ARCHITECTURE.md grows from 1,004
to 1,015 lines. No new written tickets were opened by this targeted review.

## Review finding and correction

Review identified a reproducible SIGINT regression: the new runtime
process group did not receive terminal cancellation, while the standalone
Pinfold caller still exited immediately. In a disposable native-overlay
store with the real image lock held, native Podman exited on SIGINT and
removed its waiter. Candidate `cdf2c419` exited on SIGINT but left its
runtime waiter behind. The observer removed only that owned waiter.

`box list` now registers existing SIGINT/SIGTERM handling before starting
the inventory worker. Cancellation sets an atomic flag and awaits worker
cleanup before returning 128 plus the signal. Pipe reads, poll waits and
exit-observer waits check cancellation at bounded intervals. Existing
owner and maintenance callers pass no cancellation flag and retain their
signal behavior. This does not promise general signal supervision for
every CLI command or cancellation after inventory output has begun.

## Test audit

The existing guarantee-9 test retains the real POSIX image lock, a fresh
inventory caller, timeout reason and empty output, waiter removal, and
successful recovery with the same live box and owner. Its acknowledgement
scenario stops the requester before the reply, reaps the owner, and then
holds the image lock while allowing the requester to resume.

The new cancellation block waits for the runtime child's actual kernel
lock-wait record before sending SIGINT or SIGTERM to the caller's process
group. It requires handled exits 130/143, empty stdout and no remaining
waiter. Existing post-unlock checks provide the positive control. The
comment names restoring default signal handling as the sabotage. Exit
codes and behavior come from the spec; waiter identity comes from the
kernel. No new test function, fake runtime or production test seam was added.

## Validation

Formatting, strict all-target native Clippy, shellcheck, shfmt with the
repository's two-space indentation, shell syntax and whitespace checks
passed. The Linux x64 musl binary built with the installed Zig toolchain.
The first strict-Clippy attempt hit the default Zig cache permission
boundary; using a writable disposable cache passed without code changes.

The actual Linux CLI on Yard passed:

| Condition | Exit | Stdout | Runtime waiter after exit |
|---|---:|---|---|
| SIGINT while waiting | 130 | empty | absent |
| SIGTERM while waiting | 143 | empty | absent |
| 20-second deadline | 1 | empty | absent |

The deadline result took 20.004 seconds and reported `runtime-timeout`.
The probe used private graph/run roots, no images and no active-store lock.
Before removal, two private overlay-directory bind mounts were explicitly
unmounted; a fresh process/descriptor/mount scan then found no references.

Linux binary SHA-256:
`bfbed9a7cc80ad9366921fedcc014c3540282807b5c6855562721cc156001b97`.
Signal evidence archive:
`/private/tmp/pinfold89-final-signal-evidence.tar.gz`, SHA-256
`fba984c0dabf5449973b3c829c027a9f63d2f18e056a23141f3891d75d5ed530`.

The first full Mac run passed 31 tests, failed two nested update builds
on the default Zig cache permission boundary, and ignored the existing
slow gate; wall time was 293.03 seconds. Assertions were unchanged. The full
rerun with writable Zig caches and the compiler cache disabled passed:
33 passed, zero failed, one existing ignored gate, in 254.82 seconds.
The Mac Bun type check and durable recovery gate also passed, including
whole-box cancellation and crash recovery without unsafe replay.
Hosted Linux x64 and arm64 validation is still pending.
