# Audit shutdown completion reassessment

2026-10-16. Pinfold #100. The user requested reassessment of the five-second
owner deadline and the smallest safe solution using stock Apple Container.
This supersedes the investigation's assumption that preserving that test-only
deadline required a native runtime correction. The Apple report stays local.

## Contract and cause

ARCHITECTURE.md requires an audit append failure to stop the box, emit
`down/audit-log`, exit 1, and leave no native box. It specifies no numerical
audit shutdown deadline. The five-second owner check was introduced with the
audit-failure scenario in `3aacad4`. It starts after the faulting request
returns and measures owner completion, including native cleanup. It is not a
measurement of when all guest work or proxy traffic stopped.

The retained native-only experiment establishes that stock Apple Container
1.5.0 can stop A's workload while A's KILL completion waits for B's bootstrap
to release a shared lock. That explains a completion dependency, not the exact
schedule of the original intermittent failure. Apple's SIGKILL completion wait
is deliberate ([PR1589](https://github.com/apple/container/pull/1589)). Apple
also acknowledges delays above ten seconds from the shared bootstrap/exec lock
([PR2063](https://github.com/apple/container/pull/2063)); that example concerns
exec starts, not an explicit shutdown-latency guarantee. No violated upstream
latency promise or committed upstream fix has been established.

## Smallest correction

Replace the audit scenario's bespoke five-second owner poll with its existing
`Up::wait` helper. That helper requires completion within 60 seconds and is
already used for other owner exits. Sixty seconds is a test hang guard, not a
new product shutdown guarantee or an added sleep. Normal completion returns
immediately. Retain the suite's separate five-minute CI budget.

Keep the failed-request assertion, exact `audit-log` reason, exit 1, direct
native absence check and successful route controls. Also reject curl's timeout
exit 28: a request that merely times out does not demonstrate failure handling.
No production source, runtime command, production timeout or ownership rule
changes. No dependency or private Apple Container build is introduced.

The audit latch rejects new accepted/handled connections after failure. It does
not synchronously abort every already active stream or request. Already logged
tunnels and in-flight route requests rely on box termination to end; route
logging happens after forwarding and can fail after an upstream side effect,
as the spec already states. This change adds no claim of immediate total egress
cutoff. The old owner-completion assertion did not directly prove one either.

Native KILL, ordinary removal, attached-output draining, child reaping and
state cleanup remain ordered before owner completion and name release. A timed
out `box down` request still fails without taking ownership from a live owner.
Allowing additional observation time does not make incomplete teardown pass.

## Test audit

Guarantee 4, `a_route_reaches_exactly_one_host_service`:

- Keep successful fixture requests and their externally supplied HTTP status.
- Keep the real host log-file-to-directory fault and failed request.
- Reject curl exit 28; its documented timeout result is the outside expected
  value. The same five-second request bound remains.
- Keep the spec's `audit-log` reason and exit 1, and the native runtime's
  independent absence observation.
- Replace only the duplicate owner polling with the existing lifecycle helper.
  Ignoring append errors or omitting owner notification still fails the block,
  as its sabotage comment names. No new test or guarantee is added.

Test source changes: four lines added, sixteen removed. Production Rust lines
changed: zero. Independent review and validation results are recorded in the
pull request before merge; failed historical runs remain retained separately.
