# Firecracker runtime investigation and recommendation

Saved 2026-10-19. Investigation and prototype observations are from
2026-10-16. This is a dated investigation, not a replacement for
[ARCHITECTURE.md](../ARCHITECTURE.md). No production implementation or
specification changes are authorized by this report.

## Recommendation

Continue investigating a Pinfold-owned Linux runtime using Firecracker and
its maintained jailer. Keep Apple `container` on macOS. Replace writable
host binds with private guest volumes in the proposed design. Give the CLI
and a Rust SDK one authoritative lifecycle and security contract.

The disposable prototype supports this direction, but does not establish
production readiness or preservation of every existing guarantee. Its final
run completed 42 observations: 40 passed, one exposed a privilege-policy
mismatch, and one failed to establish control-channel peer rejection.

The strongest reason to prefer this design over Kata is control of the
privilege, secret-delivery and ownership boundaries. It is not a claim that
writing an execution service is cheap. Pinfold would own guest bootstrap,
interactive execution, image preparation, accounting and recovery. These
are substantial maintenance responsibilities.

Do not migrate production yet. First resolve the two prototype gaps,
qualify the privileged launcher, and demonstrate the remaining lifecycle,
storage and resource guarantees on both platforms. Useful Rust client work
can proceed independently against the existing backend contract.

The source and non-sensitive evidence are preserved in
[the disposable prototype bundle](2026-10-19-firecracker-runtime-prototype.tar.gz).
It contains the Rust guest, Python host fixtures, dependency lockfile,
artifact manifest, final observations, boot logs, selected earlier failures
and cleanup evidence. It contains no VM images or real credentials.

## Scope and evidence

The investigation started with the difference between Linux's rootless
Podman shared-kernel isolation and Apple's VM-per-box implementation. It
then compared QEMU, Firecracker, existing execution runtimes, storage
models, and Rust integration with Switchyard.

The initial source comparison reviewed Pinfold at `f20492b`, including its
37 guarantee rows. The Switchyard review used commit
`0b8b16622ee6cea1455511e9ae0268d0db869696`. The principal runtime versions
were Firecracker 1.17.0, Kata 4.2.0, containerd 2.2.0 and Apple container
1.5.0. These identify the investigated versions; this report does not claim
they are the latest releases on its save date.

There are three different kinds of evidence:

- Source and documentation review established component capabilities,
  deployment obligations and likely integration conflicts.
- A host-only Git/rsync fixture established basic transfer feasibility. It
  did not establish a safe host import boundary.
- A real nested-KVM experiment established the bounded Linux observations
  described below. It did not exercise Apple's volume implementation,
  Kata, QEMU, or Pinfold's production proxy.

Same security guarantees do not require the same VM monitor. Even QEMU
uses different host hypervisors on macOS and Linux. Conversely, using the
same monitor does not establish identical confinement or filesystem behavior.

## Why the recommendation changed from QEMU

QEMU initially looked attractive because the original requirement included
live writable host directories. Firecracker's reviewed stable API has
block devices and vsock but no native directory-sharing device. QEMU has
maintained filesystem implementations, including 9p, and a common VM
management interface. That advantage matters when preserving live sharing.

The move to private guest filesystems removes that particular reason to
prefer QEMU. Firecracker's block storage and narrow device model fit the
revised design. Retaining Apple's working backend also avoids replacing
its execution, terminal and VM-management integration.

| Responsibility | QEMU on both platforms | Apple plus Firecracker |
|---|---|---|
| macOS backend | Replace the working backend | Retain it |
| Live host directories | Existing filesystem device options; semantics still need proof | Apple supports them; Linux requires an additional server/client if retained |
| Private guest filesystems | Supported through block storage | Natural fit on both backends |
| Linux guest exec and TTY | New integration | New integration |
| Host confinement | Platform-specific manager work remains | Apple integration plus Linux jailer and launcher |
| Images and builds | Builder and VM-root preparation still required | Same work on Linux |
| Shared implementation | More VM management code can be common | Smaller migration, different backend implementations |

