# Builder-root publication readiness

Date: 2026-10-28. Preparation only; publication is not authorized.
Producer/target: `ff6276cb832dfc78e4cb716857e33af1f00f12b8`.

This supplements the proposal and release notes read from exact commit
`4e961e2181494b85dfce2312a8b93541dbe60795` in the separate phase 6b
checkout. It does not modify that checkout or its proposal. The operator
must route these corrections to the publication owner before approval.

## Bytes and destination

Promote the existing `dist-final/` files from the preserved phase 6a
bundle. Do not rebuild or substitute a CI build's different filesystem.

Bundle:
`/Users/adam/Code/pinfold/target/phase6-builder-bootstrap-2026-10-28/evidence.tar.gz`

Bundle size: 267038450; SHA-256:
`f8681084207eabc7004667342bd97df650d2ee101aa365d8dd75965a03dbd97b`.

Staging directory:
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/publication/`

| Name | Bytes | SHA-256 |
|---|---:|---|
| pinfold-builder-linux-x64-e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0.tar.gz | 114456820 | e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0 |
| manifest.json | 2047 | 55edb75f268a715fe3efb520a75bf429d901820bb35805bdb236703a454bcc26 |
| SHA256SUMS | 636 | ce6cd09bb0ad07aa9cc45d0d4b02df78455cb9e4360e9e2a8d43ccf178e60db4 |
| packages.tsv | 12498 | 041d0dd2bb81bb7d9a73b335f6bbdf4d0160e6d2e682b802d6a5339927f18caf |
| pins.json | 679 | 437cd68c0e9b9c1dc8e779e74e0757332d720fb80d9a8bcff7fd5b94ea2f3687 |
| debian-archive.asc | 25106 | 312f12f3a3043d9aadc5ba532410dab2b45f91eb76230e7509bf6ced903738d4 |
| Release | 138612 | ed56aac47e7911e65ee63aae8d67e29f20e023f840e49f8ed037043a33de138a |
| Release.gpg | 1760 | 9f80d5c8e4f9fd918a8b4fedd35c00ecc6c1a3ca9eddbc981c83823d4a4b8dc4 |

Repository: `adamaltmejd/pinfold`.
Proposed tag:
`builder-root-e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0`.
Prerelease: true. Latest: false. Target: the full producer commit above.
Exactly eight assets; no CLI binary. Notes are copied unchanged in
`2026-10-28-builder-artifact-release-notes.md`.

The expected download URL is the repository's GitHub release-download
path using that exact tag and archive name. It is a proposal, not an
available or installed pin. Both authenticated tag and release lookups
returned HTTP 404 during this audit. Latest stable was `v0.3.3`.

## Verified in this audit

All eight staged files equal their preserved `dist-final/` members byte
for byte and match the table. All 91 files in the bundle's member receipt
were rehashed: 929008845 raw bytes. SHA256SUMS names exactly the other
seven files, and every listed checksum matches.

The archive has exactly one regular member, `root.ext4`, 1073741824 bytes,
SHA-256
`710189b13d814e4026e1095bd393dad99465e4042ced5dae069c59fd79cbaa80`.
The raw bytes were independently streamed and hashed. The gzip stream was
consumed through CRC/footer/EOF. No filesystem was mounted or parsed.

Preserved recipe sources match both the manifest and producer main. Their
recipe identity is
`d985bd63260afaedcbaf90c3fbfc95f5d077b03cd13407137c3b4c4724ac5e0f`.
The signed Debian Release verifies with all three pinned primary keys:
`B8B80B5B623EAB6AD8775C45B7C5D7D6350947F8`,
`04B54C3CDCA79751B16BC6B5225629DF75B188BD`, and
`41587F7DB8C774BCCF131416762F67A0B2C39DE4`.
The first two signatures use signing subkeys. Verification checks the
primary-key field of GnuPG's VALIDSIG status, not its first fingerprint.

The preserved and live BuildKit asset API agree: asset 660559865,
`buildkit-v0.34.1.linux-amd64.tar.gz`, 92218962 bytes,
`sha256:a59539daf547fe0958efe2d07e41c0a6944e1112a66d7e519ec1cf5b673ef15b`.
It was published 2026-10-20 at 21:39:16 UTC, over seven full days earlier.
The producer ties the package index to the signed Release, downloads each
pinned package, requires debootstrap signature verification against the
offline mirror and checks the complete installed package closure.

The public sidecars were inspected; they contain package metadata, public
keys, digests and tool versions. The 1 GiB raw root was scanned for private
key headers and common GitHub/OpenAI secret-key prefixes; no markers were
found. This limited scan is not proof against every possible secret form.
The content argument also depends on the reviewed clean bootstrap recipe
and its isolated build, which receives no caller credential. The full
evidence bundle, logs, proof dumps and publisher response are not assets.

## Execution after explicit artifact-only approval

1. Reverify staging against this table, including SHA256SUMS itself.
   Require exactly the eight names, regular files and no changed bytes.
   Recheck producer source, reviewed notes and the remote target commit.
2. Authenticated API lookups must establish that neither tag nor release
   exists. Require actual HTTP 404 responses; an authentication, transport,
   permission or server failure is not evidence of absence. Stop if either
   exists. Do not use clobber, force, asset replacement or a release workflow.
3. Use `gh release create` with the exact proposed tag, `--repo`, full
   `--target`, `--prerelease`, `--latest=false`, `--title`, `--notes-file`
   and eight explicit asset paths. No `v*` tag or workflow dispatch.
   Preserve command output, IDs and API responses privately.
4. After any failure, interruption or unknown outcome, inspect the exact
   tag, release ID and complete asset list before deciding what happened.
   Installed gh 2.103.3 creates a draft, uploads assets and publishes using
   separate requests. On upload/publish failure it attempts release
   deletion; cleanup can also fail. Never assume rollback. Never repeat
   creation, delete a partial result or reupload blindly. An unexpected
   partial or conflicting state requires operator review before mutation.
5. Read the authenticated release response and paginate its asset list.
   Require the exact tag, public prerelease state, exactly the eight names,
   every size and every publisher `sha256:` digest in this table. Require
   each asset's uploaded state and intended download URL. Resolve the actual
   remote tag through any annotated tag object to the exact producer commit;
   `target_commitish` alone is not proof. `--target` does not retarget an
   existing tag. Verify latest stable still points to `v0.3.3` or its
   independently authorized CLI successor, never this artifact prerelease.
6. Download all eight assets independently to a new private directory and
   reverify the table. For the archive, also verify the sole raw member,
   its size/hash and gzip integrity against the reviewed expectations above.
   Missing publisher digests or any mismatch prevents consumer admission;
   preserve receipts and investigate without replacing the asset.

This describes a future operation. No draft, tag, release or asset was
created by the audit. GitHub's storage immutability setting is not assumed;
content-addressed naming and the no-replacement rule do not establish that
setting. Consumer digest checks remain required even with repository trust.

## Release applicability and remaining gates

AGENTS.md requires the whole-tree cleanup/test sweep/spec pass before
every release. This preparation applies it; the guest-kernel convention
does not supply an exemption. Its concrete artifact flow is a separate
prerelease, `latest=false`, targeting a source commit. The CLI release
workflow triggers on `v*` tags or explicit invocation; neither belongs here.

The skill's version-bump, pin-update and `v<version>` steps describe a CLI
release and would publish unrelated deliverables or change these reviewed
inputs. The task explicitly excludes them. No pins, dependencies or
workspace version move. No artifact rebuild or substituted CI output.

The cleanup report and test proposals are separate dated records. The
cleanup branch's coordinated Mac suite failed: 23 passed, 1 failed,
1 ignored, exit 101, 438.252 seconds including Cargo. Its live-login box
was intentionally stopped after a separately observed upstream 429;
the live command then exited 137. The gate receipt preserves that
distinction. A draft PR is explicitly authorized for review despite this
failed gate; Linux CI and the operator's final decision remain required.
No pass or publication-gate exception is implied. Source checks do not
replace the builder proof, and the proof does not establish production
runtime behavior.

Only after operator review of this preparation should Adam receive the
concrete request to publish these eight bytes under this one tag. The
publication approval must not authorize a CLI release or phase 6b merge.

After actual publication, phase 6b still needs the real URL in an
independently reviewed embedded consumer pin, compressed and raw digest
checks, the bounded HTTPS-only cancellable installer, offline-cache
behavior and a fresh normal cache-miss CLI installation with no preseed or
test seam. Numerical resource policy, production transport/jail/egress,
metadata fidelity, cancellation/cleanup, preservation/migration, real
Mac/Linux gates and two independent final-head reviews remain the feature
operator's gates. This audit decides no numeric runtime limits.

Publication recovery references: installed `gh release create --help`;
[create implementation](https://github.com/cli/cli/blob/v2.103.3/pkg/cmd/release/create/create.go#L463-L520);
[asset API](https://docs.github.com/en/rest/releases/assets#get-a-release-asset).
