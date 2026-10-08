# Apple teardown investigation and test audit

This continues issue 83 after the two failed release gates recorded in
`2026-10-08-release-validation.md`. Both failures retained the live owner's
claim at the caller's deadline. Neither justified increasing that deadline.

## Reproduction and correction

A real CLI probe compared quiet teardown with a guest continuously writing
to `/proc/1/fd/1`, which feeds the attached runtime stream. Baseline quiet
shutdown took 0.162 seconds; active output took 3.266 seconds and produced
the same paired native I/O completion errors seen in the release failures.
Both diagnostic requests still completed within the caller deadline.

Pinfold removed the runtime synchronously on its only Tokio thread, which
also drains that stream. Moving removal to `spawn_blocking` kept the drain
active. The same quiet and noisy probes then took 0.160 and 0.182 seconds,
with no native I/O completion timeout. Child reaping, state cleanup and
acknowledgement retain their order, including a worker failure.

Apple container 1.5.0 has a second hazard. Its
[forced deletion](https://github.com/apple/container/blob/1.5.0/Sources/Services/ContainerAPIService/Server/Containers/ContainersService.swift#L833)
invokes graceful-stop with SIGKILL. That creates a second process waiter
beside the runtime monitor. Each consumes completion events from the same
[I/O tracker](https://github.com/apple/containerization/blob/bc994b88df46207fad7775b0eabc51947e315881/Sources/Containerization/LinuxProcess.swift#L451).
Split events can leave both waiters incomplete.

The adapter now uses explicit SIGKILL followed by non-forcing removal.
Apple's [kill endpoint](https://github.com/apple/container/blob/1.5.0/Sources/Services/RuntimeLinux/Server/RuntimeService.swift#L566)
waits on the existing monitor's completion. Plain removal cannot re-enter
the competing stop path if killing failed. Stopped or absent boxes can
refuse kill; checked removal and the existing absence check decide success.
Partial-start cleanup is supported by the same native removal path.

The old forced removal already used SIGKILL. No grace period, ownership
rule, acknowledgement deadline or security control changed. The paired
timeout mechanism is reproduced with active output; the precise schedule
of the two original quiet-box failures remains unavailable.

## Guarantee 9 regression review

The existing lifecycle test now starts continuous attached output before
its first acknowledged teardown. A guest FIFO confirms a successful write
to init's stdout. The expected token comes from the fixture. Successful
down, absent box and completed owner come from the spec and remain the
existing assertions. No new test or runtime-log wording assertion was added.

The comment names both sabotages: synchronous removal stops the drain;
forced Apple removal restores competing waiters. Detection of the original
timeout is schedule-dependent. The test exercises the harder scenario; it
does not claim every restoration of either bug deterministically fails.
No manual sabotage run was performed. The focused lifecycle test passed
in 92.33 seconds, including stopped-owner and startup-cancellation cases.
Formatting and strict all-target Clippy passed. Full release gates follow.

The patch adds 20 net E2E lines and 11 net production Rust lines. Independent
source and test review found no blocker. Diagnostic scripts, per-box
results and native logs are retained under `/private/tmp/pinfold-issue83/`.
