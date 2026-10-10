# Y-12 guest comparison and cancellation guards

Astra found that accepted guest destinations containing `..` could escape
the scan's lexical masking comparison. An ordinary repository could then
look as though its protected files were exposed writable through the parent,
although the runtime child mount masks that occurrence.

The scanner now resolves guest `.` and `..` components lexically before
masking and exact-destination comparison. It uses the existing Component
push/pop pattern. It does not resolve guest paths through the host filesystem
or change the runtime plan or mount permissions. The last export at an exact
destination masks earlier exports, including the implicit init mount.

Guarantee 22's existing accepted fixture spells the read-only child's guest
destination `REPO/.git/../.git`. The child remains before its writable parent
in the runtime mount list. Comparing unnormalized destinations makes this
ordinary fixture falsely refuse admission. Existing readiness, history,
status and project-write observations catch that regression. Both negative
admission orders and existing runtime read-only assertions remain. No new
test function, guest alias attempt or protected-write assertion was added.

Astra also found that cancellation was checked only within each directory's
entry loop. A queue of empty directories could drain without observing it.
The scanner now checks the existing flag before reading each popped
directory, including empty ones. It retains the existing preparation worker
and failed-start cleanup. No new lifecycle mechanism or dependency is added.

This changes production traversal. The operator will refresh matched host
startup measurements. Earlier copied-function evidence in
`/yard/proof/y12-cost` describes the pre-repair source and is not a timing
of this candidate. It was not rerun. No large proof fixture was created.
Alias-complete isolation and host mutation remain outside this admission
check's claim.

Worker checks passed with `CARGO_HOME=/usr/local/cargo`: locked offline
build, format check and locked offline all-target clippy with warnings
denied. The Mac and both Linux runtime suites remain host landing checks.
