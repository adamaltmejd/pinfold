Superseded by [2026-10-15-native-worktree-protection.md](2026-10-15-native-worktree-protection.md).

# Existing Mac worktrees: filesystem enforcement

2026-10-15. Follow-up to the [earlier worktree report](2026-10-15-worktree-recommendation.md).
The requirement is now explicit: existing worktrees on default case-insensitive
Mac storage, with no user-created volume or worktree relocation. This report
supersedes the earlier storage recommendation for that requirement. It does not
change Pinfold's binding contract or enable worktrees.

## Recommendation

Pursue a **host filesystem service inside Pinfold's security boundary**, with an
existing filesystem client in the Linux guest. The service opens the original
host files, authorizes the actual objects and namespace operations, and performs
allowed writes itself. Do not expose the same working files through an additional
raw writable VM share. Keep Apple Container, Pi's interactive role, caller-owned
commits and Switchyard orchestration.

The practical path demonstrated here is **SSHFS over a forwarded Unix stream**.
SSHFS runs inside the existing VM; no macFUSE installation, host-mounted volume,
SSH login service, SSH keys or host TCP listener is needed. Working files stay
where they are. The guest and host editor use the same files. There is no working
copy or Git-history copy per launch, and no import/synchronization step.

This is a real alternative to refusing default Mac filesystems. A disposable
prototype mounted a genuine dirty linked worktree on this Mac's existing
case-insensitive filesystem. It supported worktree-root edits and host Git interaction
while blocking the tested aliases and protected namespace changes. Two VMs used
one service successfully. **The prototype is not a complete filesystem or a
production security proof.** Its metadata-operation restrictions alone make it
unsuitable to ship unchanged.

For production, keep authorization in Pinfold's Rust binary. The Go prototype
was a small way to test the boundary with an existing protocol library; it is
not a recommendation to introduce a separate Go runtime or adopt a new container
platform. A Rust SFTP library with handle-preserving callbacks is a plausible
starting point, subject to the specific corrections and fresh tests below.

## Why the mechanism is different

Earlier guest pathname masks left a raw host writer able to reach the protected
inode through another spelling or hardlink. Here every guest write goes through
the host service. The guest filesystem client is untrusted: direct protocol
requests from a guest also reached the same checks.

The prototype combines these controls:

- Open without truncation, then inspect the opened object before modifying it.
  Retain that descriptor for data writes and reauthorize each write.
- Confine path resolution with Go's `os.Root`; protect actual file identities and
  canonical host paths obtained from opened objects. No custom lowercase table
  substitutes for APFS resolution.
- Protect source and destination names, protected directories and every ancestor
  whose replacement could redirect the host's protected path. Authorize parents
  before creation, so `O_CREAT` cannot modify a denied directory first.
- Serialize guest mutations across all connections to the service. Other writable
  routes to the same host files must not bypass that serialization and policy.
- Snapshot identities in the fixture's protected trees, deny regular-file
  mutations when link count is not one, and refuse guest hardlink creation.
  These last restrictions are conservative prototype limitations, not a claim
  that a link-count check alone solves protection.
- Deny unsupported mutations. Unsupported handle metadata requests do not fall
  back to pathname-based writes.

Trusted host processes may edit and atomically save ordinary source, stage and
commit. They must not concurrently change protection roots, effective hook/config
locations, relocate the worktree, move writable objects into protected areas,
or introduce aliases across the protection boundary. The service mutex does not
lock arbitrary host processes. The ordered late-alias/held-handle diagnostics
show useful defenses, not atomic safety against arbitrary host structural races.

## Alternatives examined

