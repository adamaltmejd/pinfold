# Offline update-check setup audit

2026-10-16. Follow-up to #100. Guarantee 29 retains every assertion and positive
control. Production source, CLI behavior and safety deadlines are unchanged.

The offline scenario used `box list` to prepare its fresh state directory before
entering the terminal helper's ten-second deadline. That command ran daily
maintenance when the scenario was introduced in `b273969`. Commit `0994236`
made listing read-only and exempted it from maintenance. The fresh offline
`attach` then performed maintenance inside the terminal deadline, while its
opt-out comparison had already completed maintenance.

The repair prepares that same fresh environment with `box prune`. This existing
working command runs maintenance without consuming the interactive update-check
cache. Its successful completion is still required by `run_ok`. The output
comparison now includes both command results in its failure message, retaining
helper exit status and stderr when stdout is empty.

Test-audit disposition: keep every assertion block in guarantee 29. The existing
comment names removing suppression or interval checks, increasing the network
timeout, and printing automatic-check failures as sabotages. The harder offline
case still uses a fresh cache and a real stalled HTTPS fixture; no timestamp or
cache record is fabricated. The host fixture supplies the request count, release
tag and observed connection lifetime. The opt-out command supplies the reference
output for the silence comparison. No assertion or expected value changes.

The original failed full suite remains a failure: 32 passed, one failed, one
ignored. Its missing helper status prevents attributing that exact sample to
maintenance. A separately instrumented natural run passed, with the request
closing in 0.988 seconds and the offline command completing in 1.375 seconds.
It directly observed maintenance's native image-list child inside the offline
terminal command. This establishes setup drift, not the historical slow phase.

Test source: three lines added, six removed. Binary source: zero lines changed.
No assertions were added, removed or relaxed. Independent source and diagnostic
reviews are retained with the local investigation evidence.
