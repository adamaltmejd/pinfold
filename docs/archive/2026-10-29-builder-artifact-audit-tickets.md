# Open test-proof tickets

Date: 2026-10-29. Reconfirmed by a fresh whole-file sweep at producer main
`5c0ffa2287a51d0fa1a8575181a0a32664b6ef3f`.
These are three Git-tracked written tickets, not scheduled Yard work or
new GitHub issues. They preserve the earlier T1/T2/T3 proposals. No
assertion, guarantee or production control is removed by this audit.

## T1 / G9: observe streams before process exit

Status: open; needs a reviewed observation design and implementation.
Serves callers waiting for stdout/stderr while a command is running.

Evidence: `crates/e2e/tests/e2e/box_.rs:70-77` calls `box_exec`, whose
`Command::output` in `crates/e2e/src/lib.rs:373-384` collects after exit.
Buffering until exit still passes. ARCHITECTURE.md:1248 claims streaming.

Done when the existing guarantee-9 test observes the fixture's known bytes
on both streams while its real guest command remains alive, then releases
the command and checks its status. Use an observable readiness/release
exchange, no sleeps, retries or binary-only test seam. Name buffering until
exit as the sabotage; review that comment without executing the sabotage.

Expected values come from the guest fixture. Preserve current assertions
until a replacement proves the same exit/content contracts. Extend the
existing test only; real Mac and Linux execution and test-audit review are
required. No new guarantee, test, dependency or production change is chosen.

## T2 / G2: prove the ordinary HTTP control was allowed

Status: open; needs a reviewed correlated audit observation.
Serves callers relying on ordinary allowlisted HTTP and its refusals.

Evidence: `box_.rs:1295-1301` accepts curl exit 0 even for a proxy HTTP
403. A previous CONNECT decision or host-service route is not proof of
this ordinary HTTP path. ARCHITECTURE.md:1243 requires positive controls.

Done when this request's own newly appended `allowed` / `allowlisted`
decision is observed, with its actual upstream response and transport
success. An earlier decision must not satisfy it. Do not merely add curl
`--fail`: an upstream GitHub 403 is distinct from a proxy policy refusal.
Expected values come from the fixture request and spec. Name refusal of
all ordinary plain requests as the sabotage, without executing it manually.

Extend guarantee 2's existing test, preserving the current control until
the replacement is proved. Require real Mac/Linux execution and test-audit
review. No egress-policy change, new endpoint, new test or weakened reason.

## T3 / G39: preserve coverage without private basenames

Status: open; observer design unresolved, not ready for implementation.
Serves the proof that the caller's known secret never enters VM artifacts.

Evidence: `pi.rs:792-800` requires `config.json`, `init.ext4` and
`mount0.ext4`; the producer locations are `runtime/firecracker/vm.rs:87-94`
and `:304-306`. The names are private layout, but the block also proves
that the live scan visited artifacts. Deleting it alone weakens the proof.
ARCHITECTURE.md:1266 still names configuration/init/disk secrecy.

Done when an outside observer establishes equivalent coverage of the
actual configuration/init/mount/disk artifacts without private basenames,
and the complete scans still reject the known fixture secret. Choose and
review that observer before editing. Preserve `holding`'s coverage output;
two guarantee-26 sites also consume it.

No introspection API, test-only flag, new test or guarantee narrowing is
authorized. Keep every existing assertion until the replacement is shown
at the real CLI/host boundary and reviewed under test-audit. Any proposed
contract narrowing requires Adam's substantive spec decision.

These are inherited proof limitations, not reproduced product failures.
Their release disposition remains an explicit operator/Adam decision.
