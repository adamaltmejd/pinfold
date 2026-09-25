# 2026-09-25: second code-cleanup pass

The `code-cleanup` skill's whole-tree read, run a second time today, on
main at e794e9b after v0.0.3 and the seven batch-4 tickets (Y-76 to
Y-82). No lane was open. No release was requested with this pass.

The first launch ran the module agents on Sonnet and the docs and trace
tables on Haiku. It was stopped before any agent reported and relaunched
on Opus and Sonnet. The skill now names the tiers with Claude's models in
parentheses (5d7a4f2, f896d00).

## Numbers

| Measure | Before (e794e9b) | After (4349b93) |
|---|---|---|
| Commits since v0.0.3 | 14 | 38 (24 from this pass) |
| Lines added / deleted since v0.0.3, whole tree | 418 / 195 (delete/add 0.47) | 1665 / 2913 (1.75) |
| Lines added / deleted since v0.0.3, `crates/` | 393 / 178 | 1588 / 2853 |
| This pass, whole tree | | 1403 / 2874 (delete/add 2.05) |
| This pass, `crates/pinfold` | | 1036 / 2029 |
| This pass, `crates/e2e` | | 314 / 801 |
| This pass, README, ARCHITECTURE.md | | 22 / 31 |
| This pass, skills | | 31 / 13 |
| Rust lines, whole tree | 12312 | 10832 |
| Largest source file | `crates/e2e/tests/box.rs`, 2786 | 2518 |
| Largest file in the binary crate | `cli.rs`, 1279 | 1084 |
| `docs/ARCHITECTURE.md` | 705 lines | 706 |
| Tickets landed since the last report (Y-75) | 7 (Y-76 to Y-82) | 7 |
| Tickets born from a lane proposal | 0 | 0 |
| Suite wall time | box 118.7 s, pi 34.4 s, cli 0.4 s (Y-79's macOS gate) | box 80.6 s, pi 39.0 s, cli 0.3 s (host run) |

Gate and host times are not comparable, as the last report noted.

Every push ran the Mac suite first and Linux CI after. One Linux run
failed: at 4349b93, `the_proxy_refuses_the_tricks`'s plain-HTTP control
(`curl http://api.github.com/`) timed out with no bytes on the x86
runner, right after its HTTPS control passed; arm passed, and the next
commit, the same code, passed on both. A second occurrence is a ticket
against that block.

## Step 1

The nine dependencies match the spec's list. Clippy with `dead_code` and
`unreachable_pub` was clean. There is no TODO in the tree. The one
revisit trigger, apple/container#1941, is still open. No open question is
answered by the suite.

The trace tables (Sonnet) found every guarantee row tested and these spec
lines untested: the environment forms of `protect`, `cpus` and `memory`;
the TOML forms of `profile` and `routes`; `doctor`; `config ROOT`; a
repeated `--label`; top-level `-h` and `help`. Adam kept them untested.
Row 16's and row 25's rewrites now cover `PINFOLD_ALLOW` and `--help`.
The flags he kept untested this morning (A6) are unchanged.

## Step 2

Eight code agents claimed about 1,660 removable lines, the spec agent
15, and the new user-facing docs agent 12. The test sweep proposed 49
assertion blocks. Landed: 1,471 net lines in 24 commits, 993 of them in
the binary crate. Filed as reworks: about 240 more.

## Landed

| Commit | Module | Removed |
|---|---|---|
| f30e378 | docs | README's restatement of the config layers; the Git section's copy of the threat model's `.git` row. |
| d999451 | runtime | A second spawn-capture block and two "first image or fail" parses (one `captured`, one `inspect<T>`); the podman preflight's two-layer chain; one-caller `cgroup_path`, `parse_list`, `build_argv`, `tighten`, `user`; `local_reference`; the seccomp filter's second pass; `empty_resolv_conf`'s create-once dance; an unreachable init-parent fallback. |
| 7311158 | proxy | `HelloError` and its reason match (the reason is the error); the socket-path length check (bind refuses the same paths); `Cursor`'s bounds arithmetic (`split_at_checked`); a second route resolve; `respond`'s nested match; the header loop's second pass. |
| 0ff4f92 | maintenance | Four path-removal functions (one `remove_paths`); retention's group-sort-rank passes (one `BTreeMap` by build time); `base_image`; a second copy of the name rule; `owner`'s two one-caller helpers; a `DefaultHasher` cache key, whose output std does not keep stable across releases. |
| 597b68d | config | The two always-together Containerfile fields; trust's one-caller read and second refusal path; init's relay loop wrapper; hand-coded profile precedence beside `Layer::over`; `var()`. |
| 0096200 | box | Four copies of the home-seeding refusal and a hand-written deepest-mount loop; `Refusal` and `UpError`'s Display impls; pi's `Shutdown` beside core's `Signals`; `PlanError`; the claim's reclaimed-flag loop; `authority_host`'s second port check; `Proxy::close`, its stop flag and wake-up connect (the owner exits after teardown; spec Lifecycle step 7). |
| 7e348e0 | e2e | Seven spawn-and-assert copies (one `run_ok`), six host-git runners, three fixture accessors, hand-rolled JSON maps, redundant null stdins. |
| a3ab551 | proxy | Nothing: the bind error names the socket again, which 7311158 had lost. |
| 4f4f7fc | docs | From README, the refused runtimes, the preflight's cgroupfs detail, why macOS builds need zig, the trust mechanics and the base image policy, each keeping what a user acts on. The spec's help sentence names `pi` and `attach`'s pass-through. |
| 472d4fb | pi | Two git runners (one `git`), `hooks_path`'s copy of the protected-path check, `ProjectState`, `sanitize`'s char loop, `run_box`'s helpers, `Git::mounts`. |
| c2bbb5a | cli | Three `down` printers, `ExecArgs`, `parse_profile`'s loop, two layers of help lookup, `attach`'s second grammar, `path_string`, `parse_age`'s four error arms. |
| a5e98cd | doctor | The verdict (problem counter, "N problems", exit 1) and trust section (A1); `report_image` (A2). |
| 18b6060 | runtime | Podman's build-cache sizing (`build_cache_bytes`, `BuildCache`, `ListedImage`'s parent and size); spec Maintenance drops "listed with their size" (A3). |
| c4c6fe0 | runtime | `exec_through_init` and `exec_argv`: `Runtime::exec` takes the init. |
| e9840b9 | core | Five sha256-then-hex sites (one `sha256_hex`). |
| 4349b93 | main | The library target nothing but main used; the skill's `-W unreachable_pub`, noise in a bin crate. |

