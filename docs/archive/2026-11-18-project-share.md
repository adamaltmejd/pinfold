# Mac project shares (#280), PR #330 fix rounds

Recorded on 2026-11-17 with the filename requested by the brief. Base `bbf396e`; initial implementation `aa01770`,
evidence `9f638bf`. Fix implementation `ba09f9e`. All three reviews and FIXES-1 were read. No test, dependency, image,
profile rebuild or release was added in this round.

## Controls and consolidations

FIX1 / Sol1 / Astra1 / Claude C1: share discovery and verification never invoke host Git. Descriptor-relative no-follow
metadata reads select roots, validate linked gitdir/commondir/backlinks, and bound directory-form commondir chains to
the protected root. Cycles, symlinks and outside storage refuse. Disk verification retains host Git. Existing-root mount
comparison precedes verification, including a share whose configuration has since become disk. Bootstrap re-resolution
also checks its expected mount before verification.

FIX2 / Astra2: Probe, Join and Keep compare the current protection layout. Snapshots record mount kinds, paths and
protection definitions, without an alias walk or Git contents, HEAD, refs, index or mtimes. A gained `.git` refuses
grants only while the non-Git owner lives. After stop it is adopted and protected as data, without host Git. The spec
warns against running host Git on such box-written metadata without inspecting it.

Round 1 FIX3–4 / Sol2–3: crossing consulted recorded roots before marker discovery. Round 2 below supersedes that broad
precedence with a share-only refusal. The existing Borrow request checks roots and refuses a running share before
registering a lease. Its separate absent-owner config fallback is retained: a stopped share needs refusal too. The extra
crossing Probe and wire flag are deleted. A running disk still crosses if its file now says share. Discovery is
metadata-only; admission compares roots before parsing current configuration or validating linked pointers. Error
fallback consults recorded roots on both platforms; successful Linux discovery is not skipped.

FIX5–6 / Claude B1–2: one shared protected-path implementation returns the exact host-path-in-project token before
project state. The duplicated project path list and share-only check method are removed. prepare is removed from
validate_project. Inspection count: three alias-admission invocations per new share owner before (client snapshot, owner
snapshot, startup), one after (startup only), zero on joins. Each invocation walks Git then the share. This is a source
call-count inspection, not a timing measurement.

FIX7: no extra Linux launch Probe or copied wait/notice loop remains. A cold Linux launch retains a read-only mount
check, guarded by existing launch/name ownership, so unsupported shares refuse before writes; live owners retain the
single existing roots Probe. Crossing maintenance stays after the owner/config decision, with signal handlers installed,
and skips early refusals. This required timing change is retained. Mac stop checks only its exact active locator; on
discovery failure it reads only that root's state file. The full state scan is deleted; Linux stop retains its original
discovery path. Original disk worktree refusal text is restored. Successful disk clients add no extra metadata traversal
after resolution. Expected mount is returned in the existing private roots reply, with no extra control round trip.

FIX8: lock-wait and path-list duplications are removed. Pi's extension guidance is deliberately retained under FIX8's
behavior exception: an existing native APPEND_SYSTEM.md blocks missing-only seeding, so deleting the extension would
remove guidance coverage. Removing Pi's native seed would break the approved three-agent seed contract. No other
consolidation was skipped.

## Real-binary evidence

Scripts and excerpts are in [the evidence directory](2026-11-18-project-share/). `apple.py` and `edges.py` were rerun on
the fixed binary, using private XDG roots and the existing default image. Every created box, disk and fixture is
removed. [apple.txt](2026-11-18-project-share/apple.txt) covers simultaneous first sessions, two writers, cancellation,
preserved Git bytes/metadata, host-commit joins, nested worktree cwd/Git, external/separate refusal, old-volume
preservation, report shapes, home persistence, damaged-marker stop and statuses. It also shows Mac disk startup
fast-forward, running joins and live push/fetch, including config edited to share. Keep/join/stop checks in that script
are sequential. Ungated leave/join and keep/leave overlaps accept either ordering; they show one box, not a specified
race winner or retention of the same generation.

[edges.txt](2026-11-18-project-share/edges.txt) records exact-token protected-root refusals with corrected controls. A
host Git wrapper observes the old rev-parse verification argv and executes real Git: its positive control fires. Share
startup, join, report, stop and post-stop adoption then record zero host-Git calls. Directory commondir outside the root
refuses before host Git. The old apple fsmonitor case is narrowed to marker absence, not an execution or read trace.
Changed roots precede malformed config and a hostile worktree pointer. A running non-Git share refuses crossing even
with a hostile marker and config now selecting disk.

Join and Keep are deterministic concurrent cases: a real candidate's image inspection helper and parent are stopped
after its metadata read; an already admitted guest writer creates `.git`; the candidate resumes. The sole allowed
outcome is 125/config-changed, with no command execution. Both passed. These exercise grant rechecks, not a new claim
about stop/departure race ordering. The unchanged [lifecycle.txt](2026-11-18-project-share/lifecycle.txt) is the initial
implementation's startup/teardown owner-death and gated orphan-removal record, not a fix-round rerun. Its keep/leave
observation establishes one box only. [guidance.txt](2026-11-18-project-share/guidance.txt) remains the initial
fixture-request delivery record: all three agents receive both seed sections; existing user files survive share-to-disk
and block reseeding. HTTP 400/agent exit 1 establishes delivery only. TTY/transport and durable retirement internals
were inspected; the suspended suite was not run.

