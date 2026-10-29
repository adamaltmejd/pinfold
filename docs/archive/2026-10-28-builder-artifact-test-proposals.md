# Test-proof proposals from the builder artifact audit

Date: 2026-10-28. Observed source: main
`ff6276cb832dfc78e4cb716857e33af1f00f12b8`.
These are written proposals, not scheduled work or reproduced product bugs.
No test assertion or guarantee was changed by this audit.

## T1: observe guarantee 9's stream before process exit

Mechanism: `crates/e2e/tests/e2e/box_.rs:69-77`, through `box_exec` in
`crates/e2e/src/lib.rs:373-379`. The helper uses `Command::output`, which
waits for exit. The collected stdout/stderr and status cannot distinguish
live forwarding from an implementation that buffers until exit.
ARCHITECTURE.md line 1241 explicitly claims streaming; line 163 specifies
stdio through. This serves programmatic callers waiting on live output.

Proposed observable result: the existing guarantee-9 test observes bytes
on stdout and stderr while its real guest command is still held alive,
then lets the guest finish and checks its status. Expected bytes come from
the guest fixture. Coordination uses an observable readiness/release
exchange, without sleeps, retries, mocked runtime or a binary-only test seam.
The test names buffering-until-exit as the change that makes it fail.

Extend the existing test, adding no test or guarantee. Remove the redundant
exit-only stream-content block if the new observation covers it. Do not
execute the sabotage. Proving the revised observation at the real Mac and
Linux CLI boundaries is required before accepting a rewrite. No production
change, new output guarantee or feature removal is proposed.

## T2: observe the ordinary plain-HTTP positive control's decision

Mechanism: `box_.rs:1294-1301`, guarantee 2. Curl currently accepts any
HTTP response; a proxy refusal returning 403 still produces transport
success. The earlier successful CONNECT and host-service route do not
prove the ordinary plain-HTTP allowlist path; `core/proxy.rs:303-309`
handles routes separately. ARCHITECTURE.md line 1236 requires the controls
and line 587 names the plain-HTTP policy. This serves callers relying on
allowed HTTP requests as well as refusals.

Proposed observable result: after the real allowed plain-HTTP request,
observe that request's own newly appended `allowed` / `allowlisted` egress
decision. An earlier CONNECT decision must not satisfy the check. Retain
transport success. Do not merely add curl `--fail`, because an upstream
GitHub 403 is distinct from a proxy policy refusal.

Extend guarantee 2's existing test; expected host/reason/decision come from
the fixture request and spec. Name refusal of every ordinary plain request
as its failure mechanism. No production egress change, new public endpoint,
new test or manually executed sabotage. Real Mac and Linux verification is
required for an accepted rewrite. No product bug is claimed from this review.

## T3: remove private filename coupling while retaining scan coverage

Mechanism: `pi.rs:792-800`, guarantee 39. The scan-coverage assertions
require implementation basenames `config.json`, `init.ext4` and
`mount0.ext4`. AGENTS.md excludes file-layout assertions. However, these
also establish that the live secret scan visited VM artifacts. Deleting
them alone weakens the absence proof and is rejected.

Proposed result: an outside observer establishes coverage of the actual VM
configuration, init and mount/disk artifacts without depending on private
basenames, while the complete state/root scans still reject the caller's
known secret. The current observer must remain until a replacement can be
specified and demonstrated through the real CLI and host boundary. This
proposal has no chosen replacement and is not implementation-ready.

Do not narrow guarantee 39 or its Shown by to make the finding disappear.
A narrowing requires Adam's substantive spec decision. Preserve `holding`'s
coverage output: two guarantee-26 call sites also consume it. No helper is
orphaned. No new introspection API or test-only binary path is authorized.

## Admission and decisions

T1 and T2 concern missing proof of existing guarantees, not evidence of a
product failure. Their scope is one existing assertion mechanism each.
They need a reviewed concrete test rewrite before becoming active work.
T3 remains a design proposal until an outside coverage observer is chosen.
None authorizes deleting a guarantee or changing a production control.

Retain the current assertions in all three cases. Operator and Adam may
decide whether these inherited test-proof limitations require repair
before the artifact-only publication. They are explicitly disclosed; this
audit does not declare them fixed or waive a release requirement.
