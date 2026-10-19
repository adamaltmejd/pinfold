# Independent review: brokered writes on existing case-insensitive APFS

2026-10-15. Design/source review only. Existing worktrees on the current volume are now mandatory; the earlier case-sensitive-volume recommendation does not meet this requirement. No runtime experiments or production edits were performed.

## Disposition

A host filesystem broker is a plausible direction. Its security boundary must encompass object writes **and namespace changes**, not a pathname callback around an otherwise unrestricted provider. A bounded first design can plausibly use held directory/file descriptors, explicit protection identities, a single serialization domain for untrusted mutations, and conservative refusals. This is a design argument, not proof of a component or integration.

There is no need to build a general synchronization system. There is, however, a need to replace every direct writable host share of the protected namespace. One remaining framework writable export, passthrough descriptor, alternate provider or broader second box can invalidate the design.

## Relevant platform correction

Do not equate absence of Linux `openat2` with absence of kernel-assisted path restrictions on macOS. Apple's current [open documentation](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/man/man2/open.2) describes `O_NOFOLLOW_ANY`, `O_RESOLVE_BENEATH` and `O_UNIQUE`. The local Command Line Tools SDK also defines these. They restrict symlink traversal, resolution beneath a directory, and opening multiply linked files. They do not identify Pinfold-protected objects, govern later writes on an existing handle, or make an arbitrary policy check atomic with a namespace mutation. Minimum-runtime behavior remains to be probed.

The [rename documentation](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/man/man2/rename.2) supplies descriptor-relative operations and no-follow/beneath flags. Exchange-style renames must be treated as mutations of both endpoints; an atomic operation is not automatically an authorized operation.

`openat_authenticated_np` is **not** an inode-equality compare-and-open primitive. The locally installed Apple manual describes authenticated-volume verification. Do not adopt it for this purpose. The SDK source used is `/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include/sys/fcntl.h`; its corresponding manuals were read locally. XNU source revision above was resolved from upstream `main` during this review.

## Smallest defensible policy

1. **One mediated namespace.** All boxes and exports that can touch a shared host namespace must have the same effective protections. Initially refuse cross-export renames, overlapping writable roots and mount/firmlink crossings that the implementation cannot prove safe. Read-only host locations must remain read-only through every guest view, including roots registered inside a protected subtree. A per-box mutex cannot serialize another broker's mutations. Either share a coordination domain or refuse concurrent overlapping writers until this is implemented.

2. **Retain roots and protections as objects.** Open export roots, protected directory roots, the linked-worktree gitfile and the ancestors whose replacement would redirect protection. Keep descriptors or equivalent non-reusable references for their lifetime. Bare cached `(device,inode)` values can become stale after deletion/reuse. Resolve effective hook locations before admission, including external aliases. Materialize absent protected directories before publishing the export, as current Pi does. Refuse unsupported symlinked administrative topology. Protected directories are recursive policies: new host index/config files under them inherit denial without a recursive file inventory.

3. **Resolve relative to held directories.** Reject traversal syntax and operate on individual path components under held dirfds, never a reconstructed absolute pathname. Check each opened directory's identity against the protected roots/ancestors and its permitted export. Existing leaf decisions use the actual opened object. On case-insensitive APFS, `.GIT` therefore reaches the same protected gitfile identity. Do not implement case folding or Unicode equivalence in user space. Creation uses exclusive creation under an authorized parent; an equivalent existing protected name must fail without changing it.

4. **Separate acquisition from mutation.** Open an existing regular file without truncation, authorization side effects or automatic creation. Inspect the returned descriptor before any `write`, truncation, mode/xattr/ACL/time change, allocation operation or other mutation. Truncate only through the authorized descriptor. Create a missing regular file exclusively only after parent/slot authorization. Reject special files, opaque filesystem control calls and unclassified operations. Do not pass raw host descriptors to untrusted code or provide a mapping/DAX/writeback route that bypasses authorization.

5. **Make namespace changes part of authorization.** Unlink/rmdir/rename/replace/exchange/link inspect both parents and all existing endpoints. Deny a protected root, gitfile, their protected ancestors, any entry inside a protected directory, and any destination slot that would replace those objects. The finite set of protected ancestors makes subtree moves decidable without scanning every source descendant: moving an ancestor of a protected root is denied. Preserve the full decision until the descriptor-relative syscall completes. A post-mutation check is too late.

