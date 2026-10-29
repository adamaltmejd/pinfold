# 512 MiB bootstrap private pin, 2026-10-29

This pin/doc-only cut uses actual bytes from a fresh factory build of producer
`cf9ea3e3c5e1fb3e8263ec4ff94bb6389a0c1825`. It changes no producer, helper,
Rust runtime, test, dependency or runtime resource cap. The intended public URL
remains unpublished. The pin authorizes private proof through the existing
verified offline archive path; it is not public adoption or publication approval.
Existing public assets remain untouched.

## Producer identities

The operator's factory receipt reports exit 0 and 107.696270240 seconds.
That duration measures production of the artifact, not ordinary image-build
latency. The operator and independent artifact auditor passed checks of all
12 retained files, SHA256SUMS, full gzip, the single regular root member,
source identity and recipe. The worker checked manifest/sidecar hashes against
the independent receipt and matched every producer source input to the manifest.

| Built object | Bytes | SHA-256 |
|---|---:|---|
| Archive | 122074383 | 1e9a62f3eb034bf8582f4ff5d6e42310b56c5fbc1de201336342647228fe18cd |
| Raw root | 536870912 | 1ba38d2c1835623c36479387f60101529848ee8575fe1f5caef164e3a48d89a2 |
| Compiled helper | 22294690 | d038bd1623499d5ef47b1aec9659854b72eed48ae2f7d066ee6f0c0a358cc738 |

The actual recipe SHA-256 is
`edac709b3e4c4dcac3d4edeb80272fc2f052fe19d1370e245e61f1d4a79daa07`.
The build.py source SHA-256 is
`d34b541c96b56da0c30ca4f223cc8e8f12e65121c7ce4c2ca92c3dcd69e031b1`.
The helper source remains
`28c4e5b5ed7fa0d6682031c5b33ea02c0460033227d495180b3cd9bc8a8c28fe`;
the inspected helper binary matches the earlier fa artifact exactly.
Software, compiler, package, key and upstream source pins are unchanged.

Evidence is retained under the operator's
`/Users/adam/.codex/worktrees/ed8e/pinfold/.local-evidence/one-builder-latency-2026-10-29/bootstrap512/`.
The manifest SHA-256 is
`e67851bd5403af2a33104a59e211fe64aaec3e539f750434f46fc628840b264c`.
The discovery-build.json SHA-256 is
`7048aa1f32e6452fbc179b3d70337b284b5aec8a7011bc3fe4306f4e618bab24`.
producer-assets.tar retains the producer outputs;
factory-tree-and-integrity-receipts.tar.gz retains inspection evidence.

## Fit and comparison scope

The retained e2fsck -fn receipt passes. The filesystem is clean, with the same
feature set as fa, 4096-byte blocks and 256-byte inodes. It has 53239 free
blocks (207.96 MiB) and 26279 free inodes. The default journal is 16 MiB rather
than fa's 32 MiB. The mkfs flags are unchanged. These are observed fresh-build
results; no old image was resized and no closure content was removed to fit.

The full tree comparison observes 6480 nodes in each image. Node types, owners,
permission bits, symlink targets, hardlink topology and all readable xattrs,
including ACLs, agree. File contents agree with one qualified exception:
`var/cache/ldconfig/aux-cache`, 4364 bytes in each image, differs in content.
Its mode remains 0600, owner/group remain root and its xattrs remain empty.
The comparison excludes filesystem timestamps, inode numbers and directory
allocation sizes.

The operator's correctness review traced the exception to the exact Debian
glibc primary source: this generated ldconfig auxiliary cache records inode,
ctime, size and device metadata. Runtime uses the unchanged /etc/ld.so.cache.
That classification comes from the operator's source review; the worker read
the retained comparison rather than repeating that investigation. The exception
is retained without deletion or normalization. The comparison does not claim
an identical file-content tree. Raw-root hashes after read-only inspection
match the original fa and 1e9 identities.

## Pending private runtime proof

The consumer embeds the actual 1e9 archive/root/recipe identities and an intended
full-SHA URL. That URL is not published; offline-only proof remains required.
The binding spec and bootstrap README state the 512 MiB read-only root and
122074383-byte archive. No runtime input, output, memory, cache or deadline cap
changes. Earlier archives remain dated records of their own capture points.

The operator previously compiled the runtime source at 260d with the fa 1 GiB
pin. That compile and its latency observations do not prove this new pin.
No 512 MiB consumer runtime or performance proof has occurred at this snapshot.
The operator owns native compilation and normal offline CLI cold/warm and
functional proof. The worker runs only cargo fmt --check and git diff --check
for this cut, with no VM, build, suite, public push or publication.
