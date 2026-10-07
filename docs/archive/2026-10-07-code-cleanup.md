# Code cleanup and spec pass for 0.1.9

This release carries the nightly cleanup fixture correction from PR #80
and Claude 2.1.291 to 2.1.292, verified against publisher digests by the
live pin updater. No other pin moved. Production Rust is unchanged from
v0.1.8. Caller changes: None.

## Numbers

| Measure | Before (merged fix cc2f942) | Release candidate |
| --- | --- | --- |
| Commits since v0.1.8 | 1 | 2 |
| Diff since v0.1.8 | +45 / -1 | +137 / -9 |
| Largest production source | cli.rs, 1,128 lines | unchanged |
| Largest test source | box_.rs, 3,492 lines | unchanged |
| Architecture | 867 lines | unchanged |
| Production Rust under src | 7,930 lines | unchanged |
| E2E Rust | 5,839 lines | unchanged |
| New written tickets | 0 | 0 |
| Mac suite | 826.83 seconds on the fix branch | 222.86 seconds |

Release delete/add ratio: 0.066 (9/137). Possible and landed cleanup net:
zero lines and zero dependencies. The archive reports account for most
additions; the fixture correction adds four lines net. No speedup is claimed.

## Whole-tree audit

The read-only module audit covered every production Rust source and
build.rs, the E2E harness and all test files, profile, spec and README.
The release scripts and workflows retain their current gates. Runtime,
clean, image and artifacts had a separate module reader; another reader
made the configuration, CLI and guarantee trace tables in scratch.

All module groups were lean under the current admission rule. Cargo's
nine direct production dependencies match the spec. Dead-code Clippy is
clean. No code TODO, FIXME or XXX was found; Public Suffix List entries
containing xxx are data. The login helper's revisit trigger remains Y-2.

No production deletion, new dependency, control change or guarantee change
was proposed. Existing rejected consolidations still lack their recorded
re-admission trigger. Source-only correctness hypotheses require CLI
reproduction before admission, rather than an unrelated release change.

## Test audit and existing tickets

- Row 15: retain every assertion. Reading EOF after killing the owner
  establishes exit without reaping; foreign state roots preserve its pid,
  while the owning root sees its released lock. Reap after removal.
- Y-4 retains row 28's distinct-executable and final-owner-race rework,
  and row 29's timeout observation rework.
- Y-5 retains row 3's accepted raw Content-Length body control and row
  24's attribution of dangling images during concurrent cleanup.
- Y-7 retains the previously recorded caller-path trace gaps.
- Y-8 remains open for the separate old-dated image-tag fixture race.
  PR #80 fixes its premature dead-owner reaping scenario only.
- Y-2, Y-3 and Y-6 remain parked. No duplicate ticket was opened.

Four existing rewrite blocks, zero new blocks to delete. Expected values
remain from the spec, host fixtures, kernel and runtime. No test or binary
seam was added. Test lines are 5,839 beside 7,930 binary source lines.

## Spec pass

Eight configuration key groups and twenty CLI verb/flag groups map to
the current spec. All 32 guarantee rows map to 32 distinct tests. Existing
Y-7 variants are not new gaps. Safe CLI help matches the current README
and spec. No restatement, drift or newly resolved open question required
an edit. The four open questions remain open.

## Verification

- Format, Clippy with warnings denied and Git whitespace checks passed.
- The first release-candidate Mac run passed 30 tests and timed out in
  both held-build fixtures. Its new Cargo executable path was absent from
  the enabled firewall's allow list. An atomic-copy scratch runner reused
  the existing allowed path, following scripts/run-mac-e2e.sh. The copied
  bytes matched the candidate. No repository or firewall rule changed.
- The full release-candidate Mac suite passed 32 tests in 222.86 seconds,
  including live login and cleanup during another caller's active build.
- All three local release builds passed. Linux outputs are static arm64
  and x64 ELF binaries; the arm64 Mac output reports pinfold 0.1.9.
  Zig emitted its deprecated linker optimization warning on Linux builds.
- Both Linux suites run on the pushed release commit before tagging.
  The release workflow rebuilds all three targets before publication.
