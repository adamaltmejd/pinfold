# Native Apple Container protection for existing worktrees

2026-10-15. Research only; no production code or binding spec changed.

## Decision

Prefer a small native-sharing candidate over the earlier SSHFS broker proposal.
On this default case-insensitive APFS filesystem, Apple Container's existing
`--read-only-path` mechanism protected a linked worktree's `.git` pointer when
applied to all eight ASCII case spellings. A matched run **without any host ACL**
blocked writes, chmod, hardlink creation, unlink, rename and replacement through
every spelling. Ordinary editing remained writable.

This validates the user's case-alias suggestion as a useful bounded mechanism.
It does not establish arbitrary protected-path safety. Keep existing hardlink and
overlapping-export admission, genuinely read-only external Git metadata, and
refuse unsupported nested protected paths until their ancestors are protected.
No new host account, helper, volume, filesystem protocol or manual setup was used.
Current worktree refusal remains until Pinfold integration and its gates pass.

The earlier broker recommendation was premature as a minimum-cost solution.
The native candidate retains the existing I/O path; there is no per-operation
SSHFS/SFTP layer. Its performance was not benchmarked here, so no exact overhead
or production startup estimate is claimed.

## Separate host identity

Apple documents that native VirtioFS accesses files with the current host user's
effective UID. Guest ownership changes do not change host ownership. The inspected
public API exposes a URL and whole-share read-only flag, not a per-share identity
or authorization callback. Container 1.5.0 launches its runtime in the user's
launchd domain. The new test runtime and Virtualization XPC process both ran as
UID 501. Guest UIDs 0, 501 and 12345 all encountered the ACL denying host user Adam;
files created by all three were owned by host UID 501.

A separate host identity is possible as an architectural direction, but not via
the guest `--user` option. A privileged installer/helper could provision and run
it automatically after administrator authorization. That is a new deployment,
process-ownership and permission model, not a small unprivileged CLI change.
No account, helper or separate-identity VM was installed or tested.

