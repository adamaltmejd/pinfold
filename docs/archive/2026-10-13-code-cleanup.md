# Release audit for v0.2.3, 2026-10-13

The release starts from main at `83ea53b`. The separate issue #89 branch
is excluded. The cleanup and test-audit skills read the whole live tree,
with bounded module reviewers and a final diff review. Archived history
was treated as evidence. All controls and numbered guarantees remain.

## Measurements

Measured before cleanup at `474a008`, and after at `21d2723`, relative to
v0.2.2. The after column excludes this report and the written ticket.

| Measure | Before | After |
|---|---:|---:|
| Commits since v0.2.2 | 23 | 29 |
| Files changed | 35 | 50 |
| Release added / deleted lines | 1593 / 168 | 1762 / 334 |
| Release delete/add ratio | 0.105 | 0.190 |
| Crates added / deleted lines | 337 / 63 | 423 / 164 |
| Crates delete/add ratio | 0.187 | 0.388 |
| Rust lines, including tests and build script | 17187 | 17166 |
| Binary Rust source lines, excluding build script | 9144 | 9118 |
| Largest Rust source | cli.rs, 1112 | cli.rs, 1106 |
| ARCHITECTURE.md lines | 1004 | 1004 |
| Written tickets from this pass | 0 | 1 |

Reviewers proposed 47 net deletions; 46 landed. Cleanup commits added 26
lines and deleted 72. Pin, lockfile, Public Suffix List and existing
main changes are included in release totals, independently of cleanup.

## Landed

- `a906d85`: CLI present returns its predicate once; failed-image JSON
  loses a temporary; six dispatcher and two parser comment lines go.
- `451d4a1`: thirteen comments repeating proxy, network and TLS names go.
- `8c2a202`: obsolete PID-only ownership comment goes. Generation and
  lifetime locks still decide liveness.
- `59117fc`: one Bun image-pin reader replaces three inline CI parsers.
- `0469813`: eligible pins and lockfiles advance with the workspace version.
  Prototype age exceptions go; new nested Earendil types had bypassed the
  direct pins' seven-day wait and failed the real typecheck.
- `21d2723`: test startup bounds admit the existing artifact deadlines.
  No assertions, retries or production timeouts change.

## Test audit

The sweep read all 34 Rust guarantee tests, host fixtures, the durable
controller gate and slow-proxy gate. Positive controls, independent runtime
observers, output-agreement guarantees and assertion failure mechanisms
were checked. No remaining assertion block was admitted for deletion.

The Bun migration removed the prototype's stderr-maintenance fixture. It
asserted a diagnostic outside guarantee 35; garbage collection remains
covered by guarantee 15's real Rust runtime scenarios. The current durable
gate retains host-boundary refusal, boxed execution, exclusive writer,
unsafe mutation/recovery and whole-box cancellation observations.

Four trust-refusal blocks in guarantee 12 assert unsuccessful exit without
a spec-named trust reason. They are the sole refusal proofs and remain.
The bounded rework is the
[written ticket](2026-10-13-trust-refusal-test-ticket.md). It requests a
stable reason in the spec and four reason assertions in the existing test.
No product bypass was claimed from source review. Two existing sabotage
comments can also name capability/uid and injecting-route mechanisms more
fully; the underlying assertions and positive controls remain.

## Rejected cuts

- Ownership labels, nested XDG checks, mount admission, profile protection,
  proxy framing, DNS/SNI policy, credential redaction and cancellation
  rechecks implement controls. Reconsider only with an operator spec decision.
- Generic cleanup/preparation engines, profile inheritance and rewritten
  argument ownership add concepts or remove no path. Reconsider a concrete
  one-module consolidation with fewer mechanisms.
- Caching fingerprints or ownership, one-byte proxy buffering and borrowed
  argument allocation changes lack a measured cost. Reconsider a measured
  bottleneck with preserved boundaries.
- Native transfer or whole-file hashing loses transfer bounds, cancellation,
  reaping or streaming memory use. Reconsider an existing mechanism keeping
  those properties.
- Shared production/test image parsing removes an independent observer.
  Retain it; reconsider only an independently specified runtime interface.
- The old-release update owner check implements the current running-boxes
  refusal. A removal requires an operator contract decision.
- Removing repeated positive controls, distinct mount-admission scenarios
  or pinned-version/output agreement assertions would lose their guarantees.
- `getAuthStatus` remains a deferral with a named upstream-removal trigger.
  Current Linux and Mac login-route gates exercise the new pinned helper successfully.

## Spec pass

Config/CLI/guarantee trace tables were checked. All 36 guarantees map to
named tests or standalone gates. Configuration scenarios not directly
named by a test are interface coverage observations, not permission to
add tests outside the fixed guarantees.

The process synopsis now documents required repeated `--label KEY[=VALUE]`
filters. Bun startup, exact pins and the uniform prototype publication wait
are reconciled with the implementation and README. The four open questions
remain unresolved. No control deletion or feature removal was proposed by
the cleanup pass. The preliminary dependency-updater archive records earlier
experiments; final CI reads Bun from the image recipe and uses no Node runtime.

## Validation

- Required Rust formatting, Clippy with warnings denied, and the additional
  dead-code check passed. Nine direct dependencies match the binding spec.
  Ruff and actionlint passed; only existing unsupported runner-label warnings
  were excluded from actionlint.
- Final source `21d2723` passed both Linux suites and prototype gates:
  [CI run 38401287181](https://github.com/adamaltmejd/pinfold/actions/runs/38401287181).
  Linux arm64 suite: 174 seconds; x64: 234 seconds. Prototype steps: eight
  and ten seconds, including installation and typecheck. Temporary branches
  were deleted by the helper.
- Full Mac suite on the final source: 33 passed, zero failed, one ignored
  slow-gate test; 581.14 seconds. The live Codex shell round trip passed.
  This run started with verified artifact cache entries; guarantee 19 still
  deleted each executable and exercised real downloads and reinstallation.
- The real Bun 1.4.3 Mac prototype gate passed authority refusals, boxed
  execution, competing-writer exclusion, crash recovery without unsafe replay,
  and whole-box cancellation. Its frozen install and typecheck passed.
- Mac release build passed under Rust 1.99.0; its executable reports 0.2.3.
  Tag publication runs both native Linux release builds and the Mac release
  build before GitHub publishes assets and SHA256SUMS.

Initial Mac runs failed while new harness artifacts were cold: first, two
ninety-second startup waits expired (31 passed, two failed, 505.79 seconds).
A direct transfer of Codex's 103.6 MB archive downloaded 95.5 MB in 120 seconds.
Test waits were aligned with the existing per-artifact deadlines: 330 seconds
to reach the first refresh fixture, and 660 seconds for two guest transfers
plus setup. A second run under a stricter 600-second readiness wait passed
32 tests but hit the production five-minute Claude download cap (1239.68
seconds). An isolated empty-login CLI probe installed the pinned Mac helper
and returned the expected login refusal in 68.4 seconds. The final full run
passed on the final 660-second source. Production download limits, positive
controls and assertions were preserved. No retry was added to any test.

Caller-change notes cover main's new mount-alias refusal and normalized
mount destinations, plus the experimental controller's required Bun package
launch commands. Action commit pins were reviewed manually; future nightly
updates retain them. Newer age-gated tools and prototype packages wait seven
full days. The audit report and written ticket are the only changes after
validated source `21d2723`.
