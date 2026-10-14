# Bounded inventory: full Mac verification

The unchanged bounded-inventory candidate passed the complete Mac Rust
suite on 2026-10-12: 33 passed, zero failed, one intentional slow-gate
exclusion, in 237.16 seconds. Formatting, strict all-target Clippy and
`git diff --check` also passed. No competing test suite was present in the
shared Apple runtime before the run.

This run used backtraces and external process observation. Guarantee 26
completed during the early parallel tests. The first-event timeout from
the preceding full run did not recur. Its cause remains unknown; this
clean run does not establish that the earlier failure was fixed.

The earlier panic identified the shared 90-second observer rather than
one of guarantee 26's four starts. The focused diagnostic and this full
run provide no failed startup to trace. No deadline was increased, test
serialized or assertion changed to obtain this pass.

The Linux held-lock validation and assertion-block audit are recorded in
`2026-10-12-test-audit.md`. This is implementation validation, not a release
audit or certification of the separate slow gates.
