# Discovery helper producer input review

Date: 2026-10-28. Source changes only; no compiler, bootstrap build, guest,
release, tag or publication was run for this review.

The bootstrap producer now compiles `builder-root/discover/main.go` with
the upstream BuildKit module and committed vendor tree. The dependency
serves a concrete gap: the supported native metadata API records resolved
zero-layer bases whose configuration affects a recipe but whose image
SourceOp is absent from the converted LLB. This is the dependency reason
for the later feature commit. No Rust dependency or installed host Go/git
requirement is added.

## Publisher and input identities

The [official Go release history](https://go.dev/doc/devel/release#go1.26.10)
dates Go 1.26.10 to 2026-10-19. That is nine calendar days before this review;
even treating publication as the end of that day leaves more than seven
full days. The existing age policy is unchanged. Eligibility is reviewed
when changing the pin, rather than reimplemented as a clock check in each
bootstrap build.

The [publisher download index](https://go.dev/dl/?mode=json&include=all)
supplies the linux-amd64 archive's SHA-256:
`ccfe7ae822d2c2f41f4fe7c956199967a4dac22d0ee0d2b2bfdb440aab3b20c5`.
The archive downloaded for source inspection is 66,936,659 bytes and
matches that value. Its listed members contain 232,612,406 file bytes,
below the producer's 512 MiB extraction bound. No Go executable was run.

BuildKit source is pinned to commit
`f5b59a1f2fb25a0cb7072c398cab9aed7835da4e`.
The [publisher's v0.34.1 tag object](https://api.github.com/repos/moby/buildkit/git/tags/401bf63d02833b70ef9393eb9702e2b5648dfaaf)
names that exact commit and reports a verified signature. Its tag date is
2026-10-20T21:05:13Z. The commit's tree is
`7cb3b9b2864994a620bfdb04c87f56ca8f2f518c`.
No independent local signature verification was performed here.

The [exact-commit source archive](https://codeload.github.com/moby/buildkit/tar.gz/f5b59a1f2fb25a0cb7072c398cab9aed7835da4e)
downloaded from the publisher is 12,143,295 bytes, SHA-256
`6608d2ee63bedf99e1e3fecbfa19868c36f34a402c410a9605529986632816c3`.
That archive hash was measured from the publisher's HTTPS response; it is
not a separately published tarball checksum. Its exact reviewed commit,
archive byte pin and the signed-tag publisher record establish the input
identity. A changed archive must not be silently rehashed. Listed source
members contain 59,663,768 file bytes. Its one symlink is the internal
`examples/kube-consistent-hash` alias, not a path outside the source tree.

The archive's module/vendor controls matched these pinned values:

| Input | SHA-256 |
|---|---|
| `go.mod` | `9114ace60db407b7b70eccc340cda6cc5ff3313400afbdd8e1f14849bc244fb3` |
| `go.sum` | `35904aa6f9ae03717b2ee03ed55e6f026de822f556e37491801d13cf13fb9e46` |
| `vendor/modules.txt` | `d7484cb1c241d6e6a52a2d79e3b96a6c67bae4f41b2a549396844aae7414bcea` |

The archive's hash covers the entire committed vendor tree, not just its
module list. The producer verifies the three control files again after
extracting the pinned bytes.

## Implemented producer boundary

The existing bounded downloader fetches Go and source inputs. Extraction
requires the expected top-level directory, rejects parent components and
caps expanded bytes. Python's explicit `data` filter checks link targets,
symlink ancestors and special files while ignoring archive ownership.
The producer therefore requires `tarfile.data_filter`, available since
Python 3.11.4. These are verified upstream producer archives, not guest
output or OCI layers.

The helper source is copied into `cmd/pinfold-discover/main.go`; its copied
hash must match the recipe input before compilation. The downloaded Go
compiler runs with an explicit environment, `GOENV=off`, `GOWORK=off`,
`GOTOOLCHAIN=local`, `GOPROXY=off`, `GOSUMDB=off`, isolated Go caches and
temporary files, CGO disabled and linux/amd64 selected. HOME is not set.
The build uses `-mod=vendor -trimpath -buildvcs=false`, with BuildKit's
version and exact commit in linker settings. No ambient Go settings,
module downloads, host compiler or host git lookup are required.

Only the resulting `/usr/local/bin/pinfold-discover` joins the guest root.
The recipe identity now includes its source. Manifest and build sidecar
record source/compiler/binary hashes, compiler version, command, explicit
environment and `go version -m` output. The output keeps the helper source
and both verified producer archives as flat sidecars, covered by
SHA256SUMS. The bootstrap tar.gz still contains only `root.ext4`.

Ruff check and formatting passed for the producer. No helper compilation,
new 1 GiB root-capacity check, bootstrap archive-size observation or runtime
validation accompanies this source checkpoint. The parent must build and
verify the new artifact, retain its actual manifest/bytes and update the
consumer pins through review. The old phase 6a archive/root identities
and offline cache contract are not replaced by this source-only change.
