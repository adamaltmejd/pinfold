# 0.2.1 final release gates

This records candidate `18cd5fe` for issues 82 and 83. Production code,
harness pins and Rust assertions last changed at `ecf747b`. The later
commit only preserves CONNECT diagnostics before failure cleanup.
Earlier audit and failed-gate reports remain historical snapshots.

## Validation

| Check | Result |
| --- | --- |
| Formatting, diff whitespace and strict all-target locked Clippy | Passed |
| Focused Mac lifecycle, including immediate exec and active-output teardown | Passed, 92.33 seconds |
| Full Mac E2E | 33 passed, zero failed, one slow-test exclusion; 220.03 seconds |
| Mac blocked-response release, guarantee 34 | Passed, 313.67 seconds |
| Mac durable recovery | Passed |
| Linux x64 E2E | 33 passed, zero failed, one slow-test exclusion; 226.61 seconds |
| Linux arm64 E2E | 33 passed, zero failed, one slow-test exclusion; 167.91 seconds |
| Linux durable recovery, both architectures | Passed |
| CONNECT activity and idle closure, guarantee 36, both Linux architectures | Passed with failure diagnostics added |
| Local release builds: Linux x64 and arm64 musl, Mac arm64 | Passed |

[Linux CI](https://github.com/adamaltmejd/pinfold/actions/runs/37744821348)
and the instrumented
[CONNECT proof](https://github.com/adamaltmejd/pinfold/actions/runs/37744816133)
ran at `18cd5fe`. Mac gates and local release builds used the same production
and Rust test inputs at `ecf747b`. The diagnostics change alters no assertion,
timeout or successful proof path. Mac tests were not repeated for it.

The durable gates observed host-boundary refusals, boxed commands, an
exclusive writer, crash recovery without unsafe replay and whole-box
cancellation. The prototype's frozen install and TypeScript check passed.
Published release artifacts will still be built by the tag workflow.

## Unresolved CONNECT finding

The earlier [proof run](https://github.com/adamaltmejd/pinfold/actions/runs/37742771770)
passed on arm64 but failed on x64. Both traffic directions delivered all
12 markers over 330 seconds. Download closure occurred near the expected
300-second idle deadline; neither upload observer reported closure before
the fixture's unchanged observation deadline. This preceded box teardown.

[Issue 85](https://github.com/adamaltmejd/pinfold/issues/85) remains open.
The failed fixture deleted its egress evidence before it could distinguish
idle enforcement from closure propagation. The added failure-only diagnostic
block now retains final markers, observed closures, process status and audit
records without replacing the original failure or preventing cleanup.

One instrumented run passed both architectures. All four x64 closures were
300.293 seconds after final traffic; arm64 closures were 300.851 seconds.
Both required idle reasons were observed. This does not explain or resolve
the earlier failure. Production proxy code is unchanged from 0.2.0. No
deadline, tolerance, assertion or retry policy was changed. The 0.2.1 release
addresses Apple readiness and teardown and carries this known finding.

## Audit totals and evidence

Before this report, the release diff from 0.2.0 contains 391 added and 91
deleted lines across 15 files: a delete/add ratio of 0.233. The audit landed
45 net cleanup deletions. Total Rust is 16,909 lines; production Rust is
9,056 and E2E is 7,794. The largest source file is the E2E `box_.rs`, at
4,848 lines; the largest production file is `cli.rs`, at 1,115 lines.
ARCHITECTURE.md has 972 lines. These counts exclude this evidence report.

Two findings became written investigation tickets: 83 was addressed by the
teardown patch; 85 remains open. The earlier whole-tree audit records the
module reviews, rejected proposals and spec pass. Pi is pinned to 1.1.0,
Claude to 2.1.294 and Codex remains 0.161.0. Caller changes: none.

The completed Mac gates left only the pre-existing shared buildkit container.
The installed binary was still 0.2.0 at validation. Scratch evidence paths
named in earlier reports were temporary and were no longer present when
work resumed. Measurements recorded in those reports remain; GitHub run
logs retain the Linux results and CONNECT failure and pass evidence.
