# Scratch Pi test audit

Candidate based on 4d0aefe. No dependency, toolchain, workflow, resource or
suite-budget change. No merge, installation or release performed by this pass.

Independent architecture/correctness review: Astra PASS after repairs.
Independent configuration/summary review: PASS, including the final startup
validation ordering. Independent changed-assertion audit: PASS.

Guarantees 12/13/18 retain their tests and extend approval, scratch isolation
and configuration fallback. New guarantees 45/46 cover export and retained-work
reports. Guarantee 10 now names automatic project branch crossing, preserving
its hostile-bundle assertions while distinguishing explicit file export.
The named sabotage and expected values come from host fixtures and the spec.

Removed one redundant box-name comparison: simultaneous successful starts
and independent observed disk contents already prove the relevant isolation.
Strengthened refusal reasons, global recipe positive control, hostile filename
presence, and the guest-modified remote-ref scenario. No other assertion removed.

Changed test files: pi.rs 1766 lines and box_.rs 4639 lines. Relevant product
modules: config 304, trust 127, context 74, launch 574, Pi summary 212, disk
export 420, init export 146 and init summary 397 lines. New helper code uses
existing dependencies and ordinary box/disk lifecycle; no test-only paths.

Checks: fmt and locked all-target Clippy PASS. Full Mac suite: 26 passed,
0 failed, 1 ignored, 106.12 seconds. Focused export and summary scenarios
also passed. The first full Mac run caught configuration validation occurring
after Git verification; the ordering was fixed and independently reviewed.
An earlier focused summary run caught incorrect Git stdin exclusion semantics;
explicit negative host OIDs fixed it. Failed receipts were retained.

Linux CI is the remaining runtime gate at this snapshot; its exact commit and
result belong in the PR and private receipts. The five-minute clock is unchanged.
Limits, cancellation and publication races have source-review coverage; the
E2E scenarios do not claim exhaustive independent proof of every boundary.