[linux.txt](2026-11-18-project-share/linux.txt) is the fix-round real musl binary: default disk config, four early share
refusals with runtime PATH unavailable, maintenance due and unchanged config/state/cache/ownership inventories. The
corrected disk config reports successfully, then names the unavailable image. The dev hostname did not resolve. The
disposable Debian fallback had no image; none was built. Linux running-owner joins/crossing and startup fast-forward
remain unestablished. The earlier Linux copy/push/fetch record is in `9f638bf`; it did not establish those cases and is
not a fix-round runtime validation.

[checks.txt](2026-11-18-project-share/checks.txt) retains build disk checks, fmt, Mac/Linux clippy, call-count
inspection and final cleanup/release. No unrelated resource was removed. Round 1 Mac builds had at least 30 GiB; round 2
disk checks are recorded separately.

Caller changes since main: Mac default share and nested linked-worktree joins; project.mount is added (main had no
project.guest); shares report null project disk and have live edits, no project disk/sync/crossing. Non-Git share
crossing uses share-no-crossing rather than not-git. Layout changes refuse while the owner lives; recorded-root stop
handles damaged markers. New missing-only home files are .claude/CLAUDE.md, .codex/AGENTS.md and
.pi/agent/APPEND_SYSTEM.md, including Linux and Mac disk homes. Existing files block seeds. Retained FIX7 changes are
cold-Linux read-only admission, deferred crossing maintenance, recorded-root error precedence and Mac damaged-marker
stop selection above. No CLI flag/exit-code changes, migration, copy-back, performance workaround, push or PR. The
stronger no-host-Git share control is the operator's FIX1 change.

## Fix round 2: disk crossing

Read FIXES-2 and both named re-reviews. `f581087` closes Astra 1 (root selection), Astra 2 (linked disk worktrees), Sol
(owner mount path rules), and the README qualification: damaged-marker stop recovery is from the recorded main root.

A locator carries an optional mount hint from its verified startup snapshot. Only a possible covering share is queried
before discovery. The existing Borrow request has a nonleasing share-only form: a live Share returns its refusal; a Disk
returns a neutral reply before root comparison and never supplies its name or a lease. The launch lock and locator
reread cover replacement. Known-dead owners are skipped; uncertain liveness fails closed. The hint grants nothing. Old
dead locators remain readable. Updating a live owner across this private protocol change requires a restart;
cross-version live owners are not supported. Normal Borrow omits the false field and keeps the existing Disk wire shape.

Other crossing retains candidate discovery and checks its worktree restriction against the effective mount. A live
crossing lease implies Disk, so validation uses Disk path rules even if the file now says Share. Without an owner, the
file still decides. The unused project-root helper/import were removed. The spec and README changed with the
implementation. No dependency, flag, exit code, suite test, image build or other implementation change was added in this
round.

The retained `edges.py BIN before|main|after` comparison uses explicit Disk projects, private XDG roots and the existing
image. Source snapshots were built from `6444bd3` (before) and `bbf396e` (main); after is `f581087`. Both push and fetch
were exercised for every row:

| Case | Before | Main | After |
|---|---|---|---|
| Running parent Disk, independent child repo | Parent box and parent-only refs | Child box and child-only refs | Child box and child-only refs |
| Linked worktree below Disk, no owner / live owner | Wrongly accepts both | 1/worktree both | 1/worktree both |
| Disk owner beneath state root, file now Share | 1/host-path-in-project | Disk path allowed with supported Disk file | Allowed with file now Share |

Main does not support Share configuration. Its Sol control deliberately retains the supported Disk file; it does not
establish acceptance of the literal edited input on main. Direct-root push/fetch was verified only with the owner running
in each linked-worktree comparison.
Child host remote refs independently confirm the selected repository. Running Share, guest-written hostile marker, and
file edited to Disk each retain 1/share-no-crossing for both verbs before and after. Output excerpts are appended to
edges.txt. The `absent` mode also verifies both verbs use the current Share file without an owner, refusing with
share-no-crossing beneath the state root. These are real-binary observations, not additional suite tests.

After the interruption, the old scratch fixture was absent. The container service was stopped; it was started with
--disable-kernel-install. Inventory then showed only the unrelated stopped buildkit and no volumes; the VM lease was
idle. No interrupted fixture resource remained. The resumed run uses fresh fixtures. The specified development host
still did not resolve. A disposable Debian VM ran the required Linux clippy check on the source-only patch and was
released. Linux running-owner runtime cases remain unestablished as disclosed above; this round requires the Mac
comparisons and both clippy targets. No image was built.

Fmt and native Mac/Linux-musl clippy passed. checks.txt records the sandbox cache failures and task-local workaround.
Every comparison removed its boxes, disks and fixture; the separate absent-owner fixture was also removed. Final native
inventory is only the unrelated stopped buildkit, with no volumes. The VM lease is idle. The container service is left
running. No image was built or removed; the existing default image is unchanged. Older repeated CLI output was shortened
to excerpts to keep this cumulative record within its evidence budget.

## Follow-up runs on the Linux dev VM

The [owner record](2026-11-18-project-share/linux-owner.md) at `6444bd3`
covers joins, startup fast-forward, divergence and crossing. The subsequent
[crossing recheck](2026-11-18-project-share/linux-crossing.md) at `3e7be92`
covers live and transient push/fetch, independent child repositories,
linked-worktree refusals and Linux share admission. The later docs correction
changes no executable code.
