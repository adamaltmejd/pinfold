# Pinfold release trace tables (read-only audit, 2026-10-15; refreshed at 0f10f69)

Source checkout: `/Users/adam/.codex/worktrees/pinfold-lean-implementation/pinfold`. Source files were read; no tests or builds were run in this trace audit. Code snapshot is commit `0f10f69`, with the guarantee-overview correction staged in `docs/ARCHITECTURE.md`. The checkout is not clean. `rg` located references. `AGENTS.md` and `docs/ARCHITECTURE.md` are the policy and binding spec. Test links below identify scenarios, not proof that a suite passed in this audit.

## Configuration keys

`config.rs` reads seven TOML fields and seven corresponding environment selectors (six named `PINFOLD_*` plus the `PINFOLD_ENV_*` family). The TOML and environment forms share a row where appropriate. `profile` in a profile's own TOML is explicitly ignored at `config.rs:56-67`.

| Key and source | Binding spec | Test that sets it | Assessment |
|---|---|---|---|
| `profile` / `PINFOLD_PROFILE` (`config.rs:127,161`) | `ARCHITECTURE.md:855-865` | `pi.rs:1069` sets env; `pi.rs:1468` sets project TOML; `pi.rs:1304` checks invalid UTF-8 | Behavior covered. Profile TOML value ignored by code, but no test sets that value. |
| `allow` / `PINFOLD_ALLOW` (`config.rs:129,163`) | `ARCHITECTURE.md:855-858,866,874-877` | `pi.rs:1174,1184` sets both; `pi.rs:1379` sets project; `pi.rs:1469` sets profile; `pi.rs:1305` invalid UTF-8 | Covered by guarantees 16 and 18. Empty override is stated in spec but no direct scenario located. |
| `routes` / `PINFOLD_ROUTES` (`config.rs:130,164`) | `ARCHITECTURE.md:867,874-878` | `pi.rs:1174,1188` sets both; `pi.rs:1071` sets env; `pi.rs:1306` invalid UTF-8 | Route replacement exercised by guarantee 16. Profile TOML route level was not located. |
| `protect` / `PINFOLD_PROTECT` (`config.rs:131,168`) | `ARCHITECTURE.md:868,874-878` | `pi.rs:805,1174,1187` sets project/env; `pi.rs:1307` invalid UTF-8 | Covered by guarantees 11 and 16. Profile TOML level not located. |
| `containerfile` (`config.rs:128`) | `ARCHITECTURE.md:869,880-882,890-893` | `pi.rs:514` sets project; `pi.rs:1484-1486` sets invalid profile value | Project trust and profile refusal covered by guarantees 12 and 18. |
| `cpus` / `PINFOLD_CPUS` (`config.rs:132,171`) | `ARCHITECTURE.md:870` | `pi.rs:1174,1185` sets both; `pi.rs:1308` invalid UTF-8 | Kernel quota asserted in guarantee 16 (`pi.rs:1201-1217`). |
| `memory` / `PINFOLD_MEMORY` (`config.rs:133,178`) | `ARCHITECTURE.md:871` | `pi.rs:1174,1186` sets both; `pi.rs:1309` invalid UTF-8 | Runtime limit asserted in guarantee 16 (`pi.rs:1195-1199`). |
| `PINFOLD_ENV_<NAME>` (`config.rs:108-114`) | `ARCHITECTURE.md:872`; host env rule at `:785-787` | `pi.rs:1540` sets secret; `pi.rs:1072` sets OpenAI key | Guest receives variable in guarantee 6 and fake model uses it in guarantee 14. |

The keys themselves all have spec lines and at least one test setting them. The unexercised layer combinations above are coverage distinctions, not grounds for adding tests under `AGENTS.md`'s fixed-list rule. If a missing behavior matters, extend its existing guarantee test without a new named test.

## CLI verbs and flags

CLI table at `ARCHITECTURE.md:895-912`; box process interface at `:117-128`; `--version`/help at `:919-922`. `main.rs:42-64` dispatches the top-level verbs, while `cli.rs` parses most flags. This table traces behavior tests. The test policy deliberately excludes argv shapes, file layout and help wording as standalone assertions.