| Mechanism | Finding |
|---|---|
| Seatbelt on the actual native writer | Better than sandboxing the VM owner, but insufficient alone. Native tests blocked case/symlink paths and protected namespace operations, while existing hardlinks and pre-opened writable descriptors still changed protected bytes. |
| Immutable inode flags plus denying flag changes | Blocked new alias opens and namespace changes in disposable native fixtures. A descriptor opened before setting the flag still wrote. Host editing interference and crash cleanup make this unsuitable as a transparent default. No live repository flags were changed. |
| Existing SFTP/SSHFS components with custom policy | Strongest tested path. Reuses protocol/client, preserves the current Mac VM and existing host files. New security-critical host authorization and lifecycle work remains. |
| Rust `russh-sftp` | Better production-language fit than the Go prototype: `fstat`/`fsetstat` retain opaque handles, and an existing Tokio Unix stream works without SSH. Published 3.0.1 still needs byte-exact path handling or explicit invalid-UTF-8 rejection and owned session cancellation. Source assessment only. |
| NFSv3 with Rust `nfsserve` | Reserve path with file-ID callbacks and an existing guest kernel client. Needs Unix-stream transport adaptation/guest relay, trusted mount setup and cache/durability validation; library documents incomplete implementation. Not tested here. |
| 9p, native macOS virtiofsd, Matchlock transport | Respectively add kernel support, a QEMU/runtime change, or a bespoke protocol integration without removing the policy/mount problem. No advantage established over the tested standard client/protocol path. |

Pinned sources, dependencies, privileges and API limitations are in
[integration research](2026-10-15-worktree-filesystem-enforcement/integration-sources.md)
and [native kernel experiments](2026-10-15-worktree-filesystem-enforcement/native-kernel.md).
The old stock Matchlock failure was not repeated.

## Evidence at the actual boundary

The host was arm64 macOS 27.0.1, build 26A434, Apple Container 1.5.0; guest kernel
6.18.15. Host Git 2.56.0, Go 1.27.2. Guest SSHFS 3.7.3 and libfuse 3.17.2 came from
hash-checked Debian packages extracted only into guest tmpfs. This follow-up invoked no image build or host software installation.
The mutable image tag was inspected at cleanup; its identity was not retained
for every launch, so exact image provenance is a reproducibility limitation. Source checkout remained branch
`codex/issue-89-bounded-listing`, HEAD
`198600b1751d889d354ed3657d8f839e8436e469`, plus its ten existing dirty tracked
files. All 393 initially captured files and the tracked patch were verified
unchanged. Pinfold was neither rebuilt nor installed in this follow-up.

| Experiment | Observed result and scope |
|---|---|
| Original dirty linked worktree | Real mounted guest edits at the worktree root, new directories, ordinary atomic replacement, Git status/log/staged diff, and reads through an ordinary relative symlink worked. Host staged state was not modified by guest reads. |
| Case, hardlink and symlink aliases | Writes/truncation through `.GIT`, old pointer/config hardlinks and pointer symlinks were denied. `.VSCODE` creation/write and Unicode spelling variants were denied. Host hashes and identities stayed unchanged. |
| Namespace operations | Pointer unlink, rename, replacement, new hardlink, both nested-hook ancestor renames and creation under protected directory aliases failed. Ordinary sibling writes succeeded. A real host commit ran the trusted hook marker. |
| Independent-review correction | The first prototype used caller spelling for protected paths. Review identified a possible ancestor bypass with alternate-case configuration. Version 2 obtains policy paths from opened objects; tests configuring `.GiT`, `.VSCODE`, `DEEP/MIDDLE`, decomposed accents and Kelvin spelling denied the tested changes. The source-identified version-1 gap was not separately exploited. |
| Continuous requests | An existing-pointer alias added by the host after service startup was denied. A new protected config inode aliased after startup was denied by the multiple-link check. These ordered tests are distinct from arbitrary host races. |
| Open handles and raw protocol | A guest sent SFTP directly, bypassing SSHFS; protected opens still failed. Writes through a previously opened ordinary descriptor were denied after controlled host link/move operations. One write after unlinking an alias failed with ENOENT because descriptor path resolution was unavailable; compatibility is not complete. |
| Concurrent guest namespace changes | Successfully exchanged two ordinary directories 100 times while issuing 300 writes through a name that alternated between an ordinary file and a protected-file symlink. 79 writes succeeded, 221 failed, the protected pointer stayed unchanged, and the ordinary file changed. This supplements an earlier weaker test whose rename attempts were mostly refused. |
| Two VMs | Both saw ordinary writes from the other. Each attempted 100 protected pointer writes; all were denied. One shared broker was tested, not multiple independently coordinated brokers. |
| Service loss | After an already-running guest made a control write, terminating its broker caused its next write to fail with a connection-abort error. Host contents stayed unchanged. An earlier attempt failed at runtime process setup and was inconclusive about the write syscall; both records are retained. |