### The test sweep

| Commit | File | Dispositions |
|---|---|---|
| 5360d0a | box.rs | 16 blocks deleted: row 9 (3), row 17 (1, its sabotage moved to the race), row 15 (6), row 23 (2), row 4 (2), row 7 (2). Row 23's shared-ref block rewritten to read the first build's file through its ref. |
| a9a42cc | pi.rs, cli.rs | 15 blocks deleted: row 6 (4), row 13 (3), row 11 (6), row 16 (1), row 18 (1). Rewritten: row 6's refusal asserts the name, not the sentence; row 11's `core.hooksPath` comes from `GIT_CONFIG_GLOBAL`, its hardest case; row 25 compares the workspace version. |
| e4140e6 | box.rs, pi.rs | Approved row cuts (A4): row 1's gateway and 100.100.100.100:53, row 10's read-only mount, row 11's `.git/hooks` and `commondir`, row 22's commit, rename and host-status clauses. |
| 8b54d49, 373ef4b | pi.rs, cli.rs | Approved row growth (A5): row 11's symlinked protect path (and the spec's protect bullet), row 16's environment layer, row 25's `box list --help`, a verb whose daily pass main runs. |

Test lines: +314 / -801. Binary lines: +1036 / -2029.

Row 17's sabotage comment, moved to the race, dates from before the
exclusive claim and may no longer bite. Nobody reran any sabotage, per
this morning's decision.

### What a user sees

- `doctor` exits 0 whenever it can report, prints no verdict and no trust line, and names a missing image as `pinfold pi` does, with the stale warning on stderr.
- On podman, `clean` and `doctor` name the build cache without a size.
- `clean --unused` has one message for a malformed age; `build --profile a --profile b` is refused; a second `attach --box` starts the command.
- Help prints `[--label KEY=VALUE]…`.
- On Linux, a socket path of 104 to 107 bytes now works; maintenance lines name images as podman lists them.
- A failed leftover-socket removal is reported, and stops that batch.

## Tickets, parked

