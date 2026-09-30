# Image input warning audit

Added guarantee 30. No existing test assertion changed. Earlier local
updater work remains in the same working tree.

Managed image builds record the captured Containerfile bytes as a SHA-256
label. Caller-context builds clear the inherited label because their
context has other inputs. Pi and doctor compare the selected image's
Containerfile; a project also checks its profile's source independently
of the recorded base digest. Missing labels produce an advisory hint.
These labels are diagnostic metadata, not a security boundary.

The new test's hardest case changes profile inputs under an unchanged
project image, before the profile's digest changes. It then rebuilds only
the profile and observes the separate base-digest hint, before rebuilding
the project. Other blocks exercise a profile directly and a trusted
project Containerfile change. Each stale launch succeeds; unchanged and
rebuilt launches have no image-outdated hint.

Expected values come from the authored fixture input changes, the spec's
image-outdated token, and successful execution of the real CLI. The test
does not assert warning prose, labels, hash implementation or internal
file layout. Its comment names removing either source comparison,
removing the base comparison and warning unconditionally as sabotage.
Earlier tests did not exercise Containerfile drift without rebuilding.
There is no test-only binary seam, mock or runtime stub.

Validation:

- Focused guarantee 30: passed in 44.92 seconds.
- Full Mac suite: 30 passed in 160.40 seconds, including both updater
  tests and the caller-image test that timed out in the earlier run.
  This successful run does not establish the cause of that earlier timeout.
- Host cargo clippy --all-targets --locked with warnings denied passed.
- cargo fmt --check and git diff --check passed.
- Linux ARM64 musl cross-build through cargo zigbuild passed.
- Linux runtime suites and release builds were not run.

The new test and its registration add 111 test lines. Image warning and
fingerprint code adds 43 binary lines and removes 10.

Release recovery and seed updates were reviewed but not implemented in
this change. The release workflow needs idempotent promotion and draft
publication to recover from publication failure. Seed updates would change
the current seed-once contract: unmodified files could follow their prior
default, while customized or unknown-baseline files must be preserved.