QEMU is still a credible fallback. Its security policy covers supported
hardware-virtualized configurations, but management software must supply
host confinement. Its guest agent's `guest-exec` captures output for later
retrieval; it does not by itself satisfy Pinfold's interactive execution
contract. QEMU 9p ownership and cache modes also need qualification before
claiming live-sharing equivalence. See [QEMU security][qemu-security],
[guest execution][qga], [filesystem options][qemu-options] and
[Linux 9p semantics][linux-9p].

Other cross-platform options were considered:

- **libkrun/krunvm:** process-oriented VMs and OCI integration, but the
  reviewed security guidance places host filesystem confinement on the
  caller. Transparent outbound socket facilities also mean that no NIC
  alone is insufficient to establish no network. [Upstream][libkrun].
- **smolvm:** more execution and image machinery already supplied, but its
  policy and trusted host process need a separate Pinfold-specific audit.
  [Security model][smolvm].
- **Docker Sandboxes:** a broader agent product with its own lifecycle and
  policy. Adopting it would be a product integration, not a small runtime
  replacement. [Documentation][docker-sandboxes].
- **Lima:** useful VM integration, but its host conveniences and sharing
  defaults are not a Pinfold confinement contract. [Threat model][lima].

None was demonstrated to preserve Pinfold's guarantees during this work.

## Shared files versus private volumes

Existing components can supply a Firecracker filesystem bridge, but they
do not remove Pinfold's security and semantic integration work.

| Component | Plausible integration | Unresolved cost |
|---|---|---|
| diod and Linux 9p client | Stream transport bridged through vsock | Confinement, ownership, caches, locks and protected paths; no automatic host/guest inotify equivalence |
| SSHFS and OpenSSH SFTP | SSHFS vsock support; SFTP over the stream | Remote notification and locking differences; starting directory is not confinement |
| NFS-Ganesha | Additional transport and export machinery | Excessive export and privilege complexity for this use case |

These were source-level candidates, not integrated runtime proofs. The
reviewed SSHFS project also warned about limited maintenance capacity.
See [diod][diod], [SSHFS][sshfs] and [SFTP server][sftp].

Private volumes avoid cross-boundary filesystem semantics during normal
agent work. Builds, watchers, locks, repository metadata and home state live
inside one Linux filesystem. A consumer can choose how to populate and
export it. Pinfold core does not need to implement repository synchronization.

For independent work, create an independent repository inside the volume.
Do not copy a linked-worktree pointer or retain host Git object alternates,
shared mutable metadata, or hardlink dependencies. This removes the shared
worktree's authority relationship; it does not make subsequent host imports
safe automatically.

The host-only Git/rsync experiment reproduced staged and unstaged changes
separately, binary edits, untracked files, symlinks and executable bits.
It did not copy host Git configuration or create an object-store dependency.
Changes in the copied repository left the source unchanged. That fixture
was removed. Conflicted indexes, submodules, ignored files, aliases and
malicious imports were not comprehensively qualified.

| Consumer workflow | Maintenance and safety implications |
|---|---|
| Explicit transfer and review | Capture starting state, export a candidate, detect host divergence, validate and apply selected changes |
| Continuous synchronization | All of the above, plus ongoing reconciliation, watcher behavior, conflicts, deletions and concurrent host edits |

Prefer explicit transfer for the first Pi consumer. Git bundles carry
history; they do not carry dirty index/worktree state. Rsync can copy selected
files; two opposing rsync runs do not implement safe two-way reconciliation.
Unison is a candidate for a later consumer synchronization feature, subject
to transport and safety validation. [Rsync][rsync], [Unison][unison].

Any host-side importer must protect host Git metadata, hooks, configuration
and editor authority. It must validate symlinks, case aliases, ancestor
replacement and deletions. Automatic synchronization grants automatic host
write authority regardless of the VM boundary. Do not host-mount a
guest-mutated filesystem merely to inspect it; access it through a temporary
box and a controlled transfer channel.

## Kata plus Firecracker versus a Pinfold runtime

Kata offers real reuse. Its agent protocol covers process creation,
streamed stdio, terminal resizing, statistics and OOM notifications. Its
documented Firecracker setup also brings containerd, guest assets and
devmapper thin-pool configuration. Pinfold would still own policy, owner
loss, generations, persistent-volume admission and consumer workflows.

