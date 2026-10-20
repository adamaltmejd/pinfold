# Pinfold worktree isolation investigation and recommendations

Consolidated 2026-10-19. Experiments were conducted on 2026-10-15; the later discussion examined SPACE, private working files, Linux VMs and whether Pinfold should continue owning sandbox orchestration. This is research and decision support. It does not change the binding architecture, enable worktrees or authorize a production migration.

## Recommendation

Evaluate an existing implementation before expanding Pinfold into a private-filesystem VM platform. Test one actual Pi or Switchyard workflow against the requirements below. Retain custom code only for demonstrated gaps, particularly local operation, network restrictions, credential isolation and caller lifecycle semantics.

The strongest candidate for preserving the original workflow is narrower: native Apple Container sharing with all eight ASCII case spellings of the linked worktree's `.git` pointer mounted read-only, genuine read-only external Git metadata, existing alias admission, and restrictions on protected ancestors. Real runtime experiments support this bounded candidate. They do not establish a complete Pinfold feature.

These recommendations address different decisions. Native sharing is the least disruptive candidate if immediate editing of the existing host folder remains essential. A private filesystem is the cleaner isolation direction if callers may choose how to import and apply work. Neither justifies building a new general filesystem service by default.

Keep the existing worktree refusal until an approved implementation and its real security tests pass. Do not adopt the earlier case-sensitive-volume or SSHFS recommendations as the final outcome of this investigation.

## Original requirements and later reconsideration

The original request required agents to edit complete existing linked worktrees on the user's default case-insensitive Mac filesystem. Root manifests and arbitrary source directories had to remain writable. Existing staged, unstaged and untracked work had to survive. Host editing should remain useful, commits should remain host/caller-owned, and checkout or history copies should not recur on every launch. Linux support mattered. Pi remained the interactive interface and Switchyard the orchestrator.

The request allowed research and disposable prototypes, but prohibited production implementation, binding-contract changes and enabling worktrees in the installed Pinfold. It also allowed explicit compromises if the preferred workflow could not be protected soundly.

The later discussion reconsidered the requirement that the agent directly write the original folder. “Start Pinfold in this folder” can remain a caller interface even if the execution environment receives a private copy. It also reconsidered whether Pinfold should own folder integration at all. Those are proposed architectural changes, not accepted replacements for the original requirements.

The final question was whether a Pinfold reduced to VM lifecycle, command execution and file transfer would duplicate existing implementations. The appropriate next decision is therefore replacement versus a small policy layer versus continued implementation, before a Linux VM migration.

## Threat model and the actual boundary

The attacker is an agent executing arbitrary commands inside a box, including after prompt injection. The relevant escalation is an agent changing host-trusted metadata so that a later ordinary host operation executes attacker-controlled code. Hooks are one route. Git configuration such as `core.fsmonitor` is another. The attacker need not persuade the guest to run Git: ordinary file writes can prepare the attack, and host Git later consumes the result.

A linked worktree has a `.git` pointer file in the working folder, with administrative and common metadata elsewhere. Protecting only the external metadata does not protect the pointer. On the tested default Mac filesystem, alternate spelling `.GIT` resolves to the same host object. A guest mask at one spelling can therefore coexist with a writable route through another spelling. Hardlinks and duplicate exports create additional routes to protected objects. Renaming a writable ancestor can replace the pathname of a protected directory without writing its original inode.

The protected set includes the pointer, per-worktree administrative directory, common directory, `commondir`, relevant configuration, hooks, object alternates and the relationships through which host Git finds them. Protection must cover namespace changes and permission changes as well as file contents.

The existing contract does not claim protection against a user explicitly running arbitrary modified build scripts, tests or package scripts on the host. It also records exclusions for scripts called by hooks from writable project content and planted nested repositories. The investigation should not turn that bounded contract into a claim that all host use of agent-edited content is safe.

Editor configuration appeared because the current contract protects designated host-executed configuration directories. `.vscode` was a fixture/example, not a proposal to build a VS Code integration. The user did not request such an integration. Any future protection policy should be justified by actual host behavior and the declared contract.

### Why a Git proxy or disabled hooks is not sufficient by itself

Removing guest Git, proxying guest Git commands, setting `GIT_DIR`, or hiding external metadata does not prevent ordinary guest writes to an exposed host pointer or configuration alias. A Git proxy can still be useful when the agent has no other writable route to host Git authority; the proxy does not establish that filesystem condition itself.

