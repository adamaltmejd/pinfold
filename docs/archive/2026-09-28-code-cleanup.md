# Code cleanup and spec pass before v0.0.7, 2026-09-28

The pass ran after v0.0.6 and eight tickets (Y-96 to Y-103). No lane was
open while it read the tree. Its own commits are bac701f..74bd083; Y-103
landed after it.

## Numbers

| | Before (bac701f) | After (release) |
|---|---|---|
| Commits since v0.0.6 | 18 | 31 |
| Diff since v0.0.6, all | +1792 −169 | +2099 −1126 |
| Diff since v0.0.6, crates | +1661 −146 | +1904 −1061 |
| Release delete/add ratio | 0.09 | 0.54 |
| Largest source file | e2e/tests/box.rs, 3457 | e2e/tests/box.rs, 3065 |
| Largest binary source file | cli.rs, 1119 | cli.rs, 1108 |
| ARCHITECTURE.md lines | 809 | 809 |
| Tickets landed since the last report | 7 (Y-96 to Y-102) | 8 (with Y-103) |
| Tickets from lane proposals | 0 | 0 |
| Suite wall time (Mac gate) | box 207.6 s, pi 39.2 s (Y-102) | box 171.4 s, pi 29.7 s (Y-103/2) |
| Merge-queue e2e gates | e2e-linux ~5 min, then e2e-macos ~3 min | one e2e gate, ~4.2 min |

The pass itself: crates/pinfold +247 −487, crates/e2e +189 −741. The
module reads listed a possible −368 in the binary and e2e helpers; −240
landed in the binary.

## Landed

- bac701f gates: one `e2e` host gate runs the Linux and macOS suites at
  once (scripts/e2e-gate.sh); CI caches dependencies (Swatinem/rust-cache,
  saved from main only; pinfold build 35 s to 5 s, lint 21 s to 4 s) and
  skips fmt and clippy on dispatched runs; the gate polls every 10 s.
- d62cf62 cli: one refused-line printer; the pi shim takes the one dispatch.
- baef74d pi: the worktree special case, `HASH_LENGTH`, the spec-refusal
  remap.
- ea352b3 config: restating docs, the hand-built `PINFOLD_CPUS` error; an
  empty `PINFOLD_ENV_` name now reaches guarantee 6's refusal.
- ee77a6e proxy: an address route is a plain `Target`, one plain forward;
  the impossible missing-token error.
- 16bde4f plan, box: the `://` early return; one `plan.login()` read.
- 1e229a8 clean, pins: docs and comments restating the spec.
- ed6fc40 readme: the kernel sentence; the Shared files link.
- f277ccc runtime, core: the box spec's `user` field (Adam: remove),
  `ImageIdentity`, `ImageStatus`, `Runtime::image_digest`,
  `Runtime::make_proxy_connectable`, `Preflight`, `run()`,
  `RefusalReason::as_str`, `total_bytes`, `StateDir::stale`, two copies of
  `now()`, Apple's three label structs.
- 74bd083 e2e: test-audit sweep. box.rs 156 blocks, 37 deleted; pi.rs and
  cli.rs 42 blocks, 9 deleted; five rewritten to the row's harder case.
  Rows 9, 11, 13, 16, 17 name what their kept blocks prove (Adam: "if it's
  meaningful, add it, but keep the suite lean"); rows 4 and 26 lost the
  clauses whose blocks went.
- 4d2d208 and Y-103 (9da11c8): an install records its files and a use
  that finds one missing reinstalls it. Found by this pass's host run:
  macOS temp cleanup had removed pi's executable from the e2e cache and
  left its directory, so every pi box failed to launch it.

## Adam's list

- The box spec's `user` field: removed.
- Behaviour blocks no row named: kept where meaningful and named in rows
  9, 11, 13, 16, 17; SIGTERM before ready (a pid-file poll) and the rest
  deleted.
- One test binary instead of three: after this release.

## Rejected

Each with the condition that re-admits it.

- `-V` and bare `help` aliases: harmless; re-admit if one shadows a verb.
- Dropping the host's `PINFOLD_ENV_GIT_CONFIG_COUNT` merge: a user's own
  git config entries would vanish silently; re-admit if no caller sets them.
- Building the TLS client on every `up`: no measurement of
  `load_native_certs`; re-admit with one.
- tls.rs's zero-length-record check, and CONNECT via `authority_host`:
  both change which request is refused or its reason (a proxy control).
- Moving the init out of `artifacts/`: 13 init builds hold 35 MB on the
  development Mac; re-admit if a user's `init/` passes 100 MB.
- Creating the box state dir 0700 at the claim instead of podman's chmod:
  touches permissions on both runtimes for 3 lines; re-admit with a race.
- config's `is_absolute` refusal and empty-route-name filter: the filter is
  the only route-name check; re-admit with route-name validation in plan.
- `Login::check`, the empty-`from` check, the guest-path dedup: each
  changes a refusal's detail or adds a seam.
- clean's `dev.pinfold.` box scan to the owner label only: on podman it
  would stop pruning foreign containers made from pinfold images; a spec
  decision if it matters.
- `TestDir` to a function: 102 call sites for −10 lines.
- CLI surface without a test (`attach`, `doctor`, `--no-cache`, `exec
  --tty`/`--workdir`, `config ROOT`, `list --label KEY`): no guarantee
  names them; re-admit with a guarantee.

## Spec pass

ARCHITECTURE.md: nothing to cut; all 26 guarantee rows match a test by
name; the open questions are all still open. README: two edits (above).

## Watch

Two of three host runs of box.rs on this Mac lost a box before `ready`
("container ... is not running" at the proxy socket chmod), in different
tests, alongside XPC interruptions in `container system logs`. The gates
did not show it. Not yet reproduced as a pinfold bug.

Y-100 asked for a by-hand podman timing of a cached caller build; it was
not run for this release.
