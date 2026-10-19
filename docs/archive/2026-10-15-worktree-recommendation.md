Superseded by [2026-10-15-native-worktree-protection.md](2026-10-15-native-worktree-protection.md).

# Pinfold linked worktrees: recommendation for approval

2026-10-15. Research and disposable experiments only. No production implementation,
binding-contract change, installed-binary change or worktree enablement.
Evidence and runnable probes: [research bundle](2026-10-15-worktree-research/README.md).

## 1. Recommended direction

Pursue **direct editing of linked worktrees on case-sensitive storage**, using
existing runtime mounts with complete protection planning. On this Mac, put the
working files on a dedicated case-sensitive APFS volume; existing common Git
metadata/history can stay on the current volume, exported only read-only.
Move working files once, preserve the existing index and dirty state, and reuse
that worktree across launches. Keep Pi interactive and Switchyard the caller.

This is an explicit compromise: it changes the worktree's storage location and
case semantics. It does **not** satisfy the preferred requirement to edit the
existing case-insensitive location unchanged. No reviewed current component
established all requested properties there. Do not enable worktrees merely after
a filesystem check or add only `--read-only-path`.

The preferred direction preserves full root/source editing, immediate ordinary
host/guest visibility, host-owned staging/commits, the Mac VM boundary, current
Linux runtime and no recurring checkout/history copy. It requires file-aware
admission, effective hook discovery, protection of every writable ancestor,
structural rejection of alternate writable views, and explicit trusted-host
conditions. These are not implemented as a complete feature in this research.
Retain the current refusal until a separately approved implementation passes
its gates.

Why this compromise: it uses mechanisms already present in Apple Container and
Podman. It avoids a new filesystem server, runtime replacement or synchronization
subsystem. The one-time relocation is measurable. By comparison, isolated copies
need ongoing conflict and recovery semantics and lose live original-host edits.
The [independent reviewer](2026-10-15-worktree-research/independent-review.md)
supports this direction for approval, not immediate enablement.

If existing placement is non-negotiable, keep refusal. The fallback is a persistent
isolated task copy with reviewed, quiescent application **and explicit recovery**.
A private `.git`, stock `git apply`, or a live OverlayFS lower does not supply that
workflow by itself. Revisit direct editing on the existing volume when an available
Mac exporter offers authorization tied to the actual opened object and namespace
operations, with alias/race evidence at its real guest boundary.

## 2. Serious alternatives

| Approach | Actual host writer / state | Decision |
|---|---|---|
| Current Apple runtime pathname masks on the existing volume | Virtualization.framework writable share; guest masks protect selected names | Reject unchanged. The historical `.GIT` and hardlink bypasses remain relevant: installed and latest stable Container are both 1.5.0. Seatbelt around the VM owner already failed at the VM boundary. |
| Case-sensitive placement plus complete mount planning | Same framework share on Mac; rootless Podman mount namespace on Linux | Recommend as constrained direction. Pointer masks and all-ancestor mounts passed selected real-runtime cases. Admission is still a required implementation. |
| Stock Matchlock VFS | Native Go service performs host `OpenFile`, rename and unlink; lexical routing/hooks | Reject stock protection. Its actual unmodified provider blocked the literal gitfile but aliases changed the same host inode in a native component probe. Hooks do not expose an already-authorized descriptor. Not a whole-VM certification. |
| NVX/OpenVMM enforcing service | Host VMM FUSE service checks paths and known identities per request | Relevant architecture, not a Mac component to adopt. Linux/Windows only; nested RO mappings can still rely on guest binds. Unobserved hardlinks remain an untested concern. Port/integration cost is substantial. |
| Docker Sandboxes / devsandbox direct worktrees | Runtime host share; parent metadata omitted or mounted separately | No established pointer protection. Hiding metadata does not protect the writable host gitfile. Current Docker docs describe passthrough, not conflict-managed synchronization. |
| Native sandbox-runtime / agent-sandbox.nix / Safehouse | Host kernel confines native child processes | Changes the Mac VM/Linux-toolchain contract. Defaults intentionally permit some or all Git writes. Not evidence for protected writable VM shares. |
| Persistent private working files | Existing file-copy/clone operation once; only the private files writable to the guest | Original isolation demonstrated through real Pinfold. Original host edits are not live. Generic application permits protected editor edits, can lose a concurrent host save and is not crash-atomic. |
| Overlay / git-lazy-mount | Stable lower plus private changes; or Linux FUSE projection of a Git tree | Live OverlayFS lowers are unsupported by the kernel contract. Stable snapshots reintroduce preparation. git-lazy-mount is a fresh Linux projection with private writable Git metadata, not transfer of an arbitrary existing dirty/index state. |

