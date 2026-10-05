# Code cleanup and spec pass for 0.1.7

The pre-release pass from the `code-cleanup` skill, with `test-audit`'s
sweep, run on PR #79's branch. That branch holds main (v0.1.6) plus the
bundled document readers and the reworked pin updater. Merging the PR was
left to the operator.

## Numbers

| | Before (PR head `80cfddc`) | After (`HEAD`) |
| --- | --- | --- |
| Commits since v0.1.6 | 2 | 13 |
| Diff since v0.1.6 | +846 / −261 | +1321 / −935 |
| `crates` since v0.1.6 | +132 / −3 | +566 / −565 |
| Largest source file | `cli.rs`, 1130 | `cli.rs`, 1122 |
| Largest test file | `box_.rs`, 3386 | `box_.rs`, 3447 |
| ARCHITECTURE.md lines | 918 | 866 |
| Written tickets opened | 0 | 7 |
| Mac suite wall time | 222 s (PR's run) | 190 s |

The release's delete/add ratio since v0.1.6 is 0.71; the PR's feature and
its archive notes are most of the additions. The cleanup pass alone is
+529 / −728 (ratio 1.38, net −199): `crates/pinfold` −89, `crates/e2e`
−39, docs −59, `scripts` −11. `box_.rs` grew by the regression scenario
for guarantee 15.

Possible net from the module agents: −477 lines (docs −8, config/profile
−28, `pi/` −18, runtime −37, clean/image/artifacts −57, e2e harness −52,
cli/update −52, box/plan −38, proxy/tls/login −40, scripts/CI −68, spec
−79). Landed net: −199.

Step 1: `cargo tree` matches the spec's dependency list; `clippy -W
dead_code` is clean; no TODO, FIXME or XXX in code.

## Landed

| Commit | Module | Removed |
| --- | --- | --- |
| 723d915 | cli, main, update | the duplicated box-refusal printer, the hand-built update usage line, the separate `--version`/`--help` match, update's draft/prerelease check, a re-validation in `latest()`, a serde compatibility default, a restating comment. `image --help` prints both image verbs. |
| e715b56 | runtime, clean, image | `local_image_id`, `IMAGE_STEM`, the per-box map in `in_use_images`, Apple's wait constants and duplicated builder lookup, a one-use closure. Fix: prune only owner-labelled boxes. |
| d3713e8 | box, plan, proxy, login | three env-default loops, `Login::check`'s second origin parse, the per-rules TLS config and https scan, three unreachable `ok_or`s, a redundant head check, nix flock where std's does the same. Fixes: `box down` flag-like names; Content-Length `+5`. |
| 078a120 | pi, profile, config, trust | insert-by-insert maps, a duplicated build command, a comment the spec states, a hand-framed hash, field-restating doc comments. |
| b7d9acb | e2e | the test-audit deletions below; one image-id filter, egress-log reader, `pinfold_in` and `allow`; the second `cargo metadata` call. |
| 6c808b6 | scripts | a KeyError abort, the restating docstring, a draft check, the exception wrapper. |
| 8de9236 | docs | the spec pass below. |
| 9ad5eec | release | codex 0.160.0 → 0.160.1; workspace 0.1.7. |

The e2e harness change spans every test file, so it landed as one commit
rather than one per test file.

### Bugs found and fixed

- `pinfold box down NAME` passed NAME into `podman rm -f -t 0 NAME` and
  `container rm -f NAME`. A flag-like name such as `--all` was read as a
  flag. Found by reading the code; not reproduced on the host, where it
  would have removed every container. Row 9's test now passes
  `--filter=label=<this box's label>`, which with the bug removes only the
  test's own box.
- `Content-Length: +5` parsed through `u64::from_str` and was forwarded.
  Row 3 now refuses it as ambiguous framing. Every invalid Content-Length
  now logs `ambiguous framing` rather than `malformed request`; it is a
  400 either way.
- Podman copies image labels onto containers, so a user's own container
  from a pinfold image looked like a dead pinfold box and was pruned. Row
  15's test now starts one with the runtime's CLI. On Apple, which does
  not copy labels, the scenario is a control.
- `bump-pins.py` aborted the nightly on a claude manifest missing a
  pinned platform; that harness now waits.

## Test audit

| Row | Disposition | Lines |
| --- | --- | --- |
| 32 | Deleted the RTF, `pdftotext` and PNG-magic blocks: one sabotage each already fails the PDF conversion or the render dimensions. | `pi.rs` |
| 6 | Deleted the pi-side non-POSIX env-name refusal; `up_refuses_before_it_creates` keeps the harder case. | `pi.rs` |
| 11 | Deleted the `mkdir .vscode` twin. | `pi.rs` |
| 14 | Rewrote the bundled-skill check to match the skill's own description, not pi's prompt XML. | `pi.rs` |
| 28, 29, 31 | Deleted the state-dir count, the reprint check and four exit-code checks the rows do not state. | `update.rs` |
| 30 | Deleted two fresh-build twins of the post-rebuild controls. | `image_warning.rs` |
| 17, 26, 9 | Deleted six repeated refused-box names, the codex detail prose check and two ready-image-id twins. | `box_.rs` |
| 5 | Reworded a comment that claimed more than the block checks. | `box_.rs` |
| 3, 9, 15 | Extended for the fixes above. | `box_.rs` |

Test lines −39 net (+257 / −296) beside binary lines −89 (+205 / −294).
The rewrites that need more than one sitting are tickets Y-4 and Y-5.

## Written tickets (parked)

- Y-2: bound the codex login helper's `getAuthStatus` wait (it can hang
  `up` while holding the host-wide lock).
