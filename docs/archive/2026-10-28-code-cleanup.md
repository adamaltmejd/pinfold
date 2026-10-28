# v0.3.3 cleanup and release preparation

Baseline: `ddb9404`, clean checkout, five commits after v0.3.2. The
baseline's Linux CI run `40499242907` passed. No other runtime suite was
active; only the shared BuildKit builder was running.

## Measurements

The before column is the baseline. Release changes are measured against
v0.3.2; the first two rows exclude this report. The next two include it.

| Measure | Before | After |
|---|---:|---:|
| Release additions / deletions | 363 / 534 | 398 / 605 |
| Release delete/add ratio | 1.47 | 1.52 |
| Release additions / deletions, including report | 363 / 534 | 534 / 605 |
| Release delete/add ratio, including report | 1.47 | 1.13 |
| Rust lines | 23,841 | 23,810 |
| Largest source file, including tests | box_.rs, 4,357 | box_.rs, 4,333 |
| Largest binary source file | cli.rs, 1,284 | cli.rs, 1,277 |
| ARCHITECTURE.md lines | 1,269 | 1,267 |
| Written tickets from this pass | 0 | 0 |
| Full Mac suite wall time, including Cargo | 134.58 s | 99.49 s |

The code audit proposed 12 net lines of deletion; both cuts landed.
The test rewrite added five lines; the approved jail-test deletion removed
24. Rust therefore fell by 31 lines. The docs pass removed five net lines.
E2E Rust lines fell from 8,098 to 8,079; binary Rust lines from 14,654 to
14,642 (excluding dated prototype sources).

## Whole-tree audit

Read-only agents read the complete CLI, box lifecycle and its helpers,
all eight runtime files, proxy and login, clean/image/artifacts/download/
disk/profile, config/dirs/trust/init, pi, E2E helpers, every E2E test file,
the spec and profiles, README and command help, and ancillary scripts.
The operator read the workflows, release tooling, main/update/build,
guest-kernel tooling, CONNECT fixture and the binding spec.

Dependencies match the nine crates under Code. Formatting and Clippy,
including the dead-code pass, passed. The config/CLI/guarantee trace
tables were built in `/tmp/pinfold-release-traces.md`. All 27 guarantee
rows map to tests or the named slow gate. `build --profile` is exercised
through the shared helper. Direct invocation gaps remain for `attach`,
`push`, `apparmor-profile`, `box exec --tty`, `update --check` and
`PINFOLD_NO_UPDATE_CHECK`; they do not justify separate tests under the
fixed-list policy. Existing tickets #211 and #212 cover related gaps.

No TODO or FIXME was found. XXX matches are temporary-file patterns and
Public Suffix List data. The two source revisit comments remain tied to
specific upstream changes; the live login gate checks the refreshed
Codex helper. No performance finding was admitted without a measurement.

## Landed findings

- `c0ef2b4`, runtime: removed the `builder()` lookup wrapper, net -5.
- `1b6f0fa`, CLI: removed the duplicate idle-project set from the cleanup
  plan; the disk map now also retains projects with no disks. The same
  exclusive locks and fresh eligibility checks remain, net -7.
- `5e53646`, docs: cut repeated Pi usage guidance, repeated seccomp text
  and rationale; clarified full's added egress hosts, net -5.
- `7f3bef8`, guarantee 13: replaced the config-derived project-state
  path with a before/after host tree snapshot. It now detects state
  written under any project id. Reused the existing snapshot helper,
  named the state-before-refusal sabotage, net +5 test lines.
- `17e892b`, guarantee 43: removed the private jail filename assertion,
  net -24 test lines, with the matching Shown by edit approved by Adam.
- `ec810cc`: version 0.3.3; publisher-verified Claude 2.1.312 and Codex
  0.165.0 pins. Other pins and Cargo dependency versions were current.
  Bun 1.4.4 was not yet eligible under the seven-day rule.

## Test audit and operator decision

Every assertion block was reviewed against its guarantee, outside
observer, expected value, sabotage and twins. Positive controls and
specified output-consistency comparisons were retained. Runtime and
run-record tests and the shared helper module were lean already.

Guarantee 43's exact jail-root basename check depended on private layout.
Adam approved dropping that block and narrowing Shown by to the surviving
own/other disk checks. Namespace, capability, descriptor, device, mount
and parent-death checks remain. The exhaustive no-extra-file assertion
is no longer claimed as test evidence. No production control changed;
no rework ticket is needed. The other finding, guarantee 13's
config-derived path check, was rewritten rather than dropping its
no-state proof. Two deficient blocks were found: one deleted, one
rewritten.

Rejected cleanup: replacing the atomic CLOEXEC PTY creation with
`nix::pty::openpty` plus later fcntl introduces an exec leak window.
Re-admit only when the platform helper supports atomic CLOEXEC.
Combining differently scoped filesystem walks or policy branches adds
an abstraction without removing a concept; re-admit only with a named
duplicate and preserved boundaries. No speculative structural refactor
was included.

## Spec pass

All guarantee rows remain, reconciled with their current tests. Row 43
now names the surviving evidence, as approved. Cut repeated statements
and rationale; Always applied still names
Firecracker's seccomp filters. The threat model, proxy rules, trust and
limits are unchanged. No open question was answered by the suite: fake-IP
DNS, Apple user namespaces, Linux arm64 KVM, herdr's reader and disk
allocation remain open. README and help agree with CLI and Process
interface. Pi help is documented pass-through, not a host help probe.

## Validation

- Baseline full Mac suite: 24 passed, 1 documented slow test ignored;
  128.65 s test time, 134.58 s including Cargo. Live Codex login passed.
- Final formatting and Clippy passed. Full Mac suite at `17e892b`: 24
  passed, 1 ignored; 97.39 s test time, 99.49 s including Cargo. Refreshed
  Codex live login passed. The first run with refreshed pins also passed,
  174.01 s test time and 175.96 s including Cargo.
- macOS arm64 release build passed; Linux x64 static-musl cross-build
  passed. Linux execution remains a separate CI gate.
- Linux CI at `17e892b` passed:
  https://github.com/adamaltmejd/pinfold/actions/runs/40652753087
- Separate CONNECT gate: https://github.com/adamaltmejd/pinfold/actions/runs/40652762110
- Slow route-response proof passed: 1 test, 315.10 s test time and
  315.24 s including Cargo. The first invocation was interrupted
  when the active tool run was resumed; no result was recorded. Its owner
  was dead and the next suite reclaimed the leftover box. This was an
  interrupted run, not a test failure or a counted pass.

## Caller changes

- `pi`, `push` and `fetch` refuse roots without `.git` as `not-git`, exit 1.
- New Pi project disks receive committed branches only. The previous
  host-tree copy is removed; uncommitted, untracked and ignored files do
  not cross. Commit work the box needs before launching.
- Existing project disks are not sanitized. Fetch work to keep, inspect
  exact disk names with `pinfold config`, remove the project disk with
  `pinfold disk rm NAME`, then relaunch to recreate it.
- No box-spec, JSON-line or CLI-flag changes.