| Area | Kata plus Firecracker | Pinfold runtime plus jailer |
|---|---|---|
| Execution | Existing process, stream, PTY and resource APIs | New trusted guest service and host transport |
| Images | OCI integration; documented devmapper path | Maintained builder plus immutable root preparation |
| Installation | Larger runtime/storage stack | Smaller intended stack, but new privileged launcher |
| Updates | Coordinate Kata, containerd, guest and VMM compatibility | Track VMM/kernel/tooling and maintain owned runtime code |
| Security adaptation | Several mismatches with Pinfold's contract | Direct control of policy, with full proof responsibility |
| Main maintenance risk | Pinfold-specific adaptations around an existing runtime | Underestimating execution and recovery machinery |

The reviewed Kata 4.2.0 Go and Rust Firecracker drivers pass
`--uid 0 --gid 0` to the jailer. In the normal rootful deployment this
retains host UID 0 in the jailed VMM. Confinement still exists; this is not
an escape finding. It does not provide the dedicated unprivileged VMM
recommended by Firecracker's production guidance. Kata's rootless-VMM
terminology must not be read as proving that the entire shim/runtime is
rootless. [Go driver][kata-go], [Rust driver][kata-rust],
[Firecracker production guidance][fc-production], [Kata rootless scope][kata-rootless].

Kata also has a long-lived privileged guest management agent outside the
workload container. Keeping the workload unprivileged would not preserve an
unqualified promise that nothing long-lived in the guest is UID 0. Using
`kata-agent` alone is not a shortcut around this integration: its internal
protocol is not a stable standalone public API. [Architecture][kata-architecture],
[agent protocol][kata-agent].

Secret handling is another concrete mismatch. Normal containerd creation
serializes the OCI specification and persists it in metadata. Putting
secrets in `process.env` conflicts with Pinfold's no-secret-values-in-files
contract. Kata would need a separate memory-only path. This was a source
finding, not a live leakage experiment. [Serialization][containerd-spec],
[metadata persistence][containerd-metadata].

Kata's `internetworking_model=none` does not mean that there is no NIC; the
reviewed configuration describes a TAP without a veth. Its Firecracker
template also defaults to disabling guest seccomp. Both need deliberate
configuration and verification. Rust Firecracker support exists for x86-64
and arm64, but packaged deployment availability is narrower than source
support. [Configuration][kata-config], [deployment values][kata-deploy].

Other reusable Firecracker runtimes were considered:

- **firecracker-containerd:** relevant VM, guest execution and block-device
  integration, but specialized containerd and storage configuration add
  operational burden. [Project][fc-containerd].
- **E2B Runtime/envd:** capable process, PTY and filesystem APIs within a
  much broader service/orchestration stack. [Project][e2b].
- **Matchlock:** close functional fit for local isolated agents, but the
  reviewed project was experimental and needs its own storage/policy audit.
  [Project][matchlock].

For Pinfold's narrow contract, the owned-runtime prototype is the better
next investigation. Kata remains worth reconsidering if production-quality
execution machinery proves too expensive and its policy adaptations can be
made upstream-compatible. No Kata runtime experiment was performed.

## Proposed architecture

### Objects and authority

| Object | Meaning and lifetime |
|---|---|
| Image | Immutable toolchain identity, reusable by many boxes |
| Volume | Persistent guest filesystem with a stable identity, independent of host checkout paths |
| Box generation | One isolated runtime instance; replaced after teardown or recovery |
| Workspace | Consumer interpretation of a volume, usually a repository and home |

Core owns images, volumes, runtime policy, proxy, admission, generations and
cleanup. Platform volume implementations expose the same abstraction; this
does not imply a portable disk format or concurrent cross-platform attachment.
Pi, Pi Durable and Switchyard own repositories, workflow state,
checkpoints and transfers. This separation was agreed during the discussion.

A first Pi workspace can use one volume with `/work/project` and
`/work/home`. Independent simultaneous tasks get separate volumes; multiple
commands may run in one box. A volume is initially exclusive to one box,
not concurrently mounted writable in multiple VMs.

### Storage API and lifetimes

Retain the box and image interfaces and add small volume create, list,
inspect and remove operations. The exact syntax remains a proposal.
Attachments name stable volume IDs and guest destinations. Destinations
must not mask trusted runtime files. Ordinary down/prune operations retain
persistent volumes; explicit removal refuses an attached volume.