Native-only probes also confirmed that this host implements `O_RESOLVE_BENEATH`,
`O_NOFOLLOW_ANY` and `O_UNIQUE`: confined relative symlink reads worked, an escaping
symlink failed, and `O_UNIQUE` refused multiply linked files. These APIs can reduce
custom resolution code in a Rust candidate. They do not replace selected-object
policy, namespace protection, or per-write checks. Behavior on the minimum
supported macOS version has not been established.

## Startup privileges and integration

The prototype root helper mounted FUSE and passed its open descriptor to stock
SSHFS, then dropped to uid 501/gid 20, cleared supplementary groups and all
capability sets, and set `no_new_privs`. The observed SSHFS daemon had zero
capabilities in the original guest user namespace. No setuid filesystem helper
or long-lived root filesystem daemon was used.

The research box nevertheless had setup capabilities in its stored runtime
configuration. A later raw nonroot runtime exec retained that bounding set.
Trying to clear it from an already-unprivileged process failed closed. A trusted
static wrapper entered as transient root successfully dropped identity and every
capability set before the shell and its child ran.

Pinfold already routes normal workload execs through its trusted static
`init exec`. This supports an integration direction without necessarily changing
Apple Container: trusted initial mount setup, drop before PID 1 creates threads,
and unconditional independent dropping on every later exec. Replace the existing
root call to image-provided `chmod` with a trusted init syscall. Validate UID/GID
provenance, saved IDs, groups, securebits, descriptor closure, environment handling
and all entry points before any image-controlled executable or loader runs.

This requires an explicit architecture change to the current literal startup
rules. It is not justification to add SYS_ADMIN to current launches and assume
a PID 1 drop covers later execs. The tested wrapper is not integrated Pinfold.
Both the raw no-capability baseline and prototype could create a new user namespace
and obtain capabilities *inside that namespace*. No claim of universally
capability-free descendants follows from the observed snapshots. That observation
is not a reproduced Pinfold CLI defect or evidence of initial-namespace/host
privilege. Preserve Linux's existing namespace/seccomp controls; Linux startup
was not changed or retested in this follow-up.

The current forwarded socket already carries egress. Production needs an explicit,
owned protocol multiplexing or separate supported socket arrangement; this
experiment used it only for filesystem traffic. Broker death must fail closed,
with no writable-share fallback. Session/resource limits, malformed frames,
backpressure, owner-death cleanup and mount readiness need integrated tests.

## Measured cost

A generated 100,001-file tree contained 476,708,864 logical bytes, including one
64 MiB blob. Fixture creation took 5.60 s and is not a launch operation. An edit
added one file, so later enumeration counted 100,002. Each table entry is a median
of three sequential samples, timed inside the guest. Images were cached and OS
caches uncontrolled; these are warm-oriented synthetic observations, not cold
startup or build benchmarks. Native comparison exposed only the disposable
benchmark directory, containing no protected files.

| Operation | Native share | Enforcing service + SSHFS |
|---|---:|---:|
| Enumerate all files | 0.121 s | 0.636 s (5.2x) |
| Read 64 MiB | 0.023 s | 0.216 s (9.3x) |
| 100 atomic edit/replace operations | 0.057 s | 0.370 s (6.5x) |
| Git status listing all fixture files | Not measured | 0.900 s |

The extra CPU/protocol work is material, although these absolute costs look
practical enough to justify integration work. Do not generalize to compilation,
package installation, databases, memory mapping, file watching or large Git
histories. Zero attribute/directory timeouts and direct I/O were used for host
visibility; there is no measured cache optimization proposal.

