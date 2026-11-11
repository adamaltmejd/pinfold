# Stage 1 implementation plan

Date: 2026-11-11. Implements stage 1 of `2026-11-11-composable-plan.md`:
issues #276, #277 and #278. Drafted by a planning subagent from the
code at 1de8560 and spot-checked by the operator against `launch.rs`,
`ownership.rs`, `plan.rs`, `box.rs` and `init/firecracker.rs`. Line
numbers are from that commit. Nothing here is implemented.

Marks: [code] read from code, [inf] inferred, [probe] must be measured.

## Decided by Adam, 2026-11-11

Adam accepted each proposal below.

1. **A killed session's agent keeps running.** On Linux the exec
   service keeps a process alive after its client goes
   (`init/firecracker.rs:913-939`) [code]. Apple is [probe], and the
   spec states only what the probe shows. So after a
   SIGKILLed launcher, its agent can keep using routes while another
   session holds the box. The spec says pinfold never signals a process
   in a box. Proposed: accept and state it; the process ends with the
   box.
2. **Keeping a box.** A flag on `pinfold pi` cannot work, because every
   argument passes through to pi. Proposed: `pinfold start` keeps the
   project's box past its last session, and `pinfold stop` ends it.
3. **Which agents a project's box carries.** Decision 15 says "every
   agent the configuration names", but no configuration key names
   agents. Proposed: always all three, with no new key, provided the
   Linux start cost measures acceptable (step 1). Until then it is
   conditional.
4. **`login-busy` after ready.** A detached owner has no stderr once it
   reports ready, so the notice is lost after that point. Proposed:
   accept; it stays on stderr until ready.
5. **`pinfold update` while a box lives.** The owner holds the
   executable's lock, so update reports `update-busy` until every box
   has stopped. Proposed: accept.

## Operator's calls, listed for the record

- The start sync runs at every box start for any agent, not only pi,
  and never on a join. Otherwise `pinfold claude` on a fresh project
  gets an empty disk, and a join could fast-forward under a running
  agent.
- The terminal reason `harness-exited` is deleted. A box held by
  sessions ends with `sessions-ended`.
- Harness env defaults and login placeholders become per exec. Box-wide,
  claude's route variables and pi's `ANTHROPIC_API_KEY` placeholder
  would reach each other's agent.
- Box names stay per launch; the project's box is found by its project
  label. A fixed name would meet `name-in-use` against a dead owner's
  Apple container (`box.rs:303-309`).
- A `codex` login route and pi's `openai-codex` route share one host
  token and one refresh per box. Today's single take would panic with
  both (`proxy.rs:127-128`).

## Today

- `Claim::take` holds a per-name flock; `prepare` picks a generation,
  binds the control socket and publishes the owner record
  (`ownership.rs:197-239`) [code].
- `Box::hold` ends the box on a signal, stdin EOF, an accepted `down`,
  an audit-log failure or init exiting (`box.rs:441-463`) [code].
- `pinfold pi` runs `Box::up` in-process with hangup handling, runs the
  agent's exec in the same process, and calls `down` on every end path
  (`launch.rs:487-557`) [code].
- Linux: the VMM has a parent-death signal tied to the creating thread
  (`jail.rs:524`). Apple: the box is a `container run -i` child of the
  owner (`apple.rs:96-156`) [code].
- A spec has one `harness`; a login route must name it; one non-pi
  login route per spec (`plan.rs:689-743`) [code].

## Design

**Detached owner.** The launcher spawns `pinfold box up --sessions`,
writes the spec to its stdin and reads `ready`, `refused` or `failed`
from its stdout. The owner calls `setsid`, and after `ready` points its
stdout and stderr at `/dev/null`. It inherits the launcher's
environment, so login and injecting values are still read once at start
from `up`'s own environment and never enter argv or a file. The VMM is
still spawned from the owner's long-lived thread.

