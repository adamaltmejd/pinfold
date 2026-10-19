# Configuration, CLI and guarantee traces, 2026-10-14

Read-only source lookup at HEAD 198600b plus the dirty working tree. Files changed concurrently: ARCHITECTURE.md now has uncommitted guarantee 37 and the Linux per-box seccomp rule; box_.rs now has its test. The line references below reflect that snapshot (ARCHITECTURE.md 1020 lines, box_.rs 5454-line source), not a committed release. This trace pass performed no test assertion audit or runtime validation; the main report covers those separate activities. Config, CLI, main, update, Pi and profiles were read whole. All guarantee rows were matched with rg to named tests/gates.

## Config keys read -> Configuration spec -> setting in E2E

Paths below are relative to the repository. pi test paths abbreviate crates/e2e/tests/e2e/pi.rs.

| Key | Config reader | Spec line | E2E setting and owning guarantee |
|---|---|---|---|
| profile / PINFOLD_PROFILE | config.rs:57,127,161 | ARCHITECTURE:858,863 | pi.rs:990 env in G14 both_pi_config_levels_load_behind_a_route; pi.rs:1389 project profile in G18; image_warning.rs:36 project profile in G30 |
| allow / PINFOLD_ALLOW | config.rs:101,129,163 | ARCHITECTURE:855,864,874 | pi.rs:1095 project allow and :1105 env allow in G16 the_highest_layer_sets_the_allowlist; :1299 in G18 |
| routes / PINFOLD_ROUTES | config.rs:104,130,164 | ARCHITECTURE:865,875 | pi.rs:1095 project route and :1109 env route in G16; :992 routed fake model in G14 |
| protect / PINFOLD_PROTECT | config.rs:105,131,168 | ARCHITECTURE:866,876 | pi.rs:726 project tooling protection in G11; :1095 project list and :1108 env list in G16 |
| containerfile | config.rs:68,128 | ARCHITECTURE:867,879 | pi.rs:513 project path in G12 a_changed_project_file_stops_the_run; :1407 forbidden profile key in G18; image_warning.rs:55 in G30 |
| cpus / PINFOLD_CPUS | config.rs:106,132,171 | ARCHITECTURE:868 | pi.rs:1095 project 2 CPUs, :1106 env 1 CPU in G16; box cgroup observer :1122 |
| memory / PINFOLD_MEMORY | config.rs:107,133,178 | ARCHITECTURE:869 | pi.rs:1095 project 1G, :1107 env 512M in G16; stat observer :1116; cli.rs:84 host config for G27 |
| PINFOLD_ENV_NAME | config.rs:108-114 | ARCHITECTURE:870; Box :305-311 | pi.rs:1461 fixture env, :145 onward G6 environment assertion; :993 OPENAI_API_KEY in G14 |

All config.rs keys have a current spec line and a real E2E setting. Profile selection ignoring its own profile key and exact empty overrides are explicit contracts; this lookup does not establish that every layer permutation is asserted. Invalid UTF-8 override values for all six direct env keys are set by G16 at pi.rs:1225-1246. Unknown project/profile keys and profile containerfile are covered at :1391-1433. No unspecced config key found.

## CLI verbs/flags -> spec -> E2E invocation

Every parsed public verb/flag has a current spec entry. 'No direct invocation found' is a trace gap, not a reproduced bug or permission to add a separate test. Argument-shape/help wording tests are excluded by AGENTS.md; any justified harder scenario belongs in an existing guarantee.