Automatic creation belongs in core as an explicit launch request. Creation
and retention are separate choices:

| Request | Outcome after confirmed box teardown |
|---|---|
| Attach an existing volume | Retain it |
| Create a persistent volume during launch | Report its ID and retain it |
| Create a box-lifetime volume during launch | Report its ID and delete after confirmed teardown |

The Pi consumer can default to creating `/work` when none is selected.
Core should still permit a box with no volume declarations, preserving the
principle that the box spec names everything it receives.

“Exit” means box teardown, not an individual command finishing. Owner death
does not permit early deletion. Automatic deletion applies only to volumes
created for that generation. Pi Durable uses persistent volumes across box
replacement; job-lifetime disposal is its own explicit final operation.

Capacity must be explicit. A sparse filesystem's capacity is not reserved
host space or complete host disk containment. Initial scope excludes
resizing, snapshots, synchronization and automatic host mounts. Consumers
can read/write through execution streams, including by starting a temporary
box for an otherwise offline volume.

### Runtime and privilege boundary

Keep Apple's execution, terminal and VM integration, mapping volumes to
its named ext4 storage. Apple 1.5.0 documents this capability; exclusive
ownership and crash behavior under Pinfold still require real validation.
Do not use global volume pruning for Pinfold's lifecycle. [Apple volumes][apple-volumes].

On Linux, use a pinned kernel, immutable runtime/toolchain roots, writable
ext4 volumes, no NIC, and narrow vsock channels. The Firecracker management
socket remains host-only. Guest-initiated egress must reach only the box's
own policy proxy; host-initiated execution is a separate authority.
No-NIC configuration alone does not prove that every vsock service is safe.
[Firecracker API][fc-api], [vsock][fc-vsock].

| Component | Proposed authority |
|---|---|
| Owner/proxy | Normal host user; credentials, policy and lifecycle |
| Launcher | Privileged fixed-function setup and cleanup; validates admitted resource identities |
| Firecracker | Dedicated unprivileged identity inside jailer confinement |
| Guest bootstrap | Mounts and resource setup, then permanent privilege drop |
| Guest execution service | Unprivileged commands, streaming, terminals, status and reaping |

Firecracker can run unprivileged with suitable KVM access. The maintained
jailer normally needs privileged setup. Removing the NIC avoids TAP setup
but does not eliminate that privilege. This is a change from rootless
Podman installation, including privileged operations on each launch rather
than merely a one-time installer permission.

The jailer assumes trusted arguments. A public privileged launcher cannot
accept arbitrary caller paths, executables, UID choices or jail options.
Fixed binaries, root-protected state, constrained IDs and narrowly admitted
resources must form its API. That public launcher was not implemented by
the prototype. [Jailer contract][fc-jailer].

Trusted bootstrap, policy and the execution service must remain independent
of caller images, writable workspaces, profiles and Pi extensions. Guest
syscall filtering is distinct from Firecracker's host-process seccomp.

### Images and profiles

Retain OCI/Containerfile toolchains as the proposed user-facing format.
Apple retains its builder; BuildKit is a candidate maintained Linux builder.
Pinfold still needs safe OCI-to-guest-root preparation, immutable artifact
identity, cache ownership and removal rules. [BuildKit][buildkit].

Profile exports become immutable snapshots selected at launch, not live
host shares. Seeds can retain their missing-file initialization semantics.
Bind both toolchain identity and runtime/kernel identity before readiness.
Never place secret values in kernel arguments, VM configuration, image
layers or persisted execution requests.

Existing proxy-held credentials remain host-only. Explicitly supplied guest
environment secrets need a separate transient execution path. Delivering a
secret to a guest process cannot stop that process from writing it itself;
the no-persistence obligation applies to Pinfold's machinery.

### Ownership and recovery

A missing owner PID is not proof that a volume is unused. The prototype's
final Linux design uses a durable, root-protected generation record and a
nondelegated cgroup. A short flock serializes state transitions. The launcher
enters that cgroup before forking, so every potential surviving VMM child is
already represented. A populated recorded cgroup refuses reattachment.

