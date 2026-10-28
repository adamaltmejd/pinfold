# Builder integration test audit

Date: 2026-10-28. Source candidate after phase 6a.
This records assertion review, not runtime passage.

Guarantee 23 keeps real COPY/launch, repeated-content identity, no-cache
random stamp, rollback and bounded failure-tail observations. Delete the
Podman-only intermediate-image block and orphan helper: it describes the
retired storage mechanism, not the caller guarantee. The existing harder
cache/no-cache scenarios remain. Expected files/stamps and label values
come from the fixture; agreement assertions are the named guarantee.

Guarantee 15 uses real trusted project builds for Linux project labels,
instead of direct native runtime builds. The unlabelled external-image
case is Apple-only because the Linux catalog admits only explicit Pinfold
builds. Completed-root survival/removal is observed on the host around
last-reference removal; omitting last-reference root deletion or ignoring
the held lease is its named sabotage. Existing twin-name/in-use cases
remain the hard case, with fixture names and expected survival from spec.

Guarantee 27 deletes the obsolete runtime-directory scenario/token. Keep
unchanged owned-tree snapshots and actual ioctl/AppArmor failures. An empty
PATH cannot prove missing mkfs because normal discovery also checks sbin;
no such assertion is invented. Expected trees are independent host snapshots.

New guarantee 44 observes actual RUN uid/capabilities/no_new_privs/userns and
interfaces, plus allowed/denied requests and production audit in the same
build. A completed image then launches after public clean and removal of
its private normal pinned-builder install. Source changes that run work on
the host, alter the approved guest state, omit explicit proxy allowance or
audit, or rebuild during launch are named sabotages. Expected values come
from the role/kernel contract, HTTP 403 and the fixture marker. Existing
agent-role tests do not exercise this builder boundary.

Linux helpers observe the actual documented persisted catalog. They do not
replace the runtime or synthesize CLI output. Private caches copy only the
normal pinned archive; the production installer verifies its bytes. No
binary test flag, mock, unit test or internal import is introduced.

This audit does not claim protocol-corruption, cancellation, cache/base
races, actual old-store preservation, resource boundaries or ACL/capability
fidelity passed. Those remain integration admission work. Run host suites
before pushing and record test/binary line counts with the final evidence.
