# Runtime transport rereview 2, 2026-11-16

Baseline `f94ef56`. Fix commits: `0bc2195` and `41b7feb`. Both review claims were confirmed against the source.
No claim was rejected. Earlier records are unchanged. No suite test,
dependency, release, push or PR was added.

## 1. Retirement survives deletion of the identity tree

The claim was valid. remove_record removed the state tree before unlinking
its owner record. Builder cleanup also removed that tree before finish.
A crash in either interval left a claim with no VMM identity, so the newly
strict liveness gate could never authorize recovery.

The private owner record now carries a retiring flag bound to its complete
generation. Once removal is confirmed under the name lock, the flag is
written by atomic file replacement, file sync and host-directory sync before
any generation deletion. That owner record lies outside the deleted tree.
It is unlinked last, followed by another directory sync. Recovery accepts
matching retirement evidence instead of re-reading the deleted identity.
A fresh generation starts with retiring=false. Missing identity alone still
proves nothing.

Successful owned box reaping supplies direct death proof. An already-ended
VMM still has its stored wait result checked before teardown succeeds. Orphan cleanup
checks the recorded VMM identity. Builder startup confirms and durably retires
any old claim before restoring permissions or cleaning abandoned sockets.
Builder completion checks identity again: a wait error is not proof of death.
Failed Linux startup with no returned child also checks runtime liveness
before undoing disks or marking retirement. A present held generation must
match the current record. The in-memory retirement flag is updated only after
successful durable publication. ClaimDrop's duplicate state deletion is gone.

### Crash-state walk

This covers persistent transitions, including interrupted filesystem calls.
All rows hold the generation's name lock until the process dies or cleanup
finishes. B means box; J means a finite builder job. The previous rereview
records the startup gate cases; this pass exercises the new deletion windows.

| Point | State left by a crash | How the name recovers |
| --- | --- | --- |
| B/J before claim/state creation | No published claim; no child can exec | Next holder prepares normally. |
| B/J state or shutdown socket created, initial identity incomplete | No published owner record; no VMM was cloned | Fresh preparation clears unfinished state and the stale shutdown socket. |
| B/J owner-publication staging write | An unused staging file may remain; either no claim, or a complete claim with durable not-started identity | Absence follows the prior row; a published claim passes the pre-start gate, records retirement and cleans up. |
| B/J attachment/description publication | Atomic old/new owner record, still before exec; disk dirty markers retain their meaning | Name-locked cleanup confirms pre-start/death and records retirement. No partial JSON is published. |
| B/J generation socket staging/marker/bind | Published claim and possibly incomplete authorized socket directory | Confirm pre-start/death, mark retiring, then resume full-generation socket cleanup. Unverifiable shortened directories remain unchanged. |
| B/J clone before PID identity replacement | Durable not-started identity, or the preceding builder phase's dead PID; new child behind gate | Owner death closes the pipe/kills the child. No new child can exec. Confirm that recorded state, then mark retirement and clean up. |
| B/J PID temporary write | Published identity unchanged; temporary file may be partial | Same gated recovery. The temporary file is removed with state after retirement. |
| B/J PID rename, sync, or before release | Complete PID/start time; child cannot enter jail until sync and token | Owner death ends the child. Confirm that exact PID ended, then durably retire. |
| B/J after release, jail entry, exec or READY | Complete durable identity; parent-death supervision active | Confirm death under the lock, publish retirement, remove resources. |
| J between phases | Last phase's complete dead PID; no next child released without a new durable identity | The next holder confirms it and retires before modifying old cache state. |
| B/J during shutdown, before successful wait/death proof | Identity retained; retirement flag false | A still-live or unknown VMM refuses cleanup. After confirmed death, publish retirement. |
| B/J death proved, before retirement staging/rename/sync completes | Identity still intact; atomic owner record is either old or retiring | Reconfirm death if old. If retiring, republish and sync that matching evidence before deleting anything. No deletion precedes successful sync. |
| B/J retirement durable, before first socket/state deletion | Matching owner record retiring=true; generation tree intact | down/prune/start or the next build resumes deletion. Exercised for both B and J. |
| B during socket deletion | Retiring record outside tree; full-generation socket staging/retiring names remain | Resume checked socket deletion. A preserved unverifiable directory does not keep the dead claim alive. |
| B/J during state removal, including removal of vmm | Retiring record remains; state may be partial or absent | Missing identity is irrelevant because matching death evidence survived. Resume removal. |
| B state gone, before shutdown/owner unlink | Retiring owner record, no identity tree | Public prune resumed and the same name executed successfully. Exercised. |
| J state gone, before socket cleanup/claim.finish | Retiring owner record outside the cache job tree | Public down resumed cleanup; the next real build succeeded. Exercised. |
| B/J shutdown socket gone, before owner unlink | Complete retiring record remains | Resume idempotent deletion, then unlink the owner record last. |
| B/J owner unlink, before host-directory sync | No visible claim; any record that could reappear after storage loss was already retiring | A fresh name starts; a reappearing retiring record resumes deletion. Power-loss behavior was inspected, not tested. |
| B/J cleanup complete, name lock still held | No claim/resources; lock file remains stable | Process death releases the lock. A new full generation starts. Box after-unlink return was exercised. |
| B/J failed preparation or Drop | Published matching claim uses the same retirement gate; unpublished state has no executable child | Preserve a running/unknown published claim; next preparation clears unpublished state. |
| J wait/startup cleanup error | No returned child or failed wait does not grant retirement authority | Recheck known pre-start/dead/retiring state; alive or unknown preserves resources. |
| B/J external identity corruption without retirement | Unknown, with owner record retained | Refuse targeted cleanup. Restore trustworthy identity/death evidence. Never infer death from missing bytes. |