Pinned source and SDK evidence: [identity investigation](2026-10-15-native-worktree-protection/identity-source.md).
Primary permission contract: [Apple VZVirtioFileSystemDevice](https://developer.apple.com/documentation/virtualization/vzvirtiofilesystemdevice).

## ACL findings

The native service honored ACLs on actual host objects, including case aliases,
symlinks and hardlinks. Denying data writes and deletion on the pointer prevented
content changes, unlink, rename and replacement. A DELETE-only ACL on each nested
ancestor prevented renaming it while allowing ordinary sibling-file creation.
This is more selective than denying creation throughout the workspace root.

However, an ACL denying the host user also affects that user's host processes.
Freezing just the `.git` pointer can still permit normal host commits, because
Git reads that pointer and writes the external administrative directory. The
experiment mounted that external directory genuinely read-only to the guest;
no deny ACL was put on the host's external metadata. Host staging/committing
succeeded while the pointer was frozen.

ACLs alone did **not** make protected objects fully read-only. The owner retains
permission-changing authority: guest chmod changed a protected pointer's host
mode despite an ACL denying writesecurity. The deny ACL and file bytes survived.
Valid guest POSIX ACL updates succeeded on an ordinary control but failed on the
protected pointer. Attempts to remove or replace the native security xattr also
failed. These are tested protocol paths, not an exhaustive proof of the closed
Virtualization implementation.

A deliberately pre-existing writable hardlink to a protected, initially
non-executable hook allowed chmod to enable it; a subsequent host fixture commit
ran its fixed harmless marker. This shows why ACL-only protection is insufficient
and why existing alias admission must remain. The layout deliberately bypassed
Pinfold admission; it is not a newly reproduced Pinfold CLI defect. In contrast,
a guest could not create a new hardlink out of the read-only hook mount.

Applying an ACL after the guest already held a writable descriptor did not revoke
the descriptor: the subsequent write succeeded. Any future ACL design must install
protection before exposing the inode through any active writable box. It must
preserve pre-existing ACLs, coordinate overlapping sessions, restore only owned
changes, and survive crashes. Host replacements of protected inodes and topology
changes remain unsupported without an additional mechanism. These costs favor
avoiding ACLs where existing read-only mounts suffice.

## Mounted evidence

All probes used disposable repositories through real native Apple Container
shares, not an SFTP simulation or mocked runtime. They used cap-drop ALL, a
uid501 main workload, no network, read-only guest root and tmpfs scratch.
Identity controls also ran guest UID0 and UID12345 with the same empty capabilities. Host
processes independently inspected bytes, modes, ACLs, identities and the hook
marker. The sample host was arm64 macOS 27.0.1; runtime 1.5.0. The exact existing
image identity was captured before the first launch in the bundle.

| Probe | Observation |
|---|---|
| Host/guest identity | ACL for host Adam denied writes by guest UID 0, 501 and 12345. A deny for host nobody did not. Host-created file ownership remained UID 501 regardless of guest UID. |
| Pointer ACL | Literal, uppercase, old hardlink and symlink writes denied. Unlink, rename and replacement denied. Ordinary atomic edit and host commit succeeded. |
| ACL manipulation | Protected-pointer ACL set/remove attempts failed, with a successful ordinary POSIX ACL-set control. Owner chmod still changed host mode. |
| Protected directory mount | Literal and uppercase hook chmod denied, including uppercase as the first workload lookup. New hardlink creation failed with EXDEV. This is not proof for all directory aliases or filesystems. |
| Deliberate pre-existing hook hardlink | chmod succeeded through the writable alias; host fixture commit produced the marker. Such a layout must be refused at admission. |
| Eight pointer masks, with ACL | All eight spellings denied write, chmod and hardlink creation. Pointer mode remained 0644, bytes unchanged; ordinary editing worked. |
| Eight pointer masks, without ACL | All eight spellings denied write/chmod (EROFS), hardlink creation (EXDEV), unlink/rename/replacement (EBUSY). Symlink-to-pointer chmod also failed. Ordinary write, chmod, hardlink, mkdir and atomic replacement all succeeded. |
| Nested ancestor without ACL | Renaming `deep`, which contained the protected `deep/middle/hooks` mount, succeeded and renamed the host directory. It was restored after the box stopped. This is an explicit unsupported-layout counterexample. |
| Native cache visibility | Two immediate guest Git reads after host commits failed to find the current branch. A fresh mount read it; a later independent commit was visible on the first read at 0.062 s. The existing architecture already documents native metadata/name caching. No universal instantaneous-visibility claim. |
| Existing writable handle | ACL installed after open did not prevent a later write. No live source files were involved. |

The eight-mask check used `.git`, `.giT`, `.gIt`, `.gIT`, `.Git`, `.GiT`, `.GIt`,
`.GIT`. Three Unicode lookalikes and 27 inserted/appended ignorable-character
spellings did not resolve to `.git` in host stat checks on this APFS volume.
That sample is not an exhaustive Unicode equivalence proof. HFS+, network
filesystems and other APFS/runtime versions were not validated. A production
admission rule must identify supported filesystem behavior explicitly.

One initial no-ACL run reused ordinary fixture names, causing its hardlink and
mkdir positive controls to report EEXIST. Its results are retained as
`eight-noacl-initial-results.json`. The final run uses distinct names and `set -e`;
all ordinary controls passed with empty stderr. Protected-operation results are
consistent across both runs. No failed observation was discarded.

## Smallest implementation direction

1. Support a bounded APFS linked-worktree layout using native pointer masks and
   actual read-only exports of discovered external metadata. Keep host-owned
   commits and the existing writable source export.
2. Preserve existing alias/overlap admission and layout validation. Do not assume
   the caller's metadata paths are canonical or complete. No other writable view
   may expose protected objects. Keep a stable host-topology contract while boxes
   run; this research does not protect arbitrary concurrent host restructuring.
3. Start with protected directories directly below the worktree root, or otherwise
   layouts whose ancestors cannot be changed. Refuse nested hook/protect layouts
   unless ancestor protection is separately implemented and validated. The tested
   DELETE ACL is a candidate for that extension, with explicit lifecycle work.
4. Extend the existing guarantees with real CLI cases for aliases, namespace
   changes, ordinary controls and host Git outcomes. Validate simultaneous boxes,
   setup/teardown failures and supported filesystem/runtime versions. Update the
   binding architecture with the same implementation; run test-audit for changed
   assertions. No production tests or implementation changed in this research.

A host-identity/helper design remains a fallback if full selective ACL authority
is required. It is substantially larger than the demonstrated pointer mechanism.
The SSHFS broker remains another fallback, not the preferred next step.

[Independent review](2026-10-15-native-worktree-protection/independent-review.md)
agrees that this is a useful research stopping point and supports the bounded
native candidate, not immediate worktree enablement.

## Reproduction and cleanup

[Evidence bundle](2026-10-15-native-worktree-protection/README.md): source probes,
raw outcomes, pinned source review, exact image identity and cleanup verification.
All owned containers were removed and all fixture ACLs cleared. The pre-existing
BuildKit container was left running. No accounts, services, images, production
permissions or installed binaries were changed. Disposable repositories, compiled
probes and isolated compiler caches were removed after retaining evidence.
