# Fixture allocation and observer search costs

Adam approved two further trials after the small fixture changes did not
demonstrate a 20-second gain. The CPU trial was reverted; cleanup ordering
and doctor image reuse remain. No assertion or production control is removed.

G44's cache-fill helper now reserves real ext4 blocks with bounded mode-zero
fallocate calls before its original write/sync/close exhaustion path. Only
successful allocations consume the existing 8 GiB byte budget. Allocation
ENOSPC ends prefill without emitting the exhaustion token; other errors fail.
Partial reservations are safe because the write loop resumes at the tracked
offset. A fragmented filesystem can still require substantial dense writes;
there is no promised 8 MiB tail bound. This tests real allocation exhaustion,
not equivalent host backing-file data traffic. G44's failed RUN, unchanged refs,
normal same-cache recovery and builder-independent launch assertions remain.

At 18453f6, ordinary CI41323453320/job124898349600 passed 26/0/1 ignored:
276.24s body, 278.798787429s budget clock, exit 0. The earlier two-change head
80271f2 passed all tests but took 333.263000177s. Against the faster released
reference 302.862042162s, this run is 24.063254733s lower. This is an observed
run-to-run difference, not isolated attribution or a universal timing bound.

The next change replaces holding()'s debug Rust substring loop with existing
nix::libc::memmem on Linux and Mac. The nonempty guard prevents a vacuous result
on platforms with different empty-needle behavior. Sparse extent traversal,
8 MiB reads, cross-chunk overlap, every secret, all call sites and live/stopped
file-coverage controls remain. No needle enters argv or diagnostics. No new
dependency, helper framework, custom build profile or timer boundary is added.

Both source changes passed independent correctness and test-audit reviews.
The real static Linux C fixture compiled with warnings denied. Scanner format
and locked all-target Clippy passed; its full Mac suite passed 24/0/1 ignored
in 112.07s, including live login. Final Linux scanner timing is pending at
preparation; later results belong to PR256 and the private trial receipts.
