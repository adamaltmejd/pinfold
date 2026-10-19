# Run records: stage 2 of the diagnostics research

Date: 2026-10-19. Baseline: main at 59c1612 (v0.2.7). Spec only; nothing
is implemented.

## Source

The research is `docs/archive/2026-10-15-diagnostics-architecture.md` and
its evidence directory, `docs/archive/2026-10-15-diagnostics-evidence/`.
They were written against a dirty 0.2.3 checkout and live on the local
branch `wip/exploration-2026-10-19`; a separate docs change may archive
them to main. This file uses `README.md`, `consumers.md`, `comparators.md`,
`independent-review.md`, `prototype-b-notes.md` and `prototype-b.patch`.

The research recommends keeping the subprocess boundary as the public
contract, with no SDK, library or daemon, in three stages:

1. Repair consumers. Outside pinfold.
2. Binary-owned retained run facts and bounded queries. This change.
3. Reason codes, an exec metadata fd and a static policy check. Deferred.

An independent review endorsed that direction with twelve corrections.
This spec adopts the ones that bear on stage 2: the identity is the
existing generation (4); bounded reads are not bounded storage (2);
snapshots state requested controls, not observed enforcement (3);
terminal records keep structured detail and separate "unavailable" from
"not observed" (8); a query never claims liveness (8, report).

## Premises rechecked on v0.2.7

