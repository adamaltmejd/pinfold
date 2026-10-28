# Pinned builder bootstrap: phase 6a

Date: 2026-10-28. Baseline: `6114b2218df9ba7d1615a02d88a4b2d05d6333ce`.
Tooling candidate: `db2bd949d63df7160cb9a71acd103c8b80f721ea`.
[Issue 230](https://github.com/adamaltmejd/pinfold/issues/230).

## Cut and authority

This implements the first bootstrap cut from the approved builder-role
direction. The operator's `2026-10-28-builder-role-approval.md` and
`2026-10-27-refactoring-decisions-and-release-handoff.md` were read in the
ed8e checkout. Neither replaces the binding architecture.

- `builder-root/build.py`: one standalone build recipe and manifest writer.
- `builder-root/pins.json`, `packages.tsv`, `debian-archive.asc`: fixed
  bootstrap inputs and publisher trust material.
- `builder-root/README.md`: build instructions and current output contract.
- `.github/workflows/builder-root.yml`: build and Actions artifact upload,
  without a release/publish job.
- `ARCHITECTURE.md`: six lines describing this implemented tooling.

There is no Rust change, runtime installer, image catalog, builder CLI,
new egress flag, Podman retirement or live mount. Linux callers still
require Podman. No tag, release or immutable release-asset upload ran.
No permanent infrastructure or shared Mac runtime was touched.

The concrete consumer is the manual nested-KVM proof in
`2026-10-28-builder-bootstrap/`. It boots the built root, runs pinned
BuildKit with a real OCI worker, and starts a fresh converter from that
same root. It is not a production transport or test-only binary path.

## Inputs and verification

Debian snapshot:
`https://snapshot.debian.org/archive/debian/20261020T000000Z`.
The recipe pins Release, its detached signature, the amd64 package index,
three public archive keys, and all 85 package names/versions/architectures,
SHA-256 values and pool paths. GnuPG verifies all three Release signatures.
The index hash is checked against the authenticated Release as well as
the pin. Each download uses HTTPS-only redirects, curl's streamed 512 MiB
cap and a five-minute deadline. Curl 8.4 or later is required because older
versions did not enforce that size cap without Content-Length.

Debootstrap sees only a local file mirror with the verified metadata and
exact downloaded package set. Signature checking is mandatory. A missing
dependency cannot fall back to a live mirror. The installed package
closure must equal `packages.tsv` before an output directory is published.

The BuildKit 0.34.1 linux-amd64 archive is 92,218,962 bytes and has SHA-256
`a59539daf547fe0958efe2d07e41c0a6944e1112a66d7e519ec1cf5b673ef15b`.
Its GitHub asset ID is 660559865, published 2026-10-20T21:39:16Z. The
[publisher API](https://api.github.com/repos/moby/buildkit/releases/tags/v0.34.1)
supplied that digest. This follows `scripts/bump-pins.py`'s existing
publisher-digest policy; it is not independent Sigstore signature
verification. No cosign dependency or custom signature implementation was
added. The asset was more than seven days old.

Only buildctl, buildkitd and buildkit-runc are extracted, as three named
regular files. Debian minbase supplies shell/tar and base libraries;
CA certificates, util-linux and e2fsprogs supply demonstrated daemon and
conversion needs. There is no Python, socat, CNI, QEMU, distro runc, curl
or git in the root. Git source support is consequently disabled; this
proof uses local directory input. Python is build/proof tooling on the
disposable host. Busybox-static is only the scratch-build fixture.

Debootstrap avoids a custom Debian unpacker/configurator. GnuPG is already
used by the kernel build. No new Rust crate is added. The VMM and guest
kernel are read from the existing `vmm.toml`; they are not embedded in or
coupled to the bootstrap root's identity.

## Packaging and reproducibility

Output is a tar.gz containing only `root.ext4`, plus sidecar manifest,
SHA256SUMS, pins, package closure, keys and signed Release metadata.
The compressed bytes' full SHA-256 names the archive. Temporary output is
renamed into the requested new directory only after success.

The manifest distinguishes recipe identity, package closure, raw disk
digest and compressed archive digest. It records Python, debootstrap,
e2fsprogs, curl, GnuPG and tar versions. The inputs/closure are reproducible;
bit-identical filesystem bytes have not been demonstrated. Dpkg scripts,
timestamps, filesystem UUIDs and host tool versions can affect them.
An immutable future artifact pin must name the emitted bytes, not assume
the recipe hash identifies identical filesystem bytes across rebuilds.

Final standalone build:

| Item | Bytes | SHA-256 |
|---|---:|---|
| Recipe | — | `d985bd63260afaedcbaf90c3fbfc95f5d077b03cd13407137c3b4c4724ac5e0f` |
| Package closure | — | `041d0dd2bb81bb7d9a73b335f6bbdf4d0160e6d2e682b802d6a5339927f18caf` |
| Raw root | 1,073,741,824 | `710189b13d814e4026e1095bd393dad99465e4042ced5dae069c59fd79cbaa80` |
| Compressed archive | 114,456,820 | `e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0` |

The exact sidecar manifest (copied as `artifact.json` here) has SHA-256
`55edb75f268a715fe3efb520a75bf429d901820bb35805bdb236703a454bcc26`.

The 1 GiB bootstrap size is the preceding trial's measured sufficient
size. It is not a new builder cache, image output or caller resource
policy. Exact production resource limits remain unapproved.

## Commands and observation limits

The exclusive disposable VM was acquired through `labvm acquire --ttl 4h`.
All access used `labvm exec`; it was not accessed through a broker,
hypervisor API or permanent host. Host prerequisites were installed with:

```sh
sudo apt-get -o DPkg::Lock::Timeout=120 install -y -qq \
  debootstrap gnupg curl ca-certificates e2fsprogs
sudo usermod -aG kvm agent
sudo apt-get -o DPkg::Lock::Timeout=120 install -y -qq busybox-static
```

No installed BuildKit or Podman was used. The first package-install
attempt met the VM's initial apt lock; the later attempt waited on it.
Debootstrap discovery used `--foreign --arch=amd64 --variant=minbase
--include=ca-certificates,e2fsprogs,util-linux --force-check-sig` against
the pinned snapshot. Its verified package archives and `deburis` produced
the committed closure; it is not generated from a live mirror at build.

From the copied source under `/home/agent/builder-cut`:

```sh
sudo python3 builder-root/build.py dist-final >build-final.log 2>&1
python3 docs/archive/2026-10-28-builder-bootstrap/proof.py \
  dist-final proof-final golden-input.ext4 >proof-final.log 2>&1
```

The golden input is the preceding trial's exact `outputs/xattr-input.ext4`,
extracted from its retained artifacts archive, never mounted on the host.
Its 67,108,864 bytes match
`3376d10f5302599a8855a20a35a4ffb1db039b1c53d3a8d9cd7507fd84c9c066`.
It names manifest
`871f87c0b192a668ac95bfe54d8e13b1ab1091cd3eade32182688fddb6e2769b`.
Expected ownership, links, whiteouts and xattr come from that external
golden fixture, not from the new bootstrap's output.

Both guests have two vCPUs and 2,560 MiB RAM, matching the preceding trial.
The builder has a fresh 8 GiB cache; the converter has a separate fresh
3 GiB work disk and never attaches that cache. Neither has a NIC or vsock.
All jobs are offline; this is no evidence about production proxy/audit.
The bootstrap attaches read-only and its raw digest is checked after both
boots. The manual init sets the user-namespace sysctl before dropping the
helper/daemon capability bound to the trial's `0xa82425fb`, with
no_new_privs. This does not exercise Pinfold's production init or VMM jail.

Early observations are retained:

- Proof 1 booted the root but timed out because the observer expected
  `bash-5.2#`, whereas Debian supplied `root@(none):/#`.
- Proofs 2 and 3 completed the scratch build and conversion, then rejected
  an expected hardlink: the mounted result had distinct inodes. That
  fresh Dockerfile fixture had created a link to a file from an earlier
  layer. This did not establish where BuildKit lost that relationship.
  The known independent OCI fixture is used for conversion evidence;
  no broad exporter guarantee is claimed or changed.
- Proof 4 dropped capabilities before setting `user.max_user_namespaces`,
  so the helper correctly failed with permission denied. The final proof
  performs that init operation before the drop, as the merged trial did.
- The first golden run passed the guest checks but its permissive serial
  decoder accepted interleaved daemon shutdown text. After waiting for
  the log drain, strict decoding correctly rejected interleaved kernel
  watchdog text during a large serial write. The final observer emits
  one encoded line per write and verifies strict base64 and gzip CRC/EOF.
  No corrupt dump is treated as a converted artifact.

The final `proof-preserved` run passed both guests. Builder boot and job
took 3.737 seconds. Converter boot, job and serial disk export took 75.896
seconds; this is not a conversion benchmark. Both guests observed only
loopback, a read-only root and helper `CapBnd=0xa82425fb`, `NoNewPrivs=1`.
The mounted golden result preserved uid 123/gid 456/mode 0751, the hardlink,
absolute symlink, deletions/replacement and `user.phase6=yes`. Its 128 MiB
raw disk has SHA-256
`2a88d3730c07c32ae29fddf5a77355c332bcfc90d60c4ee96364ee286539ade6`;
its verified gzip has SHA-256
`de861aea08671d18ee319d3c9650ab468f7d05474685e2330ce8a01e79ca4f03`.
The pinned bootstrap root's digest remained unchanged after both boots.

The real build entry point also refused three altered source copies:
an altered key file, incorrect Release digest, and incorrect first-package
digest. Each exited 1 and published no output directory. GnuPG with only
one of the three signing keys exited 2; its missing-key error was not
ignored. These checks used actual upstream downloads, without a mock.
`provenance.json` records that host buildctl, buildkitd and Podman were
absent. No additional root package was needed to pass the golden proof.

These are manual artifact/tool observations, not automated Pinfold
guarantees. Production jail, audited egress, receiver validation, claims,
cancellation, cache recovery, migration and final suites remain later cuts.

## Source checks

Ruff check and format passed for the two Python scripts. Shellcheck passed
for each common-plus-role script as executed, and shfmt passed. Actionlint
passed with only its stale unknown-`ubuntu-26.04` runner diagnostic excluded;
that runner is already the repository's release/kernel/CI convention.
`cargo fmt --check` passed. No Rust source or Cargo dependency changed;
clippy and runtime suites were not run for this tooling-only cut.

## Preservation and cleanup

Raw files are retained outside the disposable VM at:

```text
/Users/adam/Code/pinfold/target/phase6-builder-bootstrap-2026-10-28/evidence.tar.gz
```

The complete bundle is 267,038,450 bytes, SHA-256
`f8681084207eabc7004667342bd97df650d2ee101aa365d8dd75965a03dbd97b`.
Its `members.json` covers 91 regular files and 929,008,845 raw bytes.
Local verification read every file, checked every size/hash, consumed
gzip padding/footer to EOF, independently hashed the final archive's
1 GiB root, and checked the final converted gzip's CRC/EOF and 128 MiB
raw digest. Both final guest logs contain the measured capability bound
and no_new_privs. No disk was mounted or filesystem-parsed on the host.

The bundle preserves both bootstrap builds, original publisher response,
signed metadata, all build/proof raw logs, proof sources, fixture
and OCI input/output disks, and converted dumps. Failed serial dumps are
retained under their failed run names; only `proof-preserved`'s converted
dump is admitted as valid. Derivable builder caches and converter work
disks are excluded. The verified final VMM/kernel downloads are retained
once. All proof VMM processes were gone before copying.

`artifact.json`, `proof.json`, `provenance.json`, `negative.json` and
`preservation.json` are compact copies/receipts. Full console output is in
the ignored bundle. The bundle also contains the exact golden input as
`golden-input.ext4`, so reproducing this cut does not require another
builder or access to a previous store. Reproduction still requires a new
exclusive disposable Linux lease, KVM, the documented build/proof host
tools and its mandatory cleanup.

After the local verification completed, `labvm release` returned
`{"ok":true,"command":"release","status":"idle"}`. The unmodified
response is `release.json`; the primary ignored output directory retains
the local verification and release receipts too. The managed worktree is
kept for the operator's final-head reviews. Nothing was merged or released.
