# Current builder artifact publication proposal

Date: 2026-10-29. Proposal only. This supersedes the operative eight-file
candidate in the older audit notes; those records remain history.

Producer target: `5c0ffa2287a51d0fa1a8575181a0a32664b6ef3f` on main.
Reviewed producer head `784988b022c37f2aaeb2d768046eca14d13802d0`
has the identical complete tree. The audit candidate is separate and may
contain only the approved comments/help/docs changes; it is not the target.

Repository: `adamaltmejd/pinfold`. Proposed prerelease, latest=false:
`builder-root-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`.
No release, tag, asset upload or consumer pin is authorized by this file.

## Exact assets

Use only the twelve regular files in:

`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/helper-artifact-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4/`

The inventory below is copied from the committed producer record. Do not
use the old `publication/` directory, e7dd archive, a glob, or a rebuild.
SHA256SUMS covers the other eleven files; its own hash is independent.

| Filename | Bytes | SHA-256 |
|---|---:|---|
| Release | 138612 | ed56aac47e7911e65ee63aae8d67e29f20e023f840e49f8ed037043a33de138a |
| Release.gpg | 1760 | 9f80d5c8e4f9fd918a8b4fedd35c00ecc6c1a3ca9eddbc981c83823d4a4b8dc4 |
| SHA256SUMS | 1032 | b8d6c09d0b10f1c61706b587a8ff2d8e27ce05f826906ac9db6c1f46ae8f1a80 |
| buildkit-source-f5b59a1f2fb25a0cb7072c398cab9aed7835da4e.tar.gz | 12143295 | 6608d2ee63bedf99e1e3fecbfa19868c36f34a402c410a9605529986632816c3 |
| debian-archive.asc | 25106 | 312f12f3a3043d9aadc5ba532410dab2b45f91eb76230e7509bf6ced903738d4 |
| discovery-build.json | 6203 | 36084dd639afd6c6532057057a0a65ca990be1681298b2e0282170fd72cd9cd7 |
| discovery-main.go | 6929 | bd5fda8f957fdb9d887a85caffeb6e78ce5c96f380bd52122847b5d6d6ceb335 |
| go1.26.10.linux-amd64.tar.gz | 66936659 | ccfe7ae822d2c2f41f4fe7c956199967a4dac22d0ee0d2b2bfdb440aab3b20c5 |
| manifest.json | 9245 | c9643a200f9ff1fd30451c091837f7025a2232c9c909756e3d14ff4f9f6636c6 |
| packages.tsv | 12498 | 041d0dd2bb81bb7d9a73b335f6bbdf4d0160e6d2e682b802d6a5339927f18caf |
| pinfold-builder-linux-x64-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4.tar.gz | 122578909 | 09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4 |
| pins.json | 1443 | 9c3028025b6aa3ca8c31d02c3a26940ac3ddb97bfc5b5700ff6737ee0ece57ea |

The sole archive member is regular `root.ext4`, 1073741824 bytes,
SHA-256 `ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
Recipe: `d4fea170a1a33385e33951a981e649fc6b07053c5e965b99dc620719e744c30c`.
Helper: 22282402 bytes, SHA-256
`0b8927187764740c328a245e21c44a08cb419aa93911d3be624e4c943f2907d0`.
Recipe identity does not promise bit-identical filesystem rebuilding.

The complete retained producer bundle is 199874818 bytes, SHA-256
`f38ed6c8b82cbee9a361d09514337304e135e1897ae9827c4361105fec665127`;
the component bundle is 6604 bytes, SHA-256
`725983f4a29deb079e7eccbf502e211e1c1918cfc6ca2abbeb4c366f0506dbeb`.
Both remain outside Git in the durable parent directory, not in the upload.

## Authorized future operation, after explicit approval

The operator must first resolve the audit/gate/ticket dispositions in the
current cleanup report. Adam's approval must name these twelve assets,
this tag, target and any explicit gate exception. Draft-PR upload approval
is not release approval. Numeric runtime policy is a separate decision.

1. Rehash exactly these twelve regular paths, sizes and digests, including
   SHA256SUMS itself. Recheck the retained archive's single member, raw
   identity and gzip CRC/EOF. Keep the independently reviewed table as
   authority; downloaded metadata cannot authorize different bytes.
2. Confirm repository/account and exact remote producer target. Require
   actual authenticated HTTP 404 responses for both proposed tag and
   release; timeout, auth, permission or server failure is not absence.
   Refuse an existing object. Refresh these checks immediately before use.
3. Use the reviewed one-off `gh release create` operation with this tag,
   `--repo adamaltmejd/pinfold`, full `--target` producer SHA above,
   `--prerelease --latest=false`, reviewed `--notes-file`, and twelve
   explicit paths. No clobber/force/replacement, v-tag, CLI version bump,
   pin update or release workflow dispatch. Do not substitute CI output.
4. Preserve returned release/asset IDs and diagnostics. The command creates
   a draft, uploads and publishes through separate API requests. On error,
   interruption or unknown outcome, inspect the exact tag, release state
   and complete paginated asset list before any mutation. gh can attempt
   release deletion after an upload/publish failure; cleanup can fail too.
   Assume neither success nor rollback. Stop for operator review of any
   partial/conflicting state. Never recreate/delete/reupload blindly;
   recovery mutations require authority. Storage immutability is not assumed.
5. Require the public prerelease state and exactly twelve uploaded assets
   with these names, sizes, publisher SHA-256 fields and download URLs.
   Resolve the actual tag through any annotation to the exact producer
   commit, not merely target_commitish. Confirm it is not latest stable.
6. Download all twelve independently into a fresh private directory and
   recheck the table, archive member/raw digest and gzip integrity. Missing
   publisher hashes or any mismatch blocks consumer admission; preserve
   receipts and investigate without replacing assets.

The proposed archive URL is the GitHub release-download URL for this exact
tag and archive name. It is not an existing download or adopted consumer
pin. The builder workflow still only builds/uploads Actions artifacts.
The CLI release workflow's v-tag or explicit invocation is not used.

## Proposed release notes

Pinned Linux x64 builder/converter bootstrap with native BuildKit metadata
discovery. Built from a signed Debian snapshot, exact package closure,
pinned BuildKit binaries, and a helper compiled offline with pinned Go
against the reviewed BuildKit source/vendor tree. Twelve assets retain the
root archive and producer/provenance inputs. Recipe, compressed archive,
raw root and helper identities are recorded separately; bit-identical
filesystem rebuilding is not claimed.

### Caller changes

None for the installed Pinfold CLI. Consumer adoption and runtime controls
remain separate work.

## Limits

Go has its publisher checksum. The BuildKit codeload source has a reviewed
measured hash bound to the publisher's verified signed-tag/commit record;
this is not a separately published source-tarball digest or independent
local signature verification. Component receipts do not prove production
transport, jail/proxy, claims, cache/lifecycle/fault handling or normal
cache-miss installation. The active runtime worker owns those gates.

The old failed audit Mac result remains failed. Producer/runtime passes
cannot be relabelled as an audit-head pass. Before any final publication
question, the operator must identify the applicable exact-head Mac/Linux,
release-build and slow-gate results or explicitly proposed exceptions.
