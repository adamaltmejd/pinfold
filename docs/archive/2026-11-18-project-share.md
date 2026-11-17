# Mac project shares (#280), PR #330 fix round 1

Recorded on 2026-11-17 with the filename requested by the brief.
Base `bbf396e`; initial implementation `aa01770`, evidence `9f638bf`.
Fix implementation `ba09f9e`. All three reviews and FIXES-1 were read.
No test, dependency, image, profile rebuild or release was added in this round.

## Controls and consolidations

FIX1 / Sol1 / Astra1 / Claude C1: share discovery and verification never
invoke host Git. Descriptor-relative no-follow metadata reads select roots,
validate linked gitdir/commondir/backlinks, and bound directory-form commondir
chains to the protected root. Cycles, symlinks and outside storage refuse.
Disk verification retains host Git. Existing-root mount comparison precedes
verification, including a share whose configuration has since become disk.
Bootstrap re-resolution also checks its expected mount before verification.

FIX2 / Astra2: Probe, Join and Keep compare the current protection layout.
Snapshots record mount kinds, paths and protection definitions, without an
alias walk or Git contents, HEAD, refs, index or mtimes. A gained `.git`
refuses grants only while the non-Git owner lives. After stop it is adopted
and protected as data, without host Git. The spec warns against running host
Git on such box-written metadata without inspecting it.

FIX3–4 / Sol2–3: crossing consults recorded roots before marker discovery.
The existing Borrow request checks roots and refuses a running share before
registering a lease. Its separate absent-owner config fallback is retained:
a stopped share needs refusal too. The extra crossing Probe and wire flag
are deleted. A running disk still crosses if its file now says share.
Discovery is metadata-only; admission compares roots before parsing current
configuration or validating linked pointers. Error fallback consults recorded
roots on both platforms; successful Linux discovery is not skipped.

FIX5–6 / Claude B1–2: one shared protected-path implementation returns the
exact host-path-in-project token before project state. The duplicated project
path list and share-only check method are removed. prepare is removed from
validate_project. Inspection count: three alias-admission invocations per new
share owner before (client snapshot, owner snapshot, startup), one after
(startup only), zero on joins. Each invocation walks Git then the share.
This is a source call-count inspection, not a timing measurement.

FIX7: no extra Linux launch Probe or copied wait/notice loop remains. A cold
Linux launch retains a read-only mount check, guarded by existing launch/name
ownership, so unsupported shares refuse before writes; live owners retain the
single existing roots Probe. Crossing maintenance stays after the owner/config
decision, with signal handlers installed, and skips early refusals. This
required timing change is retained. Mac stop checks only its exact active
locator; on discovery failure it reads only that root's state file. The full
state scan is deleted; Linux stop retains its original discovery path.
Original disk worktree refusal text is restored. Successful disk clients add
no extra metadata traversal after resolution. Expected mount is returned in
the existing private roots reply, with no extra control round trip.

FIX8: lock-wait and path-list duplications are removed. Pi's extension guidance
is deliberately retained under FIX8's behavior exception: an existing native
APPEND_SYSTEM.md blocks missing-only seeding, so deleting the extension would
remove guidance coverage. Removing Pi's native seed would break the approved
three-agent seed contract. No other consolidation was skipped.

## Real-binary evidence

Scripts and excerpts are in [the evidence directory](2026-11-18-project-share/).
`apple.py` and `edges.py` were rerun on the fixed binary, using private XDG
roots and the existing default image. Every created box, disk and fixture is
removed. [apple.txt](2026-11-18-project-share/apple.txt) covers simultaneous
first sessions, two writers, cancellation, preserved Git bytes/metadata,
host-commit joins, nested worktree cwd/Git, external/separate refusal, old-volume
preservation, report shapes, home persistence, damaged-marker stop and statuses.
It also shows Mac disk startup fast-forward, running joins and live push/fetch,
including config edited to share. Keep/join/stop checks in that script are
sequential. Ungated leave/join and keep/leave overlaps accept either ordering;
they show one box, not a specified race winner or retention of the same generation.

[edges.txt](2026-11-18-project-share/edges.txt) records exact-token protected-root
refusals with corrected controls. A host Git wrapper observes the old
rev-parse verification argv and executes real Git: its positive control fires.
Share startup, join, report, stop and post-stop adoption then record zero
host-Git calls. Directory commondir outside the root refuses before host Git.
The old apple fsmonitor case is narrowed to marker absence, not an execution
or read trace. Changed roots precede malformed config and a hostile worktree
pointer. A running non-Git share refuses crossing even with a hostile marker
and config now selecting disk.

Join and Keep are deterministic concurrent cases: a real candidate's image
inspection helper and parent are stopped after its metadata read; an already
admitted guest writer creates `.git`; the candidate resumes. The sole allowed
outcome is 125/config-changed, with no command execution. Both passed. These
exercise grant rechecks, not a new claim about stop/departure race ordering.
The unchanged [lifecycle.txt](2026-11-18-project-share/lifecycle.txt) is the
initial implementation's startup/teardown owner-death and gated orphan-removal
record, not a fix-round rerun. Its keep/leave observation establishes one box
only. [guidance.txt](2026-11-18-project-share/guidance.txt) remains the initial
fixture-request delivery record: all three agents receive both seed sections;
existing user files survive share-to-disk and block reseeding. HTTP 400/agent
exit 1 establishes delivery only. TTY/transport and durable retirement internals
were inspected; the suspended suite was not run.

[linux.txt](2026-11-18-project-share/linux.txt) is the fix-round real musl binary:
default disk config, four early share refusals with runtime PATH unavailable,
maintenance due and unchanged config/state/cache/ownership inventories. The
corrected disk config reports successfully, then names the unavailable image.
The dev hostname did not resolve. The disposable Debian fallback had no image;
none was built. Linux running-owner joins/crossing and startup fast-forward
remain unestablished. The earlier Linux copy/push/fetch record is in `9f638bf`;
it did not establish those cases and is not a fix-round runtime validation.

[checks.txt](2026-11-18-project-share/checks.txt) retains build disk checks,
fmt, Mac/Linux clippy, call-count inspection and final cleanup/release. No
unrelated resource was removed. All fix-round Mac builds had at least 30 GiB.

Caller changes since main: Mac default share and nested linked-worktree joins;
project.mount is added (main had no project.guest); shares report null project
disk and have live edits, no project disk/sync/crossing. Non-Git share crossing
uses share-no-crossing rather than not-git. Layout changes refuse while the
owner lives; recorded-root stop handles damaged markers. New missing-only home
files are .claude/CLAUDE.md, .codex/AGENTS.md and .pi/agent/APPEND_SYSTEM.md,
including Linux and Mac disk homes. Existing files block seeds. Retained FIX7
changes are cold-Linux read-only admission, deferred crossing maintenance,
recorded-root error precedence and Mac damaged-marker stop selection above.
No CLI flag/exit-code changes, migration, copy-back, performance workaround,
push or PR. The stronger no-host-Git share control is the operator's FIX1 change.