One second-box sample took 0.882 s to native start, 0.131 s to unpack client
packages into tmpfs, and 0.095 s from mount-helper dispatch through a successful
file read. Unmount was 0.062 s and removal 0.099 s. Client packages would be shipped
in a production image, not unpacked on every launch. These are not Pinfold-ready
numbers. The experiment snapshots only small protected metadata trees; large
policy-inventory and multi-export admission costs remain unmeasured. It copies
neither working files nor history. Approximately 742 MiB of temporary fixtures,
packages, compilers' caches and binaries were created and removed; the compact
source/results bundle remains.

## Smallest proposed production scope and remaining gates

1. Implement one core host filesystem service and policy for admitted exports,
   with descriptor-based data/metadata operations, safe parent-relative namespace
   operations, protected root/ancestor identity handling and bounded session
   ownership. No Pi-side security wrapper. Coordinate every writable view of a
   protected authority; one-broker results do not prove cross-process policies.
2. Reuse SSHFS as an untrusted guest client. Prototype the Rust protocol backend
   with handle-preserving callbacks and byte-exact paths. `russh-sftp` 3.0.1 is a
   candidate, not validated production code. Keep native kernel confinement as
   defense in depth, not the sole alias policy.
3. Add trusted mount/exec setup with explicit privilege dropping and transport
   ownership. Update the binding spec in the same production change. Preserve
   original-host worktrees, no volume requirement, no copying/import fallback,
   and host-owned commits.
4. Add restricted-layout discovery for gitfile, common/per-worktree metadata,
   `commondir`, effective hooks/config and protected editor directories. Reject
   malformed/dangling/unsupported layouts and alternative writable routes. Fix
   the already-known effective-hook alias classification separately; this research
   did not change it.

The prototype refuses chmod, timestamp changes, handle truncation and hardlink
creation. Its Go library also translates FSTAT into a pathname query, which gives
incorrect retained-handle semantics after replacement. Ordinary symlink creation
and append were corrected in version 2, but absolute symlinks, unlink-open
semantics, locking, fsync, xattrs and file watching are not established. Do not
ship these restrictions silently or describe the prototype as complete source
filesystem compatibility. A Rust implementation needs fresh boundary evidence;
Go prototype success does not transfer automatically.

Extend existing G11/G22 with pointer/config aliases, creation and both sides of
namespace changes, held descriptors, nested/alternate-spelling ancestors,
normalization and cross-export/box cases. Include actual host bytes/identities
and host Git markers, with allowed controls. Extend G10 for host visibility and
supported file semantics, G17 for the admitted representation, and the relevant
privilege/lifecycle guarantees for first and later execs, service death and
cleanup. Add explicit protocol/resource limits and malformed-layout cases to
those guarantees. Run test-audit on changed assertions and the normal real Mac,
Linux x86-64/arm64 and prototype gates. No production assertion changed here.

Independent review supports this as a useful research stopping point and a
**direction for approval**, with worktree refusal retained until implementation
and its gates pass. Default Mac storage is no longer the required compromise;
new host enforcement code and guest mount integration are the engineering cost.

## Cleanup and reproduction

The compact bundle contained
sources, pinned dependencies, commands, raw outcomes, binary/source hashes and
chronological independent review. Failed and inconclusive observations are
retained and distinguished above. Only its prose is kept here:
[design review](2026-10-15-worktree-filesystem-enforcement/design-review.md), [independent review](2026-10-15-worktree-filesystem-enforcement/independent-review.md),
[native kernel](2026-10-15-worktree-filesystem-enforcement/native-kernel.md) and [integration sources](2026-10-15-worktree-filesystem-enforcement/integration-sources.md).

All owned research boxes and guest mounts were removed; the host broker processes
ended. Final runtime inventory retained only the pre-existing BuildKit container.
No host volume, Linux lease, new image, host installation, production commit,
installed-binary change or live Yard operation was created. Native immutable flags
were cleared before deleting every fixture. Bulky source fixtures, packages and
build caches were removed after retaining evidence. Final cleanup and unchanged
checkout verification are recorded in the bundle.
