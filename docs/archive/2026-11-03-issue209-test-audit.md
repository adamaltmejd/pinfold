# Issue 209: image retention proof

G15 reuses its disk creator and live-project boxes as two retention pins.
Four real project builds P0–P3 differ by fixture recipe labels A–D. P0's
box stays alive through P2, then stops. P1 remains held while P3 makes P0
older than the newest two. The existing caller-retire build now derives
from P2 between P2 and P3. Runtime inventory must contain P1, P2 and P3,
not P0. The existing live disk and caller-retirement controls remain.

Test blocks: `crates/e2e/tests/e2e/runtime.rs:79-161,198-232`.
Binary mechanisms: `core/image.rs:87-99` clears inherited family labels;
`core/clean.rs:362-388` keeps two builds and skips only the pinned image.
The comments name sabotage: leaving the caller's inherited project family
evicts P2's rollback ref; stopping at pinned P1 keeps unused P0. Removing
the pin check loses P0 before its box stops. Prior blocks proved explicit
retirement and live/idle cleanup, not this retention ordering.

The maintenance contract supplies the expected survival/removal. Host
runtime inventory supplies each build's ID and immutable ref, independently
of Pinfold's build output. Four distinct runtime IDs prevent cached identical
images from making the pin observations vacuous. No private labels or tag
timestamp order are asserted.

Three additional project builds replace one platform-specific fixture;
there are no additional box launches, test rows, helpers or production
changes. Linux leaves managed images to ordinary retention; Apple uses
the existing image cleanup guard. CI cost must be measured, not assumed.

Independent correctness/security and lean/test-admission reviews passed.
Format and locked all-target Clippy passed. The ordinary Mac suite passed
26 tests, with the existing slow test ignored, in 105.23 seconds.
