# Linux builder decisions and admission, 2026-10-30

PR252 replaces Linux's Podman image path with pinned native BuildKit in one
finite Firecracker builder VM and a fresh independent converter. This record
consolidates the decisions for reviewed runtime commit
4f87c982bbf0c1a13f29a22f0e3278774d32fd2f. ARCHITECTURE.md remains binding.
The [validation record](2026-10-30-builder-validation.md) separates source
reasoning, component trials and real CLI observations.

## Implemented choices

- Freeze caller context and consulted image identities. The pinned native
  resolver supplies configurations before frontend optimization can omit
  meaningful zero-layer sources. Every consultation counts; metadata lookup
  closes before RUN. Served configurations may be replayed, including epoch
  paths. No host Dockerfile parser or registry fallback replaces this boundary.
- Native aliases have separate OCI context stores even when IDs agree.
  Native discovery pins external image digests and preserves exact local
  bindings. The helper is producer-only Go/vendor code; the host needs no Go,
  registry credentials, SSH agent or generic guest RPC.
- The host accepts a bounded complete runnable OCI graph. It rejects unsupported
  attachments, nested indexes, external URLs, unknown media and unreferenced
  objects. It does not unpack caller layers or mount a guest-writable filesystem.
  The receipt is not an attestation or independent build provenance.
- The converter has separate storage, no builder cache and no egress. Its
  private FROM-only configuration removes inherited ONBUILD triggers, including
  case-insensitive aliases, without removing the published image's triggers.
  It admits a completed opaque root before publication. Launch never builds.
- Rename commits the catalog generation. Unique and latest references publish
  together. A post-rename directory-sync error cannot undo a committed success.
  Running roots and selected build inputs retain leases; deletion cannot
  retarget an already selected identity.
- Reuse normal layers after complete recipe failure only across the physical
  clean barrier: daemon teardown/sync/unmount, VMM reap and host backing sync.
  Incomplete replies, interrupted jobs and failed audit cannot certify clean.
  Recovery may selectively prune mutable RUN cache mounts, not ordinary layers.
- Managed profile/project builds use the layer cache by default. Build
  --no-cache forces fresh RUN/package updates; refresh is no longer implicit.
  Caller-image caching is unchanged. Existing Ring hashes replace host software
  SHA work while preserving every full read and digest check. Standalone init
  retains its existing Sha2 closure; no new dependency or verification shortcut.

No resident builder, snapshot service, converter/builder merger, hotplug,
host registry/FUSE service or wider guest authority was adopted. The earlier
lifecycle comparator that replaced physical read-only authority with guest
staging/policy was rejected. Intermediate proposals are historical evidence,
not alternative supported paths.

## Selected-input handoff: reviewed source argument

The host creates a private selected backing file once and reserves
640905216 bytes before VMM creation. It retains the original File; VMM
configuration and jail provide read-only access. A distinct staging path is
formatted and its formatter waited for. Population owns the retained File,
checks type/device/inode/length and bounded sector-aligned staging, copies
without replacing or resizing the destination, requires sync_all success,
explicitly drops the writer, then checks cancellation/deadline again.
Only success reaches the authorization write. Init defers opening this
attachment until authorization, requires BLKFLSBUF, and mounts read-only.

Exact source: [reservation and attachment](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/crates/pinfold/src/core/image/linux/job.rs#L798),
[staging/population/authorization](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/crates/pinfold/src/core/image/linux/job.rs#L903),
[population checks and writer closure](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/crates/pinfold/src/core/image/linux/job.rs#L1256),
[jail constraints](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/crates/pinfold/src/core/runtime/firecracker/jail.rs#L271),
[deferred attachment](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/crates/pinfold/src/init/firecracker.rs#L318),
[invalidation and mount](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/crates/pinfold/src/init/firecracker.rs#L393).

This is trusted-code reasoning, not mechanized proof or an externally observed
VMM FD lifetime. Pre-authorization failures cannot publish; cancellation after
authorization cannot retract an already sent frame. Accepted real builds and
G44 failure/recovery exercise the path but do not observe every host syscall.
Complete writer-FD ordering, deliberate guest-cache priming, handoff-specific
SIGINT and compromised-producer malformed-frame/graph injection were not
demonstrated. A proposed large tracer was rejected and never implemented.

## Explicit admission decisions

On 2026-10-30 Adam answered:

- "Accept the documented evidence scope": accept the reviewed source argument
  plus retained real end-to-end results, with the limitations above, instead
  of the earlier archive's stronger external instrumentation expectation.
  The architecture and security controls remain unchanged.
- "Approve these limits with the stated evidence limits":

  | Resource | Approved current value |
  | --- | --- |
  | Finite role VM | 2 vCPUs; 2560M memory |
  | Builder cache / work disk | 8 GiB / 3 GiB |
  | Per finite role deadline | 300 seconds |
  | Input / aggregate result / completed root | 512 MiB bounds |
  | JSON / header / object count | 1 MiB / 8 KiB / 4096 |
  | Retained job log | 64 KiB |

  Bootstrap geometry is separate. Historical fixture fit does not establish
  arbitrary-project or concurrent capacity. Existing PID and outer controls
  remain unchanged; see the validation record's unperformed boundary cases.
- "Approve the one-time 309.90-second exception": exact runtime 4f87 and final
  ordinary run 41057246696 at 309.900624968 seconds. CI remains failed.
  Future 300-second suite policy, per-role deadlines, assertions and timer
  stay unchanged. This does not automatically admit a different head.

The original operator approval/source-assurance records postdate runtime 4f87;
they are separately preserved as described in [provenance](2026-10-30-builder-provenance.md).
This documentation-only successor must demonstrate unchanged non-archive
bytes and explicitly reconcile the head-bound exception before merge.
No merge or CLI release is authorized by these decisions.

## Caller contract and preserved state

Linux builds require Firecracker/KVM and acquire the digest-verified bootstrap.
Linux build and image build accept repeatable --allow HOST independently of
agent egress; Apple rejects this Linux-only addition. The pinned frontend
cannot be replaced by #syntax. Only a managed project's selected local profile
identity appears in its public base label; caller/profile base is empty on
both platforms. Other consulted identities stay internal.

Podman-only references require rebuilding. Old Podman stores, derived roots,
managed disks and persistent Apple volumes are not deleted or converted.
A fresh trial does not prove preservation of an existing real store.
Strict live mounts remain deferred. Final release audit, Caller changes,
merge authorization and CLI tagging/release remain separate.
