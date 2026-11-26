# Code cleanup before v0.9.0 (2026-11-26)

Whole-tree read at ec4c840e (67 commits since v0.8.1), 16 module readers plus
three test-audit sweeps, judged and landed in PR #492.

## Numbers

| | Before (ec4c840e) | After (#492) |
|---|---|---|
| Rust lines, tracked `crates/**/*.rs` | 39648 | 39309 |
| Largest source file | `pi/session.rs`, 2443 | `pi/session.rs`, 2428 |
| ARCHITECTURE.md lines | 2546 | 2459 |

- The cleanup itself, in `crates/`: +388 / −729 (net −341). Whole PR, with this report: +739 / −1014.
- The release since v0.8.1, in `crates/`: +3290 / −2289, a delete/add ratio of 0.70.
- Readers estimated about −860 code lines possible; −341 landed. Earlier passes saw the same 2–3× overestimate.
- Written tickets: 3 (#493, #494, #495).

## Landed (one commit per module)

- core/image/linux: base_metadata parses only what admission did not
  check; publish uses verify_root; the linux.rs delegates are re-exports; job.rs uses
  exec_transport::put and JSON_BYTES.
- core/runtime: one tree_size; Vmm::exited through nix::poll; the
  sockets remove_directory wrapper and flag go; Cancellable serves Apple disk
  preparation (PreparationInput gone) and loses its read_exact/write_all
  overrides; one Apple helper argv builder; inspect_image inlined.
- init: firecracker.rs disk helpers inlined, mount_at walks through
  make_directories, BOOT layout doc names the trailing CA; read_frame
  through FrameReader; one session/drain deadline.
- core: ca.rs one key generator; proxy/tls one ClientHello reader, the
  header read inside parse_head, extract_if reaping, the dead rebind, a
  redundant cancel check and reason escaping go; process.rs one fd array;
  ownership.rs small; disk_export.rs uses nix's safe wrappers (five unsafe
  blocks gone) and drops the unreachable name check; crossing.rs folds the
  not-a-repository refusal into object_width; runs.rs inlines private_dir;
  disk.rs imports.
- cli/plan/pi: one DEFAULT_DISK_SIZE; RefusalReason::token in cli;
  forward_diagnostics takes text; the snapshot's disk comparison goes
  (Adam; the disk set derives from compared fields); context.rs seen set,
  remote.rs object_format, state.rs disk_origins inlined.
- e2e: one try_wait_until; Client::kill and the empty system gitconfig go.
- Spec, README, profiles, bump-pins: see below.

Behaviour notes. An early-EOF error reads "unexpected end of file" where it
read "failed to fill whole buffer". A symlink at a stored Linux image root is
now rewritten rather than followed. A repository removed mid-crossing refuses
as `spec` rather than `failed`. Profile Containerfile comment edits mark
existing images outdated.

## Test-audit sweep

| Test | Block | Tag | Disposition | Row |
|---|---|---|---|---|
| crossing.rs | `!hooks.exists()` | detector | deleted | 10 |
| crossing.rs | `host_tags.len() == 2` | detector | deleted | 10 |
| crossing.rs | `main == fetched` after merge | detector | deleted | 10 |
| disk.rs | line count and `disk` field | detector | deleted | 11 |
| records.rs | `second_id != first_id` | twin | deleted | 38 |
| credentials.rs | `finish() == 0` | twin | deleted, call kept | 26 |
| owner.rs | prune-removal checks after `drop(hold)` | unrelated | rewritten: lock held through them | 8 |
| clean.rs | `detached_children` + /proc wait | seam | kept: the rewrite passed on the dev VM, but review found `stop` can return before the owner drops its maintenance lock | 15 |

Test lines: crossing −3, disk −1, records −1, credentials 0, owner 0,
lib.rs −22. Sweep C (jail, share, chain, build) and sessions.rs: lean.

## Spec pass

- Adam approved trimming Linux image preparation to its contract, the pin
  policy to its seven-day line (the rest is in scripts/bump-pins.py), row
  26's Shown-by cell, and the Rex relay's step detail.
- Restatement and duplicate cuts across Process supervision, Transport,
  Configuration, Images, Profiles and Maintenance; quoted user-facing
  sentences became descriptions with their tokens.
- Contradictions fixed: row 15 is Linux only; the Box spec term no longer
  claims nothing is added; `--` is out of the no-daily-pass list (it runs the
  pass deferred); a disk is an ext4 image on Linux and an Apple volume on
  macOS; the Codex app-server-daemon seed is named; a `from` value reaches
  the runtime client's environment on macOS only.
- Guarantees table reconciled with the tests: 15 rows, 15 tests.
- Open questions: the empty herdr question is closed; two remain.

## Review

Sol reviewed the whole branch. Fixed: the image-preparation trim had dropped
the builder input device barrier (restored); row 15's "Linux only" moved from
the guarantee to its test; the clean.rs rewrite was reverted (race on the
owner's maintenance lock); the e2e reap and group-exit waits keep their old
bounds. Declined: the snapshot shape change breaks a client and owner from
different builds, as #464 and #477 already do in this release; the release
notes tell callers to stop running owners before upgrading.

## Adam's

- The four trims above, and dropping the snapshot's disk comparison: yes.

## Rejected (re-admit trigger)

- Session frame version field: guards a client and owner from different
  builds. Re-admit if `update` refuses while an owner runs.
- Generic session Frame<T>: renames the payload key on the wire. Re-admit with
  a version bump.
- Derived untagged Env deserializer: same refusals, but the detail becomes
  "data did not match any variant". Re-admit if serde names the field.
- plan.rs guest-check dedupe, box.rs first validate_generated: change which
  refusal wins for a doubly bad spec.
- profile.rs write_profile through Share::files: drops mode bits, adds a refusal.
- box.rs resolve_logins through startup_work: changes signal precedence.
- disk.rs in-use check before creation: changes failure order.
- launch.rs require_repo: an at-use recheck. Re-admit if borrow fails closed
  with the same token.
- Post-fork descriptor remap helper and set_capabilities(mask): jail code.
  Re-admit with a dedicated review.
- Apple wait_ready try_wait probe: the exec race area fixed in #466.
- directory_entries through nix dir: nix 0.31 leaks the fd when fdopendir fails.
- init builder send re-checks: the guest side of the output bounds.
- init accept_host peer-CID check: backs up the no-loopback statement.
- sockets.rs deadline pre-checks: an expired deadline would report NotFound.
- vm.rs init-disk helpers through job.rs: different deadline and error text.
- crossing.rs 4096-byte check: the malformed-listing refusal would lose to
  the box's exit status.
- ownership.rs down() before remove_record: a retiring record after a crash
  could leave a VM up.
- records.rs inspect-render twin: may be the only proof of rendering.
- cli config text from its JSON: a behaviour change.
- e2e Project::configure via `config --json`: weakens the observation.
- Containerfile checksum re-check, TARGETARCH through resolved.json: kept.
- Under the two-line bar once written: config.rs share refusal, plan.rs
  inject.check, launch.rs utf8(root), Apple feed(), job.rs write_input,
  init receive_into, snapshot share re-resolve (also not identical).

## Tickets and unfiled

- #493 disk names accept uppercase (reproduced through the CLI).
- #494 Apple "image not found" stderr matched above the adapter.
- #495 gate `pinfold init` to Linux.
- Unfiled: builder install's `tar -xOzf` under spawn_guarded may leave gzip
  on cancel. Not reproduced through the CLI.

## Notes

- Linux CI ran 4m06s–5m04s on today's PRs, at the five-minute budget.
- #491 (the vsock backlog, one line in init/firecracker.rs) merged after the
  read; its file was read whole by the init reader at ec4c840e.
