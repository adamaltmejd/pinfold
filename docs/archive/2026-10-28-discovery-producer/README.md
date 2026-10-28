# Discovery helper producer prerequisite

Date: 2026-10-28. Isolated source prerequisite based on live main
`ff6276cb832dfc78e4cb716857e33af1f00f12b8`. This record describes preserved
producer/component results. Preparing this commit ran no build or runtime.
No tag, release, upload, public artifact URL or consumer adoption is proposed
by this commit. The installed CLI still does not consume the bootstrap.

## Scope and dependency reason

The producer adds the native metadata helper. The supported BuildKit gateway
API resolves zero-layer image configurations that affect a recipe even when
native `frontend.convertllb` emits no image SourceOp. The helper invokes that
conversion subrequest without solving RUN or using an exporter. Its output
contains bounded resolved identifiers and descriptor digests, not configs.
Go and BuildKit's committed vendor closure are needed to compile that API
client. They are producer inputs only: no host Go/git requirement, Rust
dependency, launcher, catalogue, guest init, runtime policy or CLI change.

The exact reviewed five recipe files were copied from committed runtime
checkpoint `b2071840ee5b1aca12720a3600a817278f586ac2`. Their bytes match the
preserved actual root's manifest. Package closure and archive keys remain
identical to main. README changes describe tooling and output only.

| Recipe input | SHA-256 |
|---|---|
| `build.py` | `ad434f7ca2686eb1c0a19e6ad721dfa82d859b6b1511b7aaeec91d6d77b34ac4` |
| `debian-archive.asc` | `312f12f3a3043d9aadc5ba532410dab2b45f91eb76230e7509bf6ced903738d4` |
| `discover/main.go` | `bd5fda8f957fdb9d887a85caffeb6e78ce5c96f380bd52122847b5d6d6ceb335` |
| `packages.tsv` | `041d0dd2bb81bb7d9a73b335f6bbdf4d0160e6d2e682b802d6a5339927f18caf` |
| `pins.json` | `9c3028025b6aa3ca8c31d02c3a26940ac3ddb97bfc5b5700ff6737ee0ece57ea` |

Recipe identity:
`d4fea170a1a33385e33951a981e649fc6b07053c5e965b99dc620719e744c30c`.

## Publisher inputs