Recovery checks the expected generation, kills the group, observes
`populated=0`, fsyncs the backing file, and clears ownership. Real
multi-volume admission, identity aliases, root reboot reconciliation and
unauthenticated caller handling need further design and adversarial tests.

The writable disk explicitly uses Firecracker `cache_type=Writeback`.
This advertises guest flush support and performs backing-file `fsync`.
The default `Unsafe` does not advertise flush. This setting is necessary
to define persistent-storage behavior; it does not prove hardware or host
power-loss durability. [Block caching][fc-caching].

## Pi and Pi Durable

Keep `pinfold pi` as a supported bundled reference consumer initially.
It owns workspace selection, population, Pi configuration, terminal use
and export/review behavior. A separate package can follow if dependencies
or release schedules justify it; package separation is not a security
boundary by itself.

Pi Durable remains experimental. Its model calls, immutable job definition
and SQLite checkpoints stay on the host. Recovery should:

1. Acquire its checkpoint-writer lock.
2. Read the previous box generation and retained volume ID.
3. Stop and confirm removal of that generation.
4. Launch a fresh generation with the same volume.
5. Report interrupted shell work without automatically replaying it.

Core does not decide replayability. Persistent files do not make commands
transactional or exactly-once. Automatically reverting a volume snapshot
could make filesystem state disagree with checkpoints and is outside the
initial design. Both consumers use the same public enforcement path.

## Rust SDK and Switchyard integration

Switchyard already had a Rust/Tokio Pinfold client at the reviewed commit.
It duplicated spec types, subprocess handling, readiness and teardown.
Unlike the Pi Durable example, it already preserved startup detail and
continued draining lifecycle output. Consolidate those working behaviors
rather than replacing them with a thinner but less reliable wrapper.

Specific source findings were:

- Missing-box detection parsed human stderr that guest output can imitate.
- OOM observation collapsed unsupported, absent and failed outcomes into
  `None`.
- Runtime identification parsed `doctor` prose and recognized Podman and
  Apple only.
- Process creation inherited Yard's execution-reservation descriptor.
  A replacement must preserve this ownership mechanism deliberately.

These are source findings at the pinned commit, not current reproduced
Switchyard defects. [Reviewed client][yard-client].

The proposed initial SDK consists of two small crates:

```text
pinfold-protocol   request, event, identity, error and statistics types
pinfold-client     typed asynchronous client over the process interface
```

Provide owned handles, explicit awaited shutdown, streaming guest stdio,
continuous lifecycle events, bounded diagnostics, volume requests,
structured runtime identity, explicit statistics availability, controlled
per-child environment/descriptor inheritance and compatibility checks.
Dropping a Rust handle alone must not be presented as confirmed teardown.

Two changes require binary-side support:

1. Expected-generation targeting for execution and teardown. A stale handle
   must not affect a new box using the same name. A client-side lookup
   cannot close the check/use race.
2. Separate execution metadata. Guest exit codes and streams must remain
   distinguishable from launch/transport failures without parsing
   guest-controlled text.

Keep enforcement in the executable initially. Existing core ownership uses
process-global stdin, signals and environment-derived credentials. Directly
embedding that implementation in Switchyard would require a larger lifecycle
redesign. A typed subprocess client gives tighter integration without
duplicating policy or moving privileged operations into Yard.

Switchyard's reviewed workflow checks the worker inside the box, tears it
down, then imports from a host-visible clone. A private volume removes that
last assumption. Yard must own initial population, candidate/proof export
and host-side acceptance, exporting before teardown or through a temporary
inspection box afterward. Core owns isolated storage and execution; Yard
owns Git interpretation, proof validation and canonical writes.
[Reviewed handoff][yard-handoff].

## Changes required in the specification

This is a public storage and deployment change, not merely a backend swap.
The agreed target removes writable host binds rather than supporting two
writable storage models. Existing `mounts` must not silently become copies.
Caller-visible breaks must be named in release notes.