| Verb / parsed flag | Spec | Behavior exercised by existing test/gate | Assessment |
|---|---|---|---|
| `pi [args…]`, shim `pi` | `:773-790,898` | `pi.rs:1538-1540` starts RPC; `pi.rs:1055` uses shim; many `pi --version` calls | Covered by guarantees 6, 11-14, 16. |
| `attach`, `--box` | `:800-803,899` | `pi.rs:395-396` selects among two live boxes; `update.rs:180` uses attach in notice scenario | Guarantee 13 covers selection. |
| `build`, `--profile` | `:900`, Images `:484-502` | `pi.rs:520,550,555` project build; `e2e/src/lib.rs:287` helper builds named profile | Both paths exercised (helper passes `--profile`). |
| `image build`, `--containerfile`, `--context`, `--label`, `--no-cache` | `:901`; process interface `:126`; build contract `:128-149` | `box_.rs:3022` guarantee 23, calls at `:3168-3170,3400-3404` | Covered. |
| `image rm` | `:902`; process interface `:127`; Maintenance `:738-740` | `box_.rs:2176` guarantee 15, helper at `:3410-3411` | Covered. |
| `allow` | `:890-893,903` | `e2e/src/lib.rs:313-314` helper called from `pi.rs`; changed policy at `pi.rs:429` | Covered by guarantee 12. |
| `profile new`, `--from`, `--builtin`, `--from-project [PATH]` | `:629-634,904` | `pi.rs:47-52` uses `--from documents --builtin`; `pi.rs:334` uses `--from-project` without PATH; `pi.rs:1001` uses `--from default`; `update.rs:296-301` uses builtin | Optional explicit PATH not located. Existing guarantee 13 covers copied project state, guarantee 31 builtin override. |
| `clean`, `--dry-run`, `--unused AGE` | `:742-755,905` | `box_.rs:2176` guarantee 15; `:2519,2681` flags | Covered. |
| `doctor` | `:754-764,906` | `cli.rs:72` guarantee 27 | Covered. |
| `artifacts` | `:884-888,907,914-917` | `box_.rs:4639-4647` guarantee 19 | Covered. |
| `update`, `--check` | `:684-714,908` | `update.rs:16` guarantee 28; `update.rs:160` guarantee 29; `:51` uses `--check` | Covered. |
| `config [ROOT]` | `:884-888,909` | `pi.rs:1368` guarantee 18 uses current directory; `pi.rs:1615-1617` helper | Explicit ROOT argument not located; effective report behavior covered. |
| `box up` | process interface `:120`, lifecycle `:91-115` | `box_.rs:25` guarantee 9 and many others | Covered. |
| `box exec`, `--tty`, `--workdir` | process interface `:121`; `cli.rs:789-818` | `box_.rs:25` guarantee 9, `:168-171,201-204` options | Covered. |
| `box stat` | process interface `:122,164-176` | `box_.rs:4544` guarantee 20 | Covered. |
| `box down` | process interface `:123` | `box_.rs:25` guarantee 9, `:408,529` | Covered. |
| `box list`, repeatable `--label KEY[=VALUE]` | process interface `:124`; listing contract `:252-272` | `box_.rs:25` guarantee 9, `:301,329` filters | Label behavior covered; no separate syntax assertion needed. |
| `box prune` | process interface `:125`; Maintenance `:718-736` | `box_.rs:2060` guarantee 8, `:2156` command | Covered. |
| `init` | `:911`; lifecycle `:103-108` | Indirectly starts in every real box; no direct user invocation required | `init` is internal PID 1, yet is in CLI table. |
| `--version` / `-V`, `--help` / `-h` | `:919-922` | `cli.rs:16` guarantee 25 tests long `--version` and `box list --help` | `-V`/`-h` aliases and help wording lack tests by policy. |

## Guarantee to test or standalone gate

Rows are from `ARCHITECTURE.md:947-983`. Each guarantee has exactly one named Rust test (1-34 and 37), except guarantee 35's standalone prototype gate and guarantee 36's Linux slow gate. Guarantee 34's Rust test is ignored in the ordinary suite and run separately. These are static links only.