The [Go release history](https://go.dev/doc/devel/release#go1.26.10) dates
Go 1.26.10 to 2026-10-19, nine calendar days before this review. Even an
end-of-day publication leaves seven full days. The
[publisher download index](https://go.dev/dl/?mode=json&include=all) gives
SHA-256 `ccfe7ae822d2c2f41f4fe7c956199967a4dac22d0ee0d2b2bfdb440aab3b20c5`
for `go1.26.10.linux-amd64.tar.gz`. The unchanged tool-age rule is checked
when reviewing pins, not against the clock on each producer run.

BuildKit v0.34.1's [annotated tag object](https://api.github.com/repos/moby/buildkit/git/tags/401bf63d02833b70ef9393eb9702e2b5648dfaaf)
points to commit `f5b59a1f2fb25a0cb7072c398cab9aed7835da4e`, tag date
2026-10-20T21:05:13Z, tree `7cb3b9b2864994a620bfdb04c87f56ca8f2f518c`.
GitHub's publisher API reports the signature verified; no independent local
signature verification is claimed. The exact-commit
[codeload archive](https://codeload.github.com/moby/buildkit/tar.gz/f5b59a1f2fb25a0cb7072c398cab9aed7835da4e)
was downloaded over HTTPS and its measured byte hash was reviewed and pinned.
It is not a separately published tarball checksum. Preserved publisher
reference, signed-tag and commit JSON are beside this record. The archive
covers all committed vendor bytes; module controls are also checked after
safe extraction and recorded in `discovery-build.json`.

Compilation uses that downloaded Go with `-mod=vendor`, `-trimpath`,
`-buildvcs=false`, `GOTOOLCHAIN=local`, `GOPROXY=off`, `GOSUMDB=off`,
`GOENV=off`, `GOWORK=off`, CGO disabled, isolated caches and linux/amd64.
It embeds the BuildKit version and exact commit. No HOME override is used.

## Preserved actual build and component proof

Durable local directory:
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/helper-artifact-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4/`.
This is retained evidence, not a public download location. All 12 regular
files were rehashed while preparing this source commit:

| Full filename | Bytes | SHA-256 |
|---|---:|---|
| `Release` | 138612 | `ed56aac47e7911e65ee63aae8d67e29f20e023f840e49f8ed037043a33de138a` |
| `Release.gpg` | 1760 | `9f80d5c8e4f9fd918a8b4fedd35c00ecc6c1a3ca9eddbc981c83823d4a4b8dc4` |
| `SHA256SUMS` | 1032 | `b8d6c09d0b10f1c61706b587a8ff2d8e27ce05f826906ac9db6c1f46ae8f1a80` |
| `buildkit-source-f5b59a1f2fb25a0cb7072c398cab9aed7835da4e.tar.gz` | 12143295 | `6608d2ee63bedf99e1e3fecbfa19868c36f34a402c410a9605529986632816c3` |
| `debian-archive.asc` | 25106 | `312f12f3a3043d9aadc5ba532410dab2b45f91eb76230e7509bf6ced903738d4` |
| `discovery-build.json` | 6203 | `36084dd639afd6c6532057057a0a65ca990be1681298b2e0282170fd72cd9cd7` |
| `discovery-main.go` | 6929 | `bd5fda8f957fdb9d887a85caffeb6e78ce5c96f380bd52122847b5d6d6ceb335` |
| `go1.26.10.linux-amd64.tar.gz` | 66936659 | `ccfe7ae822d2c2f41f4fe7c956199967a4dac22d0ee0d2b2bfdb440aab3b20c5` |
| `manifest.json` | 9245 | `c9643a200f9ff1fd30451c091837f7025a2232c9c909756e3d14ff4f9f6636c6` |
| `packages.tsv` | 12498 | `041d0dd2bb81bb7d9a73b335f6bbdf4d0160e6d2e682b802d6a5339927f18caf` |
| `pinfold-builder-linux-x64-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4.tar.gz` | 122578909 | `09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4` |
| `pins.json` | 1443 | `9c3028025b6aa3ca8c31d02c3a26940ac3ddb97bfc5b5700ff6737ee0ece57ea` |

The archive contains the single named `root.ext4`, 1,073,741,824 bytes,
SHA-256 `ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
The separately recorded root/helper inspection receipt is retained as
`independent-receipt.json`; no root was mounted or extracted on the host.
The helper is 22,282,402 bytes, SHA-256
`0b8927187764740c328a245e21c44a08cb419aa93911d3be624e4c943f2907d0`.
Its exact compiled source is 6,929 bytes and matches the recipe source above.
Filesystem byte-for-byte reproducibility is not claimed.

The complete original producer evidence bundle
`bootstrap-helper-evidence.tar.gz` has SHA-256
`f38ed6c8b82cbee9a361d09514337304e135e1897ae9827c4361105fec665127`.
The compact component bundle `helper-api-proof-3.tar.gz` has SHA-256
`725983f4a29deb079e7eccbf502e211e1c1918cfc6ca2abbeb4c366f0506dbeb`.
Both remain in the durable parent directory, outside Git.

Selected raw JSON/stdout/recipe/fixture receipts from that component bundle
are committed under `api-proof/`. They bind the same helper binary hash.
The real pinned BuildKit daemon and a loopback OCI registry showed:
zero-layer ENV plus ONBUILD RUN false affects metadata but native conversion
has no image SourceOp; helper resolution equals the independent fixture's
manifest digest. The layered positive control has the expected native source
and helper digest. Two OCI aliases retain distinct base0/base1 stores for the
same image, ARG/COPY reachability omits an unreachable external stage, and
scratch returns no resolutions. These are component results, not Pinfold
jail, proxy, claim, normal CLI, publication or execution guarantees. No test
is added to the automated guarantee suite.

## Remaining gates

The parent must run the host suite on this exact commit before push or a
bounded draft PR. Required repository format/clippy checks, Linux x64 and
full Mac suites, release builds and normal review remain separate gates.
No slow check was run while preparing this commit. Rebuilding cannot be
assumed to reproduce the retained filesystem bytes. Any artifact publication
needs its own approval and immutable byte identity; no tag name is invented
here. Runtime integration and consumer pin adoption remain separate work.

## Caller changes

None. This prerequisite changes producer tooling and artifact contents only;
no installed CLI flag, JSON line, exit code or box specification changes.
