# Upstream write deadlines

Issue #175. With unchanged product code at 60e086a, G34's new upload case
failed on Mac: its producer ended after 300.113s, but the upstream remained
open beyond 330s. The response control closed after 300.014s. The full suite
reported 26 passed and 1 failed. Direct reuse of Deadline alone also failed;
Darwin's blocking socket writes needed explicit nonblocking mode.

The private TCP wrapper now uses Deadline for plain HTTP and below rustls.
On Mac it restores blocking mode after either write outcome. Every write
error drops the sole upstream socket: rustls can defer a transport error
after accepting plaintext, so shutdown without release was insufficient.
CONNECT, read timeouts and the production five-minute duration are unchanged.

G34's small POST/response is the positive control. Both blocked directions
run concurrently in one box and one production interval. Host fixtures
observe peer closure without draining the blocked upload. A host-timed
producer completion marker excludes early producer failure followed by a
read timeout. Expected bytes/times come from fixtures and the contract.
Restoring the old upstream writer is the named failure; the response-only
case did not exercise it. No new test, dependency or binary seam was added.

Independent correctness/security and lean/test-admission reviews passed.
Format and locked all-target Clippy passed. The final full Mac suite passed
27/0/0 in 353.03s: producer 300.114s, response closure 300.015s and upload
closure 305.042s. Linux results are recorded on the PR. The blocked negative
case is plain HTTP; TLS placement is source-reviewed and the existing HTTPS
scenarios supply compatibility coverage, not a blocked-TLS runtime proof.

## Caller changes

None. This enforces the existing timeout without changing CLI, JSON or spec
inputs.