| Ticket | What | Workflow |
|---|---|---|
| Y-83 | `box down ""` deletes every box's state dir. Reproduced with scratch state. | default |
| Y-84 | podman copies the host's proxy variables into every box. Reproduced through `box up` on rootless podman 5.4.2. | default |
| Y-85 | `pinfold pi` takes `.git`, or a whitespace-named top level's parent, as the project root. Both reproduced with `pinfold config`. | default |
| Y-88 | `up` accepts a spec mount at `/opt/pinfold/profile` or `/opt/pinfold/pi`; the profile's mount silently wins. Reproduced on Apple. | default |
| Y-86 | e2e helpers build their own pinfold command, about -140. Depends on Y-83, Y-84, Y-85, Y-88. | heavy |
| Y-87 | proxy: one copy loop, one forward path, one URL split, about -100. | heavy |

## Adam's list and answers

| # | Question | Answer |
|---|---|---|
| A1 | Cut doctor's verdict and trust section (spec L608 names no verdict) | Yes: a5e98cd. |
| A2 | Doctor reuses pi's image check | Yes: a5e98cd. |
| A3 | Podman's build cache named, not sized (spec L511-513) | Yes: 18b6060. |
| A4 | Row cuts: row 1's two addresses, row 10's read-only mount, row 11's hooks and commondir, row 22's commit/rename/status | All: e4140e6. |
| A5 | Row growth: refuse `pi` inside `.git`, row 11's symlinked protect, row 16's environment layer, row 25's `--help` | All: Y-85 for the first, 8b54d49 for the rest. |
| A6 | Cut pi's `GIT_CONFIG_COUNT` merge | No (spec L596). |
| A7 | Untested spec lines from the trace tables | Keep untested. |

## Rejected, with the condition that re-admits it

| Finding | Lines | Why not | Re-admit when |
|---|---|---|---|
| README: the opening, the isolation caveat, the Requirements table | -10 | A user needs them before reading the spec | Never on its own |
| Containerfile: the gh step's header comment | -1 | One of the file's step headers | The other headers go |
| runtime: `run` through `output` | -16 | Hides pull progress and a failed pull's error | A spec line says builds print nothing |
| runtime: nine `#[serde(default)]` on Apple's list JSON | -9 | Checked against container 1.4.1 only; pinfold does not pin it | pinfold pins the runtime's version |
| proxy: TLS config in a `OnceLock` | -9 | Moves a root-load failure from `up`'s refusal to the first dial | A spec line puts root loading at first dial |
| proxy: `ClientConfig::builder()` | -3 | Panics if feature unification adds aws-lc-rs | Never on its own |
| proxy: CONNECT's authority through `authority_host` | -3 | A portless CONNECT goes from 400 to 403 | A spec line on malformed CONNECT |
| plan: the empty-`from` refusal | -5 | `env::var("")` reads an `=foo` entry on macOS | Never |
| cli: `clean` through the daily pass | -7 | `clean` would continue past a failure and exit 0 | A spec line says it does |
| cli: `profile new --from-project`'s directory check | -12 | The error would stop naming the path | `copy_tree`'s errors name the path |
| main: the shim folded into dispatch | -1 | One line | Never on its own |
| pi: `ensure_image` naming `pinfold build` | -6 | With a project Containerfile that builds the wrong image | Never |
| pi: the worktree refusal arm | -12 | The refusal would stop naming worktrees | Its wording is reworked anyway |
| pi: the arm stripping core's `spec: ` prefix | -3 | `pinfold pi` would print "spec: invalid box spec" | Never on its own |
| box.rs: `stat` on an absent box exits 3 | 1 block | Spec L165; exec's block shares the code, not the contract | A row covers `stat`'s absent box elsewhere |
| pi.rs: a write into an existing `.vscode` | 2 blocks | Row 11 names writing `.vscode/`; `tooling` is the configured list | Row 11 drops "writing `.vscode/`" |

Re-admitted from the last report: the owner-gone test's `box_prune` and
the lifecycle body's `runtime_image` scan (Y-80 changed both bodies),
landed in 7e348e0.

## Skill changes

5d7a4f2 and f896d00 name the tiers. 2647f45 splits the docs read: the
spec agent keeps ARCHITECTURE.md and `profile/`, and a user-facing docs
agent reads README and the CLI's help for what a user acts on. 4349b93
drops `-W unreachable_pub` with the library target.