| Existing contract reviewed | Proposed replacement or qualification |
|---|---|
| Linux rootless Podman, shared host kernel | Firecracker VM per box, KVM requirements and constrained privileged installation |
| Live writable host directories; sharing guarantee 10 | Private guest volumes; host visibility depends on consumer transfer policy |
| Host/guest shared locks and Linux notifications | Guest-local filesystem behavior; no promise of live host event/lock equivalence |
| Host `.git` and editor paths protected through mounts; guarantees 11/22 | Host checkout absent; independent guest `.git` writable; consumer imports protect host authority |
| Project-path home and shared home state; guarantee 13 | Home in a selected workspace volume; independent tasks use independent volumes |
| Live profile exports | Immutable per-launch snapshots |
| Cleanup of project state; guarantee 15 | Persistent volumes retained; explicit deletion; box-lifetime volumes removed only after confirmed teardown |
| Runtime-supplied exec and TTY | Retain Apple integration; implement Linux guest execution contract |
| Workload cgroup statistics; guarantee 20 | Guest workload accounting with host VMM death reported separately |
| Host-selected Linux workload seccomp; guarantee 37 | Explicitly specified guest workload policy plus separate host VMM filtering |
| Workload-visible PID 1 uses host uid:gid | Choose and specify guest UID mapping; prototype fixed UID 1000 is not proof of this existing rule |
| Nothing long-lived as UID 0; empty bounding set | Specify guest and host-manager scope explicitly; stock jailer host CapBnd is an unresolved mismatch |
| Durable fresh-generation recovery; guarantee 35 | Also retain volume identity and prove exclusive reattachment |

Preserve the policy objectives: default-deny egress, exact allowlisting and
logging, host-held credential isolation, exact environment handling,
immutable trusted runtime, unprivileged workloads, generation-safe cleanup,
whole-box cancellation, and fail-closed egress on owner loss. A common
contract needs independent evidence on both backends.

Project trust, configuration precedence, profile authority and pinned
artifact verification also remain requirements. Moving project contents
into a volume must not let guest-written configuration change host policy
without the existing trust decision.

The initial claim that either VM option could preserve every current
guarantee without a spec change was too strong. Likewise, current tests do
not prove all host export protections after arbitrary guest-kernel
compromise. Any such stronger claim needs an explicit threat model and
host-enforced boundary tests.

## Disposable prototype results

### Environment and construction

The 2026-10-16 experiment used a leased Debian 13 x86-64 VM with nested KVM,
host kernel `6.12.107+deb13-cloud-amd64`, Firecracker/jailer 1.17.0, and
upstream CI guest kernel 6.1.102. The guest kernel was a test artifact,
not a proposed production security baseline. The archive checksum matched
the checksum published with the Firecracker release.

Each VM had one vCPU, 192 MiB RAM, a 64 MiB read-only root image, a private
128 MiB ext4 data image and no network device. The guest was a static Rust
init/probe binary. Python supplied the fixed-input host launcher, an
unprivileged client and the test driver. No production crate was changed.

The kernel and image hashes, exact download URLs, version strings and
observations are in the preserved bundle. `complete=true` in `results.json`
means the experiment reached its end; it does not mean all criteria passed.

### Observations

| Area | Observed result |
|---|---|
| Guest identity | PID 1 and commands had real/effective/saved/filesystem UID/GID 1000, no supplementary groups, all five capability sets empty, NNP and seccomp enabled |
| Guest restrictions | Root write EROFS; raw disk EACCES; setuid(0), user namespace creation and remount EPERM; normal data-volume write succeeded |
| Host VMM | UID/GID 60001, no supplementary groups or active capabilities, NNP and seccomp enabled; separate mount/PID/network namespaces and verified jail-root inode |
| Guardian | UID/GID 60002, no groups, all capability sets empty and NNP enabled after setup |
| Secret delivery | Synthetic secret passed through an anonymous pipe and preconnected vsock descriptor to the unprivileged caller and designated guest command; digest matched |
| Secret errors | Malformed requests and failed exec returned fixed errors; subsequent command did not inherit secret environment |
| Secret scan | No matches in surviving trial regular files, raw ext4 files, captured logs or sampled host argv; guest command argv digest matched the expected secret-free value |
| Scanner control | A separate public canary crossing a 1 MiB read boundary was found and disappeared after deletion |
| Startup crash | Pre-spawn death was reclaimable; stopped pre-exec child remained represented and prevented another attachment |
| Owner/guardian death | VM continued writing; a new attachment was refused after both caller and guardian SIGKILL |
| Stale cleanup | Wrong generation was refused without stopping the live VM |
| Recovery | Cgroup killed and observed empty before release; same volume inode reopened with marker and counter retained; new writes succeeded |
| Runtime state | Completed requests reaped; runtime root hash unchanged; no secret inherited by next command |

