# Independent adversarial review: linked worktree research

2026-10-15. Read-only review of the user's request, current primary architecture/source, the five named historical reports, and `case_probe.py`/`case-probe.json`. No shared runtime operations or production edits were performed by this reviewer.

## Decision

Case-sensitive worktree storage is a credible smallest *storage compromise*, not demonstrated safe worktree support. It preserves direct source edits, avoids recurring copies, retains the VM boundary and can preserve existing staged/dirty state through a deliberate one-time move. It abandons the requested existing case-insensitive placement. The recommendation must say so clearly and seek approval before adoption.

The current evidence supports pursuing a tightly restricted layout. It does not support enabling worktrees with only a filesystem case check and `--read-only-path`. In particular, the retained probe demonstrates successful protected-pointer mutation through a host-created alias during the live run. Admission and trusted-host assumptions remain material security conditions.

An isolated persistent copy is a more expensive workflow compromise, but remains the stronger fallback when existing storage placement cannot change. It replaces transparent host editing with explicit synchronization/import and needs a real conflict/recovery design. It is not automatically secure because it has a private `.git`.

## Findings against the current evidence

1. **The hook-alias fix is not in the current primary implementation.** `crates/pinfold/src/pi/git.rs` still returns `None` for an absolute lexical outside-root hooks path before resolving it. The current source HEAD is `198600b1751d889d354ed3657d8f839e8436e469`, with unrelated dirty files. The measured-build report's B1 is a separate candidate, not a property of this current primary tree. A new worktree recommendation depends on resolving this known bypass, including missing leaves and symlinked ancestors.
2. **The hardlink probe is a live host mutation.** The script creates `preexisting-alias` after starting the box, writes through it successfully, then restores `.git`. It demonstrates the lack of continuous identity enforcement. It is not an admission-refusal experiment. Keep a separate true-preexisting-alias case that exercises actual admission before runtime start.
3. **Host execution observation is narrower than it appears.** The protected-file fingerprints before/after the first attack group are useful. The subsequent host commit runs after pointer restoration and forces `core.fsmonitor=false`. This does not show that normal host Git safely consumes all post-attack state. Each important denied group should be followed, before repair, by normal host `status` and an appropriate harmless hook operation. A known-unsafe alias case should be isolated and clearly expected to fail integrity.
4. **Only root-level protected directories were tested.** The configured `protect` list permits nested directories. A writable ancestor rename/replacement can change which host name a guest mount protects. This deserves the next decisive probe before a larger benchmark.
5. **No ordinary Git lifecycle/concurrent-box evidence yet.** The probe proves one in-place source edit becoming visible, not host atomic-save visibility, live staged/index changes, host commits while the guest reads, overlapping boxes, or pointer changes from worktree maintenance.
6. **The runtime result is native Apple Container evidence, not Pinfold integration.** The command has no Pinfold admission, Pi plan discovery or ready event. `container run -d` elapsed time should not be labelled Pinfold readiness. No Linux inference follows from this result.

## Smallest decisive additional cases

Run these sequentially, with host bytes/identity observers and normal-write controls. Refuse a layout when an essential case fails; do not repair it by collecting more literal path spellings.