| # | Guarantee | Test or gate |
|---|---|---|
| 1 | No network but loopback | `box_.rs:3464 box_has_no_network_but_loopback` |
| 2 | Only allowlisted hosts get through | `box_.rs:1807 only_allowlisted_hosts_get_through` |
| 3 | The proxy refuses the tricks | `box_.rs:1864 the_proxy_refuses_the_tricks` |
| 4 | A route reaches exactly one host service | `box_.rs:3526 a_route_reaches_exactly_one_host_service` |
| 5 | Nothing can gain privileges | `box_.rs:1492 nothing_can_gain_privileges` |
| 6 | The environment is exactly the spec | `pi.rs:116 the_environment_is_exactly_the_spec` |
| 7 | No egress means no way out | `box_.rs:4485 no_egress_means_no_way_out` |
| 8 | Losing the owner fails closed | `box_.rs:2060 losing_the_owner_fails_closed` |
| 9 | The lifecycle works for a caller | `box_.rs:25 box_lifecycle_works_for_a_caller` |
| 10 | Host and box share files seamlessly | `box_.rs:1397 box_shares_files_with_the_host` |
| 11 | The box cannot write `.git` or protected config | `pi.rs:560 the_box_cannot_write_git_or_protected_config` |
| 12 | A changed project file stops the run | `pi.rs:430 a_changed_project_file_stops_the_run` |
| 13 | Project state persists and stays separate | `pi.rs:265 project_state_persists_and_stays_separate` |
| 14 | Both pi config levels load behind a route | `pi.rs:973 both_pi_config_levels_load_behind_a_route` |
| 15 | Cleanup removes only pinfold's garbage | `box_.rs:2176 cleanup_removes_only_pinfolds_garbage` |
| 16 | The highest layer sets the allowlist | `pi.rs:1150 the_highest_layer_sets_the_allowlist` |
| 17 | up refuses before it creates | `box_.rs:942 up_refuses_before_it_creates` |
| 18 | A caller reads the effective configuration as data | `pi.rs:1368 a_caller_reads_the_effective_configuration_as_data` |
| 19 | A caller-owned box launches the pinned harness | `box_.rs:4619 a_caller_owned_box_launches_the_pinned_harness` |
| 20 | A caller can tell an OOM kill from a failure | `box_.rs:4544 a_caller_can_tell_an_oom_kill_from_a_failure` |
| 21 | An injecting route keeps the credential on the host | `box_.rs:3763 an_injecting_route_keeps_the_credential_on_the_host` |
| 22 | A caller-owned box cannot write .git | `box_.rs:4730 a_caller_owned_box_cannot_write_git` |
| 23 | A caller builds an image from its own tree | `box_.rs:3022 a_caller_builds_an_image_from_its_own_tree` |
| 24 | Every build reruns its steps | `box_.rs:2957 every_build_reruns_its_steps` |
| 25 | `--version` needs no runtime | `cli.rs:16 version_needs_no_runtime` |
| 26 | A login route keeps the login on the host | `box_.rs:3859 a_login_route_keeps_the_login_on_the_host` |
| 27 | Doctor reports without changing state | `cli.rs:72 doctor_reports_without_changing_state` |
| 28 | Updates verify before atomic replacement | `update.rs:16 host_updates_are_verified_and_atomic` |
| 29 | Interactive update checks are bounded | `update.rs:160 interactive_update_checks_are_bounded` |
| 30 | Changed image inputs prompt a rebuild | `image_warning.rs:12 changed_image_inputs_prompt_a_rebuild` |
| 31 | Changed bundled profiles are reported once | `update.rs:241 changed_bundled_profiles_are_reported_once` |
| 32 | The documents profile reads documents locally | `pi.rs:27 the_documents_profile_reads_documents_locally` |
| 33 | Writable projects exclude host authority | `pi.rs:1329 writable_projects_exclude_host_authority` |
| 34 | Blocked route responses release the upstream | `box_.rs:3643 blocked_route_responses_release_the_upstream`, ignored slow gate per spec `:935-937` |
| 35 | Durable recovery preserves boxed execution | `examples/pi-durable/durable_recovery_preserves_boxed_execution.ts`, standalone gate per spec `:931-934` |
| 36 | CONNECT tunnels share activity | `scripts/slow-proxy.sh`, standalone Linux slow gate per spec `:938-943` |
| 37 | Linux boxes apply their own seccomp profile | `box_.rs:1579 linux_boxes_apply_their_own_seccomp_profile` (Linux E2E; syscall and benign control at `box_.rs:1790-1800`) |

## Profile and spec pass

Read all 11 files under `profile/` and the corresponding spec at `ARCHITECTURE.md:534-547,615-659`. The image fragments match the three tool sets: default base packages, documents' Bun/AnyDoc/Poppler, full's gh/rtk/ponytail, and hardening strip. The pi package files match the described extension and skill selection. `profile/pinfold.toml` sets no keys, as `:657-659` states. No source-specific profile drift or defensible YAGNI cut found in this bounded read. The `read-documents` skill is user guidance; the binary remains the security boundary under `AGENTS.md`.

`ARCHITECTURE.md:381-382` has an explicit Apple `container#1941` revisit trigger; parent task reports it remains open today, so it has not fired. `core/login.rs:136` has a `getAuthStatus` pin-removal trigger; parent task reports the Codex pin has not removed the method. The four open questions remain at `ARCHITECTURE.md:1016-1023`. This audit found no evidence within profile or spec to close one; in particular, `HERDR_AGENT=pi` in `pi/launch.rs:150` shows what Pinfold sets, not where herdr reads it.

No dependency mismatch was found by static comparison of `crates/pinfold/Cargo.toml:7-16` with `ARCHITECTURE.md:1006-1010` (normal direct dependencies). A `cargo tree` run was outside this subtask. The initial trace was made while the worktree had four uncommitted pin edits. At this refresh, code HEAD is `0f10f69` and the guarantee-overview correction is staged; this audit changed no repository file.
