# Independent concrete broker review

2026-10-15. Source review plus inspection of `guest-probe.json`; no runtime calls.
Reviewed broker source SHA-256: `5879f5e02ee0f2abfc200504039d6086e0179c6728e8a8e5394e2639862d8e4c`.
Results SHA-256: `cd10ef87cbbc89b95523abd08f8ff75ee871c3d64034dee86363f91ea36260f1`.
Paths are under `/private/tmp/pinfold-alias-research-20261015/`.

The result is meaningful real guest evidence for a host-enforced boundary on the existing case-insensitive volume. The tested pointer, hardlink, symlink, editor, namespace and normalization attacks preserved host bytes; root edits, atomic replacement, Git reads and selected live host operations worked. It is still a prototype with a recursive protected-inode snapshot, single-broker policy, restricted operations and the separately acknowledged capability-setup integration gap.

## Corrections needed

### 1. Normalize protected policy from actual opened objects, not EvalSymlinks spelling

`main.go:83` saves `filepath.EvalSymlinks` output in `protected`; `authorize` lines 34–36 compares it lexically against `F_GETPATH`. Go's local `path/filepath/symlink.go` appends ordinary components using their supplied spelling; it resolves symlinks, not APFS case/normalization equivalence.

A configured path such as `wt/DEEP/middle/hooks` can identify the actual `wt/deep/middle/hooks` directory while retaining `DEEP` in policy text. The snapshot protects the existing hook/root inodes, but the ancestor `deep` is not in that subtree snapshot. The ancestor test can miss it because the two lexical prefixes differ. A guest ancestor rename followed by creation of a replacement hook tree then has new identities at the host-trusted location. This is a concrete source-level authorization gap; this exact alternative-policy-spelling sequence has not been runtime-tested by this reviewer.

Small correction: obtain `base` and each protected root's policy path from an opened object's `F_GETPATH`, fail closed on failure, and/or maintain a separate set of actual ancestor identities checked for namespace mutation only. Do not mark ancestors wholly read-only: ordinary sibling contents must remain writable. The existing fixture results use correctly spelled policy arguments and therefore do not settle this case.

### 2. Relative symlink creation is rejected before dispatch

`Filecmd:60` runs `clean(r.Filepath)` before inspecting `r.Method`. For `Symlink`, Filepath is raw link target text, as confirmed by the local pkg/sftp `requestFromPacket` implementation. A relative target fails the absolute-path check. The actual guest result confirms this failure.

Small correction: dispatch Symlink before cleaning Filepath; clean and authorize only the new link name, and preserve target text. Keep target use confined by Root and reject unsupported absolute-link traversal separately. Existing relative symlink reading already succeeded.

### 3. Append requests are silently handled as positional writes

`open:50` ignores `r.Pflags().Append`; `handle.WriteAt:26` writes at the supplied offset. A direct SFTP append request can therefore modify earlier contents. Do not infer proper append behavior from a shell append happy path when the client happens to provide the current EOF offset.

Small correction: either reject Append explicitly for this experiment or preserve an append flag and implement an authorized append operation. If using `O_APPEND`, the wrapper must call `File.Write`, not Go's incompatible `File.WriteAt`. Preserve host atomic-append behavior rather than emulating it with an unlocked size read. The pkg/sftp FileWriter interface explicitly warns about O_APPEND/WriteAt interaction.

### 4. Namespace mutations skip the multiple-link guard

`authorize:37` checks Nlink only when `namespace == false`. A host-created protected inode absent from the startup snapshot can gain a writable alias; content writes correctly fail, as observed, but Remove/Rename of that alias can pass. Unlinking it changes the protected inode's link count/ctime. This is a remaining mismatch if “read-only metadata” includes inode metadata, rather than only content and the protected name. It is not a demonstrated route to rewriting its contents under the current assumptions.

Small conservative correction: apply the regular-file Nlink check to namespace endpoints too. State that ordinary multiply linked files have restricted operations. Directory link counts need different treatment.

## Useful filesystem limits that must remain explicit