| Priority | Case | Why it decides something |
|---|---|---|
| 1 | `dir/hooks` and `dir/editor` protected; guest renames `dir`, replaces it with a new directory or symlink, then writes protected names through both old/new paths | Tests the allowed nested-protection topology, rather than just the protected mountpoint |
| 1 | Existing `.git` hardlink in an ordinary writable file and in a second writable export; run actual admission | Separates effective refusal from a manual fixture assumption |
| 1 | Same worktree exported under two guest paths; metadata root also under a writable ancestor export; case-insensitive common metadata with uppercase aliases | Rules out reopening the known bypass through another export even though the worktree itself is case-sensitive |
| 1 | Real host hooks path aliases into the case-sensitive worktree, including an external symlinked parent and missing leaf | The existing B1 defect is independent of `.git` case sensitivity |
| 2 | New symlink and hardlink attempts to the pointer and protected descendants, through each admitted guest path | Ensures alias rejection is not limited to the one direct path tried |
| 2 | Common Git metadata on existing case-insensitive storage, exported only read-only; per-worktree config, `commondir`, index/HEAD and effective hooks verified | Tests the likely practical migration layout; common metadata need not be moved if never reachable through any writable export |
| 2 | Host atomic source replacement, stage/unstage and commit while guest reads; staged/unstaged/untracked bytes observed separately before/after | Defines useful supported host interaction and tests bind/export freshness after Git replaces files |
| 2 | Two independently configured boxes with the same project and protected set; different linked worktrees sharing common metadata | Demonstrates that concurrency does not silently introduce a writable view |
| 3 | Inject a host rename/symlink/hardlink between discovery, alias scan and runtime launch | Establishes exactly which race remains excluded; a final recheck alone does not make the interval atomic |

A failed priority-1 ancestor or overlap case is a reason to narrow the layout or reject direct sharing. It is not a reason to call startup scanning continuous enforcement. Unsupported submodules, symlinked administrative paths, alternate object stores and malformed pointer/commondir graphs may be refused explicitly for the first implementation.

## Trusted-host assumption

A trusted host is already implicit, but "no topology mutation" is too broad to be usable: normal editors save by rename, and Git replaces its index and refs. Define the forbidden set rather than freezing the entire checkout.

A potentially acceptable explicit rule is:

> From before discovery until the last relevant box stops, trusted host processes must not change export roots, the gitfile, protected-directory placement or ancestors, effective hooks/configuration destinations, or create a writable alias to a protected inode. Ordinary source edits and caller-owned Git operations that do not change those relationships remain allowed.

This is a proposed conditional guarantee, not present continuous enforcement. It requires a practical list of allowed operations. `git worktree move/remove/repair`, changing `core.hooksPath`, changing configuration includes, moving protected directories and creating hardlinks across the boundary must stop relevant boxes first. A worktree lock is advisory Git maintenance coordination, not protection against an editor or `ln`.

Host staging and commits can plausibly remain allowed because an entire metadata directory is read-only in the guest, but this needs observation after atomic replacements. Host checkout/switch may change tracked protected configuration; classify it separately. A second box is untrusted, not a "trusted host mutation" exemption: its every writable view must satisfy the same boundary.

This constraint is acceptable to offer as a documented compromise if the decisive admitted-layout attacks fail and the user accepts storage placement and maintenance restrictions. It cannot be silently introduced under the current unconditional statement that the box never writes host `.git`. If ordinary supported guest operations can create the alias themselves, the proposal fails regardless of host trust.

## Isolated copy/import fallback

A private working copy avoids exposing the original writable gitfile. Preparing it once per task can amortize checkout/history costs across launches. It still changes the workflow substantially:

- Copying only HEAD loses unstaged, staged, untracked and ignored working state. Preserve the source index baseline separately from filesystem content. Import working files without changing the original index unless explicitly requested; a patch from HEAD cannot represent that distinction.
- Local clone defaults may hardlink objects. Borrowed objects depend on host retention. Private object storage or deliberate read-only history access must be explicit. A CoW file clone is different from a hardlink; inspect identities.
- A live lower filesystem is not a safe OverlayFS snapshot. It needs a stable snapshot or a documented restriction on host changes. Preparation itself can observe a mixed state without a stable source interval.
- A per-file baseline hash detects host edits before the check. It cannot prevent a host save between check and replacement. Atomic rename is atomic replacement, not compare-and-swap on contents. A safe automatic importer needs a brief exclusive application interval honored by every host writer, or it must emit conflict/review artifacts instead of overwriting the host path.
- Bind application to an exact project identity and baseline; reject traversal, case/Unicode equivalent protected names, protected ancestors, metadata destinations and symlink redirection. Use descriptor-relative operations with no-follow checks. Existing hardlinked ordinary destinations should be replaced, never truncated through the inode. A source-controlled archive must not be unpacked directly over the repository.
- Creation, deletion, rename, binary contents, executable mode and symlinks need explicit semantics. "Git patch" is not a complete plan for ignored/untracked files and filesystem metadata.
- Multi-file import is not a filesystem transaction. Persist original/new identities and durable progress before applying. Recovery must be idempotent and refuse rollback over later host edits; preserve both versions. Killing after the first rename is a decisive recovery test.
- Concurrent boxes require separate baselines and conflict detection. A shared persistent private copy reintroduces concurrent-agent contamination unless ownership is serialized. Reusing it across launches also retains any untrusted build outputs/configuration it contains.

