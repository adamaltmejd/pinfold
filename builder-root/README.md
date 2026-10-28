# Builder bootstrap

On Linux x64, install debootstrap, GnuPG, curl 8.4+, ca-certificates,
e2fsprogs, GNU tar and Python 3.11.4+ with `tarfile.data_filter`. Build into a new directory:

```sh
sudo python3 builder-root/build.py target/builder-root
cd target/builder-root
sha256sum -c SHA256SUMS
```

No installed BuildKit, Go or git is needed. `pins.json` fixes the Debian
snapshot, signed Release, package index, archive keys, BuildKit asset and
source commit/archive, committed Go module/vendor controls and Go compiler.
`packages.tsv` fixes every package's name, version, architecture, SHA-256
and snapshot path. Downloads are HTTPS-only, at most 512 MiB and five
minutes each. Debootstrap runs against the downloaded file mirror with
signature verification required, then its installed closure is checked.

The artifact contains only `root.ext4`. It includes Debian minbase,
CA certificates, e2fsprogs, util-linux and BuildKit's buildctl, buildkitd
and buildkit-runc, plus the compiled `pinfold-discover` native metadata helper.
Go and the upstream source/vendor tree remain producer inputs outside the root.
It includes no bootstrap init or transport service.
The manual boot/build/conversion consumer is recorded under
`docs/archive/2026-10-28-builder-bootstrap/`.

The manifest separates recipe identity, raw root digest and compressed
artifact digest. Filesystem bytes are not claimed to be bit-identical
across builds. The archive filename contains its full SHA-256.
Host tool versions are recorded. Build inputs move together in reviewed
release work; never rehash an existing version. BuildKit uses the existing
stable-release/seven-day publisher-digest policy.

The producer downloads the pinned Go linux-amd64 archive and exact BuildKit
commit archive with the same HTTPS/digest bounds. Source extraction requires
their expected top-level directories, caps expanded file bytes at 512 MiB
per archive and uses the stdlib data filter. It compiles `discover/main.go`
in that module with its committed vendor tree: `-mod=vendor`, `-trimpath`,
`-buildvcs=false`, CGO disabled, local toolchain selection and module/sum
network access disabled. BuildKit version and commit are embedded explicitly.
No host compiler, caller Go settings or new module resolution is used.

The helper source joins the recipe identity. `manifest.json` and
`discovery-build.json` record its source/binary hashes, compiler, source pins,
build command, isolated environment and `go version -m` output. The output
also retains `discovery-main.go` and the exact Go/BuildKit producer archives.
`SHA256SUMS` covers these sidecars too; the bootstrap tar.gz still contains
only `root.ext4`. Go pins follow the same reviewed stable-release and
seven-full-day rule as the other producer tools.

The public archive keys are Debian's automatic keys for 12 and 13 and
stable key for 13, from `https://ftp-master.debian.org/keys.html`:

- `B8B80B5B623EAB6AD8775C45B7C5D7D6350947F8`
- `04B54C3CDCA79751B16BC6B5225629DF75B188BD`
- `41587F7DB8C774BCCF131416762F67A0B2C39DE4`

The workflow uploads build artifacts only. Public immutable distribution
awaits separate approval. The draft CLI has no builder download URL and
refuses a missing offline cache with `builder-unavailable`.

`crates/pinfold/builder.toml` embeds the candidate helper archive and raw
root identities. This producer's helper-bearing output needs separately
reviewed artifact pins before that CLI can consume it. A rebuild is not
expected to reproduce the phase 6a bytes.
The normal offline cache accepts that exact preserved archive, independently
of its sidecar manifest. With `archive` naming the preserved tar.gz:

```sh
digest=09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4
printf '%s  %s\n' "$digest" "$archive" | sha256sum -c -
cache="${XDG_CACHE_HOME:-$HOME/.cache}/pinfold/artifacts/builder/$digest"
mkdir -p "$cache"
install -m 600 "$archive" "$cache/pinfold-builder-linux-x64-$digest.tar.gz"
```

The archive is retained in the integration evidence bundle recorded in
`docs/archive/2026-10-28-builder-helper-integration.md`. Before extraction, the CLI
copies at most 512 MiB into its own staging directory and checks the
embedded size and SHA-256. It streams only `root.ext4` into a fixed 0600
file, checks the embedded raw size and SHA-256, and publishes the complete
`linux-x64/` install atomically. Verification and installation have a
five-minute deadline and support cancellation. Every use verifies the raw
root again. Damaged installed bytes are refused with `builder-integrity`;
remove the `linux-x64/` install to reinstall from the retained archive.

No tag, release or asset publication is implied by this cache contract.
