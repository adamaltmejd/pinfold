# Helper bootstrap publication proposal

Date: 2026-10-29. Proposal only; no publication is authorized by this file.
This supersedes the older phase 6a byte selection in the 28 October
publication proposal. The operation's immutable-release safeguards remain.

Promote the already reviewed helper bootstrap; do not rebuild a filesystem
and assume identical bytes. The twelve files are preserved at:

`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/helper-artifact-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4/`

All twelve were independently rehashed for this proposal. `SHA256SUMS`
lists eleven files; its own SHA-256 is
`b8d6c09d0b10f1c61706b587a8ff2d8e27ce05f826906ac9db6c1f46ae8f1a80`.
The archive is 122,578,909 bytes, SHA-256
`09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`.
The 1,073,741,824-byte root has independently embedded SHA-256
`ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
Recipe identity is
`d4fea170a1a33385e33951a981e649fc6b07053c5e965b99dc620719e744c30c`.

The sidecars are `manifest.json`, `pins.json`, `packages.tsv`,
`debian-archive.asc`, `Release`, `Release.gpg`, `discovery-main.go`,
`discovery-build.json`, the pinned BuildKit source archive, the pinned Go
archive and `SHA256SUMS`. The manifest is evidence, not digest authority.
Its SHA-256 is
`c9643a200f9ff1fd30451c091837f7025a2232c9c909756e3d14ff4f9f6636c6`.

The proposed destination is `adamaltmejd/pinfold` on GitHub. Create one new
artifact-only prerelease, never latest, targeting producer merge
`5c0ffa2287a51d0fa1a8575181a0a32664b6ef3f`. Its five producer inputs agree
with the reviewed producer source `784988b` and the runtime branch.
The proposed tag is:

`builder-root-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`

The proposed archive URL, not yet an installed or available asset, is:

`https://github.com/adamaltmejd/pinfold/releases/download/builder-root-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4/pinfold-builder-linux-x64-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4.tar.gz`

Explicit approval must identify this artifact-only publication and
destination. Rehash the files again immediately before publication and
refuse an existing tag/release. Do not force tags, replace assets, mark
latest, create a CLI version tag or dispatch the CLI release workflow.
Source PR approval is not artifact-publication approval.

After publication, verify the publisher's asset name, bytes and SHA-256;
independently download the actual archive and verify both embedded pins.
Only then add the actual URL to `builder.toml` and use the existing bounded,
cancellable HTTPS downloader on a normal cache miss. Exercise fresh-cache
installation through the real CLI. No test-only source, pin override or
unverified rebuild is part of this proposal.

The existing producer CI artifact has different root/archive bytes and
cannot fill the reviewed 093 pin. Final runtime gates, numeric policy and
admission remain separate. No merge, CLI release or numeric approval is
requested by this proposal.
