# Stock Podman contention reproduction and cleanup

On `yard-sthlm`, stock Podman 6.1.4 reproduced long native listings without
Pinfold or Switchyard in the experiment. Preparing an exact image made its
repeat fast. Building a derived image reintroduced the ownership conversion
and global lock contention. This does not resolve issue #89.

## Cleanup before investigation

Removed 55 disposable investigation graph roots and build targets below
`/home/yard/validation/pinfold-89-lxc-20261012`. A root-readable scan of
process command lines, working directories, executables, file descriptors
and mount namespaces found no references immediately before deletion.
Image, layer and container JSON metadata was archived first. Reports,
source, candidate binaries, patches, original OCI input and logs remain.

Root-filesystem free space increased from 16,040,173,568 to 32,707,768,320
bytes: 15.52 GiB recovered. Local cleanup removed roughly 2.6 GiB of
incremental compiler caches, an old isolated test build, Zig caches and
extracted Apple installer files. The signed installer and evidence remain.
No active runtime store was reset or pruned.

## Experiment

Used the installed unchanged Podman 6.1.4 binary, upstream commit
`464c15fec06f0074c916f9eff87c4806869c2833`, SHA-256
`11326740634a0929f438256c4e3ff90273c4399a279c68e9fde02be25f75c2a5`.
Effective graph/run roots were asserted to be private before mutation.
Storage reported native overlay, no graph options, and
`Supports shifting: false`. Private file locks kept this experiment apart
from the active runtime's shared-memory locks.

A locally generated scratch rootfs contained a static ownership-checking
executable, root-owned and UID/GID 1000 sentinels, and 150,000 one-byte files
with alternating root/1000 ownership. It was imported without network
access. A derived image added 10,000 files through `COPY`, without `RUN`.
The rootfs tar checksum was
`d9cf48edd91137980488c2acf50c70c4d8e9ad9df4f2db64d1445b52c00a9a4c`.

Each run used `--userns keep-id --user 1000:1000 --network none`, dropped
all capabilities, enabled no-new-privileges, used a read-only rootfs, and
mounted one disposable host sentinel read-only. Each container was removed
before its warm repeat. The experiment used a 180-second diagnostic bound
per command and a 20 GiB free-space floor. These are experiment safeguards,
not changes to Pinfold deadlines or tests.

| Image/run | Writer seconds | Concurrent listing seconds |
|---|---:|---:|
| Base, cold | 73.069 | 30.238 |
| Base, warm | 0.149 | 0.093 |
| Derived, cold | 85.718 | 85.615 |
| Derived, warm | 0.234 | 0.155 |

All four writers and these listings exited zero. All guest checks reported
UID/GID 1000, root sentinel ownership 0:0, user sentinel ownership
1000:1000, and mounted host sentinel ownership 1000:1000. The host
sentinel's ownership and SHA-256 were unchanged. Post-removal listings
succeeded; final private container inventory was empty. Derived-image
build time was 3.232 seconds. These are single synthetic trials on a shared
host, not whole-workload throughput estimates.

## Actual lock chains and diagnostic corrections

The initial observer checked children of only the main Go thread and
missed the ownership helper. A separate all-thread observer started a
native listing while the cold helper was still alive. Its 30.238-second
result is the cold listing above. The original driver's 0.099-second cold
listing started after the writer completed and is not contention evidence.
Both records are retained rather than rewriting the original result.

The corrected continuation used all-thread descendant discovery. One
initial build invocation rejected `--pull never` as extra positional input;
`--pull=never` succeeded. The failed invocation and original result remain.
The corrected combined script is retained for reuse; the actual run used
the original driver, separate observer and corrected continuation.

For the base image, listing PID 1364826 waited on image lock
`fc:08:525530`, held by writer 1364703, start ticks 126596900. Direct
helper 1364713, start ticks 126596907, ran `storage-chown-by-maps`.
The writer also held layer lock `fc:08:525536`.

For the derived image, listing 1365178, start ticks 126609883, waited on
the same image lock. Writer 1365161, start ticks 126609873, held both
locks; direct helper 1365172, start ticks 126609880, performed conversion.
The descriptor snapshot independently matched writer fd 10 and listing
fd 8 to the image lock, and writer fd 9 to the layer lock. Actual POSIX
wait records, path device/inode identities and ancestry agree.

## Implication and source review

Exact-image preparation moves conversion earlier, but does not eliminate
contention when a new image needs conversion. This experiment demonstrates
the runtime behavior independently of Pinfold. It does not assign the
historical EAGAIN failure or the separate exec-cleanup EPERM failure.
It also does not validate Pinfold's complete timeout/ownership/capacity
contract: no Pinfold binary ran in this experiment.

In Podman 6.1.4's vendored storage source, `CreateContainer` holds layer
and image WRITE locks (`store.go:1963`, `1974`) across mapped-template
creation (`1907`), `UpdateLayerIDMap` (`layers.go:1692`) and overlay
ownership conversion (`overlay.go:2508`). Normal storage shutdown also
takes layer WRITE (`store.go:3846`), so bypassing image initialization alone
would leave another blocking path.

A plausible storage-side correction would stage conversion outside both
global locks, then revalidate and publish under lock. Existing `PutLayer`
staging provides a pattern, not a drop-in fix. Source lifetime, concurrent
creators, incomplete-layer recovery, mappings, labels, hardlinks, xattrs
and lock ordering need correctness work. No patch was implemented or
validated here. No verified stock setting currently replaces that work
under the stated constraints.

## Evidence retained; experiment storage removed

Remote evidence directory:
`/home/yard/validation/pinfold-89-lxc-20261012/stock-minimal-20261013`.
Contains raw observations, results, stdout/stderr, descriptor identities,
metadata archive, ownership fixture and cleanup record. Driver and
continuation scripts are in its parent directory.

Archive, copied locally and checksum-verified:
`/private/tmp/pinfold89-stock-minimal-evidence-20261013.tar.gz`.
SHA-256:
`1711411991eb13c6ab0b9708d10340c84f520fbf267ab1c2bc90a7d03bf69b3e`.

After successful completion and another reference scan, removed the new
private graph root, run root and derived build context. The graph root
occupied 1,949,048,832 bytes before removal. Final host free space was
about 30 GiB. The active graph/run paths remained the normal per-user
paths; Yard PID 965905 remained running with its October 9 start time,
and its current box was running. No service restart, security change,
FUSE change, concurrency reduction, release or upstream publication occurred.