6. **Start with conservative hardlink behavior.** Refuse guest hardlink creation. For regular-file content/attribute mutations, require `fstat(fd).st_nlink == 1`, not merely `<= 1`; unlinked retained handles need explicit semantics. Conservatively refuse namespace mutations of multiply linked regular endpoints too, since unlinking an alias changes the same inode's link count/ctime and the broker may not know which name is protected. Do not apply regular-file link-count assumptions to directories. Normal hardlinked source files then become read-only/refused; disclose this compatibility restriction.

7. **Serialize the complete untrusted decision and mutation.** The simplest proof uses one lock covering lookup/authorization plus all guest data and namespace mutations that can change the authorization facts. Serializing only `rename` calls while another thread executes `open(O_TRUNC)`, `link`, `ftruncate` or writes a retained handle leaves gaps. A narrower lock protocol is an optimization requiring proof and measurement. No host callback, worker thread, async completion or alternate export may mutate outside this domain.

8. **Treat handles as capabilities with limits.** A guest handle is an opaque broker-owned reference with export scope, object identity and immutable maximum rights. A read-only/protected handle never upgrades when a name disappears or policy changes. Recheck mutable conditions such as link count for each mutation. Never reopen the stored pathname as an implementation shortcut. Cache eviction, inode reuse, handle reuse, restart, reconnect and delayed write requests must fail closed. Retaining a descriptor preserves identity, not permission forever.

Symlinks need not be banned from project contents. The narrow approach exposes them as symlinks and lets the guest filesystem resolve them within its exported namespace; the host broker never follows them implicitly. Symlink creation/readlink/unlink have their own non-following operations. A provider that instead resolves host symlinks must implement bounded expansion against the same export graph, including absolute targets, escapes and loops. That is a larger first scope.

## What serialization does and does not prove

Under the rules above, guest threads cannot swap a checked component, add a hardlink or move an ordinary inode into a protected location between the broker's authorization and mutation. Component walking plus held descriptors can therefore form a sound argument against **broker-mediated guest races**, even without an `openat2` call. This conclusion requires that all operations in the claim actually use that broker. It is not established by source shape alone.

It does not stop an independent host editor, Git command or second broker. Trusted host assumptions must name the relationships that may not change, beginning before admission:

- Export roots and protected roots/ancestors do not move, switch to symlinks or become different mounts.
- The gitfile and effective protection/configuration destinations do not change.
- The host does not move an object with guest write authority into a protected tree, create a protected/writable hardlink relationship, or move a directory retained by the broker outside its authorized export.
- A new overlapping writer cannot start with a weaker policy or independent serialization domain.

Ordinary source file atomic replacement **within the same allowed tree** can remain supported. Existing guest descriptors can then refer to the old unlinked object; reopening the name sees the new object. That is normal concurrent filesystem behavior, not lossless collaborative editing. Refusing writes on `nlink == 0` gives conservative stale-handle behavior. A no-follow dirfd walk alone does not prevent a trusted host from moving an already-open parent directory outside the export during the operation.

Host Git can replace index/refs/config files *within a stable protected metadata root*: root-based policy covers the new children without enumerating them. Worktree move/repair, root replacement and changing effective hooks destinations remain stop-box operations. A host atomically replacing an ordinary source file is materially different from promoting that writable inode into protected metadata.

## Is `nlink > 1` refusal enough to avoid recursive inventory?

**Useful, but only with the namespace policy.** If a current protected regular file is also reachable by an otherwise permitted hardlink, its link count is at least two. Refusing its mutation by opened descriptor closes that route without discovering every protected descendant in advance. Protected-root traversal handles ordinary descendants; the explicit gitfile identity handles case/normalization aliases; global export policy handles duplicate exports, which can have `nlink == 1`.

It does not protect rename/unlink, ancestors, future names, directory aliases, mount aliases, or writes through a descriptor already admitted before a new link. It cannot supply synchronization with a host-created link between `fstat` and `pwrite`. `O_UNIQUE` improves open-time checking but is not a lifetime uniqueness lease. Denying all guest link creation and checking each mutation makes the guest-side proof smaller; the trusted-host condition remains.

There is a subtle scope limit: a host may replace a protected file whose old inode also has an external alias. That alias can become singly linked while an old host reader still holds the retired inode. A fresh broker open cannot infer its historical protected status solely from `nlink == 1`. Previously protected guest handles must stay read-only. If the guarantee must also protect **all retired metadata inodes still held by arbitrary host processes**, per-write link counts and directory roots alone are insufficient. Either exclude protected-to-writable alias demotion in the host assumption, reject such initial layouts with an admission inventory, or maintain stronger identity tracking. The first implementation must state this boundary; it cannot simultaneously claim no inventory, unrestricted host metadata replacement and historical-object protection.

## Decisive validation, before integration claims

Use an actual guest-facing broker, host identity/byte observers and ordinary allowed controls. Required cases are:

- `.git` case aliases, normalized protected names, preexisting aliases, duplicate exports and roots registered inside protected directories.
- Open-before-link, retained handle after rename/unlink, source/destination exchange, multi-level protected ancestor replacement, and an empty protected directory.
- Deterministically coordinated competing guest create/rename/unlink/link/write requests at each authorization boundary, including two boxes; no sleep-based evidence.
- Ordinary host atomic saves and live index/ref replacement, with fresh versus retained handles observed separately.
- Deliberate host protected-topology changes to show exactly what the documented assumption excludes; no claim that a scan prevents them.
- Mutations through truncate/setattr/xattr/ACL/allocation/copy paths and delayed writeback, including restart/disconnect and stale handle IDs.
- Representative existing-tree workload measurements after correctness, including hardlink refusal compatibility, serialization cost and metadata-heavy enumeration. A simple native callback test does not establish the VM/broker boundary.

Reject these shortcuts: `realpath` then pathname open; lexical deny rules; `O_NOFOLLOW` on the leaf alone; `nlink` only at startup/open; a case-folded spelling list; denying writes but allowing rename; per-box locks while another box shares the tree; raw writable passthrough for performance; and filesystem watchers as synchronous enforcement. Apple's [secure file guidance](https://developer.apple.com/library/archive/documentation/Security/Conceptual/SecureCodingGuide/Articles/RaceConditions.html) recommends descriptor-based operations and explains the check/use problem; the policy above is a separate proposed design, not an Apple certification.

## Recommendation

Pursue a small host broker with explicit object and namespace authorization, initially rejecting hardlink mutation and unsupported overlapping writers. Keep the VM/network/privilege contract. Do not relax worktree refusal until the actual transport/provider passes the relevant boundary cases. If the practical component cannot mediate every mutation and coordinate shared namespaces, it is not an acceptable shortcut for mandatory existing-volume support.

Only this review note was created. No source checkout, clone, mounts, boxes, background processes or generated build artifacts remain from this review.

## Assessment of the proposed Go/SFTP prototype

The parent proposed Go 1.27 `os.Root`, `pkg/sftp.RequestServer`, one broker mutex, non-truncating open followed by descriptor authorization, protected inode snapshot plus `F_GETPATH` checks, per-write `nlink == 1`, fd-based chmod/truncate and fail-closed unsupported FSETSTAT. This is a coherent **bounded experiment** under its stated single-broker and stable protected-topology assumptions.

The local toolchain is Go 1.27.2. Its `src/os/root.go` says Root follows confined relative symlinks, disallows absolute symlink targets, does not prohibit mount boundaries or devices, and follows its root directory across rename. It also explicitly warns about pathname Chmod/Chown/Chtimes races. The [official Root documentation](https://pkg.go.dev/os@go1.27.2#Root) and [Go's traversal article](https://go.dev/blog/osroot) support choosing this existing traversal primitive instead of recreating it. Root still supplies confinement, not Pinfold's internal protected-path policy.

Required qualifications for the prototype:

- Authorize parent/slot **before** any creation operation. Dropping `O_TRUNC` does not make `O_CREAT`, mkdir or symlink creation non-mutating.
- Keep `fstat`, policy decision and actual WriteAt/Truncate/Chmod in the same broker critical section. Reject unimplemented SFTP extensions; do not delegate them to unrestricted filesystem defaults.
- `F_GETPATH` is a current observation. Its errors, deleted objects or ambiguous paths must fail closed. It must not be the justification for escaping Root's confinement or for a pathname reopen. A snapshot of bare inode numbers needs protection against reuse; retaining references or revalidating generations is a later implementation concern.
- Preserve immutable maximum rights on every retained handle. If a read-only/protected object loses a name, it does not thereby acquire write authority. A protected subtree snapshot covers this fixture more strongly than a pure no-inventory policy: the evidence must name that fact.
- Fail-closed FSETSTAT is correct when the callback loses handle identity. Emulating it by looking up an old pathname changes its target and creates a different race. Record the resulting client compatibility limitation before proposing a minimal library callback change.
- Absolute source symlinks are a compatibility limitation of Root even when their target happens to be inside the intended host tree. Do not describe symlink support as transparent without checking the actual repository's usage.
- A mutex shared by SFTP clients of this broker does not cover another broker or direct writable share. The fixture may have one broker; concurrent production boxes need an ownership/coordination rule.

The next review should inspect the actual request dispatch and provider code, especially default handlers, creation flags and the use of returned descriptors. Passing adversarial fixture requests cannot compensate for an unexamined alternate mutation callback.