These costs argue against building a general sync subsystem solely to avoid a one-time storage move. If no sound direct layout survives the small cases, a persistent isolated task copy with a deliberately serialized reviewed import is easier to specify than a transparent live merge service.

## Review disposition

Provisional support for case-sensitive direct-edit research as the smallest compromise. No approval of production worktree enablement from the current evidence. Address priority-1 cases, label the hardlink/host observer evidence accurately, state the permitted host operations precisely, and retain explicit Linux/Pinfold-integration gaps. No runtime resources were created by this reviewer; only this Markdown file was written.

## Additional review: normalization aliases

The main investigator identified APFS normalization equivalence as a separate risk from case folding. Test it before treating case-sensitive storage as an alias-free namespace. A protected path containing a composed/decomposed Unicode component can have multiple guest spellings for one host entry.

Merely limiting protected names to ASCII may be insufficient. A local Python `unicodedata.normalize('NFD', character)` scan found three non-ASCII characters whose canonical decomposition is wholly ASCII: U+212A KELVIN SIGN becomes `K`, U+037E GREEK QUESTION MARK becomes `;`, and U+1FEF GREEK VARIA becomes the ASCII grave accent. This is a source of adversarial test inputs, **not evidence that APFS implements exactly Python's normalization rules**. Probe a protected ASCII `K/hooks` through the Kelvin-sign spelling as well as a non-ASCII composed/decomposed path.

A narrower first-support rule could accept only lowercase ASCII letters, digits, dots, underscores and hyphens in protected relative components, subject to real alias probes. `.git`, `.vscode`, `.claude` and `.idea` fit; arbitrary custom `protect` and hook directories may not. Reject unsupported names explicitly; do not lowercase or rename them. This is another workflow restriction to report. Parent components outside an individually mounted export are guest filesystem paths, but a writable export of an ancestor can expose their host normalization semantics again. Hence protection applies to the complete effective export graph, not just the selected worktree's nominal filesystem type.

## Follow-up review after native runtime evidence

The reviewer read `ancestor_probe.py`, `ancestor-probe.json`, `case_followup.py` and `case-followup.json`. No additional runtime was started by this reviewer.

### Newly established facts

- A protected `config/hooks` leaf does not prevent guest rename of its writable `config` ancestor. The dedicated leaf-only variant renamed the ancestor, installed a replacement hook at the original host name, and a real host commit wrote the fixture's `guest-hook` marker. This is a concrete host-hook redirection failure, not merely a hypothetical rename concern. The hook fingerprint at the moved inode does not save the original path's integrity.
- Binding the immediate ancestor as a separate writable guest mount blocked its rename with `EBUSY`. Its ordinary source sibling remained writable. Replacement hook writes failed, the host hook bytes remained trusted and a real host commit wrote `trusted`.
- A second writable export at `/other` bypassed protection at `/workspace/.git`; host pointer bytes changed. Such a layout must be refused or protected in every view.
- Two native boxes exchanged ordinary source writes. A host empty commit advanced HEAD and the new HEAD was visible through guest Git. This supports basic live metadata visibility. It does not test staging/unstaging, a deliberate index replacement, host atomic source saves, or a worktree-maintenance operation. The scripts force `core.fsmonitor=false` for these host commits; the hook observations remain valid, but they are not unrestricted host-configuration tests.
- The tested composed/decomposed `hóoks` and ASCII `K`/Kelvin-sign directory spellings returned `EROFS`; host bytes remained unchanged. These are useful negative results. The preliminary normalization concern did not reproduce. Do not impose an ASCII restriction on the basis of that concern alone. The results are not an exhaustive APFS equivalence proof and concern protected directories, not every possible file-mount or export arrangement.

