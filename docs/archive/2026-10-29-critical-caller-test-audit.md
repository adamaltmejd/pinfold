# Critical caller-build test cut

Date: 2026-10-29. Adam explicitly approved G23's A/B/F scope: successful
A, changed B and ordinary failed F. Shared default-image setup is separate.
This deliberately removes dedicated coverage; production image behavior
and security controls remain unchanged.

| G23 block | Disposition and outside expected value |
|---|---|
| A/B context and refs | Retain two builds. Runtime-observed ids/refs agree with build output; A's unique ref reads A after B, and latest reads B. Expected bytes and labels are literal caller fixtures. |
| Linux post-commit EIO | Retain the existing real kernel injection on A, with its receipt and successful outcome. It adds no build. |
| Ordinary failed F | Retain the 200,000-byte unterminated diagnostic, spec-derived 64 KiB bound and caller-file final marker. Compare every caller ref-to-id mapping before/after, with positive presence checks; unrelated Apple namespaces are excluded. |
| Three-success retention and source composition | Remove the third success, zero-layer profile/multiple-source composition and their assertions. A's older unique handle still survives B, without claiming the removed matrices. |
| ONBUILD parent/child | Delete the child build and trigger assertions. Production ONBUILD behavior remains unchanged. |
| Deduplication, no-cache and profile reruns | Delete the repeat, explicit no-cache and two profile rerun builds and their stamp checks. No post-failure cache-retention proof is claimed. |
| Selected-profile freshness | Delete the four-build helper in `pi.rs`, now orphaned. The production shared base/profile rule is unchanged. |
| Apple overlapping color settings | Delete the held/concurrent builds and orphaned `HeldBuild` fixture. |
| Remaining orphan helpers | Delete `file_from_image` and `built_unique_ref`; their removed scenarios were their only remaining callers. Shared image command/result helpers remain unchanged. |

Named sabotage covers missing context, reused or moved refs, stale ids,
lost image labels, reversed post-commit success, premature failed-build
publication and lost/unbounded failure tails. The hard case is A's handle
after changed B, followed by an ordinary failure on the same name. The
reference map comes from outside runtime inventory; its prior snapshot
is the expected value for the unchanged-ref guarantee.

G23 now has exactly three case builds on both platforms. `box_.rs` shrinks
from 4,780 to 4,484 Rust lines and the orphan helper removal takes `pi.rs`
from 1,471 to 1,407. Production Rust remains 18,096 lines and is unchanged.
ARCHITECTURE.md's row 23 names only retained proof; production contracts
for the removed variants remain in the image specification.

G44, its readiness helper and `fill_cache.c` are byte-identical to
`97da8da6396c74979b8ea9432def2a464c06b48a`. Other test functions and surviving
shared helpers are unchanged. Scoped source reviews and formatting
completed. No compiler, suite, VM or public push ran for this cut;
platform correctness and performance remain unverified.
