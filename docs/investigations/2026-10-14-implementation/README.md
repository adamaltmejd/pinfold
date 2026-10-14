# Measured implementation tracking

Base: `a1be9213efc3585d75bead7f2c2de4073167fc7d` (main, v0.2.4).
Branch: `codex/lean-measured-builds`. The primary dirty checkout and original
experiments are preserved. The unrelated per-box seccomp change is excluded.

| Ticket | Implemented commit | Independent reviewer | Acceptance boundary |
|---|---|---|---|
| B1 effective hooks | cfd2eb4 | runtime reviewer | G11 |
| B2 failed readiness output | 7feaf89 | proxy reviewer | G9 |
| O1 read-only identity index | c9677e7 | runtime reviewer | G11/G17/G22 |
| O2 embedded digest | a00161c | box reviewer | artifacts/lifecycle |
| O3 Apple observation reuse | 98c1b7f | box reviewer | G9 |
| O6 dedicated shared init | 1a9281a | box reviewer | full suites and slow gates |
| O4 durable teardown inventories | 4d39b2e | proxy reviewer | G35 on three hosts |
| C2 claimed resolver artifact | a7495da | box reviewer | Linux isolation/lifecycle |
| C3 update child owner | b3b620e | proxy reviewer | G28 |
| O5 disposable Rust outputs | 0156384 | proxy reviewer | existing environment forwarding |
| B3 virtual scratch base | 0a50a74 | proxy reviewer | existing G15/G23 build paths |
| #92 fixture debug information | 86c2b4f | proxy reviewer | unchanged G28/G31; full-suite budget |

All selected implementations have independent source approval. Reviewers did
not author their assigned changes. Reports are dated archive files:
[paths](../../archive/2026-10-14-lean-review-paths.md),
[lifecycle](../../archive/2026-10-14-lean-review-lifecycle.md),
[build/runtime](../../archive/2026-10-14-lean-review-build-runtime.md), and
[scratch](../../archive/2026-10-14-lean-review-scratch.md). The additional
[fixture build change](../../archive/2026-10-14-fixture-build-cost.md) has its
own measurement and independent review record.

Formatting and Clippy passed. The uninterrupted full Mac suite passed 33/0/1
in 231.81 seconds after the fixture build change; G35 also passed separately.
The associated pull request
records final release-build sizes, Linux CI and slow-gate results. Those are
separate runtime/build evidence, not implications of source approval.

O5 uses the existing opt-in setting; no new default or cache. O7 proxy
buffering remains deferred until representative workload evidence justifies
it. C1's image-tag loop rewrite remains rejected as less direct.

The [test audit](../../archive/2026-10-14-test-audit.md) records dispositions,
change counts and the assisted first run, which was not accepted as a gate.

The completion audit identified one remaining C2 observation: directly check
the Linux resolver is empty and read-only across a sibling box teardown.
The associated PR records that real-CLI probe and the final #92 disposition.
