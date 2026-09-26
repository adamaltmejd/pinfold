# 2026-09-26: code-cleanup pass and spec pass for v0.0.6

The `code-cleanup` skill before the v0.0.6 patch release, on main at
16c8a12 after v0.0.5 and three tickets (Y-93 to Y-95). No lane was open.

The v0.0.5 pass (`2026-09-26-code-cleanup.md`) ran this morning. Like
it, this pass read only what changed since the last release: `core/proxy`,
`cli` with `core/box` and `plan`, `crates/e2e/src/lib.rs`, the spec's
diff, the test sweep over the changed blocks, and the user-facing docs.

## Numbers

| Measure | Before (16c8a12) | After (01236d6, the last cleanup commit) |
|---|---|---|
| Commits since v0.0.5 | 7 | 12 (5 from this pass and release) |
| Lines added / deleted since v0.0.5, whole tree | 331 / 61 (delete/add 0.18) | 316 / 128 (0.41) |
| Lines added / deleted since v0.0.5, `crates/` | 271 / 46 | 258 / 108 |
| This pass, whole tree (before the release commit) | | 106 / 188 (delete/add 1.77) |
| This pass, `crates/pinfold` | | 75 / 145 |
| This pass, `crates/e2e` | | 19 / 24 |
| This pass, `docs/ARCHITECTURE.md` | | 12 / 19 |
| Rust lines, whole tree | 11228 | 11153 |
| Largest source file | `crates/e2e/tests/box.rs`, 2646 | 2646 |
| Largest file in the binary crate | `cli.rs`, 1087 | 1071 |
| `docs/ARCHITECTURE.md` | 755 lines | 748 |
| Tickets landed since the last report | 3 (Y-93 to Y-95) | 3 |
| Tickets born from a lane proposal | 0 | 0 |
| Suite wall time (Mac) | box 83.3 s, pi 23.5 s, cli 0.3 s (Y-94 gate) | box 65.9 s, pi 26.2 s, cli 0.3 s (host run) |

## Step 1

The nine dependencies match the spec's list. Clippy with `dead_code` was
clean. There is no TODO in the tree. No open question is answered by the
suite.

## Step 2

Code agents (Opus) claimed 81 removable lines: proxy 41, cli/box/plan
26, e2e lib 14. The spec agent claimed 9, the docs agent 2. The test
sweep kept 6 of 8 changed blocks, asked to harden one and delete one.

## Landed

| Commit | Module | Removed |
|---|---|---|
| a52f882 | proxy | `RouteLine` (route fields are a `json!` value merged into the line); plain's and route's separate dial-failure 502s (`forward` takes the `Option` and answers 502 once, logging first); `read_status_line`'s copy of `read_head`'s loop (one `read_until`); the send and status-line error arms as one; connect's second malformed-request refusal; a doc sentence `forward` states. -46. |
| 62fa34c | plan, cli | `SpecError`, a `Refusal` with a fixed reason, and cli's copy between them: `Plan::from_reader` returns the `Refusal`. The reserved-label check moved into it, removing `hold_up`'s second refusal site and the comment explaining it. -24. |
| d7ec536 | e2e, spec row 4 | Row 4's route request is a `DELETE`, so a binary that logs a constant `GET` fails (the old request was curl's default). The twin request-count block (the fixture's 404 already proves it answered). The fixture's per-constant docs and longer path parse; `default_image`'s stale build-once clause. -5. |
| 01236d6 | spec | The codex sandbox's reason, which was wrong on macOS; the serde clause and null enumeration of `refused`'s `box`; the Routes bullet's restated plain-origin sentence; the per-build `id` line Images already states; a usage hint on `labels`; `egress_log`'s restated provenance. -7. |

Landed net -82 of 92 claimed.

## Spec pile

The operator answered it (Adam delegates the spec pile). One item: row 4
named `GET`, curl's default, as the method its route line carries. Now
it names `DELETE` (d7ec536). The sabotage, logging "GET" for every
route, is named in the test's comment and was not rerun, per the
2026-09-25 decision.

## Rejected

| Finding | Why | Re-admit when |
|---|---|---|
| proxy: a `Stream` trait with a blanket impl to box both upstream kinds | Adds an abstraction; after the `Option` change only the two `forward` calls remain. | The two calls diverge again. |
| box: compute `egress_log` in `Box::up` rather than returning it from `start` | Moves a fallible `dirs::egress_dir()` out of `start`, whose errors tear down the claim, for -4 lines. | `egress_dir` is infallible. |
| e2e: inline `image_digest` into its one caller | Puts runtime-specific inspect parsing into a long test for -4 lines. | The test shrinks or the helper gains a caller. |
| e2e: row 4 and row 13 parse the egress log once instead of twice | Net zero lines. | It saves lines. |
| e2e: `built_unique_ref` filters on id only | Changes a helper inside a guarantee's test for -2 lines. | That test is otherwise edited. |
| README: cut "Isolation differs by platform…" | Not a restatement: the requirements table names runtimes, not the isolation a user gets. | The table states isolation. |

## Spec pass

The spec agent's cuts landed in 01236d6. Rows 2, 4, 9 and 17, the rows
the release touched, match their tests; row 4 was edited to match
d7ec536. The route line's null status has no row; the spec does not
require one. No open question was resolved. Help matches the spec's CLI
block. README's install examples named 0.0.4; the release commit names
0.0.6.

## Release

`scripts/bump-pins.sh` moved codex 0.157.0 to 0.157.1. pi 0.87.1, claude
2.1.283, bun 1.4.2, rtk 0.50.0 and ponytail 4.10.0 were already the latest.
The Mac suite passed on the moved pins before the tag.