**Sessions.** A session is a connection held open on the owner's
existing control socket. The kernel closes it when the client dies, so
a SIGKILLed launcher needs no polling. The request is the generation
plus one verb: stop (today's), join, keep. A held session may send
leave; the owner replies `left`, or tears the box down and replies
after teardown. When the box ends for another reason, the owner tells
every session.

**End of the box.** The owner ends the box when stdin has closed and no
session is open, unless kept. The starter joins before it closes stdin,
so a launcher that dies between `ready` and its join still takes the
box down.

**Join race.** The owner handles requests in one loop and drops its
listener before teardown. A late joiner sees the box gone, blocks on
the old box's name lock until teardown releases it, then starts a new
box. A new per-project launch lock, exclusive, covers discovery through
ready, so a second launch waits for a starting box instead of racing
it.

**Config comparison.** A joiner compares its fresh resolution with the
running box's launch record (guarantee 39): image reference and id,
profile, harnesses, disks, env names, allow list, route kinds and
targets, cpus, memory. Values, tokens and `from` names are not in the
record and are not compared; the box keeps the credentials it started
with. Trust is checked first. Refusal: `config-changed` on stderr,
naming the first differing field and the run, exit 1.

**Harness list.** `"harnesses": [NAME, …]`, distinct names; `login`
must name a listed one; one login route per non-pi harness, pi one per
provider. `box exec --harness NAME` sets that harness's defaults and
placeholders for one exec.

**One launch path.** `launch::run(agent, args)` execs
`/opt/pinfold/<agent>/<agent>`. Pi alone gets its login extension and
exit summary. `attach` joins as a session and never starts a box.

## Order

Each step is green on its own.

| Step | Issue | Change | Deletes |
|---|---|---|---|
| 1 | #278 | Core harness list: `plan.rs`, `box.rs`, `proxy.rs`, `login.rs`; the pi layer passes a one-element list. Measure Linux start time with three harnesses mounted. | The single-harness check; the shared `seen_login`. |
| 2 | #278 | Per-exec harness env, `box exec --harness`, `pinfold claude` and `pinfold codex`. A second agent's start is still refused as `disk-in-use`. | — |
| 3 | #276 | Detached owner, session verbs on the control socket, `sessions-ended`, `pinfold start` and `stop`, `attach` as a session. The launcher becomes a session client. | The pi layer's in-process owner path (`launch.rs:487-557`); `harness-exited`. |
| 4 | #277 | Join, the launch lock, `config-changed`. | The second-start `disk-in-use` rule. |

Step 1 depends on nothing and goes first.

## Spec edits

Caller changes are marked ★.

- Terms: a box is owned by one `box up` process and shared by a
  project's sessions. New term **Session**: one launch's agent or
  `attach` in a project's box; it ends when its client does.
- Lifecycle: `up` does step 8 on `down`, SIGTERM or stdin EOF; with
  `--sessions`, on stdin EOF once no session is open.
- ★ Process interface: `box up --sessions`; `down` reason
  `sessions-ended`; `box exec --harness NAME`; `harnesses` replaces
  `harness`.
- ★ Run records: `harnesses` is a list; `harness-exited` goes.
- Login routes: per harness; the shared Codex token; placeholders per
  exec; `login-busy` until ready.
- Process supervision: a closed terminal or signal ends that launch's
  session; the box ends with its last.
- pi layer: one box per project carrying all three harnesses; join,
  launch lock and `config-changed`; lifetime; attach; sync at every box
  start.
- ★ CLI: `pinfold claude`, `pinfold codex`, `pinfold start`,
  `pinfold stop`.
- Updates: `update-busy` while any box lives.

Guarantee rows:

- 8: the owner SIGKILLed is `box up --sessions`.
- 19: a spec listing all three runs each at its pin; the three CLI
  verbs print their pins.
- 26: one box carries claude's, codex's and pi's `openai-codex` routes;
  the two Codex routes ask the helper once; an exec for one harness
  sees none of another's placeholders.
- 39: the terminal record is `sessions-ended`.
- New 47, **A project's sessions share one box**: while `pinfold pi`
  runs, a second `pinfold pi` and `attach` join it: one box, one
  generation, owned by neither launcher. After the first launcher's
  SIGKILL the second still answers. When it finishes the box is gone
  with terminal `sessions-ended`. A launch racing that finish runs in
  exactly one box. After a re-allowed allow-list change a launch is
  refused `config-changed` and the started list still decides. After
  `pinfold start` the box outlives its last session until
  `pinfold stop`. A SIGKILLed last session ends the box.

## Verification

Adam's decision, 2026-11-11: the end-to-end suite is removed for this
build and rewritten once stage 2 lands. No step adds a test. Each
step's builder exercises the real binary through the cases below and
records the commands and what it observed in a dated archive file.

- Sessions: while `pinfold pi` runs, a second launch and `attach` join
  one box. A SIGKILLed first launcher leaves the second working. The
  box ends with its last session. A launch racing that end runs in
  exactly one box. A SIGKILLed last session ends the box.
- Configuration: an unchanged join succeeds; a changed allow list is
  refused as `config-changed`, and the started list still decides.
- Keep: after `pinfold start` the box outlives its last session until
  `pinfold stop`.
- Owner death: after SIGKILL of the owner, egress fails closed.
- Harnesses: one box runs each agent at its pin; each login reaches its
  fixture with the host credential; the two Codex routes ask the helper
  once; an exec for one harness sees none of another's placeholders.

The mistakes most worth checking for: sessions counted per process
instead of per connection; the comparison skipped; the listener dropped
after teardown instead of before; `setsid` missing; the proxy running
in the launcher; only the first harness mounted; a token per route;
box-wide placeholders.

## Risks and probes

- Linux re-images every mount at each start (`vm.rs:129-150`), so three
  harnesses cost every start. Measure in step 1. If it matters, cache
  each pinned harness's image by version, or let a key name the agents.
- [probe] Whether killing a `container exec` client ends its guest
  process on Apple.
- [probe] Whether the `--ssh` socket forward survives the owner's
  `setsid`.
- [probe] Connect to a unix socket with no listener is refused at once
  on both systems.
- [probe] Whether claude prefers `ANTHROPIC_API_KEY`, whether pi reads
  `ANTHROPIC_BASE_URL`, and whether codex takes `--sandbox` at top
  level.
- An Apple orphan box holds the project disk until pruned; the launch
  path must remove it itself.
