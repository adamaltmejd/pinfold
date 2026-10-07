# Nightly cleanup fixture race

[Nightly run 37595855614](https://github.com/adamaltmejd/pinfold/actions/runs/37595855614)
passed both Linux suites and 31 of 32 Mac tests. Guarantee 15's cleanup
test failed because its dead box was missing before explicit clean.

The test killed and reaped its owner before observing the fixture. A
parallel test's first command could then run automatic maintenance from
another state root and remove that box. The explicit-removal mutex does
not guard automatic maintenance.

## Candidate audit

- Row 15: keep every assertion unchanged. After SIGKILL, read stdout to
  EOF through the existing helper, without reaping. Other state roots see
  the retained zombie pid as alive. This root sees the released owner lock
  and can remove the box. Reap after the dead-box removal observation.
  The owner-loss test already uses this fixture lifetime.
- Failure proof remains the existing sabotage comments: skip dead boxes
  during clean, or let dead boxes protect project state. The spec supplies
  the expected removal; the host fixture and runtime supply observations.
- No new test, assertion, helper, guarantee, dependency or binary change.
  The independent candidate review approved the fixture correction.

Test lines: box_.rs 3,492; the whole E2E harness and tests 5,839.
Binary source lines: 7,930, unchanged; clean.rs 468 and core/box.rs 784.

## Verification

- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`
  and `git diff --check`: passed.
- Full Mac suite with real Apple boxes and no competing suite: 32 passed,
  zero failed or ignored, in 826.83 seconds. Includes cleanup, the held
  build across clean, owner loss, document readers and live login. Cold
  harness downloads contributed to the local wall time.
- Linux verification belongs to PR CI. The original night's Linux success
  is evidence about the failed candidate, not this fix.
- Release builds and a release pass were not run. The nightly preparation
  guard includes the E2E crate, so merging this change requires a manual
  release before automatic pin releases resume.