### Assessment of ancestor mount pinning

Ancestor pinning is a plausible narrow use of the existing runtime capability. It need not make source siblings read-only. The proposed rule must pin **every** writable ancestor from the protected directory's parent up to the worktree export root, not only the immediate parent. For `a/b/hooks`, binding `a/b` while leaving `a` unpinned still leaves an ancestor that can be moved. The worktree export root is already a mountpoint.

This adds mount-plan machinery and one writable bind per distinct ancestor. It is no longer a one-flag worktree change. Deduplicate shared ancestors and include their runtime setup/admission costs in the recommendation. Pinning prevents guest renames through that mount namespace; it does not pin the host name against trusted host moves, or cover another box/export whose corresponding ancestors are unpinned.

The leaf-only failure also concerns existing nested protected directories, independently of linked worktrees and case-sensitive placement. The native reproduction is not yet a reproduction through Pinfold's CLI, so report it as a reproduced runtime mechanism plus a source-relevant production concern until the real CLI path is exercised.

### Admission is the remaining decisive boundary

The current mount check observes regular-file identities through directory exports. `--read-only-path` on a gitfile is not presently a first-class protected file in the box schema. A prototype planner must supply its identity and effective protection to admission; simply adding the runtime flag bypasses that part of the model.

Existing-file identity checking cannot by itself exclude every dangerous overlapping export. An empty protected editor or hooks directory has no regular-file identities to compare. An additional writable view can let the guest create a new protected file there. Even a protected directory currently containing files can gain an unobserved alternate path through an empty shared parent. The first support boundary should structurally reject additional duplicate, ancestor or descendant writable host exports that reopen a protected location, except the planner's intentional in-place ancestor pins whose descendants remain masked. Alternatively it needs an effective-view algorithm that proves each alias view receives the same protection. Do not describe the existing scan as already proving this.

The same issue applies to case-insensitive common metadata under a separately writable parent export. Exporting only its dedicated metadata root read-only is a reasonable narrow initial layout. Any common/per-worktree metadata or effective hooks path reachable through another writable export must be rejected, whether or not current regular-file identities happen to expose the overlap.

### Smallest remaining experiments and gates

1. **Depth-two ancestors:** protect `a/b/hooks`; pin `a` and `a/b`; attempt rename/replacement at each depth and modify ordinary siblings at each depth. Observe the original host hook and actual host hook marker. This is the most direct remaining runtime check for the proposed mechanism.
2. **Empty protected directory with alternate export:** create an empty `.vscode` or empty effective hooks directory, expose the worktree twice and attempt file creation through the unprotected view. Then demonstrate an actual planner/admission refusal of that layout and success without the alternate export. This tests the protection of future files, not just existing aliases.
3. **True admission before startup:** a preexisting pointer hardlink and an overlapping extra export must refuse through the candidate's real admission path; removing them permits ordinary writes. Include an absent configured protected directory. A manual choice not to launch the known-unsafe fixture is not admission evidence.
4. **Host atomic source save plus stage/unstage:** observe exact working bytes and index contents from host and guest separately. The existing empty-commit observation is useful but does not replace this. Keep worktree move/repair and effective protection changes outside the active-run contract.