- **FSTAT loses the original handle.** The local pkg/sftp v1.13.10 `request-server.go:271–282` dispatches FSTAT as Stat(original pathname). Following host atomic replacement, a retained read handle can return bytes from the old inode while FSTAT describes the new inode. This is a compatibility problem, not a demonstrated protection bypass. The same dispatch pattern for FSETSTAT explains why current fail-closed Setstat is appropriate. A later callback fix should preserve handle identity for both operations.
- **Setstat is deliberately absent.** Creating executable scripts, chmod, timestamps and some truncate/copy workflows will fail. The results support file-content editing, not complete POSIX filesystem compatibility. Retain refusal until a narrowly implemented fd-based mutation path preserves the original handle.
- **Standard Rename and PosixRename are conflated.** Both call replacing `Root.Rename`. If claiming direct SFTP compatibility, implement the non-overwriting standard Rename semantics atomically or report it unsupported. A preceding existence check does not provide an atomic no-replace guarantee against host activity.
- **List opens before checking type.** `Filelist:77` uses blocking `Root.Open` while holding the global mutex. An existing FIFO at a requested directory path can block the entire broker. Open with directory/nonblocking constraints and verify type before enumeration; reject unsupported entries without waiting. The fixture has not tested special files.
- **Writable opens always request O_RDWR.** This unnecessarily refuses a write-only host file. Match requested access when useful; never add a write capability to a read handle.
- **Snapshot cost and lifetime are real.** Every protected descendant is walked at startup, including Git objects. No claim of avoiding recursive metadata inventory is supported by this implementation. Bare identity entries can also become stale/reused; that predominantly causes false denial here because current protected paths still have a path check. Treat production identity lifetime as unresolved.

## What looks correct in this bounded experiment

Creation validates the actual parent before exclusive creation. Existing-file open removes truncation until descriptor authorization. WriteAt reauthorizes its retained descriptor while holding the same broker mutex. Namespace operations inspect source/destination and protected ancestors while serialized. Root prevents symlink traversal outside the fixture root. Unsupported commands and Setstat fail closed rather than reaching an unrestricted fallback. All accepted SFTP connections share the same broker object and mutex.

These observations assume no other writable share or broker can reach the fixture, and that the trusted host does not move protected roots, change effective hook locations, promote guest-writable objects into protected metadata, or move retained directories outside the export. The tests introducing late aliases establish that the particular content-write attempts were denied; they do not make those host relationships atomically controlled.

The guest result's trusted hook marker and preserved index are useful external observers. It does not yet establish adversarial concurrent guest mutation safety, retained-handle semantics, complete opcode coverage, useful executable-bit operations, broad repository compatibility or production Pinfold privilege integration. Correct the small issues above before expanding any claim beyond this bounded result.

Only this review note was created by the reviewer. No runtime resources or source modifications were made.

## Follow-up: broker v2 and trusted startup proposal

Reviewed v2 source and `followup-probe.json`. Protected roots now use descriptor-derived paths; the deliberately mixed-case and normalization policy spellings deny both ancestor renames and protected writes. Relative symlink creation and append now succeed. The regular-file link-count guard covers namespace endpoints, and listing uses directory/nonblocking open. These address the four numbered corrections and the blocking-directory-open issue above. The observed concurrent loop preserved the six protected file identities and bytes; it is stress evidence, not a deterministic proof of every interleaving. Setstat, FSTAT handle identity, standard Rename semantics, recursive inventory cost and cross-broker coordination remain explicit limitations. The base path still comes from EvalSymlinks; canonicalizing it from the opened root would avoid spelling-dependent false refusals.

Source-only disposition on privileged setup: **a viable direction, requiring a spec change and integration evidence, not an upstream-runtime prerequisite**. Current Runtime::exec always names the trusted mounted binary and `init exec --`; this supplies the right choke point. Runtime exec is a newly launched process, so dropping PID 1 alone is insufficient. Every workload exec must unconditionally drop before invoking image code. Host raw runtime commands remain trusted caller operations, provided no runtime control channel is exposed to the guest.

The minimal contract must cover these paths:

- Trusted static setup performs bounded syscalls only. It must not execute image `chmod`, `fusermount`, SSHFS, shells or dynamic helpers while privileged. Replace the existing Apple root chmod path with a trusted, fixed-target operation, preferably descriptor-based and rejecting symlink substitution. SSHFS starts after the drop and remains untrusted from the host broker's perspective.
- Drop PID 1 before its existing proxy relay creates threads. Drop real/effective/saved uid and gid, supplementary groups, effective/permitted/inheritable/ambient and bounding capability sets, disable capability retention and set irreversible no_new_privs. Preserve the privileges needed to complete the drop until their final use. Fail closed on any syscall or postcondition failure. A single-threaded setup avoids per-thread capability divergence.
- UID/GID and setup paths come from trusted host launch configuration. Guest environment, writable files, cwd, PATH, loader variables and runtime-restored box variables must not select privileged code or privileged targets. Later execs must also be safe while an adversarial guest is already running. Setup-only descriptors and namespace/control handles must not leak into the workload.
- Readiness follows completed mounts and verified credential drops. No long-lived root helper or privileged recovery/reconnect process remains. Mount/client failure must not reveal an unrestricted writable host share. Use nosuid/nodev where appropriate; no_new_privs prevents privilege gain through exec, but does not literally remove setuid bits, so retain the separate image-build guarantee.
- Empty CapBnd and no_new_privs do not alone prove that descendants cannot acquire capabilities in a new user namespace. Preserve Linux's existing namespace-blocking seccomp policy and establish equivalent Apple guest behavior. Cover clone/clone3/unshare and relevant namespace joins. The kernel explicitly says [no_new_privs does not prevent privilege changes outside execve](https://www.kernel.org/doc/html/latest/userspace-api/no_new_privs.html). Do not infer all descendant behavior from one process-status snapshot.
- If adding setup capabilities on Linux, audit the generated runtime seccomp filter: capability-conditioned rules may be expanded at container creation and survive later capability dropping. Keeping this setup change Apple-only initially avoids altering the existing Linux launch contract. Otherwise apply an adequately restrictive workload filter after setup.

