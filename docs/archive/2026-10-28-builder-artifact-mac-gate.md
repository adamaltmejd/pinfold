# Builder artifact preparation: coordinated Mac gate

Date: 2026-10-28. Tested source/document head:
`3d6b1068904b3131cea5ac0c9d0fbb60b93c8241`.
This supplements the audit as recorded at that commit; the earlier dated
records are not rewritten to track later state.

## Ownership and invocation

The phase 6b worker explicitly released its unused Mac reservation to audit
chat `01a18301-5c62-7a23-8837-a499d4a450c0`, with no active Mac suite and
its previous lock released. The operator subsequently confirmed the handoff.
No concurrent Mac runtime suite was started by this audit.

The full unfiltered command was `cargo test -p e2e --locked`. It used the
normal `scripts/run-mac-e2e.sh` Cargo runner, a separate Cargo target and
Zig cache under `/private/tmp`, and no compiler wrapper. These are host
build settings, not changes to the binary's seams or runtime controls.
There was no retry, ignored-test override or temporary test change.

## Result and observation limits

Result: **23 passed, 1 failed, 1 ignored**, exit 101. Rust test time:
434.74 seconds. Measured full command wall time: **438.252 seconds**.
The ignored test is the existing five-minute route-response gate.

The live macOS portion of guarantee 26 did not finish. Its own run
`d81a060f8936030de02412b59b497b654f02677340cb8f15e4643c27cadadb2e`
reported an allowed `POST /backend-api/codex/responses` through
`codex.internal`, reason `route`, upstream status **429**, at
`2026-10-28T15:22:29Z`. This was read through the real `box events` CLI.
Other direct/telemetry hosts were refused as not allowlisted. The route
observation establishes an upstream HTTP response, not a source regression
or a complete diagnosis of the service/account condition.

The owner and Codex exec remained active. The operator inspected only this
run, then stopped its `pinfold-e2e-68131-login-live` box through the normal
`box down` command. Consequently the test reported command exit 137 rather
than expected 0. The failure is therefore an upstream-429 stall followed
by an intentional owned-box stop, not a naturally completed live check.
The remaining cleanup, disk, owner-death and retention tests then passed.
No other test or caller box was stopped.

The test's normal cleanup removed its scratch state and egress log. The
429 record above is an observation transcribed from the captured tool
response, not a newly retrieved raw egress receipt. The complete test log
and timer receipt are preserved privately under:

`/Users/adam/Code/pinfold/target/builder-artifact-release-audit-2026-10-28/`

| File | Bytes | SHA-256 |
|---|---:|---|
| pinfold-builder-artifact-mac-gate.log | 5372 | b67cb4e67a0620d16a1f2a5d2bfb8579e5a8647480ea395b8c28fd8db598cdde |
| pinfold-builder-artifact-mac-gate.json | 165 | e705d14f2f555341a907ad2b018c3880c5ba50b6d2a1436410c783ae926a1bca |

The timer receipt retains its original `/private/tmp` log path. Both
copies are mode 0600. The source artifact/provenance bytes were not changed.

## Admission and next action

This is a **failed full Mac gate**. The 23 passing tests, main Linux CI,
artifact provenance checks and independent static reviews cannot substitute
for it. The source changes remain comments, README and help grouping;
all assertions, producer inputs, pins, dependencies and controls are intact.

After seeing the actual failure, the operator explicitly requested a
concrete report and reviewable PR, while preserving this result and making
no exception. A draft PR can therefore expose the candidate for review;
its creation does not establish a green Mac gate or authorize a release.

Before publication, the operator must resolve the gate under the existing
policy or obtain Adam's explicit, narrow artifact-only gate exception.
That exception is not granted or requested by this audit. There is no
automatic live-test retry, skipped assertion or inferred waiver. The
separate three test-proof proposals also remain for operator judgment.

The suite PID 68131, owned box-up PID 76503 and exec PID 76718 were all
absent at the final process check. The Mac reservation is released; this
audit will start no further runtime suite without a new coordinated slot.