| Premise | On main | Where |
|---|---|---|
| Egress log is keyed by box name, so a reused name sees the previous run's lines | Still true. The log is `egress/<name>.jsonl`, opened create+append at proxy start. | `core/box.rs:543`, `core/proxy.rs:65` |
| `Layer::over` drops which layer won | Still true. Each key is `higher.or(lower)`; no source is kept. | `config.rs:183` |
| A generation exists but `ready` has no run id | Still true. `dev.pinfold.generation` is a box label (so it is inside `ready.labels`); there is no top-level field. | `core/box.rs:261`, `core/ownership.rs:18`, `cli.rs:269` |
| Teardown deletes all per-box state | Still true. The claimed state dir is `remove_dir_all`'d, so run records must live elsewhere. | `core/ownership.rs:147` |
| Audit append failure stops the box | Still true. The first failed append latches and the owner ends with `audit-log`, exit 1. | `core/proxy.rs:730`, `core/box.rs:343` |
| `down` detail | `down` carries `detail` only for `exited`. | `cli.rs:299` |
| Closed readiness stdout leaves an orphan (research prerequisite) | **Fixed.** Guarantee 9 now shows exit 1 after removing the runtime box and its state. No longer blocks stage 2. | ARCHITECTURE guarantee 9 |
| `box down` and maintenance (#101–#104) | `down` and `list` skip daily maintenance; `down` addresses the observed generation and the owner acknowledges after teardown. The new queries join the skip list. The terminal record is written before that acknowledgement, so "`down` returned, `inspect` shows terminal" needs no wait. | `main.rs:91`, ARCHITECTURE Process interface |
| Egress logs older than 14 days are pruned daily | Still true, by file mtime. Replaced by the run retention rule. | `core/clean.rs:239` |

## What the spec adds

| Stage 2 item | Spec | Guarantee |
|---|---|---|
| `run_id` (the existing generation) in `ready` and `down` | Process interface | 38 |
| Generation-keyed egress log | Egress proxy, Log | 38 |
| Launch record of allowlisted nonsecret applied policy | Run records | 39 |
| Terminal record separate from `ready` | Run records | 40 |
| `box inspect`, `box events` with availability states | Run records | 41 |
| Retention: 14 days, 128 MiB of inactive runs | Maintenance | 42 |
| Pi host-side explanation | pi layer, After a run | none (stderr prose) |

### Tests: hardest case and sabotage

Each row's sabotage is the change to the binary that the test must catch.

| # | Hardest case | Sabotage |
|---|---|---|
| 38 | Back-to-back reuse of one name, with the first run's log read again after the second has appended. | Key the log by box name again; or emit a fresh random `run_id` instead of the generation. |
| 39 | The project file changes under a live pi box and fails trust; the record must still show the started policy. An injecting header and a `PINFOLD_ENV_SECRET` value are present to leak. | Build the record by re-reading config at query time; or serialize the plan wholesale (writes env values and header values). |
| 40 | Owner SIGKILL, then `prune`: a cleaner could be tempted to write a cause. Plus `down` racing the read. | Write the terminal record after acknowledging `down`; write a `signal` record from `prune`; drop `detail`. |
| 41 | Runtime stalled on a real image-store lock while the box keeps appending during paging. | Call runtime inventory (or take the name lock) inside `inspect`; use a line-count cursor that skips or repeats lines under concurrent append. |
| 42 | A live run that is as old as an evicted one; quota pressure with a live run present. | Age out runs without checking the lifetime lock; evict newest first; truncate the active log to meet the quota. |

Two test seams need Adam's acceptance. Guarantee 42 ages a run directory
on the host (as a user's clock would) and pads an inactive run's egress
log on the host to reach 128 MiB, because filling 128 MiB through real
requests does not fit the CI budget. Both act on paths the spec names.
The e2e helper that builds `egress/<name>.jsonl` must switch to
`ready.egress_log`.

Cost: five new rows, each starting real boxes. 38, 40 and 41 can share
fixtures with existing box tests' setup but are separate tests by rule.
The suite's five-minute budget needs remeasuring when they land.

## Write failures

The egress log append rule is unchanged and stays the only write failure
that stops a running box. Run records are diagnostics, not a control.

- Launch record (recommended): fail the start with `failed` before
  `ready`. This makes "every `ready` run_id has a launch record" an
  invariant and keeps the record off the hot path. Alternative:
  best effort, `ready` still printed and `inspect` reports `unavailable`.
  A full disk will usually stop the box through the audit rule soon after
  either way.
- Terminal record: best effort. Failure goes to stderr; the `down` line,
  reason and exit code are unchanged; `inspect` reports `unavailable`.
- Records never delay or alter an audit append. Eviction never touches a
  live run, so the quota cannot truncate an active audit log.

## Launch record timing

The record is written after the box accepts `exec` and immediately
before `ready`. Writing it at the claim would leave a run directory for
starts that end in `image-changed`, a runtime start failure or a late
`mount-alias` refusal, contradicting "`failed` and `refused` retain no
run" and guarantee 17's "no state". Failed-start cleanup removes the
directory.

## Launch record fields

The rule is closed: a field is a value the box spec carries or that
`ready`, `list`, `artifacts` or `doctor` already prints. Never env values, header
names, values or prefixes, login tokens, argv or command output. Route
targets (`host:port` or origin) are included; the prototype left them out
from caution, but they are not secret and they explain a route's 502.

Per-setting provenance (which layer won) is left out. It is pi-layer
knowledge and core knows nothing of pi; carrying it into a core record
needs either a box-spec field or a pi-owned file in the run directory. The
recorded values already answer the demonstrated question (the policy of a
live run differs from current config). Provenance is listed as future
work.

## Caller changes

For the release notes of the release that implements this.

- **Hard break:** the egress log moves from
  `~/.local/state/pinfold/egress/<box>.jsonl` to
  `~/.local/state/pinfold/runs/<run_id>/egress.jsonl`. A caller that
  composes the path breaks; one that reads `ready.egress_log` does not.
  Switchyard does not read the path; Pi Durable runs without egress.
  Existing name-keyed logs are not migrated: the first maintenance pass
  after upgrade deletes the `egress/` directory, with up to 14 days of
  history.
- `ready` gains `run_id`.
- `down` gains `run_id` (null when `up` ended before `ready`).
- Terminal records use the `down` reasons plus `harness-exited`, a
  terminal-record-only token for a pi box whose harness ended. It is not
  a `down` reason.
- New commands `box inspect RUN` and `box events RUN [--limit N] [--after
  CURSOR]`, each one JSON object. Exit 0 on an answer, 1 on a malformed
  argument, 3 on a run with no records.
- A launch-record write failure is a new cause of `failed` before `ready`
  (if the recommended policy is chosen).
- Egress logs older than 14 days of inactive runs, and more under 128 MiB
  quota pressure, are removed by maintenance as before; the quota is new.
- Egress log files become 0600 in a 0700 directory.
- `pinfold pi` prints a new stderr notice after an abnormal end. Not a
  machine contract.

No change to `refused`, `failed`'s shape, `exec`, `stat`, `list`, `prune`,
image commands, exit codes of existing commands, or stdin ownership.

## Decisions for Adam

Answered 2026-10-19: all seven as recommended. Stage 2 authorized; 14
days and 128 MiB; a launch-record write failure fails the start; the pi
notice ships; both test seams allowed; legacy logs are deleted on upgrade
(option 6's alternative, now in the spec); `harness-exited`.

1. Authorize stage 2 as specified.
2. Retention defaults: 14 days and 128 MiB of inactive runs. The quota can
   shorten audit retention below 14 days. These are proposed defaults, not
   measured capacity needs.
3. Pi host-side explanation on stderr after an abnormal end.
4. Write-failure policy: launch record fails the start (recommended) or is
   best effort.
5. The two host-side seams in guarantee 42.
6. Legacy `egress/<box>.jsonl` logs. The spec lets them age out under the
   14-day rule, which is a compatibility path the repository rules
   otherwise forbid. The alternative is to remove the legacy directory at
   the implementing release's first maintenance pass, losing under 14
   days of audit history for runs before the upgrade.
7. Pi box terminal reason: the new token `harness-exited` (above), rather
   than reusing `exited`, which means the box's init ended.

## Future work (stage 3, not specified here)

Each needs its own ticket and guarantee when admitted.

- Exec metadata fd: an opt-in `--diagnostic-fd` for `box exec` that
  separates "no box" exit 3 from a guest's exit 3. Needs inherited-fd
  design on both runtimes. The research did not prototype it.
- Stable reason codes and stages for launch, runtime and upstream
  failures (`upstream-connect-failed`, `upstream-tls-failed`, …), and
  availability causes for `stat` fields.
- Static policy check for a run (`passes-static` only), sharing the proxy's
  predicate, with an evaluator version in the launch record.
- Per-setting provenance in the launch record (above).
- A disk cap for live egress logs (already an open question).

## Stage 1 consumer repairs (not done here)

- `examples/pi-durable`: keep owner stderr and `detail` on startup errors;
  keep draining `up`'s stdout after `ready` and retain the terminal line;
  treat process exit and pipe EOF as separate facts. Reference adapter:
  the research's `consumer.ts`.
- Switchyard issue: keep `down.detail`; stop parsing stderr prose to tell
  an absent box from a guest exit 3 (needs stage 3's fd; until then treat
  exit 3 as ambiguous); stop collapsing unsupported and failed `stat`
  reads into one `None`.