The counter observed before crash was 2 and after recovery 21. This is one
fault-injection scenario, not a recovery rate or durability benchmark.
The guest-modified disk was never mounted into the host for inspection.

### Two unresolved criteria

**Host VMM bounding capabilities.** Stock jailer left
`CapBnd=000001ffffffffff`. Effective, permitted, inheritable and ambient
sets were empty and NNP was set. A nonempty bounding set is not equivalent
to currently possessing those capabilities, but an exact empty-bounding-set
criterion was not met. Do not silently weaken it. Clarify its intended
scope and prove any required extra bootstrap mechanism before adoption.

**Guest-origin control-channel rejection.** The local vsock attempt returned
timeout errno 110 rather than the explicit host-only rejection. Source
contains a host-CID check, and host-initiated execution worked, but its
rejection branch was not established by the real-runtime negative test.
A timeout is not a pass. This requires a reachable adversarial peer case
and a positive control at the same boundary.

### Findings that changed the fixture

- The kernel already mounted devtmpfs; bootstrap had to remount it with
  `nosuid` instead of attempting a second mount.
- Review caught an x32 syscall-number bypass in the initial namespace
  filter and missing child reaping. Both were corrected before the final run.
- Putting the child into a cgroup only after fork leaves a guardian-death
  race. Entering the cgroup before fork closes that identified interval.
- A forked child inherited a lifetime flock and blocked recovery before
  exec. The final fixture uses a short transition lock and cgroup occupancy.
- Textual `/proc/PID/root` reported `/` across the jailer's mount namespace.
  Comparing device/inode identity corrected the observer.
- The descriptor-receiving Python client needed an explicit socket timeout.
  This was a client-fixture correction, not a runtime security finding.

### Limits and cleanup

The guest filter is a small prototype namespace-denial filter, not the full
Pinfold workload policy. Output capture is unbounded. There is no production
TTY implementation, hardened multiuser launcher, dynamic per-box UID
allocation, arbitrary-path admission or multi-volume transaction protocol.

The secret scan is bounded: it does not trace every transient argv, deleted
file or unrelated log. Host and guest swap were disabled and core limits
were zero. The test used only random synthetic values. It does not prove
that a guest cannot intentionally persist a secret it receives.

There was no ARM/macOS runtime trial, host reboot or power-loss experiment,
malicious guest-kernel escape test, production egress-policy integration,
CPU/RAM/disk accounting validation, OCI image pipeline, ephemeral-deletion
test, or Rust host SDK implementation. A fixed disk capacity does not cover
all host storage consumption.

Before release, independent inspection found no trial cgroups, no live
Firecracker/jailer processes and no volume ownership record. Non-sensitive
evidence was copied out. `labvm release` returned success with status idle.
The repository was clean after the prototype. This save adds only archival
documentation and its evidence bundle.

## Recommended next acceptance gates

1. **Complete confinement proof.** Resolve the host CapBnd criterion and
   demonstrate rejection of guest-origin control requests. Test all
   long-lived guest processes, trusted-artifact masking and policy bypasses.
2. **Qualify the launcher.** Use a constrained resource-ID API, root-owned
   registry and per-instance identities. Test caller authorization, aliases,
   concurrent launch/delete/recover and every startup crash interval.
3. **Complete execution.** Streaming, bounded buffering, PTY/resize, exact
   environment, exit/signal status, descriptor ownership and whole-box
   cancellation must survive owner and transport failure.
4. **Integrate egress.** Keep the existing proxy policy and host credential
   injection. Prove no alternate NIC/vsock path and fail-closed owner loss.
5. **Complete storage lifecycle.** Persistent and ephemeral behavior, stale
   generation protection, multi-volume failure, failed flush, host reboot
   and interrupted cleanup need tests. State power-loss guarantees narrowly.
6. **Demonstrate Apple parity.** Apply the same admission, crash recovery,
   retention and lifecycle contract to Apple named volumes.
