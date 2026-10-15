# Issue 92 build serialization diagnostic audit, 2026-10-15

Diagnostic only. No speedup or acceptance result is claimed. Do not merge
this temporary workflow or instrumentation.

A is the instrumented test helper at `36adf149`. B adds four Linux-only
lines to serialize all real CLI calls through `build_profile` with a scoped
stdlib mutex. Its SHA-256 is
`096e347180224187ddeb379007b0e8abf854e2612a09b786a76df5539a59293f`.
The runner derives and verifies both exact variants; its committed test
helper remains A. The shipping source stays `30725e08`.

No assertion block, expected value, guarantee row, named sabotage or
deadline changes. Fresh independent image builds, private target isolation,
rootless runtime controls and runtime cleanup guards remain. The mutex
releases before callers' assertions, prompts and fixture drops, including
panic unwinding. It does not cover direct caller image-build commands.
Two independent reviews found no recursive acquisition or lock-order cycle.

Existing phase spans begin after mutex acquisition. Build durations exclude
its waiting; complete test and Cargo durations include it. Exact mutex wait
time is not observed. The fixed two-host ABBA protocol compares both B runs
with final A, retains the initial A and all failures, and reports harness
compilation asymmetry. It is not the frozen nine-run acceptance protocol.

Before push, the full real Mac B suite passed 33 tests with one ignored:
270.51 seconds in libtest, 272.47 seconds for the complete Cargo command.
The four new lines are Linux-only, so this establishes no Mac speedup.
`cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`
passed. Mac logs are `target/issue-92-2026-10-15/build-serial-mac-*.log`.
At this diagnostic B, test Rust source is 8,713 lines; binary-crate Rust
source is 9,449 lines. No assertion disposition was required.
