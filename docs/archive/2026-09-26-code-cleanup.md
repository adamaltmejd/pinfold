# 2026-09-26: code-cleanup pass and spec pass for v0.0.5

The `code-cleanup` skill before the v0.0.5 release, on main at 142ab2e
after v0.0.4 and four tickets (Y-89 to Y-92). No lane was open.

The last whole-tree pass (2026-09-25, `2026-09-25-code-cleanup-2.md`)
ran the evening before. This pass read only the modules that changed
since v0.0.4: `cli`, `core/box` with `plan`, `core/clean` with `image`
and `artifacts` and `harnesses.toml`, the spec with `profile/` and
`scripts/`, the test sweep over the changed blocks, and the user-facing
docs. `core/runtime`, `core/proxy`, `config`, `dirs`, `trust`, `init`
and `crates/e2e/src/lib.rs` were unchanged apart from comments. `pi/`
changed by one constant and was not read.

## Numbers

| Measure | Before (142ab2e) | After (release commit) |
|---|---|---|
| Commits since v0.0.4 | 8 | 16 (8 from this pass and release) |
| Lines added / deleted since v0.0.4, whole tree | 829 / 271 (delete/add 0.33) | 926 / 464 (0.50) |
| Lines added / deleted since v0.0.4, `crates/` | 550 / 191 | 640 / 376 |
| This pass, whole tree | | 229 / 325 (delete/add 1.42) |
| This pass, `crates/pinfold` | | 151 / 252 |
| This pass, `crates/e2e` | | 65 / 59 |
| This pass, docs, scripts, skills | | 10 / 11 |
| Rust lines, whole tree | 11095 | 11003 |
| Largest source file | `crates/e2e/tests/box.rs`, 2566 | 2572 |
| Largest file in the binary crate | `cli.rs`, 1095 | 1081 |
| `docs/ARCHITECTURE.md` | 739 lines | 740 |
| Tickets landed since the last report | 4 (Y-89 to Y-92) | 4 |
| Tickets born from a lane proposal | 0 | 0 |
| Suite wall time | | box 99.2 s, pi 27.8 s, cli 0.3 s (host run, fresh profile image) |

## Step 1

The nine dependencies match the spec's list. Clippy with `dead_code` was
clean. There is no TODO in the tree. No open question is answered by
the suite.

## Step 2

Code agents (Opus) claimed about 136 removable lines, the spec agent 6.
The docs agent found nothing ("Lean already"). The test sweep kept all
eight changed blocks and asked for three to reach their hardest case.

## Landed

| Commit | Module | Removed |
|---|---|---|
| b21f012 | box, plan | `ResolvedProfile` and one-caller `resolve_seeding`; the one-field `Parts` (now `Option<Child>`); one-caller `open_mount_dir`; `validate_guests`' second duplicate loop; the longer `valid_memory`; two doc lines restating their item. |
| b509482 | plan, image | The second copy of the `dev.pinfold.` label refusal: one `plan::check_reserved_labels` for `box up` and `image build`. |
| 467104a | artifacts, image, clean | `os_arch()` and its unreachable error (a `const`); the `harnesses()` accessor; the empty-assets refusal for a state the embedded pin file cannot reach; `Build.repository` (derived from the family label); restated comments and the pin file's header; the longer `path_bytes`. |
| ec1504c | cli | `clean`'s second listing (it removes exactly what it measured and printed); a one-use constant; `syntax`'s collect-and-join; `--from-project`'s clone-and-peek (`next_if`); `profile new`'s parent guard. |
| 85fa3dd | scripts | `bump-pins.sh`'s one-caller `sha256()`. |
| 8503cdf | e2e | Hardest cases, no assertion removed: row 15's dead box names the other project (sabotage: record a project as live before the liveness check), and three runtime build blocks became one closure; row 17 refuses `255M` beside `1000`; row 19 asserts `PINFOLD_ALLOW` for every harness. |
| 61c9452 | spec, skill | Row 15 names `--dry-run`, `--unused 0s` and a dead box protecting nothing; the `--unused` bullet states the live-box exception the test proves. The skill's release step runs `scripts/bump-pins.sh`. |

The three test edits were written without being run against their
sabotages, per the 2026-09-25 decision; the suite passed with them.

## Rejected

| Finding | Why | Re-admit when |
|---|---|---|
| doctor's `image:` section, as a duplicate of `config`'s `image_built` | Not a duplicate: doctor also compares a project image's recorded profile image with the current one, which the spec names. | `config` reports staleness too. |
| `validate` dropping its own `validate_guests(None)` | Changes a refusal's reason (`profile` before `spec`) for a spec with both faults. | Never as a cleanup. |
| `base_digest` via `from_utf8_lossy` | Changes behavior on a non-UTF-8 Containerfile for -2 lines. | Never as a cleanup. |
| A `latest_tag` helper in `bump-pins.sh` | Net zero lines. | It saves lines. |

## Spec pass

ARCHITECTURE.md and README.md: the spec agent and the docs agent found
no restatement or rationale to cut. Help matches the spec's CLI block
byte for byte. Rows 17, 19 and 23 match their tests; row 15 was edited
to match (61c9452). No open question was resolved.

## Release

`scripts/bump-pins.sh` moved nothing: pi 0.87.1, claude 2.1.283 and
codex 0.157.0 were already the latest releases, as were bun, rtk 0.50.0
and ponytail. Its first run failed on an empty codex tag from the GitHub
API and wrote nothing; a rerun passed. The script's GitHub calls are
unauthenticated, so a second such failure is a reason to route them
through `gh api`.

The stale Mac profile image was deleted before the suite so the rtk pin
from Y-92 was built and exercised.

## For Adam

- A candidate can edit `scripts/e2e-linux.sh`, the merge-queue gate that
  judges it, and under `approve = "auto"` that lands unread. Y-92's gate
  repair did exactly that (142ab2e, correct: SSH push to HTTPS with gh's
  login). Adding `scripts/` to `protected_paths` in `.yard/config.toml`
  makes such a change stop for the operator. Needs your yes (`.yard`).