For caller-controlled host Git invocations, `git -c core.hooksPath=/dev/null ...` can disable hooks for that invocation. It does not govern later terminal/editor Git commands and does not disable other execution mechanisms such as fsmonitor. Protecting metadata remains necessary under the existing shared-folder contract. A host Git proxy also needs its own constrained operation, argument, environment and configuration policy; arbitrary forwarded commands would not be a security boundary.

Mounting `.git` read-only is directionally correct, but on the tested case-insensitive host there are eight ASCII spellings, not just `.git` and `.GIT`. The relevant case sensitivity is that of the filesystem resolving the host request, not simply the Linux guest filesystem.

## Evidence baseline

The original experimental source baseline was `/Users/adam/Code/pinfold`, branch `codex/issue-89-bounded-listing`, commit `198600b1751d889d354ed3657d8f839e8436e469`, plus ten pre-existing dirty tracked files. The retained patch and manifests are part of the baseline; clean HEAD alone is not the tested source. The initial baseline build took 38.55 seconds with existing toolchain/registry caches. Its SHA-256 was `f87a01813fdbbdca12bd418b047b53b275ca99f3835e8f1d4177b92f8560bf26`.

| Environment | Recorded versions and scope |
|---|---|
| Mac | arm64 macOS 27.0.1, build 26A434; Apple Container 1.5.0; Git 2.56.0; Rust/Cargo 1.99.0 |
| Linux | Disposable Debian 13.7 x86-64; ext4; kernel 6.12.107+deb13-cloud-amd64; rootless Podman 6.1.4 with systemd cgroups; Git 2.47.3 |
| Broker follow-up | Guest kernel 6.18.15; Go 1.27.2; SSHFS 3.7.3; libfuse 3.17.2 |
| Native sharing source | Apple Container `d265d669ecae041bf338cb3b39c4118316d138f0`; Containerization 0.47.0 at `bc994b88df46207fad7775b0eabc51947e315881` |
| SPACE source review | `b9fc395e06189460262c261b8bf34dfa5a305745`; no runtime execution |

The broker follow-up did not retain the mutable image identity for every launch. The later native-sharing bundle captured its image identity before testing. These distinctions limit exact reproduction and must not be flattened into a single fully pinned environment.

This consolidated file is saved in `/Users/adam/.codex/worktrees/7459/pinfold`, whose HEAD at writing was `a1be9213efc3585d75bead7f2c2de4073167fc7d`. Its binding architecture still states worktree refusal and Apple Container on Mac/rootless Podman on Linux. Historical findings about separate dirty candidates are not claims about all subsequent branches or releases.

## How the recommendation changed

| Stage | Conclusion | Why it is no longer the general recommendation |
|---|---|---|
| Initial mount research | Move working files once to case-sensitive APFS; keep common history where it is. | It violated the clarified requirement to use existing default Mac storage without another volume. |
| Filesystem enforcement follow-up | Put an object-aware host broker in the write path, using guest SSHFS. | It demonstrated a different enforcement boundary, but introduced substantial cost and incomplete filesystem semantics. Calling it the minimum-cost direction was premature. |
| Native service and ACL research | Prefer eight native pointer masks, existing alias admission and genuine read-only metadata for bounded layouts. | This remains the strongest direct-sharing candidate, but needs integrated discovery, admission and tests. |
| SPACE and architecture discussion | Consider private working files and caller-owned integration. | This changes the product workflow; it is not an equivalent fix for direct editing. |
| Replacement discussion | Evaluate existing implementations before building a private-filesystem platform. | No replacement winner or approved migration has been established. |

The user's performance objection was justified. The broker measurements do not demonstrate acceptable real development performance. Real workloads could be worse where they amplify metadata operations, serial requests or cache misses; CPU-heavy workloads could dilute the slowdown. Neither direction was measured as a complete workflow.

## Native sharing and ACL findings

### Bounded native pointer protection

The final experiment used all of `.git`, `.giT`, `.gIt`, `.gIT`, `.Git`, `.GiT`, `.GIt` and `.GIT` as read-only pointer masks. It used no host ACL. Through real Apple Container shares, all eight spellings denied content writes and chmod with EROFS, new hardlinks with EXDEV, and unlink, rename-away and replacement with EBUSY. Host pointer bytes and mode 0644 survived. Symlink-to-pointer chmod also failed.