New Linux GDB cases killed the real owner after durable retirement and before
first deletion, after state removal but before owner unlink, and after the
owner-removal function returned. A corrected same-name box executed after
public down, prune or direct start recovery. Two real builder runs were
killed after retirement before deletion and after state removal before
finish. The next real builds succeeded; the state-gone case also exercised
public down of the builder claim. A separate real VMM was SIGKILLed at
ready while caller stdin stayed open. Its owner reported exited/status 137,
retired the claim and state, then a same-name exec/down control passed. The observer read retiring=true and the
expected state/owner presence at every barrier.

On Apple, a real owner reached ready. A fixture file plus a read-only state
directory forced deletion to fail after runtime removal. The matching owner
record remained retiring=true and the native container was absent. Repairing
only that directory's permissions let public down resume deletion, followed
by a fresh same-name exec/down control.

## 2. Own readiness and independent inventory

The claim was valid. finish_start used the global list, and one vmm_state
failure propagated out of that list after another guest had already said
READY. Startup now inspects only its own record and liveness.

Inventory decodes and reports errors per record. After a liveness error it
re-reads that exact claim: absent, replaced or retiring observations are
skipped. Persistent unknown liveness is returned as running=false and
stopped=false. This retains its disk claims and lets other inventory continue.
The public state value is unknown, rather than falsely reporting stopped.
Targeted cleanup still refuses unknown liveness. Unreadable ownership JSON
remains fail-closed in consumers that must know its disk attachments.

One real Linux owner was killed and its identity was made unreadable at a
GDB barrier in another box's finish_start, after that guest's READY. The
healthy box reached public ready, executed and tore down with status 0.
Inventory reported the first box as unknown; its disk removal refused
as disk-in-use. After repairing the identity, down and disk removal succeeded.

A second GDB case held a real inventory snapshot, then another process
completed down of one snapshotted generation. Its state and claim were both
absent before the observer resumed. Inventory rechecked and skipped it without
an unknown-liveness error. Another live box remained listed and answered
exec. A fresh same-name box then executed and tore down successfully.

## Commands, checks and limits

The previous disposable VM lease expired before this pass. A fresh authorized
lease was acquired. Its tools, KVM group access and baseline checkout were
prepared, then source changes moved as a patch and the static musl binary
built. No prior dirty checkout was overwritten. A new minimal probe image
uses Alpine 3.22 and creates /data; no real credential was read or used.

```sh
python3 retirement.py "$BIN" "$IMAGE"
python3 isolation.py "$BIN" "$IMAGE"
python3 ended.py "$BIN" "$IMAGE"
python3 apple-retire.py "$BIN" "$IMAGE"
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --locked --target x86_64-unknown-linux-musl -- -D warnings
```

The scripts and bounded output live in the matching directory:
[linux.txt](2026-11-16-runtime-transport-rereview-2/linux.txt),
[mac.txt](2026-11-16-runtime-transport-rereview-2/mac.txt),
[checks.txt](2026-11-16-runtime-transport-rereview-2/checks.txt).
Cases use isolated state/config roots, fresh maintenance stamps, the real
cache, debugger instruction barriers and readiness/pidfd events. No sleeps,
fake runtime or test-only binary path supplied a pass.

Observer corrections: the first retirement GDB run omitted CLI arguments
and exited at help without reaching a barrier. Its invocation now supplies
those arguments. The first snapshot observer filtered on an unavailable Rust
pretty-printed name and never triggered; it now holds the first per-record
liveness observation after the complete inventory snapshot. The Apple
observer initially expected a deletion error on stderr; the real public box
API reports it in failed JSON on stdout. Each was corrected and exercised;
none of those failed observations supplied a pass. The owned-exit observer
also initially closed stdin, making EOF compete with VMM death, and guessed
the wrong public reason token. It now keeps stdin open through the actual
exited event and checks status 137 before the corrected control.

Only inspected in this pass: startup intervals already covered by prior
records, a partial recursive deletion beyond the new end-of-tree barriers,
retirement rename-to-directory-sync and owner-unlink-to-sync intervals,
wait/PIDFD/channel/IO failure injection, signal-only versus EOF-only child
exit, instruction-level Apple crash windows, and storage power-loss behavior.
The full Apple terminal suite was not repeated. Its changed ownership path
was exercised with a real runtime and an actual deletion failure/control.
Earlier unrelated unverified cases remain unverified.

Caller changes: matching durable retirement evidence resumes interrupted
cleanup; unknown Linux liveness is reported as state=unknown and retains
disk ownership; unrelated liveness failures no longer fail startup or global
inventory. No CLI flags or JSON schema fields were added. New private owner
records include the retirement bit. Fixture boxes and disks were removed.
Probe images are retained on the new trial VM; no existing Mac image was
created, rebuilt or removed. The new lease remains active. No push or PR.

Final free disk: Mac Data 34 GiB; Linux 26 GiB. Retained image aliases
are listed in checks.txt, alongside their successful build observations.
