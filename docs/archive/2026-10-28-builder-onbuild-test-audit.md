# ONBUILD guarantee 23 supplement

Date: 2026-10-28. Base: `a75e0418b9929b4e407f5d19863b2c72c931b318`.
Assertion review only. No runtime run accompanies this supplement.

Extend `a_caller_builds_an_image_from_its_own_tree`; add no test or helper.
Guarantee 23 now states that a parent's ONBUILD remains deferred at launch
and executes when a child builds from its reference.

The existing caller recipe records a fixed ONBUILD marker. Its first ref
launches after two later builds have moved latest. Through ordinary
`box exec`, the parent must have no marker. A child built from that exact
first ref then launches and reads the fixed marker. The child has its own
ImageCleanup guard. Existing identity, retention, cache and failure-tail
assertions remain.

The two named sabotages are using the original OCI config for FROM-only
conversion, which writes the marker prematurely, and clearing ONBUILD in
the published config, which loses the child's marker. The positive child
rules out a parent check that passes only because the trigger was removed.
The previous COPY and cache cases contain no inherited build instruction
and cannot detect either error. Expected marker bytes come from the
caller's fixed recipe; its timing comes from the guarantee. Both observers
use the real CLI and guest filesystem, with no internal import or test path.

At this source snapshot, added assertions are in
`crates/e2e/tests/e2e/box_.rs:1564-1588`; fixture and sabotage are at
`:1480-1502`. The binary fix is
`crates/pinfold/src/core/image/linux/job.rs:643-739`, including the private
converter config rewrite. The test file changes 29 added lines and one
removed line; that binary file changes 68 added lines and one removed line.
Rust formatting passed. The parent session must run the final-head real
Linux and Mac gates before pushing this assertion change.
