# Firecracker BuildKit trial and proposed builder role

Date: 2026-10-28. [Issue 230](https://github.com/adamaltmejd/pinfold/issues/230).
Source: main at d7656d7dbe832635615f3dd69ae3840e68c6f7ab.
Evidence/proposal only. No production code, dependency, assertion or binding
contract changes. Podman remains required.

## Recommendation and limits

Propose rootful BuildKit's OCI worker with overlayfs in an explicit Linux
builder role: pinned read-only bootstrap, writable owned cache, no NIC and
host-proxy egress through vsock. Keep the agent role exact. Use the same
pinned BuildKit in a fresh converter VM for OCI-to-ext4 conversion; the
trial does not justify another unpacking dependency.

This establishes feasibility, not retirement readiness. Adam's substantive
role/control approval and the acceptance gates below remain necessary.
The exact approved handoff and operator workflow were read in the ed8e
checkout. Phase 4's strict live visibility guarantee remains deferred.
No VMM migration, Linux live filesystem, automatic migration or external
image-import API is proposed. The trial used its own managed worktree and
exclusive disposable Linux lease; it touched no Mac runtime/image store.

## Execution and bootstrap

Nested Firecracker ran as the non-root agent user with KVM group access
inside the leased Debian 13 VM: host kernel 6.12.107, two vCPUs on an
i5-9500T, 3,922 MiB host RAM. Builder/converter guests used two vCPUs and
2,560 MiB. These are sufficient tested allocations, not minimums.

| Input | Version and SHA-256 |
|---|---|
| BuildKit Linux amd64 archive | 0.34.1; a59539daf547fe0958efe2d07e41c0a6944e1112a66d7e519ec1cf5b673ef15b |
| Firecracker archive | 1.17.0; 06094a1108ae9e82aa4c23a775aa92758f53f1175d422270d9d6162cb9ade558 |
| Pinfold guest kernel | 6.18.57; 1a1b28b34f89a8e49d90c72997f1b89bfd236ff44669dda9e91486c89adafc99 |
| Final trial bootstrap root | c88035159668fa2d0a1eb80a6f19426b276c4549d23ecdac3090921866286974 |

The VMM/kernel pins match baseline crates/pinfold/vmm.toml. BuildKit's
archive matched the upstream release asset digest before extraction.
Debootstrap and Debian's signed archive assembled the root, independently
of installed BuildKit/Podman. Package versions and root bytes are retained.

This is not a published production artifact or a bit-reproducible build.
Debian inputs were live; timestamps and filesystem UUIDs vary. Production
needs a clean CI bootstrap with a pinned Debian snapshot/package closure,
source/signature evidence, immutable release bytes and the existing
SHA-256-verified bounded installer. No installed builder is needed to
produce its first root. Trim trial Python/socat and unused CNI/QEMU tools
there. Pins move under the existing release policy, not at launch.

The trial RPC executes arbitrary root commands. Its Python proxy lacks
Pinfold's production framing/SNI/security behavior. Neither helper is a
candidate production API. The trial did not exercise the production VMM
jail, claims, audit-failure handling or cancellation. No Pinfold suite ran.

## Privileges

Real builds used a read-only bootstrap disk, writable build/cache storage,
tmpfs /tmp and /run, proc/sysfs, devtmpfs/devpts and cgroup v2. User
namespaces stayed disabled. No NIC, TAP, CNI, FUSE or containerd was used.
The pinned kernel already supports overlayfs and loop block devices.

Trusted post-bootstrap services succeeded with no_new_privs=1 and
bounding/effective/permitted sets 0x00000000a82425fb: CHOWN, DAC_OVERRIDE,
FOWNER, FSETID, KILL, SETGID, SETUID, SETPCAP, NET_BIND_SERVICE, NET_RAW,
SYS_CHROOT, SYS_ADMIN, MKNOD, AUDIT_WRITE and SETFCAP. This is sufficient,
not a proved minimum. Final converter and RUN checks bounded both daemon
and RPC; earlier narrowed native checks bounded the daemon only.

Removing SYS_ADMIN made the same native build fail its bind mount with
operation not permitted; its allowed counterpart succeeded. Ordinary RUN
was root with OCI's 14-capability set 0xa80425fb, no SYS_ADMIN,
no_new_privs=1 and seccomp enabled. Its sandbox hid the daemon socket.
Explicit RUN --network=host was refused as network.host is not allowed;
default RUN succeeded in that same VM.

Rootless BuildKit adds RootlessKit, mappings and user namespaces while
still needing mounts. It does not fit today's agent controls. Do not use
the discouraged no-process-sandbox shortcut.
[Rootless requirements](https://github.com/moby/buildkit/blob/v0.34.1/docs/rootless.md).

## Egress

The OCI worker's host network means the guest's namespace here, with only
loopback. Daemon proxy variables and explicit build proxy arguments used
127.0.0.1:3128, relayed over vsock to an allowlisting host proxy. Clear
NO_PROXY; run guest buildctl with empty Docker configuration and no
secret/SSH providers or host registry auth.

Daemon fetches and real RUN steps worked. In the same VM, RUN reached
api.github.com, received proxy 403 with not allowlisted for example.com,
and failed a direct 1.1.1.1:80 connection immediately with curl exit 7.
The candidate proxy recorded allowed/denied decisions. This proves the
topology, not Pinfold's production proxy.

Observed recipe hosts were registry-1.docker.io, auth.docker.io,
production.cloudfront.docker.com, deb.debian.org, github.com,
release-assets.githubusercontent.com, registry.npmjs.org and cli.github.com.
api.github.com was a control, not a recipe requirement. Use exact hosts,
including redirects; no general CDN suffix is justified.

Propose those eight initial Linux build hosts and explicit repeatable
--allow HOST additions on build and image build, reusing core domain
validation. These are proposed CLI/caller changes. Do not reuse agent
allowlists or auto-authorize origins in recipe text. A new redirect must
fail with an audited denial until explicitly allowed. Authenticated
registries need separate evidence/authority; do not copy host auth files.

BuildKit's --proxy-network covers exec traffic, not registry/Git/HTTP ADD
fetches, and adds inner networking/CA behavior. The simpler no-NIC
arrangement worked, so it is not selected.
[Proxy scope](https://github.com/moby/buildkit/blob/v0.34.1/docs/proxy.md).

## OCI output, identity and local bases

BuildKit exported OCI directories inside the guest. The scratch adapter
sent typed length-prefixed metadata/blob frames over vsock. The host
derived ordinary 0600 staging paths from kind or SHA-256, applied no guest
path/owner/mode/link/xattr metadata, and never extracted layers or
mounted/read a builder-writable disk.

The model bounded headers to 8 KiB, JSON to 1 MiB, payload to 512 MiB and
frame count. It checked exact sizes/hashes, one linux/amd64 image,
supported media, config platform, local descriptor references, diff-id
count, and absence of URLs/unreferenced objects. Truncation, corrupt
digest, duplicate frames, oversized declarations, path-like kinds,
ownership fields, trailing bytes and missing metadata were refused without
changing the prior generation.

This is standalone model evidence. Expected-label admission, name/catalog
transactions, interruption recovery, deadlines and concurrent publishers
are not implemented. The production receiver must check those too,
including the complete bounded descriptor graph and final success.
Select the runnable manifest explicitly, not an attestation manifest.
Publish verified immutable objects before one atomic catalog generation
containing the unique build ref and latest together.

Use the runnable manifest digest as image identity. Cached cold/warm
default shared efd07e7e24f83a6b583f3757bc9abdb8ff7c73c7b84591a012957defd22ff35b.
Full cold/warm/restart shared
b9ab91b85b2f7b0dbf13d49024587388a9e2150258fe78e22449ab5e23168364.
Outer frame bytes differed because the OCI index includes creation time.
Keep unique refs/names outside image content.
[OCI exporter](https://github.com/moby/buildkit/blob/v0.34.1/exporter/oci/export.go).

A fresh converter with empty BuildKit state loaded validated read-only OCI
input with all proxy destinations denied. ARG-expanded FROM, stages,
COPY --from and inherited Env worked without a registry. BuildKit strips
:latest from the context option key. Keep it in FROM if wanted:

    --oci-layout localbase=/input
    --opt context:localhost/pinfold/local=oci-layout://localbase@sha256:DIGEST

The original key including :latest was ignored and attempted a localhost
registry request. Its failure is retained.
[Normalization](https://github.com/moby/buildkit/blob/v0.34.1/frontend/dockerui/config.go#L444-L461).

Snapshot catalog bases to immutable identities and protect them for the
build's lifetime. Supply catalog contexts to the frontend; do not build
another Dockerfile parser or let missing managed bases fall back to pulls.

## Conversion and launch

Use a fresh converter with the immutable bootstrap, new work disk,
host-written read-only validated OCI input, no builder cache, no NIC and
no egress. Guest-writable build state is not conversion authority. The
host writes input files/disks from its fixed-path validated objects; it
never reads a guest-writable disk.

A FROM-only request exported a flattened tar through pinned BuildKit.
GNU tar unpacked it only inside the converter, then guest mkfs.ext4 wrote
a raw disk. The host received bounded hashed bytes and stored them
opaquely. BuildKit handles OCI whiteouts. Repeated plain layer extraction
would not. Do not use type=local: its receiver rewrites owners.
[Local receiver](https://github.com/moby/buildkit/blob/v0.34.1/session/filesync/diffcopy.go#L111-L121).

The guest-mounted ext4 preserved uid 123/gid 456, mode 0751, hardlinks,
symlink, deleted/replaced layer contents and a golden user.phase6 xattr.
The first ADD fixture had already lost its xattr before OCI export; its
failed check is not a conversion defect. An independent OCI fixture
settled that boundary. Security capability/ACL metadata still needs coverage.

The converted 128 MiB fixture root booted separately with one read-only
drive, one vCPU and 256 MiB RAM: no builder, cache, vsock or NIC. The console
showed expected owners/links/whiteout results. This is a real Firecracker
boot, not a Pinfold launch. An initial poweroff-waiting observer timed out;
the final observer watches markers and terminates the VM. It does not
establish that 128 MiB guest RAM suffices.

Propose a derivation key over manifest, converter-root/tool pins and exact
filesystem recipe; record the raw disk digest separately. Bit-identical
mkfs output is not claimed. Lock the key; publish only complete successful
output. No reusable partial root.

Finish conversion before publishing successful build refs. Launch only
resolves catalog plus completed representation and never starts a builder
or converter. Missing/damaged roots require explicit rebuild preparation.
This moves today's first-launch conversion cost to explicit builds.

## Measurements and cache

Single observations on this nested host, not comparative benchmarks.
Seconds are guest command time, including export/helper work and sync
where specified, excluding VM boot and host transfer/catalog. Cold means
empty BuildKit worker, not cold host/CDN caches. Native default/documents
completed build/OCI export, then failed because the frame helper was absent;
the next boot recovered those artifacts. The full native job failed ENOSPC.

| Operation | Seconds |
|---|---:|
| Native default, no cache | 27.55 |
| Native default, cached | 0.77 |
| Native documents, no RUN cache | 79.05 |
| Overlay default, empty worker | 20.76 |
| Overlay default, cached | 1.68 |
| Overlay default, --no-cache | 19.01 |
| Overlay full, no RUN cache | 44.79 |
| Overlay full, cached | 2.20 |
| Overlay full after VM restart | 2.73 |
| Fresh full-image conversion | 17.15 |

The final bootstrap waited for BuildKit server readiness: 1.90 seconds for
the golden converter, 2.42 for full conversion and 2.73 for builder restart.
Earlier boot figures timed the RPC alone. The full OCI payload was
151,250,736 bytes. Its root tree used 384,220 KiB with 7,567 entries; the
existing sizing formula produced a 451,499,008-byte ext4 root. The guest
mounted it and checked tool paths, profile files and no setuid/setgid
files. This does not prove every installed tool works.

Native filled an 8 GiB cache after repeated managed builds: 7.45 GB
BuildKit usage, then full failed with ENOSPC. Guest prune reclaimed the
records and kept exported artifacts. Overlay reported 1.21 GB BuildKit
usage and 2.5 GiB filesystem use including exports; final probes raised
these to 1.37 GB and 2.8 GiB. Sequences differed; this is not an isolated
benchmark. It justifies overlayfs and explicit cache bounds, not a native
fallback or a universal capacity guarantee.

Start with one version-keyed 8 GiB builder cache and exclusive lifetime
claim. Serialize cancellably before adding a resident builder or multiple
cache owners. Fresh full conversion used 1.6 GiB of a 3 GiB work disk.
Graceful teardown must stop work, flush/unmount and release claims;
discard derivable cache after an unclean job. The trial synced before
VMM termination and observed restart reuse; it did not prove crash safety.

Doctor can report allocated backing-file bytes/ownership without booting
or reading its filesystem. Clean must hold the same claim and remove only
idle builder cache. Published OCI/roots survive cache removal. du/prune
inside a guest is separate from host allocated-byte reporting.

The trial's 512 MiB protocol cap fit built-ins, not arbitrary caller
images. Propose initial 8 GiB stream/root caps, 1 MiB JSON, 4,096
descriptors, 8 KiB headers, a 15-minute job deadline and five-minute
stream deadline, with resource refusal.
Larger limits are untested and need approval/boundary checks. Bootstrap
downloads retain the existing 512 MiB/five-minute contract.

Distro Podman 5.4.2 was below the required 6.1.4. No fair current-Podman or
replacement-CLI timing was measured; infer no speedup. The actual integrated
Linux suite must demonstrate the five-minute budget. Small fixture costs
make a trial plausible, not proven. Extend existing guarantees 23
(naming/labels/bases/cache/tails), 15 (retention/in-use), 27 (read-only
doctor), and specify a new builder-isolation guarantee. No runtime stubs
or test-only binary paths.

## Proposed control changes, cuts and remaining gates

Agent Always applied, trust, secrets, disks and outer VMM jail/parent
lifetime/seccomp/vsock controls stay exact. Explicit Linux image preparation
would change these controls only in an internal builder role:

- Guest-root RUN with measured OCI capabilities, rather than host uid and
  empty capabilities; trusted daemon additionally needs SYS_ADMIN/mounts.
  Keep no_new_privs, disabled user namespaces and no insecure entitlements.
- Writable build/cache state with immutable bootstrap. No agent homes,
  managed agent disks, host binds, host auth or unrelated devices.
- Enforced proxy egress replaces unrestricted trusted Linux builds.
  Apple image builds retain current behavior.
- Explicit bounded host image output replaces the agent's fetch-only path
  for this role; output reaches only verified image staging/catalog/roots.
- Neutral manifest identity and completed conversion before build success.
  Launch does no building. Proposed --allow and size limits are caller changes.

After Adam's approval, use bounded implementation cuts under issue 230:

1. Standalone bootstrap release/pins without installed-builder dependency.
2. Neutral catalog, bounded receiver and atomic publication.
3. Isolated builder/fresh converter with common claims/cancellation and
   the production proxy/audit.
4. Only after replacement validation: remove Podman adapter/catalog/cache,
   export conversion, obsolete preflight/XDG_RUNTIME_DIR and cleanup paths,
   with binding-doc updates.

Keep common naming/label/retention policy: unique refs plus latest, host
identity/fingerprint/base labels, managed no-cache, caller cache choice,
40-line/64 KiB failed-build tail, two-build retention, young caller
protection, shared names and running-image protection. BuildKit supplies no
local catalog. Build inputs need their own in-use leases through deletion
and concurrent publication.

Use the approved rebuild transition. Leave old Podman storage and managed
agent disks untouched. Old-store-only refs stop resolving; rebuild
profiles/projects with build and caller images with image build. No
migration/import API. Eventual Caller changes must name this break,
proxy/allowlist/auth behavior, identity/preparation changes and limits.

Before production admission still require:

- Substantive role/control approval and independent correctness/security
  plus simplification/spec-drift reviews.
- Production bootstrap release/pin/download verification.
- Real CLI/profile builds using production proxy, exact allowed/denied
  reasons, audit failure, owner death/cancellation and unchanged VMM jail.
- Real receiver labels/graphs/bounds and interrupted output/conversion/
  catalog publication preserving prior generations, without partial reuse.
  The scratch malformed-frame model is not that evidence.
- Build/clean/rm concurrency, cache exclusivity, local-base leases and
  abandoned ownership; CLI naming/labels/retention/tails.
- Actual agent reuse with builder stopped, managed disks preserved,
  final Mac/Linux gates and measured suite budget.

These are implementation gates, not permission to weaken current controls.


Raw evidence and reproduction: [evidence index](2026-10-28-builder-trial/README.md).