7. **Establish resource semantics.** Test guest CPU/memory/pid enforcement,
   OOM reporting, host VMM accounting and disk exhaustion. Host VMM thread
   counts are not guest process counts.
8. **Prepare the public migration.** Finalize protocol/client types, OCI
   preparation, consumer export flows and explicit API breaks. Rewrite
   the binding spec only with an approved design and matched evidence.

Linux CI must provide usable KVM on both x86-64 and arm64. The leased nested
KVM run establishes feasibility on that host only. It does not establish
availability or reliability on Pinfold's normal GitHub-hosted runners;
dedicated KVM-capable runners may be necessary.

Runtime prototypes and SDK work can proceed separately. The initial SDK
can improve current integrations without committing to Firecracker. No
production migration, package split, or broader platform was implemented
or approved by the investigation.

## Sources

Versioned links identify reviewed source. Unversioned links record the
upstream references used during the investigation and can change.

[qemu-security]: https://www.qemu.org/docs/master/system/security.html
[qga]: https://www.qemu.org/docs/master/interop/qemu-ga-ref.html#command-guest-exec
[qemu-options]: https://www.qemu.org/docs/master/system/qemu-manpage.html
[linux-9p]: https://www.kernel.org/doc/html/latest/filesystems/9p.html
[libkrun]: https://github.com/libkrun/libkrun#security-model
[smolvm]: https://github.com/smol-machines/smolvm/blob/main/docs/security-model.md
[docker-sandboxes]: https://docs.docker.com/ai/sandboxes/security/
[lima]: https://github.com/lima-vm/lima/blob/master/.oss-scanner/threat_model.md
[diod]: https://github.com/chaos/diod
[sshfs]: https://github.com/libfuse/sshfs
[sftp]: https://man.openbsd.org/sftp-server.8
[rsync]: https://rsync.samba.org/
[unison]: https://github.com/bcpierce00/unison/releases/tag/v2.54.0
[kata-go]: https://github.com/kata-containers/kata-containers/blob/4.2.0/src/runtime/virtcontainers/fc.go#L347
[kata-rust]: https://github.com/kata-containers/kata-containers/blob/4.2.0/src/runtime-rs/crates/hypervisor/src/firecracker/inner.rs#L69
[kata-agent]: https://github.com/kata-containers/kata-containers/blob/4.2.0/src/libs/protocols/protos/agent.proto
[kata-architecture]: https://github.com/kata-containers/kata-containers/tree/4.2.0/docs/design/architecture
[kata-rootless]: https://kata-containers.github.io/kata-containers/how-to/how-to-run-rootless-vmm/
[kata-config]: https://github.com/kata-containers/kata-containers/blob/4.2.0/src/runtime/config/configuration-fc.toml.in
[kata-deploy]: https://github.com/kata-containers/kata-containers/blob/4.2.0/tools/packaging/kata-deploy/helm-chart/kata-deploy/values.yaml
[containerd-spec]: https://github.com/containerd/containerd/blob/v2.2.0/client/container_opts.go#L292
[containerd-metadata]: https://github.com/containerd/containerd/blob/v2.2.0/core/metadata/containers.go#L371
[fc-api]: https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/src/firecracker/swagger/firecracker.yaml
[fc-production]: https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/docs/prod-host-setup.md#jailer-configuration
[fc-jailer]: https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/docs/jailer.md
[fc-vsock]: https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/docs/vsock.md
[fc-caching]: https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/docs/api_requests/block-caching.md
[fc-containerd]: https://github.com/firecracker-microvm/firecracker-containerd
[e2b]: https://github.com/e2b-dev/runtime
[matchlock]: https://github.com/jingkaihe/matchlock
[apple-volumes]: https://github.com/apple/container/blob/1.5.0/docs/volumes.md
[buildkit]: https://github.com/moby/buildkit
[yard-client]: https://github.com/adamaltmejd/switchyard/blob/0b8b16622ee6cea1455511e9ae0268d0db869696/crates/yard/src/box.rs
[yard-handoff]: https://github.com/adamaltmejd/switchyard/blob/0b8b16622ee6cea1455511e9ae0268d0db869696/crates/yard/src/jobs/supervise.rs#L1688
