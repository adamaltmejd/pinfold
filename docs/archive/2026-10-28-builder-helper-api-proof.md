# Native helper API proof

Date: 2026-10-28. Production source checkpoint remains `3f6e660`.
The helper is not yet part of that source or a bootstrap artifact.

The current prototype source SHA-256 is
`d0c43945eff413f9065970597b60f7a3ed1641bfd599410be687c9fd8f2a88fe`.
It compiled using verified Go 1.26.10 and BuildKit commit
`f5b59a1f2fb25a0cb7072c398cab9aed7835da4e` with its committed vendor tree.
Module downloads and automatic toolchain selection were disabled.
The resulting helper SHA-256 is
`2d849f73f6e177b9185546b71df16d050087a45e0aa5e7f18d6db0b61954172a`.
The older 1ec6ab source compile is not this proof.

An owned loopback OCI registry served independently generated zero-layer
and one-layer images. Native conversion omitted the zero-layer image's
source operation, while the helper recorded its expected image digest.
The image retained ENV and ONBUILD RUN false; conversion succeeded without
solving RUN. The layered positive control produced the expected source
operation and helper resolution. A second proof also passed two local OCI
aliases sharing one image, an ARG-selected base, COPY from a stage, an
unreachable external stage and scratch. The local aliases retained distinct
base0/base1 store identities; scratch returned no resolutions.

Both VMM-free component trials used the real pinned BuildKit daemon and
native frontend. They do not establish Pinfold jail, proxy, ownership,
normal CLI, image publication or execution controls. They establish the
supported API behavior needed before integrating the helper.

Raw receipts were copied immediately to the durable evidence directory:
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.

| File | SHA-256 |
|---|---|
| helper-api-proof-1.tar.gz | 54591a9f95b42906ca00f06cfb25d473ea1041ff344ea1a0d001cc5d3b72f6ba |
| helper-api-proof-2.tar.gz | c4a550fb4ca21299eb55fea65c0e1a40dac313d9204a03b1ade148eb186590d2 |
| fixture.py | 4510393962afa034e5098c4917004f2360b191b92095e1d7f4a70f0e8a8e0e74 |
| helper-api-proof-driver.py | 30236d5ab23cfcf2253ba62abbbc58bc7c0c682e0dc6ea4b15b67a43112a084e |

The directory also holds the exact helper binary, driver source and full
compile/build-info log. Daemon state and sockets are excluded from bundles.

## Smallest integration

The production helper needs only flat resolver records, not the full LLB
graph or source maps. Keep requested/effective identifiers, the resolved
image descriptor digest and an optional OCI store. A descriptor may be an
index rather than the runnable manifest. The host maps each local store
against the frozen ordered Discover request, checks its image/dummy URI,
deduplicates leases by image ID and preserves selected aliases. It pins
external replies with the existing default-deny execution source policy.
The selected-profile public base field does not change.

The helper belongs beside buildctl inside a newly pinned bootstrap. Go and
the reviewed BuildKit source/vendor closure are producer inputs only.
No host Go requirement, public configuration or permissive policy is needed.
Normal integrated proofs and fresh independent review remain admission gates.

The earlier VM expired before its detailed ACL receipts and roots were copied
out. Its locally streamed logs and source remain, but the missing artifacts
are not retained evidence. Reproduce that component proof before acceptance.
The replacement lease was released when work paused. This resumed trial uses
a separately acquired eight-hour lease; each result is collected immediately.
