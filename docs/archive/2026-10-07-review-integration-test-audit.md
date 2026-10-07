# PR 81 review integration and test audit

2026-10-07. Review baseline: c0ee064. The ten inline review comments were
accepted. Sol 6.1 agents implemented the ownership/cleanup, lifecycle and
pipe changes. Each production group received independent review by an
agent who had not authored it.

## Dispositions

| Review finding | Change |
|---|---|
| Probe locks cause false name-in-use | Claim acquisition waits through contention for up to ten seconds, remains signal-cancellable and never bypasses the lock using a pid. |
| Audit receiver can be polled after completion | Explicit Off/Waiting/Failed state retains failure after consuming the receiver. |
| Abort bookkeeping hides signal outcome | Cleanup errors are diagnosed without replacing the intended stop; actual runtime removal failure still overrides it. |
| Cleanup inventories once per project | Candidate project locks are taken before one fresh runtime/state inventory and held through removal. |
| Cleanup eligibility duplicated | Measurement and removal share one eligibility function, including cache existence; removal still requires the measured path to match. |
| Orphan cleanup lists twice | The guarded removal reports whether it removed the matching runtime generation. |
| Download and login pipe loops duplicate | Shared nonblocking/deadline/poll helpers preserve flags, cancellation, bounds and reaping. |
| Durable prototype outside binding spec | Architecture now owns its scope, dependencies, manual pin maintenance and named standalone guarantee 35. Supported prototype validation remains macOS. |
| Optional signals have no None caller | Both callers pass signals directly; dead branches removed. |
| Teardown reply consumed only a prefix | Full bounded done/gone/failed tokens are matched. The absolute deadline is recalculated before every read/write, including fragmented replies. |

Independent review found the fragmented-reply deadline problem in the first
revision; it was fixed and reviewed again. No new dependency was added.

## Test audit

- G9: extend the existing stopped-owner scenario. The host observes SIGSTOP
  before a second XDG root attempts the same name. The refusal must consume
  the spec's ten-second contention budget and finish within fifteen seconds.
  Removing the contention wait is the named sabotage. Existing checks prove
  the refusal reason, preserve the owner's state and run the same spec after
  acknowledged teardown. The prior block did not detect premature refusal.
- G35: register the existing actual-box prototype fixture under a guarantee
  name. Keep host-boundary refusals with the successful boxed control, the
  competing writer with its original-owner control, and crash/cancellation
  observations supplied by the host fixture and runtime. Specify
  checkpoint-in-use and tool-cancellation exit 130 as outside expected values.
  Add the missing writer-lock sabotage comment.
- G35: delete the second mutation-count assertion. The surviving assertion
  observes the host file before the resumed model response, which is stronger.
  Delete the final request-count assertion: interrupted tool output and the
  host mutation already prove the contract, without counting protocol turns.
- G4, G15, G19, G26, G28 and G29: retain their existing assertions for audit
  shutdown, safe cleanup, real harness loading, helper cancellation, atomic
  updates and bounded checks. The consolidation does not add duplicate tests.

The ordinary Rust suite has 7,378 test/harness lines beside 9,060 production
Rust lines. The standalone TypeScript fixture has 227 lines beside its
272-line controller. These are source line counts, not release audit metrics.

## Validation

- Full Apple Container suite: 33 passed, one intentionally ignored slow
  proxy test, 177.56 seconds. Log: /private/tmp/pinfold-review-mac-20261007.log.
- Guarantee 35: passed with actual Mac boxes after the Rust suite, including
  crash recovery, competing writer and whole-box cancellation. Log:
  /private/tmp/pinfold-review-durable-20261007.log.
- cargo fmt --check, strict all-target Clippy, git diff --check and the
  prototype typecheck passed after integration.
- Linux results are attached to this commit's PR checks; they were not yet
  available when this record was written.
- The separate five-minute proxy gate was not rerun: this revision changes
  no proxy implementation or assertions. It passed in the earlier record.

The completed audit receiver cannot currently be awaited a second time
through the CLI: both callers wait once, then tear down. Its reentrancy fix
is independently source-reviewed, with G4 covering observable audit failure.
The exact claimed pre-ready cancellation plus a host bookkeeping failure
has no deterministic synchronization seam in the current CLI. That rare
combination is source-reviewed; no internal manifest fixture, runtime stub
or flaky timing test was added. The claim regression proves bounded waiting
under a real live owner, not the exact maintenance-probe interleaving.
Linux execution of the durable prototype remains unverified.
