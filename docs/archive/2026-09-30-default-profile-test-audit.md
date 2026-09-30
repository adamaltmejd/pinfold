# Default profile notice audit

Added guarantee 31. Seed-once behavior is unchanged. No existing test
assertion changed. Earlier updater and image-warning work remains in the
same working tree.

Interactive launches compare the bundled profile's fingerprint separately
from the daily network check. Unknown history establishes a silent
baseline. Self-update records a missing baseline before replacement,
without making installation depend on successful notice bookkeeping.
Changed defaults produce one notice, leaving saved settings and user
profiles alone. The notice suggests an explicit copy through the new
profile new --builtin option, which bypasses user overrides.

The new test builds a second real executable from copied production
source with changed bundled settings. Its private target prevents a build
from replacing the executable other tests use. The fixture copies the
repository's pinned Rust toolchain. No mock, runtime stub or binary test
flag was added.

Test-audit gate:

- Row 31 covers change notices and an explicit fresh-default copy.
- The sabotage comment names removing the fingerprint comparison,
  putting it behind the network interval, failing to record the new
  fingerprint, warning without history and resolving --builtin through
  the user's override.
- Existing rows did not exercise changed embedded defaults. The harder
  case changes them inside the network interval, then launches again;
  the copy is made with a user default already overriding the bundle.
- Expectations come from authored fixture source bytes, the spec's
  default-profile-changed token and the host HTTPS fixture's request log.
  Preserved bytes are compared with the original authored user override.
  Warning prose and fingerprint representation are not asserted.

Validation:

- Focused row 31 passed in 22.82 seconds.
- Full Mac suite: 31 passed in 177.70 seconds, including the private
  changed-default binary build. This is within the five-minute budget on
  this host; it does not establish cold CI build timing.
- After the review's toolchain-copy fix, focused row 31 passed again in
  19.36 seconds. No assertions changed after the full run.
- cargo fmt --check, git diff --check and cargo clippy --all-targets
  --locked with warnings denied passed.
- Linux ARM64 musl cross-build through cargo zigbuild passed.
- Linux runtime suites and release builds were not run.

The new test and its source-build helpers occupy 125 lines. The enclosing
test file is 478 lines; the updater module is 332, profile module 217 and
CLI module 1150. No existing assertion block was deleted or rewritten.