Pinfold integration, Linux runtime validation, malformed topology and the discovery-to-use interval remain explicit production gates. For a research recommendation it is defensible to leave these as gates while recommending the direction, provided the report does not call the prototype an implemented safe feature.

### Updated disposition

The added evidence strengthens case-sensitive storage plus ancestor mount pinning as a practical candidate and concretely rejects leaf-only nested protection. I support recommending that constrained direction for approval over a new synchronization subsystem. Approval should be for the proposed support contract and follow-up implementation work, not a claim that all listed security guarantees already hold. Keep existing worktree refusal until admission and real platform gates are complete.

## Final documentation consistency review

Reviewed only the completed `deep-probe.json`, `migration-probe.json`, `core-boundary-probe.json`, `linux-probe.json`, `isolated-probe.json`, `apply-crash.json` and `apply-race.json` for this final pass. No new experiments were requested or run.

**Disposition:** the evidence now supports recommending constrained case-sensitive storage with direct editing, while retaining current worktree refusal pending implementation. It does not demonstrate a safe complete-worktree solution on the requested existing case-insensitive volume. This storage compromise must remain explicit.

Evidence gaps addressed:

- Both levels of a protected path's writable ancestors were pinned; guest renames failed while an ordinary sibling write succeeded. This addresses the identified depth-two mechanism gap.
- Empty-directory overlap was demonstrated unsafe. The baseline-built Pinfold core emitted `ready` for both the leaf-only and empty-overlap arrangements; guest changes reached host paths. Its explicitly pinned arrangement preserved the trusted host value. These are real Pinfold **core box** observations, not a completed Pi worktree integration or an implemented structural-refusal test.
- Native Linux Podman 6.1.4 observations agree on pointer protection, new-link refusal, both pinned ancestors and ordinary writes. Host atomic save, staging and commit were visible; guest reads preserved the index. The live host-added alias still changed the pointer. Linux evidence therefore supports the same conditional topology boundary rather than unconditional identity protection.
- Mac atomic replacement and staged-diff visibility are now observed. The small migration fixture preserved staged/dirty/untracked/ignored state and its index while relocating working files to another device and repairing the host backlink. This supports migration feasibility; its roughly 21 ms copy-plus-repair result is only a tiny-fixture measurement, not representative migration performance or proof for every Git layout.
- The isolated-copy probe preserved original protected files/index, supported a selected ordinary-file import and refused an already-present content conflict. A new host file remained invisible in the copy. It does not establish a safe general importer.
- Real Git apply interruption left 19 of 10,000 target files missing; a retry failed. A separate application lost a host atomic save made after Git read the target, despite exit 0. This directly substantiates the recovery and check-to-write race concerns. Do not describe ordinary `git apply` as transactional, crash-recoverable or safe with unrestricted concurrent host writes. Quiescence addresses concurrent writers but does not by itself provide crash recovery.

Remaining material blocker: the proposed admission/protection plan is not implemented or validated as one end-to-end feature. File-aware admission, structural overlapping-export refusal (including empty paths), effective hook resolution, malformed-layout handling and the discovery-to-use assumption remain production gates. Existing regular-file hardlink admission is necessary but does not substitute for those gates. Protected topology must remain stable from before discovery through the relevant boxes' lifetime; permitted ordinary source edits and caller-owned staging/commits should be stated separately.

The smallest proposed production scope is a restricted-layout planner and corresponding runtime file protection, using whole-directory metadata exports, all required ancestor binds, structural overlap refusal and existing identity checks. Include the known effective-hook alias correction. Extend the existing guarantees/tests and document the explicit storage and trusted-host conditions. Do not add a general exporter, sync/import service, alternate runtime or orchestration layer to this first change. The current refusal remains until those changes and required platform gates pass.

The fallback description is consistent if it says persistent isolated task copy plus quiescent, reviewed application **with explicit recovery**, and admits loss of seamless host visibility. The final recommendation is ready for the user's decision about direction; it is not approval to enable the feature.
