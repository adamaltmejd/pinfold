# 0.2.1 release validation held

The audited candidate is `4e195c6` on local branch `codex/release-0.2.1`.
The release was not published. No branch push, PR, tag or installation was
made. The issue 82 change, version and harness pins, and release audit are
committed locally. See `2026-10-08-code-cleanup.md` for the audit snapshot.

## Results

| Check | Result |
| --- | --- |
| Formatting and diff whitespace | Passed |
| All-target locked Clippy with warnings denied | Passed |
| Production dependency and dead-code audit | Passed |
| Durable prototype frozen install and TypeScript check | Passed |
| Mac durable recovery gate against the candidate binary | Passed |
| Full Mac Rust suite at `0202a73` | 32 passed, one failed, one ignored; 292.35 seconds |
| Full Mac Rust suite at `4e195c6` | 32 passed, one failed, one ignored; 556.90 seconds |
| Linux suites, slow guarantees and release builds | Not run for this candidate |

The durable gate observed host-boundary refusals, boxed execution, an
exclusive checkpoint writer, crash recovery without unsafe replay, and
whole-box cancellation. It does not replace the failed Rust suite.

The issue 82 fix before the version and harness-pin changes passed the
focused lifecycle test and full Mac suite. That earlier pass does not
validate the final release candidate. The original first-exec failure's
cause remains unconfirmed; `2026-10-08-test-audit.md` records that limit.

## Release blocker

Both release-suite failures occurred during acknowledged shutdown, after
the tests' behavioral assertions passed. The first was
`box_shares_files_with_the_host`; the second was
`nothing_can_gain_privileges`. Both reached the same `box down` helper and
reported `Resource temporarily unavailable (os error 35)`.

Exact-box native logs show paired Apple I/O completion timeouts, followed
by delayed runtime-service removal. This fits the owner's ten-second
acknowledgement deadline. It does not establish the source of the delay.
[Issue 83](https://github.com/adamaltmejd/pinfold/issues/83) retains both
occurrences and the next investigation boundary. This failure is distinct
from issue 82's immediate-exec observation.

Apple container 1.5.0 source has monitor and graceful-stop paths that can
both wait on process completion. Its pinned containerization dependency
counts stdout/stderr completion events from one stream separately for
each waiter. Split events are a plausible explanation for the paired
timeouts, not a demonstrated cause. Closing stdin is not supported by
that mechanism: stdin is excluded from its completion tracker.

A bounded native experiment ran eight real boxes, four at a time, with
the actual guest init and continuous host stdout draining. Four kept
stdin open during removal; four closed it first. Every exec and removal
succeeded; removal took 0.121 to 0.340 seconds. This did not reproduce the
delay or establish a workaround. No teardown code, timeout, assertion or
retry policy was changed to suppress the release failures.

The next evidence needed is owner-side timing around runtime removal and
child reaping, correlated with the native completion events. The required
Mac gate remains failed. Linux CI and release builds were not started.

## Retained evidence and cleanup

Scratch evidence is under `/private/tmp/pinfold-release-021/`:

- `macos.log`, `macos-final.log`: both complete suite outputs.
- `files-runtime.log`, `privileges-runtime.log`: exact-box native logs.
- `native-teardown.py`, `native-teardown.log`, `native-teardown/`: bounded
  native experiment and per-box results.
- `durable-macos.log`, `pins.log`, `trace-tables.md`: independent gate,
  publisher-verified pin updates and audit trace tables.

Final native inventory contained only the existing shared `buildkit`
container. No test or diagnostic box remained. The installed binary and
shared runtime configuration were not changed.
