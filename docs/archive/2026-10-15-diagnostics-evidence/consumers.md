# Consumer investigation, 2026-10-15

Read-only source review at `<checkout>`, HEAD
`198600b1751d889d354ed3657d8f839e8436e469`, including existing dirty files.
No build or runtime experiment performed by this agent. Parent coordinates them.
Disposable adapter: `/private/tmp/pd-1015/consumer.ts` (host-only, no dependencies
beyond Bun/Node stdlib, 145 lines at creation).

## Pi Durable hypotheses

All source references below are relative to `<checkout>`.

`examples/pi-durable/prototype.ts:72-89`: helper pipes both streams and
counts their combined bytes against 1 MiB, but retains only stdout. Failure
throws `pinfold box list failed (1)` style text with status, not stderr or
stdout. Its timeout and output overflow both kill the child, and status alone
does not distinguish the two. This is consumer loss, not missing binary data.

`prototype.ts:128-150`: owner's stderr is inherited, not discarded by the
process. It can reach a terminal/outer log but is not attached to the error.
It reads JSON until `ready`; `refused`/`failed`/`down` become an exception
containing event and reason, dropping `detail`. On success it returns and
closes readline. Thus later terminal events are not interpreted or retained.
`#closed` (`130`) resolves for any close regardless of code or signal.
`ready.labels` and `ready.egress_log` are ignored. The present spec has no
egress (`136`), so that omission establishes no live network-diagnostic need.

`prototype.ts:173-182`: bash merges stdout/stderr into one tool-output stream,
and maps null exit codes to 1, dropping the host process's signal. This is
legitimate presentation only if intentionally requested; a reusable transport
must retain channel identity and exact code/signal. The current output cap is
the harness's output limit (`240`), not a general Pinfold stream guarantee.

`prototype.ts:153-170,219-224,261-270`: controller cancellation explicitly
stops the whole box. Stdin is kept open, SIGTERM goes to its own owner before
`box down`, and recovery rejects overlapping writers. Do not replace this
with destructor-driven teardown for arbitrary observers, generic command
abort, retries, or resuming unknown work. A tool-abort failure kills only the
exec wrapper (`177`); it does not prove a guest process has stopped.

Minimal consumer repair: preserve refusal objects/detail, bounded stderr
with truncation marker, continually drain owner stdout, retain terminal
event and close status separately, make explicit stop await both. This is
one adapter repair; no SDK or new public service is required to recover
those facts. In-memory evidence disappears if this controller dies; robust
postmortems require a binary-owned retained run record or another trusted
recorder and cannot be supplied by a decoder alone.

## Pi and exec boundary

`crates/pinfold/src/pi/launch.rs:226-268`: Pi calls the same core in process,
starts runtime exec with inherited streams, waits for exec/control/audit or
signal, tears down once, and returns exit status. It neither uses nor loses
process-interface JSON. Adding a subprocess SDK here would add plumbing;
render diagnostics from typed core data at the existing Pi boundary.

`core/box.rs:343-360`: process-interface hold observes runtime child exit;
Pi's `stop_requested` observes control and audit only. Unexpected init/runtime
child exit reaches Pi indirectly through its exec. Whether a pathological
runtime exit can leave Pi hanging is a runtime experiment, not established
by this source review.

`core/box.rs:558-594`: non-ready startup stdout is ignored; after ready the
runtime's attached stdout is drained and discarded. `runtime/mod.rs:524-533`
only overrides stdin/stdout, so runtime stderr inherits. A machine-readable
`failed.detail` can be generic while human stderr holds the runtime cause.
Consumers can preserve received stderr today; a stable error code and
structured cause for runtime failure must come from the binary.

`cli.rs:321-355,838-842`: exec preserves all guest exit codes, and pinfold also
uses 3 for absent box. Guest exit 3 and management failure are inherently
ambiguous without a separate origin/status channel. Never classify by just
exit 3; never put diagnostic JSON on raw command stdout or stderr. A separate
opt-in metadata FD or dedicated operation-status facility is needed if
classification is required. Runtime error prose cannot be safely parsed.

`runtime/mod.rs:394-428`: non-TTY runtime exec gets its own process group.
Killing a controller's pinfold wrapper group does not establish termination
of that runtime client or guest command. Whole-box down is the documented
cancellation operation. A thin SDK must not promise per-command cancellation.

`cli.rs:264-277`: ready is printed/flushed before hold; output failure can
interrupt control flow. Existing dirty-tree investigation already records
closed-output work. Any richer event stream must explicitly preserve owner
cleanup and fail-closed behavior on blocked/broken output; observer
disconnect must not automatically close owner's stdin.

## Actual other controller: Switchyard

Local `~/Code/switchyard-v3` is absent. Read current GitHub source
through authenticated `gh api`, without cloning or editing it. Verified
`main` SHA `81da8c677a0ab43966588429c24a8affedb9e8ec`, 2026-10-15. Read its
AGENTS.md and relevant binding architecture paragraphs before judging it.