| Verb or flag | Parser/source | Spec line | E2E usage / gap |
|---|---|---|---|
| pi; pi shim; arguments | main.rs:19,63; pi/launch.rs:24 | :896; :779 | PiRpc pi.rs:1459; real Pi arguments in G6/G13/G14; shim implemented but not a distinct guarantee |
| attach command | cli.rs:123 | :897,:798-801 | pi.rs:390-403 G13 selected box and explicit command |
| attach --box | cli.rs:178 | :897,:800 | pi.rs:395 G13; update.rs:168 G29 missing-box fixture |
| attach -- separator | cli.rs:126,178 | :919-920 forwarding rule; synopsis command | pi.rs:397 G13 |
| attach default bash; implicit only-box selection | cli.rs:129,155 | :897,:798-800 | no successful direct default-shell/one-box invocation found in bounded trace |
| build | cli.rs:847 | :898 | pi.rs:519,554 G12; image_warning.rs:89 G30 |
| build --profile | cli.rs:849 | :898 | box_.rs:2190-era G15 builds; image_warning.rs:profile build helper; E2E default_image helper |
| image build | cli.rs:921 | :899,:126 | box_.rs:2977 G23; :3355 helper |
| image build --containerfile | cli.rs:994 | :899 | box_.rs:3355 G23 helper |
| image build --context | cli.rs:999 | :899 | box_.rs:3357 G23 helper |
| image build --label KEY=VALUE | cli.rs:1004 | :899,:511-512 | box_.rs:3359 G23 |
| image build --no-cache | cli.rs:1012 | :899,:513 | box_.rs:3123-3135 G23 random RUN stamp |
| image rm | cli.rs:957 | :900,:736-738 | box_.rs:3365 G23 helper and G15 shared-image retirement |
| allow | cli.rs:111 | :901,:888-891 | pi.rs:1098 G16; G12 allowed controls at :554; all trust fixture helpers |
| profile new | cli.rs:1033 | :902 | pi.rs:333 G13; :922 G14 |
| profile new --from | cli.rs:1097 | :902,:627-629 | pi.rs:922 G14; update.rs:287 G31 |
| profile new --builtin | cli.rs:1096 | :902,:628-629 | pi.rs:49-51 G32; update.rs:289 G31 override bypass |
| profile new --from-project omitted PATH | cli.rs:1104 | :902,:630-632 | pi.rs:333 G13 |
| profile new --from-project explicit PATH | cli.rs:1105 | :902,:630-632 | no direct invocation found; existing G13 covers copy behavior using current project |
| clean | cli.rs:429 | :903,:740 | box_.rs:2131 G15 |
| clean --dry-run | cli.rs:582 | :903,:752 | box_.rs:2474,2540 G15 |
| clean --unused AGE | cli.rs:583,598 | :903,:748-750 | box_.rs:2636 G15 (0s); other suffix shapes not separately tested by design |
| doctor | cli.rs:624 | :904,:752-762 | cli.rs:72 G27 doctor_reports_without_changing_state |
| artifacts | cli.rs:715 | :905,:912-915 | G19 box_.rs:4574 compares guest harness versions with artifacts pins |
| update | update.rs:175 | :906,:682-698 | update.rs:16 G28 |
| update --check | update.rs:178 | :906,:682 | update.rs:52 G28 |
| config | cli.rs:728 | :907,:882-886 | pi.rs:1289 G18; helper :1537 |
| config ROOT explicit argument | cli.rs:731 | :907 | no direct invocation found; G18 uses current_dir |
| box up | cli.rs:202 | :120 | box_.rs:4999 helper, G9 :25, G17 :909 |
| box exec | cli.rs:321,779 | :121,:162-163 | G9 box_.rs:25; helpers throughout |
| box exec --tty | cli.rs:790 | :121 | box_.rs:171 G9 |
| box exec --workdir | cli.rs:791 | :121 | box_.rs:138 G9; durable gate :45 |
| box exec -- separator | cli.rs:789 | :121 | all box_exec helpers; G9 |
| box stat | cli.rs:335 | :122,:165-176 | box_.rs:4499 G20; pi.rs:1116 G16 |
| box down | cli.rs:191 | :123,:241-250 | G9 :375,:496 etc; absent/invalid-name controls |
| box list --label KEY=VALUE | cli.rs:817 | :124,:259-260 | box_.rs:206 G9; helper :5055 owner label |
| box list --label KEY | cli.rs:827 | :124,:259-260 | update.rs:204 G29 setup; not a direct key-existence assertion |
| box list repeated --label AND | cli.rs:384,817 | :124 | no repeated Pinfold list-label invocation found; box_.rs:2331 is native image build, not this seam |
| box prune | cli.rs:409 | :125,:271 | box_.rs:2111 G8 |
| init | main.rs:52 | :909,:103-105 | real PID1 through every native box; G9 child reaping; no direct host init invocation needed |
| --version / -V | main.rs:43 | :917 | cli.rs:39 G25 --version; -V alias has no direct invocation |
| --help / -h and nested help | main.rs:47; cli.rs:90 | :917-920 | cli.rs:59 G25 box list --help; update.rs:180 G29 attach --help; -h alias no direct invocation |

