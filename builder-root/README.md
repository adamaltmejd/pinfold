# Builder bootstrap

On Linux x64, install debootstrap, GnuPG, curl 8.4+, ca-certificates,
e2fsprogs, GNU tar and Python 3.11+. Build into a new directory:

```sh
sudo python3 builder-root/build.py target/builder-root
cd target/builder-root
sha256sum -c SHA256SUMS
```

No installed BuildKit is needed. `pins.json` fixes the Debian
snapshot, signed Release, package index, archive keys and BuildKit asset.
`packages.tsv` fixes every package's name, version, architecture, SHA-256
and snapshot path. Downloads are HTTPS-only, at most 512 MiB and five
minutes each. Debootstrap runs against the downloaded file mirror with
signature verification required, then its installed closure is checked.

The artifact contains only `root.ext4`. It includes Debian minbase,
CA certificates, e2fsprogs, util-linux and BuildKit's buildctl, buildkitd
and buildkit-runc. It includes no bootstrap init or transport service.
The manual boot/build/conversion consumer is recorded under
`docs/archive/2026-10-28-builder-bootstrap/`.

The manifest separates recipe identity, raw root digest and compressed
artifact digest. Filesystem bytes are not claimed to be bit-identical
across builds. The archive filename contains its full SHA-256.
Host tool versions are recorded. Build inputs move together in reviewed
release work; never rehash an existing version. BuildKit uses the existing
stable-release/seven-day publisher-digest policy.

The public archive keys are Debian's automatic keys for 12 and 13 and
stable key for 13, from `https://ftp-master.debian.org/keys.html`:

- `B8B80B5B623EAB6AD8775C45B7C5D7D6350947F8`
- `04B54C3CDCA79751B16BC6B5225629DF75B188BD`
- `41587F7DB8C774BCCF131416762F67A0B2C39DE4`

The workflow uploads build artifacts only. Public immutable distribution
awaits separate approval. The draft CLI has no builder download URL and
refuses a missing offline cache with `builder-unavailable`.

`crates/pinfold/builder.toml` embeds the reviewed phase 6a archive and raw
root identities. A rebuild is not expected to reproduce those bytes.
The normal offline cache accepts that exact preserved archive, independently
of its sidecar manifest. With `archive` naming the preserved tar.gz:

```sh
digest=e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0
printf '%s  %s\n' "$digest" "$archive" | sha256sum -c -
cache="${XDG_CACHE_HOME:-$HOME/.cache}/pinfold/artifacts/builder/$digest"
mkdir -p "$cache"
install -m 600 "$archive" "$cache/pinfold-builder-linux-x64-$digest.tar.gz"
```

The archive is retained in the bootstrap evidence bundle recorded in
`docs/archive/2026-10-28-builder-bootstrap.md`. Before extraction, the CLI
copies at most 512 MiB into its own staging directory and checks the
embedded size and SHA-256. It streams only `root.ext4` into a fixed 0600
file, checks the embedded raw size and SHA-256, and publishes the complete
`linux-x64/` install atomically. Verification and installation have a
five-minute deadline and support cancellation. Every use verifies the raw
root again. Damaged installed bytes are refused with `builder-integrity`;
remove the `linux-x64/` install to reinstall from the retained archive.

No tag, release or asset publication is implied by this cache contract.
