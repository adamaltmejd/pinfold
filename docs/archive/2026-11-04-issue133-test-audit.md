# Issue 133: live audit log cap

Approved contract: each box or Linux build has a fixed 128 MiB egress log
cap. The next complete encoded JSONL line, including its newline, must fit
in full; equality fits. An over-cap line writes nothing and follows the
existing audit-log failure path to stop the box or fail the build. Logs
are not rotated or truncated, and decisions are not silently dropped.
Route auditing remains after forwarding; owner notification and teardown
remain asynchronous.
Linux builds retain their existing publication boundary: an audit failure
arriving after catalog commit does not reverse the successful publication.

Test admission:

1. Guarantee 42, `run_records_are_retained_within_bounds`, is extended.
   No new test or runtime is added. Its retention cases stay in place.
2. The test comment names removal of the live cap or checking only the
   current size. Either permits another line beyond 128 MiB and fails the
   owner-exit or unchanged-length assertions. No sabotage was run.
3. Existing blocks exercise inactive retention and continued appends to a
   live log. They do not fill it. After the same box's successful route
   request, the host pads its real log to cap minus one byte with complete
   valid JSONL. The next real request cannot fit even one complete line.
   This tests the boundary after live retention, without parsing the large
   log or duplicating production serialization. Exact fit and lock
   interleavings remain source-reviewed, not additional runtime claims.
4. The specification supplies 128 MiB, exit 1 and `audit-log`. The host
   file supplies the log length; the owner process supplies its exit and
   down line. The host HTTP fixture and earlier request supply the
   positive control in the same box.

The implementation uses the existing append/failure mutex, opens the log
once per append, checks that file's metadata plus the encoded line with
checked addition, and preserves the existing latched owner notification.
The shared proxy also serves Linux builds. No counter, extra descriptor,
lock, dependency, flag or stop reason is added.

Changed lines against `3a8e1fb`: test +30/-3 (33); binary +13/-1 (14).
The padding helper now appends exact byte counts, using legal trailing
whitespace before the last newline. Existing retention pads still exceed
their intended aggregate quota.

Independent correctness/security and lean/spec/test-admission reviews passed.
Operator validation passed: format, locked all-target Clippy with warnings
denied, focused Mac G42 (1 passed, 41.92 seconds), and the full ordinary Mac
suite (26 passed, 1 ignored, 95.67 seconds). Logs are retained under
`/private/tmp/pinfold-133-*`. No sabotage was run. Linux CI remains a
separate PR gate; its slow workflow explicitly selects the ignored test.
The global timing and assertion-pruning audit remains separate.