Pinned primary-source details, privileges and dependencies are in
[filesystem components](2026-10-15-worktree-research/filesystem-components.md)
and [other alternatives](2026-10-15-worktree-research/alternatives.md).
Matchlock requires a new trusted service/guest FUSE integration or a different
runtime; Linux Firecracker requires KVM/provisioning. Native tools need Seatbelt
or bubblewrap and alter the containment boundary. NVX lacks Mac support.
git-lazy-mount adds FUSE, a private partial clone and durable overlay semantics.
The recommended direction adds no runtime dependency or privileged service;
a temporary user-attached APFS image demonstrated storage feasibility. It does
not justify automatically provisioning a permanent volume.

Primary anchors: [Apple Container 1.5.0](https://github.com/apple/container/releases/tag/1.5.0),
[Apple APFS formats](https://support.apple.com/en-ie/guide/disk-utility/dsku19ed921c/mac),
[OverlayFS underlying-filesystem restriction](https://www.kernel.org/doc/html/latest/filesystems/overlayfs.html),
[Docker Git workflows](https://docs.docker.com/ai/sandboxes/workflows/git/).
No speed or security claim is borrowed from vendor benchmarks.

## 3. Reproducible evidence and costs

Baseline: primary branch `codex/issue-89-bounded-listing`, HEAD
`198600b1751d889d354ed3657d8f839e8436e469`, **plus ten pre-existing dirty tracked
files**, captured in the bundle's `baseline.patch`. The 285-file initial manifest
and final comparison show no original file changed. A clean HEAD alone is not
the tested source. The isolated release build took 38.55 s with existing toolchain
and registry caches; it was not a cold toolchain build. Binary SHA-256:
`f87a01813fdbbdca12bd418b047b53b275ca99f3835e8f1d4177b92f8560bf26`.
No experimental production-source modification was needed for the mechanisms.

Mac: arm64 macOS 27.0.1 (26A434), Apple Container 1.5.0, Git 2.56.0,
Rust/Cargo 1.99.0. Native fixtures ran without network, with capabilities dropped,
read-only rootfs and host uid/gid. Pinfold probes used the exact baseline build.
Linux: disposable Debian 13.7 x86-64, ext4, kernel 6.12.107+deb13-cloud-amd64,
rootless Podman 6.1.4 with systemd cgroups, Git 2.47.3. Upstream Podman source
`464c15fec06f0074c916f9eff87c4806869c2833`; binary hash and image IDs are retained.
Linux evidence is native runtime evidence, not a completed Pinfold worktree feature.

| Observation | Evidence tier and result |
|---|---|
| Complete worktree edits and Git reads | Real Apple VM and Linux Podman: root edits/new directories, status/log, staged/dirty state and read-only common metadata worked. |
| Pointer operations | Case-sensitive Mac/Linux: literal write, unlink, rename, replacement, symlink writes and newly created guest hardlinks failed; host fingerprints stayed unchanged. `.GIT` was a distinct file. |
| Existing aliases / overlapping exports | Matchlock actual native provider allowed case/hardlink/symlink alternatives. Native duplicate writable exports changed protected host bytes. A host-created hardlink introduced after readiness bypassed both Mac/Linux pointer masks. These are failures, not allowed layouts. |
| Nested hooks | Leaf-only protection allowed ancestor rename and replacement; a real host commit executed a harmless `guest-hook` marker. Binding the writable ancestor blocked rename and preserved the trusted hook/marker. Both levels of `deep/middle/hooks` were subsequently pinned; both renames failed and sibling edits succeeded. |
| Current core admission gaps | Baseline-built `pinfold box up` admitted the leaf-only ancestor arrangement and a writable alias to an empty protected directory. Guest changes reached host paths. Explicit ancestor binds preserved the host value. Empty overlap requires structural admission, not only a regular-file identity scan. |
| Normal host interaction | Mac/Linux host atomic source saves were visible; staging replaced the host index and guest reads showed the staged diff without modifying that index. Host commits advanced visible HEAD. Two Mac boxes exchanged ordinary source writes. This is not conflict-free concurrent editing. |
| Normalization | Composed/decomposed `hóoks` and `K`/Kelvin-sign directory accesses failed with EROFS and unchanged host bytes. No blanket ASCII restriction follows from these limited negative results. |
| One-time migration | A real dirty linked worktree moved from default APFS to the case-sensitive image; `git worktree repair` updated its backlink. Index hash, working bytes, staged/dirty/untracked/ignored status and history-object list matched. Common metadata remained on the original device. Native Git reads and root edits then worked. Tiny-fixture timing is not a migration benchmark. |
| Isolated copy | Real Pinfold: APFS clone created independent inodes, including a copied pre-existing gitfile hardlink. Guest modifications changed only private files; original metadata/index/config stayed intact. New original-host files were invisible. Fixed ordinary-file application preserved the original index and refused an existing content conflict. |
| Application boundaries | Git apply rejected a symlink parent and replaced a hardlinked target without modifying its other link. It accepted an editor-config change: it is not Pinfold's policy filter. The invalid-gitfile `.GIT` case is inconclusive. |
| Application race/crash | Linux event-triggered tests: a host atomic save after Git read the target was overwritten despite exit 0. Killing an application of 10,000 files at its first mutation left 19 files missing; retry failed. No sleeps, mocks or synthetic Git implementation. |

The known external hook-alias fix remains absent from the primary checkout:
`pi/git.rs` is still blob `cc9a8297bd2ab7dc0b37a8d6df941cef5ef01687`, with the
lexical outside-root return. The October 14 measured-build report describes
B1 in a separate candidate. This research did not land it. The new ancestor
and empty-overlap observations are additional protection concerns, not fixes.

Performance used both the actual 95-file Pinfold source subset (1.29 MB; excludes
archive/build outputs) and a reproducible file-heavy working-tree surrogate:
100,002 files, 476,708,872 logical bytes. It is a synthetic filesystem workload,
not a claim about a specific monorepo's builds. Three samples per main phase,
cached images, uncontrolled OS caches. First trials and ranges are retained.
No reboot-cold, model-response, cold-download or complete admission-planner
benchmark was performed. A short independent fixture run overlapped part of
the host copy series; treat these as approximate costs, not an SLA.

| Phase | Mac file-heavy observation | Linux ext4 observation |
|---|---|---|
| One-time private file preparation | APFS clone median 8.38 s (8.34–8.83) | `cp --reflink=auto` 4.47 s first; 2.83/2.76 s later; forced reflink unsupported |
| One-time case-volume relocation copy | Median 32.96 s (31.80–35.19), no Git history copied | Not required for the tested case-sensitive ext4 layout |
| Subsequent private-copy launch | Pinfold ready median 0.93 s (0.87–1.29) | Native start 0.48 s first; 0.19/0.21 s later |
| Subsequent case-volume launch | Native Container started median 0.55 s (0.51–0.61); excludes future Pinfold admission | Native protection tested separately; no integrated candidate timing |
| Root edit + complete enumeration | Pinfold private copy median 0.35 s; native case-volume 0.31 s | 1.25 s first; 0.46/0.49 s later |
| Teardown | Pinfold private copy 0.18 s; native case-volume 0.12 s medians | Zero-grace native removal 3.68 s first; 0.24/0.25 s later |
| Ten-file change application in the large tree | Base hashing 12.41 s; diff discovery 20.23 s; check 0.039 s; apply 0.033 s | Correctness/race/crash tested separately; no corresponding large-tree import timing |

Native-start and Pinfold-ready rows measure different boundaries; subtracting
them is not a worktree speedup estimate. Native benchmark pointers were synthetic;
repository semantics were tested separately. No copies occur in the repeated
launch loop. The small source tree's preparation was only 0.015/0.030 s for
clone/cross-volume copy, which would badly understate the file-heavy cost.

In an otherwise isolated 2 GiB APFS image, 476.7 MB logical source files consumed
516.1 MB of free space including metadata. An APFS clone added 28.7 MB; replacing
its 64 MiB file added another 69.6 MB. Source bytes remained independent and
free space returned to its starting value after removal. These are synced
volume-free-space deltas. Ordinary `st_blocks` double-counts shared extents.
On the Linux ext4 fixture each full copy reported 476.7 MB allocated. Existing
history was never recopied on launch. Private-copy preparation and source
snapshot consistency still need a quiescent preparation interval.

## 4. Conditions, uncertainty and remaining gates

The proposed boundary assumes trusted host processes keep **protected
relationships** stable from before discovery until the relevant boxes stop:
export roots, gitfile placement, protected directories and their ancestors,
effective hooks/config destinations, and aliases crossing from protected to
writable files. A host can still edit/save ordinary source, stage and commit.
Stop relevant boxes before worktree move/remove/repair, changing hooks/config
locations, moving protected directories, or creating aliases to protected files.
Host checkout/switch needs separate classification because it can change tracked
protected paths. An advisory Git worktree lock is not filesystem enforcement.

Two boxes are untrusted participants: every writable view must obey the same
plan. A second broader export is not a trusted-host exception. Ordinary source
writes retain normal last-writer semantics, including possible overwrites by
agents/editors; neither this recommendation nor current direct sharing merges
concurrent edits. The new race observation specifically rejects claiming an
automatic importer preserves unrestricted concurrent host saves.

The remaining material blocker is **integrated admission**, not evidence that
one literal mount works. No implemented candidate currently refuses malformed
worktree graphs, unsafe extra exports or file aliases under the proposed rules.
Discovery-to-use races were not made atomic. The live-host-alias failures show
why a final stat or startup scan cannot be presented as continuous enforcement.

First support should explicitly refuse unsupported submodules, symlinked
administrative/protected paths, unresolved/dangling layouts, unsupported object
alternates and additional writable exports that reopen a protected location.
These are proposed refusal rules, not validated production behavior. File
identity, common/per-worktree config, `commondir`, effective hook paths (including
external aliases and nonexistent leaves), and every relevant ancestor belong
in the admission plan. Preserve the existing trust and authority-root checks.

Not established: a complete Pinfold/Pi worktree implementation, Linux arm64,
all malformed-layout or discovery-race cases, every filesystem equivalence,
multiple different worktrees sharing metadata concurrently, ACL/xattr/LFS or
submodule migration, host checkout/switch, storage failure/power-loss recovery,
model interaction, or a general safe importer. The retained experiments are
bounded evidence, not a complete security proof. The independent review's
final consistency pass addresses the recommendation and these limits.

## 5. Smallest proposed production work

Subject to approval, implement one restricted-layout worktree plan using the
current runtimes. Keep security in the Pinfold binary. Do not add an agent-side
Git wrapper, general exporter, synchronization service, new orchestrator or
runtime-selection framework.

1. First correct the existing effective-hook alias classification and the
   reproduced nested-ancestor/empty-overlap gaps. Their existing guarantees
   already require protection; they also affect ordinary checkouts/core callers.
2. Add the minimum core representation for a read-only regular-file boundary
   inside a declared directory export. Include it in admission and runtime
   translation. On Mac the demonstrated primitive is `--read-only-path`; on
   Linux it is a read-only bind. The representation/API is a design decision,
   not a tested patch in this report.
3. In Pi, discover/validate a deliberately supported linked layout; require
   case-sensitive writable storage on Mac, export common/per-worktree metadata
   only read-only, protect the gitfile and effective host-executed config, and
   bind every writable ancestor in place. Deduplicate shared ancestors. Reject
   alternate writable routes structurally, including empty directories, while
   allowing the planner's deliberate in-place ancestor binds.
4. State the storage, maintenance and trusted-host conditions in the binding
   architecture in the same production change. The common Git history need not
   move. Migration is explicit host-owned work with preserved originals until
   verified, not an automatic launch fallback.

Extend existing G11/G22 for linked pointers, actual host markers, nested
ancestors, case/hardlink/symlink variants, empty/overlapping exports, hooks aliases,
common/per-worktree configuration and malformed/refused layouts. Extend G17
for any new box-spec representation and G10 for the supported live file behavior;
keep G33's authority separation. Use allowed controls in the same fixtures and
external expected values. A newly promised conditional worktree behavior must
be reflected in the guarantee table; do not add tests merely for implementation
shapes. Run the repository's test-audit on changed assertions, fmt, Clippy,
full Mac and both Linux architecture suites, and relevant caller/prototype gates.
Measure the resulting planner/admission costs before making launch-time claims.

No production diff is proposed for approval yet. This decision is approval of
the constrained direction and its explicit compromises. Existing worktree refusal
remains until that later change and its real gates pass.

## 6. Cleanup and preservation

Final cleanup evidence is in `cleanup.json` and `verification.json` in the bundle.
The Linux runtime had no remaining containers; its disposable lease was released
and separately confirmed idle. Mac inventory had no research containers; the
pre-existing BuildKit container and two unrelated `pinfold-e2e-31772-*` test
containers were left untouched. No Mac image was built or removed. The owned APFS
image was detached after confirming its exact mount association; synthetic
repositories, native probes, build outputs, Go caches/source clones and temporary
working trees were removed. Compact scripts, results, baseline patch/manifests,
primary-source notes and independent review remain under this dated archive.

Ten small stable Pinfold per-name lock files remain intentionally, following the
ownership contract that never unlinks them. Their owner records and sockets are
absent; exact paths are listed in `verification.json`. No live research boxes,
processes, mounts or leased resources remain. No broad prune/reset, service
change, installed Pinfold change, production commit or live Yard modification
was performed. All 285 original captured primary-checkout files and the complete
pre-existing tracked diff were verified unchanged.