Ordinary writes, chmod, hardlinks, directory creation and atomic replacement succeeded in the same fixture. Other native probes exercised root-level edits, Git reads, host staging/commits and host edits. External metadata was genuinely exported read-only. No account, helper, volume or extra filesystem protocol was required.

An initial no-ACL run reused ordinary fixture names and produced EEXIST in two positive controls. It is retained as an unsuccessful control run. The corrected run used distinct names and `set -e`; all ordinary controls passed. Protection results were consistent across both runs.

The nested-path counterexample is essential: while `deep/middle/hooks` had a protected leaf mount, renaming `deep` succeeded and renamed the host directory. The successful pointer result therefore does not justify arbitrary nested protection. Initial support should refuse layouts whose protected ancestors remain displaceable through writable exports, or implement and test an ancestor guard.

The name checks sampled three Unicode lookalikes and 27 inserted/appended ignorable-character spellings that did not resolve to `.git` on this APFS volume. This is not exhaustive normalization coverage. HFS+, network filesystems and other runtime/APFS versions were not validated.

### Host identity

Apple's native sharing uses the effective host user's permissions. The inspected public `VZSharedDirectory` API exposes a host URL and whole-share read-only flag, not a per-share host identity or per-operation authorization callback. Guest UID selection does not select the host writer identity. Guest UIDs 0, 501 and 12345 all encountered the ACL for host UID 501, and guest-created files belonged to host UID 501.

A separate host identity is a plausible conventional design. It would require privileged provisioning/launch authority or a pre-existing identity, plus ownership and permission management. An automatic installer could remove manual account setup, but not administrator authorization or helper maintenance. No separate-identity VM was installed or tested.

