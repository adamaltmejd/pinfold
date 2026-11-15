# Runtime transport rereview 3, 2026-11-16

Reviewed `f94ef56..8142b3d`; baseline `8142b3d`. Fix commits: `3594d5e`,
`cc1744d`. All three claims were confirmed. None was rejected.
Earlier records are unchanged. No suite test or dependency was added.

## 1. Display does not grant ownership or deletion authority

The claim was valid. disk::attachments retained a first ownership snapshot,
then trusted a second runtime inventory. Linux inventory silently omitted
an unreadable record. With its name lock free, that omission could release
a disk attachment despite unknown liveness.

Runtime::list is now complete and strict. Unreadable records or unknown
Linux VMM liveness return an error. The separate list_display method permits
partial results and unknown rows; its only consumer is public box list.
Targeted inspect reads a single Linux claim strictly. Exec/stat/fetch and
orphan removal use it, preserving unrelated operations without granting
resource authority from partial inventory. Own readiness remains targeted.

The audit also found missing state-directory filtering in attachments.
An absent boxes directory or generation path no longer silently drops its
claim: membership resolves absent paths, then strict liveness decides.
Non-retiring claims retain their disks even after confirmed death, until
down/prune publishes retirement. Complete inventory errors stop disk remove,
clean and prune. Known nonterminal native containers also protect project
state while orphan removal is attempted.

Failed-start disk rollback now follows retirement publication. Linux
preparation error rollback does too. On Apple a preparation helper may have
held a disk, so an attempted helper keeps the claim protected until successful
stop/removal. An error preserves the claim and disks instead of undoing them.
The helper retains its child while waiting; pipe errors cannot consume the
only child handle before cleanup. It reuses the bounded diagnostic drain:
stdout is unused/null, the last 64 KiB of stderr stays in memory and is
escaped when reported. No diagnostic file was introduced.

### Every inventory caller

The search covered Runtime::list, list_display and inspect calls throughout
crates/pinfold/src. ImageService and disk listing methods are separate APIs.

| Caller | Kind and authority after this change |
| --- | --- |
| cli::list | Display only; the sole list_display consumer. Linux may omit unreadable rows and report unknown liveness. |
| cli::present, used by exec/stat/fetch | Target availability; strict inspect. A different claim cannot decide this target's presence. |
| Box::up Apple name collision | Ownership decision; complete native list. Linux startup does not use global inventory here. |
| Apple::down absence check | Removal proof; complete native list after kill/stop/rm. |
| Apple wait_until_running | Readiness; complete native list. Linux finish_start uses only its own strict record. |
| clean::boxes | Shared doctor measurement, clean/project protection and prune inventory; strict list. Unreadable liveness aborts the decision. Known nonterminal rows pin their projects. |
| clean::remove_orphan | Deletion; strict target inspect, full-generation comparison and name lock before down/retirement. |
| clean::prune_leftover_states | Record-less socket sweep uses strict list plus strict records. Per-record removal uses target down under its name lock; unreadable records prevent record-less sweeping. |
| clean::in_use_images | Image retention and image rm; strict list plus Linux image leases. |
| disk::attachments | Disk display, claim/recreate, export, rm, startup undo and project crossing all share strict list and retained claims. Display does not create a separate weaker disk authority. |
| Runtime default inspect | Uses strict list on Apple. Linux overrides with one strict claim. |
| Runtime default list_display | Uses native list on Apple. Linux overrides only the display path. |

### Real cases

A real Linux box wrote and synced a marker to its 1 GiB disk. The owner was
SIGKILLed, the actual VMM's pidfd became readable, and an independent name-lock
probe succeeded. GDB then held disk::attachments after its first claim read
and before the decision inventory. The observer made that owner record mode
000. Real disk rm failed; the entire backing file's SHA-256, device, inode
and size were unchanged. Clean and prune also refused without modifying the
disk or claim. Repairing only record readability, while VMM identity remained
unknown, still refused all three mutation commands.

The observer also removed the complete boxes/state tree by renaming only its
fixture directory. Disk rm refused instead of taking the old empty-inventory
shortcut. All backing bytes remained unchanged. Display reported unknown,
while another real box completed startup, exec, stat and down. Restoring
identity displayed stopped, but the unretired claim still pinned its disk.
Targeted down published retirement and removed the claim; the identical disk
rm then succeeded. These observations also exercise item 3's output contract.

On Apple a generated Pi directory under a seeded symlink made the actual
preparation helper fail. The route used an unused synthetic value and a local
unreached origin; no real credential or login was used. The non-retiring
claim and disk survived; disk rm refused disk-in-use. Targeted down, correcting
the fixture and a same-name real helper/exec/down succeeded, then disk rm did.

## 2. Cache purge retires the surviving builder claim

The claim was valid. purge_build_cache took the cache lock after orphan
pruning, then removed job/vmm without retiring a newly dead builder claim.
It now proves target death and publishes durable retirement under that same
lock before writable restoration or cache deletion. Exact-generation record
cleanup is last. A cache-error fallback that deletes cache.ext4 also publishes
retirement first. Separately leased immutable catalog garbage is not a job's
state and keeps its existing catalog/lease checks.

Two real debuggers supplied external barriers. A real builder had spawned
Firecracker, held its name lock, and retained a complete identity. Clean
passed its orphan phases while that builder was live, then stopped before
taking the cache lock. The observer killed the builder and observed VMM death
through pidfd. Clean acquired the lock and published retiring=true while
job/vmm still existed. It was then SIGKILLed after the entire cache was gone
but before owner-record removal. The surviving owner record remained retiring.
Public down resumed cleanup and a subsequent real image build succeeded.
No sleep, runtime stub or binary hook supplied a pass.

