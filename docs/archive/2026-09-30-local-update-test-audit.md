# Local update test audit

Added guarantees 28 and 29. No existing assertion changed.

Guarantee 28 observes the real CLI through a host HTTPS release fixture,
using curl's normal HTTPS_PROXY and CURL_CA_BUNDLE environment variables.
The fixture serves the real binary and computes its SHA256SUMS with Python.
The test checks a second attempt after checksum refusal, a live legacy
state owner, and a live owner using the same executable with another XDG
state root. After both owners exit, it checks the installed bytes, inode,
permissions, symlink and executable usability. Removing checksum checking,
replacing before checking, writing in place, dropping either owner guard,
or replacing argv[0] instead of the resolved executable is its sabotage.
The two owner guards protect distinct mechanisms.

Guarantee 29 runs the real CLI in a terminal through Python's stdlib PTY.
The fixture records requests. A stalled response never arrives; curl's
one-second timeout ends the request. A second launch must make no request.
The offline launch's output agrees with a launch with checks disabled.
Removing terminal gating, the opt-out, daily suppression or the timeout,
or printing automatic-check errors is its sabotage. Expected values come
from fixture requests, its release tag, the host clock and the disabled
launch. No assertion inspects cache layout or selected diagnostic prose.

The tests use Python 3 and OpenSSL on the host. The binary adds no dependency.
The release fixture uses a local CA; it adds no public test endpoint.
There are no runtime stubs or test-only binary flags.

Validation:

- Both new tests passed together: 2 passed in 8.18 seconds.
- The strengthened silent-offline assertion passed separately in 2.24 seconds.
- The full Mac suite completed in 229.47 seconds: 28 passed, 1 failed.
  Guarantee 23's held image build never reached its host fixture within
  60 seconds. Its child was killed; stdout and stderr were empty.
  The unchanged guarantee 23 test passed in isolation in 24.81 seconds.
  The full-suite failure remains unresolved; isolation does not establish
  its cause or make the full gate green.
- cargo fmt --check and host cargo clippy --all-targets --locked with
  warnings denied passed.
- The Linux ARM64 musl cross-build through cargo zigbuild passed.
  A plain cross-target Clippy attempt lacked aarch64-linux-musl-gcc.
- A live update --check reported the current published v0.1.0 as current.
- Linux runtime suites and release builds were not run.

The installer tests exercise replacement through the local HTTPS fixture.
They do not claim installation of a newer published GitHub release.
The updater and its dispatch add 325 binary lines. The two tests, their
fixture and module registration add 420 test lines.
