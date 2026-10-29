# Reproduced caller label output omission

Date: 2026-10-29. Written follow-up; no fix or label-policy change is
included. This is a bug against the existing `image build` JSON contract:
ARCHITECTURE.md says `labels` includes every label put on the image.

The real Linux CLI JSON-bound control built a scratch image containing
twelve literal Dockerfile labels, `proof.pad00` through `proof.pad11`.
The accepted immutable OCI config contains all twelve with their exact
32 KiB fixture values. The successful `built` line contains only the four
host-supplied `dev.pinfold` keys. Every fixture key is missing. Both
observations are retained in `json-limit-success.tar.gz` under the
integration evidence directory, specifically `positive.stdout`,
`positive.label-observation.json` and the copied accepted config.

The executed binary is
`fdefe37d46cade9d2e4b85c55b2f48ba63b0ad0e94cbd84fcd68090779e602b7`.
The actual trial is described in the accompanying JSON/payload receipt.
It uses ordinary Dockerfile LABEL instructions and no binary test seam.

The shared `core/image.rs` success path resolves the built image to obtain
its ID, discards the rest of that image information, then reports only the
requested label map. The same behavior predates this Linux replacement at
producer baseline `ff6276`. The shared source path suggests Apple is also
affected; this trial does not provide an Apple runtime reproduction.

The smallest proposed fix is to retain labels from the already resolved
image information, while preserving today's nonfatal fallback when a
successful runtime cannot resolve its result. Extend the existing caller
image guarantee's label scenario; do not add a test function or a parser.
Operator review must confirm that row before implementation. Ordinary
Dockerfile labels remain ordinary metadata. This bug does not justify an
exact label allowlist, new reserved-family policy or image-based authority.