## 3. Document unknown output

The output sentence now permits running, stopped and unknown. Unknown was
already emitted by the previous fix; this pass documents it as part of the
caller contract. The unknown-to-stopped real display control is described above.

## Deletion audit

Before finishing, the deletion search was rerun against the final source:
[deletions.txt](2026-11-16-runtime-transport-rereview-3/deletions.txt).
These are the generation-state, disk and VMM-identity routes, including their
callers. Unexposed scratch creation is distinguished from a formerly running
generation; it has no possible VMM user and is exclusively owned.

| Deletion route | Proof or fail-closed gate |
| --- | --- |
| ownership::remove_record; Claim::prepare old generation; ClaimDrop | Full-generation match and confirmed death/owned reaping or existing retirement; durable owner retirement before sockets/state; owner unlinked last. |
| Claim::prepare fresh state reset | Name lock; previous claim retired/removed; current state is pre-exposure. |
| Box startup abort and Claimed::undo_locked | Join workers, successful owned abort/down or strict target death, publish retirement, disk locks and fresh strict attachments before disk unlink. |
| disk::claim error rollback | Linux pre-exposure retirement precedes undo. Apple attempted-helper errors preserve resources; no retirement/undo from uncertainty. |
| disk::claim unfilled recreation | Disk locks, strict attachments and no unretired prior holder. The new generation has not started a VM or exposed the new inode. |
| disk::remove and project cleanup disk removal | Disk lock, strict complete attachment inventory and retirement/absence of every prior claimant. Unknown/unreadable claims retain the disk. |
| Firecracker::remove_disk and Apple::remove_disk | Raw backends are reached only through those guarded claim/undo/remove routes. |
| Firecracker::create_disk failed creation | Newly created exclusive inode; never exposed to a VM; local rollback. |
| Disk UNCLEAN/UNFILLED marker removal | Owned sync/unmount proof, retired failed-start rollback or guarded disk removal. These are state markers, not liveness substitutes. |
| vm::write_init_disk and mount_image | Fresh generation, before VM spawn, after prior generation retirement. Only private init/mount images and their scratch trees are replaced. |
| firecracker::write_identity | Atomic complete replacement; the child stays gated until file and directory sync. The identity itself is deleted only with retired generation state. |
| builder preparation init tree and dirty-cache reset | Exclusive cache claim; prior generation retired; current phase not spawned. |
| builder phase endpoint reset | Phase-private sockets; previous phase wait succeeded or current phase has not spawned. |
| builder clean_cache fallback | Owned phase termination plus newly published retirement before fallback cache-disk unlink. Failure to publish preserves the disk. |
| builder transaction and abandoned socket cleanup | Cache-name lock and retire_previous proof before chmod/state/socket deletion; finish removes the claim last. |
| Linux purge_build_cache | Same cache lock; retire_orphan proof/publication before chmod/cache deletion; exact-generation owner cleanup last. |
| Sockets failed creation | Current fresh, unexposed staging directory only. |
| Sockets::remove/remove_directory/unlink_at | Full-generation cleanup authorization; generation retirement or strict record-less absence/runtime checks first. Unsafe entries remain. |
| shutdown socket replacement/removal | Private UID/type checks and name ownership; no VM is exposed through it. |
| clean::remove_orphan and record-less sweeps | Name lock, target generation/down and durable retirement; strict inventory for record-less sweeps; errors preserve resources. |
| clean::remove_paths via CleanPlan | Generation paths use the routes above. Project state additionally needs fresh strict project protection and exclusive maintenance ownership. |

The remaining raw matches are run-record retention, immutable catalog/artifact
storage, installer/build contexts, export staging, atomic-write staging and
Git crossing/quarantine files. They are not generation identity trees or
mutable VM disk backing. They retain their strict retention, lease/catalog,
exclusive installation, project-guard or own-uncommitted-scratch checks.
No deletion decision uses list_display. No new bypass or dependency was added.

## Checks, observer limits and caller changes

Scripts and bounded observations are in the matching directory:
[linux.txt](2026-11-16-runtime-transport-rereview-3/linux.txt),
[mac.txt](2026-11-16-runtime-transport-rereview-3/mac.txt),
[checks.txt](2026-11-16-runtime-transport-rereview-3/checks.txt).
Both platforms passed cargo fmt --check and strict all-target Clippy; Linux
used the static musl target on the retained authorized trial VM.

Observer correction: the first cache script found two identical JOB_TIME
lines. Its barrier is now scoped to run_job. The first Apple fixture reached
main mount failure rather than helper preparation; the corrected fixture uses
a generated Pi directory and reached the actual init setup refusal. Those
initial observations supplied no pass. One collapsible-if lint was corrected.

Only inspected: injected wait/pipe/IPC failure outcomes, the cache-error
fallback's forced IO failure, instruction-level Apple crash windows and
storage power-loss durability. The full terminal suite was not repeated.
Earlier unrelated unverified cases remain unverified. All new cases removed
their boxes/disks; the new probe image image-probe-cache-purge-19473 is retained.
No Mac image was created, rebuilt or removed. No release, push or PR.

Caller changes: state=unknown is documented. Dead non-retiring claims keep
disks pinned until down/prune retires them, including different-name reuse,
export and crossing. Preparation-helper errors preserve claims/disks for
checked cleanup; their stderr is bounded to the last 64 KiB in memory and
escaped. CLI flags and JSON fields are unchanged. Partial display is never
used as ownership authority.

Final free disk: Mac Data 33 GiB; Linux 25 GiB. The active trial lease
was retained. Only BRIEF.md and PLAN.md remain outside the committed work.