Sources: [Apple permission contract](https://developer.apple.com/documentation/virtualization/vzvirtiofilesystemdevice), [pinned native-share construction](https://github.com/apple/containerization/blob/bc994b88df46207fad7775b0eabc51947e315881/Sources/Containerization/VZVirtualMachineInstance.swift#L494-L513), and the retained [identity investigation](2026-10-15-native-worktree-protection/identity-source.md).

### What ACLs did and did not establish

ACLs on actual host objects denied tested data writes through case, symlink and hardlink aliases. Pointer deletion restrictions blocked unlink/replacement. DELETE-only ACLs on nested ancestors blocked renames while permitting ordinary sibling creation. Freezing the pointer still allowed host commits because the host wrote external metadata rather than the pointer.

However, ACLs denying the host user affect that user's host processes too. Owner chmod changed a protected pointer's mode despite a writesecurity deny, although bytes and ACL survived. A deliberately pre-existing writable hardlink to a non-executable protected hook allowed chmod to enable it; a host fixture commit then ran a harmless marker. That fixture bypassed Pinfold admission deliberately and is not a newly reproduced Pinfold CLI defect. It shows why existing alias admission must remain.

Installing an ACL after opening a writable descriptor did not revoke that descriptor. Any ACL design would need protection before exposure, coordination across boxes, preservation of existing ACLs, ownership of changes, crash recovery and restrictions on host inode replacement. Those costs make ACLs an optional ancestor-protection extension, not the default pointer solution.

Two immediate guest Git reads after host commits failed to find the current branch; a fresh mount read it. A later independent commit became visible on the first read at 0.062 seconds. Native metadata/name caching prevents a universal instantaneous-visibility claim.

## Filesystem broker findings

The disposable broker performed host writes itself and reused SSHFS in the guest over a forwarded Unix stream. It required no macFUSE, SSH login daemon, SSH keys, host TCP listener or source/history copy. Two VMs used one broker. Direct guest protocol requests still reached its authorization checks.

The prototype opened without truncating, inspected the selected object, retained descriptors for writes and reauthorized them. It used confined resolution, actual identities/canonical host paths, namespace and ancestor checks, and serialized guest mutations. It conservatively denied mutations of multiply linked regular files and refused guest hardlink creation. These restrictions are not a claim that link counts alone solve protection.

It denied tested case/hardlink/symlink attacks, pointer changes and nested ancestor renames while allowing ordinary source edits and host commits. Independent review found that version 1 used caller spelling for some policy paths. Version 2 derived paths from opened objects and passed alternate-case/normalization checks. The version-1 issue was source-identified, not separately exploited.

The service mutex did not serialize arbitrary host processes. Trusted host processes still had to keep protected relationships stable. Ordered late-alias and held-handle tests are evidence for particular defenses, not atomic safety under arbitrary host restructuring.

The prototype is not shippable unchanged. It refuses chmod, timestamp changes, handle truncation and hardlink creation. Its Go library's FSTAT becomes a pathname query, violating retained-handle semantics after replacement. Locking, fsync, xattrs, file watching, absolute symlinks and unlink-open behavior were not established. A Rust implementation would need fresh evidence. `russh-sftp` 3.0.1 was a source-reviewed candidate, not validated production code.

FUSE setup also required trusted transient privilege. Dropping capabilities for PID 1 did not automatically remove the stored bounding set from later raw runtime execs. A trusted wrapper demonstrated independent dropping for later commands, but this was not integrated Pinfold. Every entry point would need validation before image-controlled code runs. Filesystem transport would also need to coexist safely with the existing egress socket, fail closed on service death and have bounded session/resource ownership.

### Measured broker costs

The fixture contained 100,001 files and 476,708,864 logical bytes, including a 64 MiB file; an edit increased the file count to 100,002. Values are medians of three sequential guest samples, cached images and uncontrolled OS caches. The native comparison contained no protected files. Zero attribute/directory timeouts and direct I/O favored host visibility.

| Operation | Native share | Broker and SSHFS |
|---|---:|---:|
| Enumerate all files | 0.121 s | 0.636 s, 5.2 times |
| Read 64 MiB | 0.023 s | 0.216 s, 9.3 times |
| 100 atomic edits/replacements | 0.057 s | 0.370 s, 6.5 times |
| Git status listing fixture files | Not measured | 0.900 s |

A single second-box sample took 0.882 s to native start, 0.131 s to unpack packages, 0.095 s from mount-helper dispatch to a successful read, 0.062 s to unmount and 0.099 s to remove. These are not integrated Pinfold readiness numbers. Compilation, package installation, memory mapping, databases, cold caches and large Git histories were not benchmarked. No exact production startup or whole-workflow estimate follows.

## Other serious approaches

These are findings from the recorded October 15 sources and probes, not a fresh market survey on October 19. Rejection here means failure to establish the original Pinfold guarantee, not a claim that each tool is generally insecure.

| Approach | Mechanism and result |
|---|---|
| Case-sensitive APFS placement | Real mounts protected the tested pointer operations. A one-time relocation preserved dirty/index state and common history, but required different storage placement. Superseded for the user's default-volume requirement. |
| Seatbelt around the VM owner | Earlier real VM probe showed that it did not constrain the writable Virtualization share. It constrained the wrong writing boundary. |
| Seatbelt on the actual native writer | Native probes blocked selected paths/namespace operations; existing hardlinks and pre-opened writable handles still wrote protected bytes. Insufficient alone. |
| Immutable inode flags | Blocked new alias opens and namespace changes in native fixtures, but not an already-open writable descriptor. Host interference and crash cleanup remain. |
| Matchlock stock VFS | The actual unmodified native provider blocked literal `.git` but allowed case/hardlink/symlink alternatives to alter the protected inode. This was a component probe, not whole-VM certification. Hooks did not expose a retained authorized descriptor. |
| NVX and OpenVMM | Enforcement in the host filesystem server is relevant. Reviewed hosts were Linux/Windows, not Mac. Some nested read-only mappings still relied on guest binds; known-alias bookkeeping did not establish unobserved-hardlink safety. Porting/integration is substantial. |
| Docker Sandboxes | Reviewed direct mode used passthrough; hiding parent worktree metadata did not establish pointer protection. Private clone mode is a useful different workflow, but did not establish preservation of arbitrary staged/dirty state. |
| devsandbox | Reviewed bindings protected external main metadata and supplied sanitized configuration, without a new mechanism protecting the writable worktree pointer. Source finding, not a live exploit. |
| Anthropic sandbox-runtime | Native Seatbelt/bubblewrap confinement; no VM share. Its default policy permits some Git writes. A custom policy would change the Mac boundary and require separate tests. |
| agent-sandbox.nix and Agent Safehouse | Native confinement with deliberate Git write authority in reviewed defaults. Their worktree support does not imply Pinfold's host-owned metadata contract. |
| NFS, 9p or another virtiofs service | Potential transport choices, but none removes object/namespace policy work. NFS integration/cache semantics were untested; other choices added kernel/runtime or bespoke protocol work without demonstrated advantage. |
| OverlayFS and git-lazy-mount | A live changing lower filesystem is outside OverlayFS's supported contract. Stable lower snapshots restore preparation costs. A fresh Linux Git projection does not preserve an arbitrary existing dirty worktree and index by itself. |

Pinned source commits and detailed dependency/privilege analysis are retained in [alternatives](2026-10-15-worktree-research/alternatives.md), [filesystem components](2026-10-15-worktree-research/filesystem-components.md), and [broker integration sources](2026-10-15-worktree-filesystem-enforcement/integration-sources.md).

## Private working files and applying changes

A real Pinfold probe using an APFS clone produced independent file identities, including for a copied pre-existing pointer hardlink. Guest changes affected the private copy while original metadata, index and configuration survived. New files written to the original host checkout were not visible in the private copy. This isolates the original, but changes the workflow.

A fixed ordinary-file application preserved the host index and refused a pre-existing content conflict. Git apply rejected a symlink parent and replaced a hardlinked target without changing its other link. It also accepted protected editor-configuration changes, so it is not a sufficient policy filter. The invalid-gitfile `.GIT` application case was inconclusive.

The harder cases failed: in an event-triggered Linux test, an atomic host save after Git read the target was overwritten despite successful exit. Killing a 10,000-file application at its first mutation left 19 files missing, and retry failed. Generic apply/check does not provide a transaction, protection policy or safe concurrent integration.

### Measured preparation and integration costs

The main surrogate contained 100,002 files and 476,708,872 logical bytes. The actual Pinfold source subset contained 95 files and 1.29 MB. Main phases had three samples, cached images and uncontrolled OS caches; some host copying overlapped a short independent fixture run. These are approximate observations, not cold-start or real-build benchmarks.

| Phase | Mac observation | Linux ext4 observation |
|---|---|---|
| Prepare private copy | APFS clone median 8.38 s, range 8.34–8.83 | Copy 4.47 s first, then 2.83/2.76 s; forced reflink unsupported |
| Cross-volume relocation | Median 32.96 s, range 31.80–35.19; no history copy | Not needed for tested ext4 layout |
| Reuse private copy | Pinfold ready median 0.93 s, range 0.87–1.29 | Native start 0.48 s first, then 0.19/0.21 s |
| Case-volume start | Native start median 0.55 s, range 0.51–0.61 | No integrated candidate timing |
| Root edit and enumeration | Private Pinfold copy median 0.35 s; native case-volume 0.31 s | 1.25 s first, then 0.46/0.49 s |
| Teardown | Private Pinfold copy median 0.18 s; native case-volume 0.12 s | Native zero-grace removal 3.68 s first, then 0.24/0.25 s |
| Apply ten-file change | Base hashing 12.41 s; discovery 20.23 s; check 0.039 s; apply 0.033 s | No corresponding large-tree timing |

Native start and Pinfold readiness measure different boundaries and cannot be subtracted to infer an optimization. No copies occurred in repeated-launch loops. The small source subset took only 0.015/0.030 s for clone/cross-volume copy and would badly understate file-heavy preparation cost.

In an isolated 2 GiB APFS image, 476.7 MB logical source consumed 516.1 MB including metadata. A clone added 28.7 MB; replacing its 64 MiB file added 69.6 MB. Removal restored initial free space. These are synced volume-free-space deltas, not `st_blocks` totals that double-count shared extents. Each Linux ext4 full copy reported 476.7 MB allocated. Snapshot consistency still requires a defined preparation boundary.

## SPACE and the different filesystem model

SPACE was reviewed at commit `b9fc395e06189460262c261b8bf34dfa5a305745`. Its [architecture](https://github.com/perplexityai/SPACE/blob/b9fc395e06189460262c261b8bf34dfa5a305745/docs/architecture.md) gives each sandbox its own writable filesystem. Its [disk design](https://github.com/perplexityai/SPACE/blob/b9fc395e06189460262c261b8bf34dfa5a305745/docs/space/design/guest-io.md) uses an ext4 image attached to Firecracker as a virtio block device and a guest file service for uploads. Bind mounts in launch preparation place that rootfs into the Firecracker jail; they are not evidence of writable sharing of a caller's checkout.

The whole-tree worktree search found contributor/CI/build workflows, not a runtime feature attaching existing Mac linked worktrees. Git worktrees could exist inside a sandbox's private filesystem without sharing the host's metadata; that is an architectural inference, not a tested SPACE Git workflow.

The reviewed [execution contract](https://github.com/perplexityai/SPACE/blob/b9fc395e06189460262c261b8bf34dfa5a305745/docs/execution-environments.md) qualified Linux x86-64/KVM/Firecracker. Its [installation guide](https://github.com/perplexityai/SPACE/blob/b9fc395e06189460262c261b8bf34dfa5a305745/docs/installation.md) explicitly did not run directly on macOS. The architecture includes PostgreSQL, controllers and worker services. Its direct adapter removes Kubernetes sandbox pods from that path; it does not mean a minimal local desktop runtime or direct host-directory access.

SPACE therefore avoids the particular shared-worktree problem rather than supplying a demonstrated alias-protection mechanism for it. It was not installed, benchmarked or certified against Pinfold's policy. Its operational architecture is a reason to question desktop fit, not evidence that no existing implementation can replace Pinfold.

Requiring a clone is insufficiently precise. A normal clone on the host, shared writable, still exposes host Git metadata. A clone or worktree confined to a private guest disk removes that shared-object path. Exporting its results to trusted host locations still requires a separate decision.

## Proposed responsibility split

Moving Linux execution to VMs and moving working files off the host are independent decisions. The former adds a separate kernel. The latter removes the shared-object attack path. Migrating Linux to VMs while retaining the same writable host exports would not resolve the filesystem problem; macOS already illustrates that.

If private working files are chosen, a possible split is:

| Execution layer | Caller such as Pi or Switchyard |
|---|---|
| Isolated execution, private storage and lifecycle | Select input files, clone/copy repositories, choose worktrees |
| Network and credential policy | Decide review and approval workflow |
| Safe, explicitly scoped file-transfer primitives | Detect integration conflicts and apply selected results |
| Resource ownership and cleanup | Decide whether guest commits are useful or commits remain host-owned |

This fits the existing intent that Pinfold core knows nothing about Git. It is still a substantial contract change. “Start here” could capture the folder into private storage; a different caller could operate solely on branches and commits. Automatic continuous copy-back would bring much of the host-authority problem back.

File transfer itself remains security-sensitive wherever implemented: confinement, traversal, symlinks, file types, sizes and resource limits cannot be delegated to the untrusted agent. The caller owns the authority to install results in a trusted checkout and must protect its metadata/configuration. A private guest repository must not be exported wholesale as trusted host `.git` state.

Arbitrary writable host mounts would need removal from this isolation promise or an explicitly separate, weaker contract. Keeping them while advertising that the agent can write only its private filesystem would be contradictory. Read-only host inputs also need careful treatment if another writable route or host replacement can expose the same objects.

The real workflow choice is whether edits must appear immediately in the original host folder. Host editors could instead connect to the guest, preview exported files or apply reviewed results. Those options change setup and integration; none has been validated as a complete Pi experience here.

## Whether to retain Pinfold

Pinfold should not become another generic VM/file-transfer platform merely because the current code exists. The possible reasons to retain it are concrete requirements: local Mac and Linux operation, low setup burden, enforced network policy, credentials kept outside agent-readable state, and caller lifecycle/command semantics. Those requirements justify custom code only where an existing system fails to meet them.

| Replacement evaluation result | Appropriate outcome |
|---|---|
| Existing system meets all required guarantees and workflow needs | Migrate callers; retire redundant Pinfold functionality. |
| Existing system provides execution/storage but lacks a small policy surface | Keep Pinfold as a small policy/integration layer, if enforcement can actually be guaranteed. |
| Existing systems lack essential requirements or impose excessive infrastructure | Retain a narrowly scoped implementation and document the demonstrated gaps. |

No complete replacement evaluation has been performed. The earlier tool survey largely tested the shared-folder requirement and cannot be reused as proof that those tools fail a new private-filesystem requirement. No claim that Pinfold's remaining features are unique is supported.

Local or self-hosted execution and custom orchestration are separate choices. Keeping work on one's own machine does not imply maintaining one's own sandbox platform.

## Next decision and acceptance evidence

First evaluate one real caller workflow against existing implementations before writing a Linux VM backend or a general broker. The trial should start with dirty and staged work, run an actual useful edit/build/test, retain state across the intended lifecycle, expose results for review, and exercise a conflicting host edit during application. Measure setup, readiness, useful work, export/application, teardown, memory and storage separately.

Require evidence that network policy is not bypassable through alternate interfaces, credentials remain outside guest-readable state, file transfer cannot escape its destination, and loss of the caller/runtime does not silently discard retained work or apply a partial result. If the caller chooses branches/commits instead of exact dirty-state integration, record that explicit workflow change. Do not silently replace the original acceptance criteria.

If direct sharing remains required, the smallest implementation candidate is the bounded native plan. It needs linked-layout discovery, real read-only external metadata, all relevant pointer aliases, structural overlap and hardlink admission, and explicit nested-ancestor refusal or enforcement. Preserve host-owned commits and authority-root validation. The historical hook-path alias classification, nested-ancestor and empty-directory-overlap findings need reconciliation against the actual implementation baseline before release.

Extend the existing relevant guarantees with real CLI scenarios: pointer/config mutation, case and hardlink aliases, both sides of rename, protected ancestors, malformed layouts, common/per-worktree metadata, effective hooks, concurrent boxes, ordinary host saves/commits, setup failure and teardown. Assert host bytes/identities and harmless host execution markers with positive controls. A startup scan must not be described as continuous protection against later host topology changes. Follow test-audit and the repository's Mac/Linux gates for any implementation.

For either direction, the current experiments do not establish Linux arm64 behavior, all malformed graphs, discovery-to-use atomicity, arbitrary concurrent host restructuring, every filesystem equivalence, submodule/LFS migration, all ACL/xattr semantics or power-loss recovery. Ordinary direct sharing has last-writer behavior, not conflict-free concurrent editing. Existing host topology assumptions must remain explicit until a different mechanism proves more.

The bounded native recommendation received independent adversarial review. The later private-filesystem and replacement recommendations are architectural judgments, not independently validated production designs. No production diff is awaiting approval in this report.

## Evidence files and cleanup

The detailed reports and compact runnable evidence remain in the primary checkout. Absolute local links are intentional because those bundles are not present in this reporting worktree. Preserve them when moving this report; it is a complete decision narrative, not a duplicate of every raw log or reproduction program.

| Record | Contents |
|---|---|
| [Original recommendation](2026-10-15-worktree-recommendation.md) | Initial alternatives, native Mac/Linux probes, copy/import failures, timing/storage, baseline and cleanup |
| [Original evidence bundle](2026-10-15-worktree-research/README.md) | Runnable probes, raw outcomes, source pins, manifests, patch, independent review and SHA256SUMS |
| [Broker report](2026-10-15-worktree-filesystem-enforcement.md) | Host enforcement design, actual mounted boundary, privilege/lifecycle findings, limitations and costs |
| [Broker evidence bundle](2026-10-15-worktree-filesystem-enforcement/README.md) | Prototype revisions, protocol/native probes, benchmarks, independent review and cleanup |
| [Native protection report](2026-10-15-native-worktree-protection.md) | Final direct-sharing candidate, identity/ACL findings and nested-ancestor counterexample |
| [Native evidence bundle](2026-10-15-native-worktree-protection/README.md) | Eight-mask probes, failed and corrected controls, exact image, raw results, checksums and cleanup |
| [Native independent review](2026-10-15-native-worktree-protection/independent-review.md) | Review evolution and final bounded verdict |

Earlier starting records were [worktree security](2026-10-09-worktree-security-and-startup.md), [Switchyard and Seatbelt](2026-10-09-switchyard-worktrees-and-seatbelt.md), [VM share probe](2026-10-09-seatbelt-vm-share-probe.md), [hook alias ticket](2026-10-14-hook-path-alias-ticket.md), and [measured build recommendations](2026-10-14-measured-build-recommendations.md).

Recorded cleanup removed owned research containers, mounts, broker processes, temporary source clones, fixtures, compiler caches and disposable builds. Fixture ACLs/immutable flags were cleared. The APFS image was detached. The Linux lease was released and confirmed idle. Unrelated runtime workloads were preserved; the later Mac inventory retained the pre-existing BuildKit container. The first phase left ten small stable per-name Pinfold lock files intentionally, with no owner records/sockets, following the never-unlink ownership contract. Exact paths are in that phase's verification file.

The original 285-file baseline and later expanded 393-file manifest/dirty patch were checked unchanged at their respective cleanup stages. No installed Pinfold, live Yard service, account, privileged helper or global runtime/security configuration was changed by this investigation. The temporary SPACE clone was removed after the source review. These are recorded cleanup results, not a fresh October 19 runtime inventory. Consolidating this report created no new boxes, mounts or services and changed no production code or binding specification.