Material trace gaps to retain as observations: config ROOT, explicit --from-project PATH, and repeated list-label AND semantics. They are documented interfaces, not YAGNI. Do not delete or add new guarantees simply because a branch has no direct invocation.

## Guarantee -> named test or standalone gate

Spec row lines are from current dirty snapshot. Rust paths abbreviate crates/e2e/tests/e2e/.

| # | Spec | Test/gate |
|---|---|---|
| 1 | ARCHITECTURE:944 | box_.rs:3419 box_has_no_network_but_loopback |
| 2 | :945 | box_.rs:1762 only_allowlisted_hosts_get_through |
| 3 | :946 | box_.rs:1819 the_proxy_refuses_the_tricks |
| 4 | :947 | box_.rs:3481 a_route_reaches_exactly_one_host_service |
| 5 | :948 | box_.rs:1459 nothing_can_gain_privileges |
| 6 | :949 | pi.rs:115 the_environment_is_exactly_the_spec |
| 7 | :950 | box_.rs:4440 no_egress_means_no_way_out |
| 8 | :951 | box_.rs:2015 losing_the_owner_fails_closed |
| 9 | :952 | box_.rs:25 box_lifecycle_works_for_a_caller |
| 10 | :953 | box_.rs:1364 box_shares_files_with_the_host |
| 11 | :954 | pi.rs:559 the_box_cannot_write_git_or_protected_config |
| 12 | :955 | pi.rs:429 a_changed_project_file_stops_the_run |
| 13 | :956 | pi.rs:264 project_state_persists_and_stays_separate |
| 14 | :957 | pi.rs:894 both_pi_config_levels_load_behind_a_route |
| 15 | :958 | box_.rs:2131 cleanup_removes_only_pinfolds_garbage |
| 16 | :959 | pi.rs:1071 the_highest_layer_sets_the_allowlist |
| 17 | :960 | box_.rs:909 up_refuses_before_it_creates |
| 18 | :961 | pi.rs:1289 a_caller_reads_the_effective_configuration_as_data |
| 19 | :962 | box_.rs:4574 a_caller_owned_box_launches_the_pinned_harness |
| 20 | :963 | box_.rs:4499 a_caller_can_tell_an_oom_kill_from_a_failure |
| 21 | :964 | box_.rs:3718 an_injecting_route_keeps_the_credential_on_the_host |
| 22 | :965 | box_.rs:4685 a_caller_owned_box_cannot_write_git |
| 23 | :966 | box_.rs:2977 a_caller_builds_an_image_from_its_own_tree |
| 24 | :967 | box_.rs:2912 every_build_reruns_its_steps |
| 25 | :968 | cli.rs:16 version_needs_no_runtime |
| 26 | :969 | box_.rs:3814 a_login_route_keeps_the_login_on_the_host |
| 27 | :970 | cli.rs:72 doctor_reports_without_changing_state |
| 28 | :971 | update.rs:16 host_updates_are_verified_and_atomic |
| 29 | :972 | update.rs:148 interactive_update_checks_are_bounded |
| 30 | :973 | image_warning.rs:12 changed_image_inputs_prompt_a_rebuild |
| 31 | :974 | update.rs:229 changed_bundled_profiles_are_reported_once |
| 32 | :975 | pi.rs:26 the_documents_profile_reads_documents_locally |
| 33 | :976 | pi.rs:1250 writable_projects_exclude_host_authority |
| 34 | :977 | box_.rs:3598 blocked_route_responses_release_the_upstream, ignored production-deadline slow test |
| 35 | :978 | examples/pi-durable/durable_recovery_preserves_boxed_execution.ts:1, standalone gate |
| 36 | :979 | scripts/slow-proxy.sh:2 invokes scripts/connect_tunnels_share_activity.py:109 host fixture, standalone Linux slow gate |
| 37 | :980 | box_.rs:1546 linux_boxes_apply_their_own_seccomp_profile, Linux only, newly uncommitted |

All 37 current guarantee rows map to a test/gate. No unmatched test-level guarantee found. This is traceability, not proof that every assertion or guarantee currently passes. G12's lack of stable refusal reason is the existing docs/archive/2026-10-13-trust-refusal-test-ticket.md:9-23; do not file a duplicate or describe it as a reproduced product bypass.
