# Helper integration candidate

Date: 2026-10-28. This candidate replaces the old e7dd bootstrap proposal.
No release, tag, public URL or publication is authorized.

The compact helper compiled and passed the real pinned BuildKit component
cases: external zero-layer and layered bases, local OCI aliases with separate
stores, ARG/COPY reachability, an unreachable stage and scratch. It emits only
resolved input records. Guest LLB/source-map parsing is removed. The host
validates bounded records against the original frozen store order and image
identities, preserves aliases, leases selected IDs once and retains the
default-deny execution policy with exact external digest pins. The public
base-field rule remains the approved shared selected-profile identity.

The producer compiles the helper from the same BuildKit source/vendor tree
using pinned Go with network/module resolution disabled. This dependency is
necessary to capture configuration-only bases missing from the native LLB
source set. Go remains a producer input, not a host consumer requirement.
The bootstrap adds no agent init or transport service.

## Preserved actual bytes

Durable root:
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.

| Item | Bytes | SHA-256 |
|---|---:|---|
| Bootstrap archive | 122578909 | 09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4 |
| Raw root | 1073741824 | ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968 |
| Helper binary | 22282402 | 0b8927187764740c328a245e21c44a08cb419aa93911d3be624e4c943f2907d0 |
| Helper source | | bd5fda8f957fdb9d887a85caffeb6e78ce5c96f380bd52122847b5d6d6ceb335 |
| Compact API proof bundle | | 725983f4a29deb079e7eccbf502e211e1c1918cfc6ca2abbeb4c366f0506dbeb |
| Complete producer bundle | | f38ed6c8b82cbee9a361d09514337304e135e1897ae9827c4361105fec665127 |

Recipe identity:
`d4fea170a1a33385e33951a981e649fc6b07053c5e965b99dc620719e744c30c`.
The archive filename is `pinfold-builder-linux-x64-<full-archive-sha>.tar.gz`.

The producer completed and verified every SHA256SUMS entry. Its full output
was copied immediately as `bootstrap-helper-evidence.tar.gz`. Independent
host verification checked all sidecars and source hashes, archive size/hash,
the single named root entry and its streamed raw size/hash. No root was
mounted. The helper binary matches the separately compiled and tested compact
helper byte-for-byte. `bootstrap-helper-independent-receipt.json` records the
verification. The preserved flat output is under `helper-artifact-<sha>/`.

The exact-commit BuildKit source archive is pinned to reviewed bytes and a
publisher signed-tag/commit record; its archive hash is measured from that
HTTPS response, not a separately published tarball checksum. The new producer
input trust chain requires independent supply-chain review. Go's archive has
the publisher's checksum and satisfies the unchanged seven-day age rule.

Consumer pins now name these exact candidate bytes. The old publication
proposal is obsolete. The normal offline cache remains the only available
consumer installation path until artifact distribution is approved.

## Gates still open at this source checkpoint

The compact API proof bypasses Pinfold's jail, proxy and claims. It does not
replace normal CLI integration, full current-head Linux/Mac suites, complete
metadata/failure/concurrency/resource evidence or the suite budget. The lost
earlier detailed metadata artifacts still need reproduction. Numeric resource
policy, immutable publication and final independent correctness/security plus
lean/spec/assertion reviews remain admission decisions.
