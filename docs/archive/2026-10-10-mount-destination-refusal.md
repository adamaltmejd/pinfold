# Y-12 duplicate destination boundary

The operator identified a mismatch between the early duplicate check and
the scanner's normalized comparisons. The check compared raw paths, while
the scanner deduplicated normalized destinations. That could admit two
caller mounts with different spellings of the same destination.

The existing scanner push/pop normalization is now one small `guest_path`
helper in core plan. `Plan::validate_guests` and the scanner both use it.
Duplicate caller destinations refuse as `spec` at the existing early
validation boundary. The scanner does not deduplicate plan mounts. Only
the runtime-added read-only init export masks a plan export at its exact
destination. No general last-export-wins rule is part of the contract.

The existing guarantee 17 refusal block now covers exact caller duplicates,
normalized-equivalent caller duplicates and a profile destination clash.
Its comment names raw comparison and late deduplication as sabotage. The
fixture supplies the paths; the observer is the real CLI refusal and the
existing host/runtime cleanup helper. Existing successful startup in the
same test supplies the allowed control. This extends one assertion block,
adds no test function and asserts no scanner-derived expected value.
Guarantee 22 retains its accepted child-before-parent `../.git` spelling.
Guarantees 11 and 22 retain their passive hardlink admission scenarios.

The earlier scan-guards archive describes the previous implementation;
its general exact-destination masking statement no longer applies.
The proof bundle remains compact: source, driver, exact commands and
original count/timing summaries only. No measurements were rerun, and no
runtime artifact or Yard state was removed. Its copied-function timings
are historical, not measurements of this candidate. Refreshed matched
host startup timings and the Mac and both Linux suites remain landing
checks. Alias-complete isolation, worktrees and host mutation remain
unresolved under Y-9/Y-10.

Worker checks passed with `CARGO_HOME=/usr/local/cargo`: `cargo build
--locked --offline`, `cargo fmt --check` and `cargo clippy --all-targets
--locked --offline -- -D warnings`. Runtime suites cannot run in this
worker, which has neither Apple container nor Podman.
