# Native ACL design review — 2026-10-15

Scope: independent source review only. No accounts, ACLs, runtime state, mounts or production files changed. The main investigator owns real native-share experiments. This note does not establish guest behavior.

## Findings

Separate host identities are the conventional sound boundary, but a current-user process cannot simply adopt another host UID. macOS `setuid(2)` permits arbitrary identity changes only with appropriate privilege; `setgroups(2)` requires superuser privilege. Automatic installation could hide account provisioning behind a privileged helper, but it would still require authorization, lifecycle/state, and security maintenance. An arbitrary existing identity such as `nobody` is a poor shared isolation principal. Per-user or per-session service identities would need careful collision, group, home traversal, ownership and cleanup treatment. Guest UID selection is not proof of host identity selection.

The local public `VZSharedDirectory.h` exposes a URL and whole-share `readOnly` flag; it exposes neither a per-share host UID nor a per-file policy callback. An API omission is not proof that the entire runtime cannot be launched under another identity, but it rules out claiming there is an obvious supported UID-mapping setting on this object.

A host-identity ACL can protect the actual inode regardless of case, Unicode, symlink, or hardlink spelling. Namespace protection is separate: denial of content writes does not prevent unlink, rename or replacement. Current public XNU explicitly gives a node's DELETE ACL precedence over the containing directory's DELETE_CHILD ACL, including when the parent permits deletion. Thus a deny-delete ACL on the pointer can preserve it without denying ordinary sibling deletion. Ancestors that could be renamed and replaced also need appropriate protection.

The owner is automatically granted WRITE_SECURITY in `vnode_authorize_simple`; an owner-deny writesecurity ACE cannot stop a native owner process from changing the ACL. Therefore same-host-UID ACLs are not a general separation of host and guest authority. Whether they can implement a narrower freeze depends on what operations the native sharing service exposes.

## Narrow same-identity freeze deserves an empirical decision

A potentially useful arrangement is to freeze only the worktree `.git` pointer and selected editor/hooks locations with host ACLs, export the external Git metadata read-only through a separate genuine native share, and let host Git update that external metadata normally. This is not refuted merely because the host and sharing process have the same UID: a Linux guest may lack a way to invoke the native ACL-removal operation.

Do not count successful guest chmod as an ACL escape without reading the host ACL and trying a subsequent write. POSIX mode changes may preserve explicit deny ACEs. Conversely, do not call the arrangement safe simply because `chmod -N` is absent from the guest. Test the exposed protocol operations that could mutate security metadata.

Current public XNU makes `com.apple.system.*` xattrs protected. Setting `com.apple.system.Security` through the ordinary host xattr syscall requires the private `com.apple.private.vfs.filesec-access` entitlement. Removing protected system xattrs through the ordinary removal syscall returns EPERM. The local Virtualization.VirtualMachine XPC executable's signature does not carry this filesec entitlement (it does carry `com.apple.private.vfs.open-by-id`). Native ACL APIs are separate; this is not a blanket proof that Virtualization cannot modify ACLs through another path. Linux `system.posix_acl_access` translation, mode updates, ioctl and xattr paths remain relevant.

A same-identity freeze denies direct host modifications to those same frozen objects. That may be acceptable for a stable `.git` pointer during ordinary commits, but it is a contract tradeoff for editor configuration changes and worktree move/repair. It is not the originally stated promise of unrestricted host editing of every protected object.

## Gaps that apply even if the native syscall matrix passes

- ACLs protect existing objects. A missing `.vscode`, hooks root, or other reserved name has no inode on which to install an ACL. Ordinary directory ACLs cannot distinguish creation of one child name. A precreated protected placeholder or another enforcement mechanism is needed.
- Deny-delete on the protected object does not prevent renaming a writable ancestor. Protect every relevant ancestor against displacement while preserving ordinary child edits. For an existing protected subtree, deny additions and removals inside it; for ordinary ancestors, avoid unnecessarily denying creation of normal siblings.
- A host atomic replacement can introduce a fresh inode without the old file's ACL. Inheritance can protect an entire metadata subtree, but cannot selectively protect only `.git` among writable worktree-root files. Stable topology and explicit stop/reconfigure behavior are real conditions, not implementation details.
- Permissions are ordinarily checked when opening a file. A preexisting writable guest descriptor may survive ACL installation. Install and verify protection before any writable export or coordinate all existing boxes; do not assume late application revokes authority.
- Multiple sessions need shared ownership/refcounts of each installed ACL. A session stopping must not remove another session's protection. Crash recovery must distinguish Pinfold's exact ACEs from user changes; never restore an old entire ACL blindly over concurrent host edits.
- Protected preexisting symlinks require referent and namespace analysis; checking the visible path alone does not establish the intended inode. Reparse and validate all paths before exposure.
- An attacker may alter modes or xattrs even when content remains protected. Decide whether availability/metadata integrity is in the guarantee. Content protection should not be confused with denial-of-service resistance.