Source permalink:
https://github.com/adamaltmejd/switchyard/blob/81da8c677a0ab43966588429c24a8affedb9e8ec/crates/yard/src/box.rs

Its 600-line Rust adapter includes box spec types (34-111), process
environment/stdio, bring-up, streaming and collected exec, stats, image
operations, capped readers and cancellation. That is not evidence that 600
lines can all disappear into a diagnostics SDK: much is caller-specific
spec assembly, bounded command collection and lifecycle ownership.

- `225-289`: retains refusal reason and detail, unlike Pi Durable. Stderr
  drains capped in a task, but only EOF startup awaits/reports it; ready,
  refused and failed paths do not retain stderr in `LiveBox`.
- `270-275,451-466`: keeps draining lifecycle after ready; preserves terminal
  reason or failed detail. It discards `down.detail` (including runtime exit
  status), keeps no observation history, and does not expose egress-log path.
- `303-315`: explicitly parses stderr prefix `pinfold box exec: no box named`
  with exit 3 and acknowledges the guest can print it. Even its suggested
  separate stat cannot conclusively establish the origin of an earlier exit
  across teardown/name reuse. This is a concrete need for typed operation
  outcome distinct from raw command exit.
- `319-324`: `oom_kills` maps runtime-unsupported, absent box, malformed JSON
  and any stat failure into one `None`. Some loss is consumer-owned; the
  binary's current `null` also lacks structured availability cause.
- `138-147,426-448`: `LiveBox` owns stdin; consuming `down(self)` closes it
  and awaits terminal drain and child close. Keep this ownership explicit.
- `516-541`: command timeout loses partial output and returns only Timeout.
  Parent-group kill needs special caution for exec's independent runtime
  group. No live cancellation bug was reproduced here.

The real workflow is worker OOM diagnosis and reliable teardown before
candidate admission, not generic SDK CRUD. `jobs/supervise.rs:623-631` asks
OOM before owner teardown; `750-755` refuses trust in candidate after a
failed teardown. `1268-1279` boxed gate collection also reads OOM before
taking the whole box down. Spec lines 570-573 explicitly distinguish OOM
from unknown on Apple. An SDK must preserve this caller's sequencing and
status distinctions, not move teardown into observer cancellation.

## Real experiment correction: process exit is not stream EOF

The coordinating agent's real runtime probe killed the owner with SIGKILL.
The owner was absent, but its runtime descendant still held inherited stderr.
Version 1's `await killed.closed` waited for Node's `close`, so it hung until
the coordinator explicitly removed the owned orphan. This is a demonstrated
adapter bug, not a hypothetical reason for a service. The exact experiment
receipt belongs to the coordinator's evidence.

The revised adapter separates `BoxOwner.exited` (child exit status/signal)
from `BoxOwner.closed` (EOF or a 1-second post-exit drain deadline). The latter
includes `outputDrain: complete|deadline`; on deadline it closes only local
pipe endpoints and preserves the bounded bytes already observed. This does
not remove the box, kill a descendant, claim cleanup, or promise complete
diagnostics. `snapshot().ownerEnd` exposes the same result. Raw exec streaming
retains its prior close-based behavior; it has no new implicit output truncation
deadline. This distinction must survive any proposed SDK API.

## Disposable adapter contract and limits

`consumer.ts` exports `BoxOwner` and `observeList`. Construction takes an
explicit binary/name/spec; `ready` retains initial detail on failure;
`snapshot` retains at most 64 lifecycle records, 64 KiB stderr tail and
truncation flags; one persistent reader drains after ready. `exec` passes
separate raw Buffer callbacks and returns code/signal plus bounded display
tails. No exit-code interpretation. `stop` explicitly signals this owner,
closes its stdin, and awaits process close. Timeout reports uncertain cleanup;
it does not claim the box is removed or kill an arbitrary process.

`observeList` owns only a query child and cancellation signals it; it never
touches owner stdin. It uses list specifically because the current binary
implements cancellation/reaping there. Do not generalize it to all commands.

Limits: no persistent records, protocol-version compatibility, per-command
cancellation, async callback backpressure, automatic orphan recovery, or
security/policy reimplementation. Raw stdout/stderr can contain secrets;
the adapter does not persist them. Parent must run only controlled fixtures
before retaining evidence. No TypeScript typecheck or runtime check has
been performed by this agent; parent owns runtime experiments.

Discriminating probes: missing image gives full refusal detail; absent exec
versus guest exit 3 demonstrates ambiguity; success writes distinct raw
stdout/stderr and returns nonzero; stop after ready retains terminal event;
real init death records terminal event when binary can observe it; SIGKILL
owner shows close-without-terminal as unknown; cancelled list leaves live
box and exec control intact. Network/Pi provenance probes belong to other
parts of the report, not invented from Durable's no-egress spec.
