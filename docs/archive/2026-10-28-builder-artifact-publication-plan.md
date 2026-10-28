# Builder artifact publication proposal

Date: 2026-10-28. Proposal only. No publication authorization is implied.
This proposes one artifact-only release, not a Pinfold CLI release.

## Exact bytes

Promote the already built and reviewed phase 6a output. Do not rebuild it
and assume identical filesystem bytes. Its source is the `dist-final/`
directory in the preserved phase 6a bundle:

`/Users/adam/Code/pinfold/target/phase6-builder-bootstrap-2026-10-28/evidence.tar.gz`

Bundle: 267,038,450 bytes, SHA-256
`f8681084207eabc7004667342bd97df650d2ee101aa365d8dd75965a03dbd97b`.
Producer tooling is merged at
`ff6276cb832dfc78e4cb716857e33af1f00f12b8`; its original recipe identity is
`d985bd63260afaedcbaf90c3fbfc95f5d077b03cd13407137c3b4c4724ac5e0f`.

The exact promotion files have been staged and rehashed at:

`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/publication/`

| File | Bytes | SHA-256 |
|---|---:|---|
| pinfold-builder-linux-x64-e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0.tar.gz | 114456820 | e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0 |
| manifest.json | 2047 | 55edb75f268a715fe3efb520a75bf429d901820bb35805bdb236703a454bcc26 |
| SHA256SUMS | 636 | ce6cd09bb0ad07aa9cc45d0d4b02df78455cb9e4360e9e2a8d43ccf178e60db4 |
| packages.tsv | 12498 | 041d0dd2bb81bb7d9a73b335f6bbdf4d0160e6d2e682b802d6a5339927f18caf |
| pins.json | 679 | 437cd68c0e9b9c1dc8e779e74e0757332d720fb80d9a8bcff7fd5b94ea2f3687 |
| debian-archive.asc | 25106 | 312f12f3a3043d9aadc5ba532410dab2b45f91eb76230e7509bf6ced903738d4 |
| Release | 138612 | ed56aac47e7911e65ee63aae8d67e29f20e023f840e49f8ed037043a33de138a |
| Release.gpg | 1760 | 9f80d5c8e4f9fd918a8b4fedd35c00ecc6c1a3ca9eddbc981c83823d4a4b8dc4 |

The archive contains the 1,073,741,824-byte root with independently pinned
SHA-256 `710189b13d814e4026e1095bd393dad99465e4042ced5dae069c59fd79cbaa80`.
The manifest is evidence, not the source of digest authority.

## Proposed destination and operation

Repository: `adamaltmejd/pinfold` on GitHub.

Proposed artifact-only tag, not an approved or existing tag:

`builder-root-e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0`

Proposed archive URL, not yet an installed pin or available asset:

`https://github.com/adamaltmejd/pinfold/releases/download/builder-root-e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0/pinfold-builder-linux-x64-e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0.tar.gz`

After explicit artifact-only approval, verify the staging checksums again
and refuse an existing tag or release. Create one new prerelease targeting
the phase 6a merge above, with `latest=false`, the eight exact files and
the reviewed notes in `2026-10-28-builder-artifact-release-notes.md`.
Use `gh release create` with `--target`, `--prerelease`, `--latest=false`
and `--notes-file`; never use clobber, asset replacement or tag force.
The full archive digest is the artifact version and name. A later build
needs a new identity, never a rehash at this one.

This follows the existing guest-kernel artifact publication pattern.
The current builder-root workflow remains build/upload-only. A one-off
promotion of the preserved reviewed bytes avoids rebuilding a different
filesystem merely to publish it. A future automated promotion workflow
needs its own reviewed producer/run identity and expected-byte authority;
the sidecar manifest alone cannot authorize publication.

The CLI release workflow triggers on `v*` tags or explicit dispatch. This
proposed tag does not match `v*`, and no dispatch is part of this operation.
The prerelease is never latest, so it does not replace `pinfold update`'s
CLI release. No CLI binary, version bump, merge or other asset is included.

## Verification and consumption after publication

Read the publisher's release/asset API response. Require the exact name,
size and publisher SHA-256 to match the reviewed archive. Preserve that
response. Download the proposed URL independently and recheck compressed
and raw-root identities against `builder.toml`, not downloaded metadata.

Then add that actual URL to the reviewed embedded builder pin and connect
cache-miss installation to the existing HTTPS-only, SHA-verified, cancellable
512 MiB/five-minute downloader. Keep raw-root verification before attachment
and normal offline archive-cache support. Exercise a fresh normal cache
with the real CLI, without preseed or a test-only override. That successful
installation is required before production admission; today's manual cache
population does not substitute for it.

Trust chain: reviewed recipe and package closure; authenticated Debian
Release/index/packages and pinned public keys; publisher-digest-verified
BuildKit archive; reviewed emitted compressed/raw bytes; authorized
repository publisher response; independently embedded consumer pins.
Recipe identity does not promise bit-identical mkfs output.

Runtime numeric policy, complete runtime gates and final independent review
remain separate. Artifact-only publication approval would waive none of them.