## Minimum decisive fixture matrix

1. Host-owned ordinary file positive control, frozen pointer, `.GIT`, preexisting hardlink, symlink and Unicode directory alias. Actual guest UIDs 0, host-number UID, and unrelated UID. Observe host inode/hash/ACL/mode independently.
2. Content write, append, truncation, rename-away, unlink, atomic replacement, hardlink creation and write through alias. Ordinary equivalent operations must succeed.
3. Guest chmod/chown, raw get/set/remove of `com.apple.system.Security`, `user.com.apple.system.Security`, Linux `system.posix_acl_access`, and ordinary xattr positive controls. Inspect host ACL after each mutation and retry actual content write.
4. Same denied direct host write (expected for same-identity freeze), normal host status/add/commit against unfrozen external Git metadata, and guest status/diff against its read-only export.
5. Held descriptor across ACL installation, host replacement with a fresh inode, and host ACL removal. These identify required lifecycle and topology exclusions rather than malicious-host claims.
6. Protected nested ancestor renames, absent reserved-name creation, two simultaneous boxes, one stopping, and crash recovery. These are needed before production admission; not all are necessary to reject an already-failing core mechanism.

## Sources and reproducibility

Local read-only observations:

- `man chmod`, ACL MANIPULATION OPTIONS.
- `man setuid`, `man setgroups`.
- `/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/System/Library/Frameworks/Virtualization.framework/Headers/VZSharedDirectory.h`.
- `codesign -d --entitlements :- /System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc`.

Primary source links:

- [Apple filesystem permission model](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/FileSystemDetails/FileSystemDetails.html). Archived documentation; kernel/current-runtime checks take priority where behavior differs.
- [Apple shared directories](https://developer.apple.com/documentation/virtualization/shared-directories).
- [XNU vnode authorization](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/vfs/vfs_subr.c#L9457): DELETE precedence at 9465–9482; owner WRITE_SECURITY exemption at 9672–9678.
- [XNU xattr protection](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/vfs/vfs_xattr.c#L408): protected prefix and ordinary write-extattr authorization.
- [XNU xattr system calls](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/vfs/vfs_syscalls.c#L13601): entitlement check at 13601–13621, set at 13813, unconditional protected-name remove denial at 13960.

The public XNU main SHA was resolved on this date with `git ls-remote`; it is supporting source evidence, not a claim that this exact commit is the running macOS kernel. Three downloaded source files used for inspection were removed after this note was written. No fixture requiring ACL cleanup was created by this reviewer.

## First mounted-result review

Read `/private/tmp/pinfold-native-acl-20261015/results.json`, `probe.py` and `xattr_probe.c` after the first native-share run. This is review of the main investigator's execution, not an independently rerun experiment.

The evidence supports a narrow content-and-namespace result: pointer writes through literal, case, hardlink and symlink names were denied for guest UIDs 0, 501 and 12345; pointer unlink/rename/replacement and protected editor-directory creation/replacement were denied. Ordinary file creation and atomic save worked. The host's deny-adam ACE applied even to guest UID 12345, while a deny-nobody ACE did not: consistent with use of host identity, not direct guest UID impersonation. Host-created-file ownership also remained UID 501.

Crucially, guest chmod changed the actual protected host pointer's mode from 0666 to 0777. Its ACL and bytes survived, and a later content-open remained denied. Thus this test is neither an ACL-removal escape nor full read-only enforcement. A current-spec-relevant follow-up is an initially non-executable hook in an in-worktree core.hooksPath: if a guest can chmod it executable through a case alias and an ordinary host commit then executes it, the host-execution boundary fails despite immutable content. An explicit writesecurity deny cannot fix the owner's implicit native authority. This must be tested before recommending ACL freezing as the production direction.

Hardlink creation succeeded and increased the pointer's link count. That is not a content escape by itself, since the ACL follows the inode; test an actual write through the newly created alias. Root/other-UID variation of every namespace syscall is unnecessary once host identity and actual relevant owner paths have been established, unless a different guest permission gate changes the outcome.

The immediate guest Git log failure after a successful host commit should first be compared with the existing accepted ~1-second native metadata/name-cache limitation (ARCHITECTURE.md Shared files). Do not attribute it to ACLs without a persistent result or a matched baseline. The protected pointer itself did not need host mutation for the commit.

Before deciding this research branch, the minimal remaining tests are valid POSIX ACL-set operations with an ordinary positive control, protected-hook mode activation, actual new-hardlink write, and delayed metadata visibility. Held-FD and ancestor tests establish lifecycle constraints. Full concurrent cleanup/recovery testing belongs to an implementation only if this mechanism survives the core threat-model test.

## Follow-up mounted-result review

Read `followup-results.json`, `followup.py`, and the current binding spec. The hook activation concern above did **not** reproduce in the tested hybrid: both `.hooks/pre-commit` and `.HOOKS/pre-commit` chmod attempts returned EROFS with the existing directory read-only mask, the mode stayed 0644, and host commits did not create the marker. It would be incorrect to report a hook execution bypass from the pointer chmod observation.

The valid POSIX ACL-set probe succeeded on an ordinary control but failed on the protected pointer. Both new-pointer-hardlink creation and the subsequent denied write were observed. Ancestor DELETE denial blocked moving either tested nested ancestor while an ordinary child write succeeded. The held-open descriptor remained writable after ACL installation, proving that protection must precede export/handle acquisition.

This leaves a **plausible narrow hybrid**, not a full ACL-only filesystem scheme:

1. Preserve native writable access to the original worktree.
2. Protect the existing linked-worktree pointer's bytes and namespace with an inode ACL installed before exposure.
3. Preserve existing read-only directory masks for protected config/hooks and a genuine read-only export of external Git metadata.
4. Preserve startup rejection of protected hardlinks and writable overlapping exports; investigate any guest-created alias path.
5. Coordinate ACL lifetime across every box that can reach the inode, and retain stable host-topology restrictions.

The pointer's chmod vulnerability is a strict read-only metadata-contract issue and permits denial of access. No host execution caused by pointer mode changes has been shown. It does not invalidate the narrower content/namespace result. If full metadata read-only behavior is required, keeping the existing literal read-only pointer mount and additionally masking the other seven ASCII case spellings is a small candidate to test; it is not demonstrated here.

A possible explanation for the regular-file/directory difference is Linux's FUSE lookup use of `d_splice_alias`: directory inodes have a single dentry alias, whereas regular files may have several. Thus a directory case alias can encounter the same read-only mounted dentry. This is a source-supported hypothesis, not proof of Apple runtime behavior for every mount/lookup order. Current FUSE lookup source: https://github.com/torvalds/linux/blob/master/fs/fuse/dir.c . Directory dentry explanation: https://docs.kernel.org/5.10/filesystems/nfs/exporting.html . A preexisting hardlink outside the directory and a second distinct export are different situations and still need rejection or enforcement.

Remaining research checks should focus on the proposed combination: preexisting-hook hardlink behavior versus the existing admission rejection; new hardlink creation from a read-only directory; alternate-case lookup ordering; and the persistent-versus-transient Git metadata refresh issue. The fresh mount saw the prior host commit, but a later host commit again produced an immediate guest lookup error. The follow-up does not establish a sufficiently long elapsed cache interval, so neither a persistent coherence defect nor normal recovery is yet demonstrated.

No read-only runtime/source review can substitute for final integrated Pinfold tests. An acceptable research conclusion is that the stock native data path plus a small pointer ACL guard is feasible enough to investigate as an implementation candidate, with explicit guarantees and lifecycle work; it is too early to enable linked worktrees.

## Third and fourth mounted-result review

Read `hybrid-results.json`, `hybrid.py`, `eight-results.json` and `eight.py`.

The third run distinguishes a dangerous layout from an admitted one. A deliberately preexisting hardlink outside the read-only hooks directory allowed chmod and the subsequent host fixture commit executed the harmless marker. This demonstrates why protected-hardlink admission is essential. The test bypassed Pinfold admission by launching Apple Container directly; it does not demonstrate that current Pinfold admits that layout. Creating a new hook hardlink from the protected directory failed EXDEV. Looking up the uppercase hooks directory first still gave EROFS on chmod. All eight pointer-case writes were denied; with only one pointer mask the uppercase chmod remained possible. Host commit visibility succeeded on the first observed guest read after 0.062 seconds in that run; earlier transient failures remain retained evidence, not erased.

The fourth run removed all preexisting pointer hardlinks, leaving nlink 1, and installed all eight ASCII pointer masks plus the ACL. For every spelling, chmod and content write returned EROFS and hardlink creation returned EXDEV. Symlink-followed chmod was also denied. Ordinary write/chmod/link/atomic replacement succeeded. The ACL alone produces EACCES in earlier runs, so these EROFS results show the masks are actively enforcing the tested operations. They do not establish whether the ACL is necessary for other operations. A final matched run without the ACL is the appropriate small counterfactual before selecting it as a production requirement.

This reviewer additionally performed only read-only host stat calls for names `.g\u200dit`, `.g\u200cit`, `.g\ufeffit`, `.g\u034fit`, `.g\u00adit`, `.git\ufe0f` against the fixture. Every call returned ENOENT. The main investigator has a larger retained Unicode check. These are measured APFS observations, not a proof over every Unicode name or every case-insensitive Mac filesystem. Apple's historical [HFS+ TN1150](https://developer.apple.com/library/archive/technotes/tn/tn1150.html) documents ignorable Unicode characters in case-insensitive comparison; avoid making an HFS+ support claim from APFS tests.

The accumulated evidence changes the relative design recommendation. A native-mask solution plus existing alias/overlap admission should be assessed before a new filesystem broker or host ACL lifecycle, if the matched no-ACL run succeeds. This uses the native data path, needs no extra identity, and avoids permanent host permission changes. It remains a narrow implementation candidate, requiring integrated linked-worktree discovery/export tests and clear supported-filesystem assumptions. Nested protected paths remain separate: the earlier ancestor-rename passes used explicit DELETE ACLs on ancestors, not just directory masks. Keep that qualification unless a dedicated existing-mask result resolves it.

## Final independent verdict

Reviewed final `eight-noacl-results.json` and `eight_noacl.py`. The preceding eight-mask run recorded successful pointer-ACL cleanup; the no-ACL script installs no ACL. In the final no-ACL run, all eight case spellings denied each of content write, chmod, new hardlink, unlink, rename-away and replacement. The host pointer's bytes and 0644 mode survived. The symlink chmod also failed. The corrected ordinary positive control uses distinct filenames and `set -e`; it exited zero with empty stderr and showed successful ordinary write/chmod/link/directory creation/atomic replacement. Earlier reused-name control errors must remain identified as superseded experimental artifacts, not counted as successful controls.

The same final run successfully renamed `deep` while a read-only mount protected `deep/middle/hooks`. This is a real counterexample to the claim that a nested directory mask alone protects the host's configured pathname. It is separate from the successful top-level pointer result and must be included prominently in the final recommendation. The fixture was restored by the experiment after container removal.

**Recommended next implementation candidate: native `.git` pointer case masks plus existing protected-hardlink/writable-overlap admission, with genuine read-only external Git metadata.** Do not add an SSHFS broker or host ACL management merely to solve the tested pointer problem. This preserves native source-file access and requires neither a new host identity nor a new volume. It is supported as a small candidate by real runtime evidence; it is not yet an integrated Pinfold guarantee.

Scope initial support to layouts whose protected paths have no writable ancestor that can be displaced inside the export, or implement and prove ancestor protection. The earlier DELETE ACL experiment provides evidence for one ancestor-guard mechanism, but its installation, shared lifetime, crash recovery and effects on host tools remain unimplemented. Refusing unsupported nested protected layouts initially is a clearer bounded contract than claiming the top-level experiment covers them.

Retain existing alias admission. The deliberate preexisting hook hardlink demonstrated actual host marker execution despite ACL content freezing. New alias creation from the tested read-only directory and pointer masks was denied, but existing external aliases are materially different. Coordinate every writable export of the repository; do not assume one protected guest path controls a second export. Keep stable host-topology assumptions explicit.

Separate host identity remains an architectural option requiring privilege/setup machinery, not a stock per-share API setting. ACLs can enforce inode-based content and namespace restrictions but are neither a full same-owner read-only mechanism nor necessary for the pointer attacks blocked by the final masks. They are a possible targeted ancestor guard, not the preferred default conclusion of this investigation.

Research can stop here. The next work should be a small reviewed implementation with binding-spec updates and integrated real-boundary tests, including supported APFS name behavior, linked-worktree discovery and external metadata routing, initial alias rejection, ordinary host commits, startup failure cleanup and the explicit nested-path restriction or guard. No production support should be announced from these raw Apple Container experiments alone.
