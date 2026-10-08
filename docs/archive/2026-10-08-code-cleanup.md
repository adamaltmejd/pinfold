# 0.2.1 release cleanup and test audit

This records the completed whole-tree review and audit edits. Final release
validation and publication are separate gates, pending at this snapshot.
The previous main commit, `c71a7ee`, has green Linux CI and is v0.2.0.

## Scope and mechanical checks

Independent read-only reviews covered CLI, main and update; box, plan and
ownership; all runtime adapters; proxy, network, TLS, pipe, login and download;
clean, image, artifacts and profile; config, dirs, trust and init; pi; E2E
helpers and every Rust test/fixture; the durable prototype and its gate;
profiles, scripts, workflows, build support, README and the architecture.

The nine production dependencies match the spec. All-target Clippy with
`-W dead_code` passed. The deferral scan found no actionable TODO; the login
getAuthStatus revisit has not fired because its Codex pin is unchanged and
G26 passed with the actual helper. The scratch configuration, CLI and
36-guarantee trace tables are at
`/private/tmp/pinfold-release-021/trace-tables.md`.

## Size snapshot

Audit entry includes the local issue 82 fix. Audit close is `3bc849d`,
excluding this report. No production cleanup was admitted.

| Measure | Audit entry | Audit close |
| --- | ---: | ---: |
| Tracked Rust lines | 16,915 | 16,878 |
| Production Rust source lines | 9,045 | 9,045 |
| E2E Rust lines | 7,811 | 7,774 |
| Largest source, E2E box_.rs | 4,828 | 4,828 |
| Largest production source, cli.rs | 1,115 | 1,115 |
| ARCHITECTURE.md lines | 972 | 972 |

Since v0.2.0, the snapshot has five commits, 112 added and 86 deleted lines;
delete/add ratio 0.768. This includes issue 82, its evidence and harness
pins, not just audit deletions. The admitted cleanup net was 45 lines;
all 45 landed (37 test lines and eight documentation lines).
One written investigation ticket was opened by this pass: #83.

## Dispositions

- `cb8ae54`: delete four row-12 assertions of the unspecified remediation
  phrase `pinfold allow`, plus their unused stderr bindings. Refusal and
  successful runs/builds after explicit trust remain. Net 20 lines removed.
- `ef069aa`: delete row 28's duplicate pre-download executable-owner
  refusal. The harder case starts an owner in another XDG root during the
  held download, then checks `update-busy`, unchanged bytes and inode.
  The distinct older-executable `running-boxes` guard remains. Net 17 removed.
- `3bc849d`: remove duplicate durable regression narrative, shorten writer
  implementation detail and correct the obsolete matching-Pi-pin claim.
  No behavior changed. Net eight lines removed.
- The other modules and assertion blocks were lean already. No new
  dependency, control change, feature removal or guarantee deletion.

No finding required a user decision about weakening a control. The proposed
`box list --label KEY` documentation gap was rejected: the process contract
already documents key-only matching. `config ROOT` has no direct invocation
in the suite, but that syntax observation is not a missing guarantee and
cannot justify a separate argument-shape test.

## Spec and pin pass

The spec's readiness wording now includes Apple boxes without egress and
matches guarantee 9's immediate exec scenario. All 36 guarantee rows map
to named tests or standalone gates. The five removed assertion blocks do
not change those rows' behavioral proof. README and built safe help agree
with the contract. The four existing open questions remain unresolved.

The required `scripts/bump-pins.py` run verified publisher digests and moved
Pi 1.0.4 to 1.1.0 and Claude 2.1.292 to 2.1.294. Codex and image tools did
not move; Ponytail 5.0.0 was not yet eligible. The durable prototype's 1.0.4
pins remain independent as specified. Version and lockfile are 0.2.1.
The release's Caller changes section is None: no schema, JSON-line field,
CLI flag or exit-code contract changed.

## Validation observed during the audit

The issue-82 fix before the pin/version changes passed its focused lifecycle
case and the full Mac suite: 33 passed, one intentional slow exclusion,
386.69 seconds. The original first-exec error remains unconfirmed; see
`2026-10-08-test-audit.md`.

The first versioned 0.2.1 Mac run (`0202a73`, before audit deletions) passed
32 and failed one, with one slow exclusion, in 292.35 seconds. The file-sharing
assertions passed; its final `box down` timed out with OS error 35. Exact-box
native logs show Apple I/O completion timeouts and delayed service removal.
[Issue 83](https://github.com/adamaltmejd/pinfold/issues/83) retains the evidence
and limits of attribution. This was not an immediate-exec failure. No
production timeout, test assertion or retry was changed to suppress it.
The final audited candidate requires its own full Mac run and Linux gates.