- Y-3: protected-path prepare leaves empty `.claude`/`.idea`/`.vscode` on
  a refusal, and cleanup is skipped when planning fails.
- Y-4: strengthen `update.rs`'s blocks for rows 28 and 29.
- Y-5: strengthen `box_.rs`'s blocks for rows 2, 3 and 24.
- Y-6: one curl downloader for harness artifacts and self-update.
- Y-7: spec lines no test exercises (trace-table gaps).
- Y-8: guarantee 15's test races other tests' daily maintenance pass.

## Adam's list

Asked at the end of the pass; all four answered as recommended and landed
before the tag.

1. `box exec`, `stat` and `down` with a valid name acted on any host
   container, not only pinfold's. Answer: restrict them to boxes carrying
   `dev.pinfold.owner`, in 0.1.7. A foreign container is now absent to all
   three; row 9 carries the scenario.
2. The top-level `help` verb was an undocumented alias of `--help`.
   Answer: delete it (a hard break, named under Caller changes).
3. "Runtime command lines are built as data." under Always applied.
   Answer: keep. The `box down` fix shows argv as data is not enough by
   itself, so names are validated too.
4. `pinfold init` was in the help table and the spec's CLI block, though
   only the box runs it. Answer: spec only. `pinfold --help` no longer
   lists it; `pinfold init --help` still prints its syntax.

## CI

Linux x64 passed on the first push. Linux arm64 failed in guarantee 15's
test at a check this pass did not change: another test's daily maintenance
pass, run in its own fresh state dir, trimmed the scenario's 1970-dated
build tags in the shared podman store first. Run 36675855532 failed the
same way on the dead box. Filed as Y-8 (P1, parked); the failed job was
re-run to confirm the race.

## Rejected

| Finding | Why not now | Re-admit when |
| --- | --- | --- |
| Profile seed walk reuses `dirs`' walker (−15) | Two modules; a symlinked seed would start being copied. | Either walker changes. |
| `dirs` manifest without sort (−1) | Keeps the manifest deterministic. | Determinism stops mattering. |
| `StateDir` holds `StateFile` (−4) | Touches cli.rs readers. | `pi/state.rs` next changes. |
| Drop the empty-top filter in launch (−1) | Unreachability argued, not shown. | A CLI repro shows it unreachable. |
| `Runtime::name` → `program` (−9) | Changes doctor's runtime text. | Doctor output changes anyway. |
| Podman runtime-dir check without `is_dir` (−5) | Loses a clear error for a non-directory. | Never alone. |
| State dir created 0700 by `DirBuilder` (−4) | Changes a control's mechanism on both adapters. | Its own change with both suites. |
| Merge Apple's wait into `make_proxy_connectable` (−4) | Crossed a parallel edit. | apple.rs next changes. |
| `Runtime::up` returns the `Command` (−3) | Trait change for three lines. | The trait changes anyway. |
| One image-trim function (−25) | A build would also trim other sources, which row 15's wording rules out. | The spec says a build trims every source. |
| Downloader consolidation (−18) | A rework across modules. | Y-6. |
| Doctor's single runtime guard (−7) | Changes doctor's output layout. | Doctor output changes anyway. |
| Update's early-return path in main (−3) | Touches update lock ordering. | Y-4. |
| `executable_lock` takes the path (−3) | Self-update locking, row 28. | Y-4. |
| `host_tty` (0) | Saves no lines. | Never. |
| `box down` waits on flock (−7) | Drops the 10 s cap; a hung owner would block forever. | A bounded flock wait exists. |
| Unreachable path-component guard (−6) | Guards writes from user profiles into `$HOME`. | Never. |
| Empty `from` special case (−5) | Reachable: an env entry with an empty name resolves. | Never. |
| One route-address parser (−4, −7) | Proxy routing is a control. | Its own change. |
| SNI parsing shrinks (−14) | Change spec-named refusal tokens. | A spec change. |
| `TestDir` → `env.dir` (−11) | Churn in every test file. | A test-file rework. |
| `localhost/` stripped once (−2) | Changes podman names `ImageCleanup` removes. | The harness's image helpers change. |
| CI edits (−57 total) | Release automation cannot be verified locally. | Each with a dispatched run. |
| Delete `profile/pinfold.toml` and the `PI_OFFLINE` line (−10) | `profile new` copies that file; the line states behavior. | Profiles stop copying it. |
| Cut ARCHITECTURE.md's release conditions (−4) | The spec is binding; AGENTS.md repeats it, not the reverse. | Never. |

## Spec pass

- ARCHITECTURE.md 918 → 866 lines: the Code file tree, runner internals,
  duplicated env defaults, label, image-rm and doctor prose, rationale
  sentences, and restatements of AGENTS.md and README.md. It now states
  `-V`/`-h`, unknown config keys and a profile's `containerfile` refusal,
  which the code enforced without a spec line.
- README.md: dropped the nightly-process sentence and the init role;
  install snippets need no per-release version edit.
- Guarantees table: 32 rows, 32 tests, one each, no gaps (trace table 3).
  Rows 3, 6, 9, 11, 15 and 32 edited to match this pass.
- Open questions: none closed. The `198.18/15` block, Apple's nested user
  namespaces, herdr's `HERDR_AGENT` and the missing disk cap are all
  still open.

## Verification

On macOS ARM64: `cargo fmt --check`, Clippy with warnings denied and Ruff
passed. Each commit builds on its own (`cargo check --all-targets`). The
full Mac suite passed (32 tests, 190 s) on the cleanup commits, again (193 s) after the answers to Adam's list, and again
on the release commit with codex 0.160.1 (194 s). `bump-pins.py` ran live: codex
moved; ponytail 4.13.0 is inside its wait. Linux suites and release
builds run in CI on the push.
