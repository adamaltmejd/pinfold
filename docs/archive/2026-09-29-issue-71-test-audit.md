# Issue 71: manual gates

Candidate: `c8de2079c192097365d0001f7ce6423746862741`.
Issue: https://github.com/adamaltmejd/pinfold/issues/71.

Missing mount hosts now produce a `spec` refusal naming the path before
claiming the box. Directory symlinks remain accepted. The issue permits
this remedy alone; runtime stderr forwarding is outside this change.

## Test audit

Keep the revised block in guarantee 17's `up_refuses_before_it_creates`.
No new test. The prior block expected failure after the claim; the revised
block requires refusal for a missing directory and a dangling symlink.
Creating the target lets the same spec and box name start in the existing
concurrent-name test.

The sabotage is discarding metadata errors. Event, reason and exit code
come from the spec; paths and existence come from the host fixture. The
existing file-mount case does not cover failed metadata lookups. The real
macOS regression failed before the fix and passed afterward.

Independent manual review found no findings at the configured P1 blocking
threshold and passed all four test-audit questions.

Rust source lines: tests 4,796; binary 7,524 (including build.rs).

## Verification

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --locked --offline -- -D warnings`: passed.
- TruffleHog Git history scan with `--no-verification --no-update --fail`:
  passed, exit 0. Host version 3.97.9; Yard's image pins 3.97.8.
- `sh scripts/e2e-gate.sh`: passed, exit 0. macOS: 26 tests passed in
  123.45 seconds. Linux x86_64 and arm64: both CI jobs passed.
- CI: https://github.com/adamaltmejd/pinfold/actions/runs/36619862225.
- The gate removed its temporary `queue/<candidate>` remote branch.

Run manually at the user's request. Formatting, Clippy and secret scanning
ran on the host; independent review was manual rather than Yard's model
seat. The repository's combined e2e script ran unchanged. No Yard
admission, approval or landing is claimed.