The decisive integration gate observes PID 1, filesystem client, proxy children, first exec and later exec with an active adversarial guest: all credential IDs/groups, every capability set, no_new_privs, no privileged inherited descriptors, denied privilege recovery, and allowed ordinary edits. The literal startup `--cap-drop ALL` contract must change; a claim that arbitrary image/client code is always unprivileged can remain. This review does not certify the wrapper implementation or current prototype as meeting that contract.

## Final evidence disposition

Read `concurrency-probe.json`, `two-box-probe.json`, `benchmark.json`, `capability-probe.json`, `namespace-probe.json` and `failclosed-probe.json`. This is a useful research stopping point. The recommendation can now be an implementation direction: an integrated Mac host broker with direct editing on existing case-insensitive storage, an untrusted stock SSHFS guest client, and trusted static setup/exec privilege dropping. Keep linked-worktree refusal until implementation and its gates pass. No volume migration or per-launch copy is needed by this demonstrated mechanism.

The added evidence narrows several gaps:

- The namespace-switching worker completed successfully while 79 permitted writes and 221 denials occurred; the pointer stayed unchanged and an ordinary target received writes. This is a stronger positive control than a race whose changer never succeeds, but remains a finite stress result.
- Two real VMs connected to the same broker exchanged ordinary file contents and received 200 protected-write denials with the pointer unchanged. This supports the single shared broker/serialization design. It does not validate two independent brokers or arbitrary overlapping export policies.
- Three benchmark repetitions give median guest-measured times of 0.636 seconds versus native 0.121 seconds for about 100,000 entries, 0.370 versus 0.057 seconds for 100 edits, and 0.216 versus 0.023 seconds for a 64 MiB read. Broker Git status was 0.900 seconds median without a native counterpart. These bounded measurements show tangible overhead (approximately 5.2x, 6.5x and 9.3x respectively), not a transparent-performance claim. They exclude production admission/inventory and cannot establish cold repository startup or representative build performance. Entry counts differed by one in the first native repetition.
- Raw nonroot runtime exec retained the setup bounding capabilities; the nonroot wrapper failed closed. The root trusted wrapper produced uid 501, gid 20, empty supplementary groups, all five capability sets zero and no_new_privs=1 in observed workload/descendant snapshots. The observed SSHFS process had the same drop. Therefore later Pinfold exec must deliberately launch the trusted drop stage with the authority needed to empty its bounding set; merely calling it as the existing nonroot runtime user will fail.
- The disconnected-client write failed with a connection-abort error after an allowed control write, and the host manifest remained unchanged. This supports failure without writable passthrough for that path. It does not establish crash durability, cancellation coverage, reconnect semantics or complete box teardown behavior.

The namespace result corrects the earlier broad phrasing that arbitrary client code could be described as "always unprivileged." Both the native cap-drop-ALL baseline and dropped prototype created a user namespace and gained uid 0/capabilities inside it. The measured wrapper claim is empty capabilities in the original guest user namespace before image code, not empty capabilities in every descendant namespace. This is not evidence of acquiring initial-namespace or host authority, nor a Pinfold CLI defect reproduction. The architecture must either state the namespace scope accurately or enforce the stronger claim; no_new_privs does not resolve this distinction. The result does not by itself defeat a host broker that mediates every write regardless of guest identity.

Moving the implementation to Rust is consistent with Pinfold's security boundary living in its binary, but the Go prototype does not validate russh-sftp or the replacement filesystem traversal code. Handle-preserving metadata operations, UTF-8/path representation, lifecycle cleanup and protocol coverage are new implementation obligations. Preserve explicit gates for protection discovery/admission and identity lifetime, current metadata compatibility gaps, guest client distribution, second-socket multiplexing with egress, shared-broker ownership and overlapping-policy coordination, and Linux support. Do not claim that selecting a Rust SFTP library settles them.

No additional research experiment is needed to choose this bounded direction. Production enablement remains a separate implementation/spec change supported by real CLI boundary evidence. Only review Markdown was changed by this reviewer; no runtime resources were created.
