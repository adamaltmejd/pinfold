# Issue 82: Apple readiness without egress

Issue: [#82](https://github.com/adamaltmejd/pinfold/issues/82).
Base: `c71a7ee`. Host: macOS 27.0.1, Darwin 27.0.0, Apple container 1.5.0.

## Evidence

The original installed-0.2.0 exec stderr and owner log were lost. Its exact
cause remains unconfirmed. Eight diagnostic launches of the installed
0.2.0 binary all passed: no egress, owner stdin held open, first exec issued
immediately after the public ready event, then stdin closure and owner exit
0. Each retained the spec, owner stdout/stderr, exec stdout/stderr and exit
codes under `/private/tmp/pinfold-issue82/baseline`. The probe stops at a
failure and captures exact-box runtime state before teardown. Passing
repetitions do not explain the reported failure.

Source inspection found a missing readiness check. `start` called Apple's
`wait_until_running` only with a proxy socket. Without egress, its final
runtime lookup required existence but ignored `running`. The guest can
report ready before Apple accepts exec. The existing wait addresses that
same gap for the proxy's root exec; the observed historical failure is in
`2026-09-28-apple-box-loss.md`. This establishes an omitted gate, not the
cause of the lost original error.

## Change

Every Apple box now uses the existing running-state wait before public
ready. Only the socket permission adjustment remains conditional on egress.
The five-second deadline, failure cleanup, security controls and Linux
startup path remain unchanged. No dependency or test-only path was added.

## Test audit

Scope: the changed assertion block in guarantee 9, not a suite-wide audit.

- Row 9 retains its existing test and assertions. The runtime image lookup
  moves before startup; the existing stream and exit-code checks now run
  immediately after ready without an intervening runtime query.
- Sabotage: restore the socket condition around Apple's running wait. The
  first exec can then fail during Apple's readiness gap. The comment names
  that scheduling dependency; this does not force a deterministic race.
- Expected values come from the shell fixture: `out\n`, `err\n`, exit 3,
  and a same-box zero-exit control. Image identity comes from the runtime.
- The first status assertion includes both output streams in diagnostics.
  No new assertion block, helper, test, sleep or retry was added.
- Test diff: 19 added / 11 removed lines. Binary diff: 8 added / 6 removed
  lines, including the existing wait's comment correction.

## Verification

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- Guarantee 9 alone on Apple: passed, 128.98 seconds including the binary
  and image build and the existing lifecycle scenarios.
- Full macOS Rust suite: 33 passed, 0 failed, 1 ignored, 386.69 seconds.
  The ignored test is the separate five-minute response-deadline gate.
  This is a local result, not a measurement of the CI budget.
- Linux suites, standalone durable gate and slow proxy gates were not run.
- An independent read-only review found no additional issues.
- Suite logs: `/private/tmp/pinfold-issue82/lifecycle.log` and
  `/private/tmp/pinfold-issue82/macos-suite.log`.
