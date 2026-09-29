# Code cleanup, 2026-09-29

Whole-tree module reads and a test-audit sweep. No release was requested.
The baseline is 6d3c1be, after manually integrating Y-114 onto current main.
The cleanup ends at 3a0373b. Yard removal is a separate subsequent change.

## Numbers

| Measurement | Before, 6d3c1be | After, 3a0373b |
|---|---:|---:|
| Commits since v0.0.9 | 7 | 15 |
| Diff since v0.0.9, all | +16901 −81 | +17006 −217 |
| Diff since v0.0.9, crates | +16765 −75 | +16862 −199 |
| Delete/add ratio since v0.0.9 | 0.0048 | 0.0128 |
| Rust lines | 12504 | 12477 |
| Largest source, e2e/tests/e2e/box_.rs | 3198 | 3183 |
| Largest binary source, cli.rs | 1140 | 1126 |
| ARCHITECTURE.md lines | 833 | 830 |
| Tickets completed since the last report | 1, Y-114 | 1, Y-114 |
| Ticket high-water mark | Y-114 | Y-114 |
| Tickets from lane proposals | 0 | 0 |
| Mac suite, 27 tests | 164.70 s | 113.30 s |

Y-114 adds 16501 lines of pinned Public Suffix List data. That accounts
for most additions since the tag. The cleanup itself is +105 −136:
binary −37, tests +10, docs −4, net −31 lines. Module findings estimated
a possible −68 lines before judgment and test repairs. No dependency
was added or removed. The wall-time difference is not attributed to cuts.

## Landed

- 6d3c1be: manually integrated Y-114's suffix refusal and its guarantee-17
  scenario, preserving the newer missing-mount and dangling-symlink cases.
  The old lane's review failed on malformed reviewer JSON, not findings.
  Its candidate gates passed; its lane was abandoned and ticket completed.
- 407dc7e, cli: removed 30 lines of restating comments. Help now selects
  the requested image or box subcommand instead of an ambiguous group line.
- 30cd368, config: removed the single-caller environment-name wrapper.
- 6a0732f, box: removed eight comments restating enum variants.
- ff19a05, clean: removed the single-caller dead-box teardown wrapper.
- 5097a18, proxy: removed the zero-length body branch; Read::take with a
  zero limit copies no bytes and preserves framing.
- 0679b6c, box tests: removed two diagnostic detector blocks. Repaired
  the login mismatch's competing refusal and separated the two literal-IP
  request forms' log observations. Net −15 test lines.
- fc18772, pi tests: removed two diagnostic path-name detector blocks.
  Asserted all nine default hosts, the fixture's CPU quota/period ratio,
  and a writable sibling beside the configured protected directory.
  Net +25 test lines.
- 3a0373b, docs: removed repeated image-retirement rules and a guessed
  future auth design. Corrected install examples and scoped pi's variable
  forwarding guidance. Removed the history link from current usage.

## Test-audit dispositions

| Row | Block | Disposition |
|---|---|---|
| 17 | Login mismatch also hit codex's forbidden-from guard | Rewrite with an otherwise valid claude login and codex harness |
| 3 | One literal-IP log entry covered both request forms | Rewrite with a distinct host observation per form |
| 3 | curl's SSL wording | Delete; handshake exit and sni-mismatch log remain |
| 19 | Damaged cache's reported cached flag | Delete; actual repair and pinned executable run remain |
| 16 | Default list checked only npm | Rewrite against nine spec hosts |
| 16 | CPU assertion fixed the runtime's period | Rewrite against the fixture's 2-CPU ratio |
| 11 | Configured protection lacked a writable sibling control | Add that control in the same box |
| 11 | Protected symlink's diagnostic path name | Delete; refusal remains |
| 11 | Inside-.git diagnostic path name | Delete; refusal remains |

Four blocks deleted, five repaired. All 27 guarantee tests remain. No
guarantee clause, binary control, or feature was removed. Expected values
come from the spec, fixture, host, and runtime. No new test function.

## Adam's list

No control, feature, or guarantee deletion proposed for approval. Adam
authorized manual Y-114 integration, a saved Y-74 brief outside Yard,
and project-specific Yard removal after this cleanup succeeds.

## Rejected

- Optional-login tuple simplification, about three lines: it changes the
  login path for little removal. Re-admit when a broader duplication on
  that path is demonstrated without changing its controls.
- remove_images' unused label parameter, about seven lines: it also
  changes the CLI caller. Re-admit as a consolidation explicitly including
  that mechanical caller update.
- CLI forms without direct test coverage and the -V/help aliases: the
  earlier report's rejection conditions remain unmet. No new guarantee
  or syntax test was invented to fill the lookup table.
- Existing platform guards, trust checks, cache recovery, and TestDir:
  no new evidence meets the previous report's re-admission conditions.
- Speedups: no before-number establishes a cost that warrants a change.

## Verification and runtime overlap

Dependencies match the Code section. Dead-code Clippy reports no warnings.
No TODO/FIXME/XXX/revisit marker in crates or profile requires action.
The trace lookup maps every configuration key to a spec row and all 27
guarantee rows to tests. CLI coverage gaps above remain observations.

cargo fmt --check and cargo clippy --all-targets --locked -- -D warnings
pass. The focused Y-114 test passes on real Apple container. The full
baseline passes 27/27. After cleanup, two overlapping host runs fail with
buildkit not found or an unexpectedly closed export stream; a concurrent
Switchyard-v3 suite uses the same builder. Apple clear_build_cache deletes
that shared builder. These runs are not recorded as passes.

After coordinating exclusive runtime access and verifying that no other
suite process remains, the unchanged cleanup tree passes all 27 tests in
113.30 seconds. Runtime access was then released to Switchyard. No retry
or sleep was added to a test or the binary. Linux verification is CI's
responsibility on the subsequent push; it was not run locally.

Safe CLI help invocations were inspected for image build/rm, all six box
subverbs, and profile new. No help-text test was added, per AGENTS.md.

## Spec pass

The guarantees table still names exactly the existing 27 tests. Duplicate
image-retirement prose and speculative future auth design were cut. The
README now describes current usage. All four open questions remain open;
no control was weakened. No pin bump, version bump, or tag was made.

Y-74 remains deferred, saved in 2026-09-29-yard-v3-handoff.md. Its old v2
prerequisites must be reassessed when installing Yard v3.
