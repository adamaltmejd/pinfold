# Builder bootstrap

This private candidate uses the
[digest-pinned archive](https://github.com/adamaltmejd/pinfold/releases/download/builder-root-1e9a62f3eb034bf8582f4ff5d6e42310b56c5fbc1de201336342647228fe18cd/pinfold-builder-linux-x64-1e9a62f3eb034bf8582f4ff5d6e42310b56c5fbc1de201336342647228fe18cd.tar.gz)
through the verified offline cache. That intended public URL is not published.
Publication and public adoption require separate approval.

To produce a bootstrap on Linux x64, install debootstrap, GnuPG, curl 8.4+,
ca-certificates, e2fsprogs, GNU tar and Python 3.11.4+ with `tarfile.data_filter`.
Build into a new directory:

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

The artifact contains only a 512 MiB read-only `root.ext4`.
It includes Debian minbase,
CA certificates, e2fsprogs, util-linux and BuildKit's buildctl, buildkitd
and buildkit-runc, plus the compiled `pinfold-discover` native metadata helper.
Go and the upstream source/vendor tree remain producer inputs outside the root.
It includes no bootstrap init or transport service.

One finite builder VM queries host-frozen local configs on demand, then builds
with host-selected read-only OCI inputs. Conversion uses a fresh independent VM.

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

The workflow uploads build artifacts only.

`crates/pinfold/builder.toml` embeds the intended URL and reviewed archive,
raw-root and recipe identities. A rebuild is not expected to reproduce
those bytes. The optional offline cache accepts the same exact archive,
independently of its sidecar manifest. With `archive` naming the tar.gz:

```sh
digest=1e9a62f3eb034bf8582f4ff5d6e42310b56c5fbc1de201336342647228fe18cd
printf '%s  %s\n' "$digest" "$archive" | sha256sum -c -
cache="${XDG_CACHE_HOME:-$HOME/.cache}/pinfold/artifacts/builder/$digest"
mkdir -p "$cache"
install -m 600 "$archive" "$cache/pinfold-builder-linux-x64-$digest.tar.gz"
```

The CLI copies the offline archive into install staging when present;
otherwise it downloads the pinned URL over HTTPS into that staging area.
The archive copy or download is bounded to its pinned 122,074,383 bytes,
below the shared 512 MiB transfer cap, and checked against its embedded
size and SHA-256. The CLI streams only `root.ext4` into a fixed 0600 file,
checks its embedded raw size and SHA-256, and publishes the complete
`linux-x64/` install atomically. Acquisition, verification and installation
share one 300-second deadline and support cancellation. Every use verifies
the raw root again. Damaged installed bytes are refused with
`builder-integrity`; remove the `linux-x64/` install to reinstall. Embedded
digests remain authority if bytes at the public URL change. Completed
images launch without this bootstrap.
