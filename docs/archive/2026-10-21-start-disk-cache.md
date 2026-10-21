# Linux start cost and the e2e budget (2026-10-21)

PR #157 (Firecracker) ran the Linux e2e suite in 251, 284 and 332 s on
identical code, against a 300 s budget. These measurements find where the
time goes and why it varies.

## Spread between CI runs

Each test's finish time, relative to the suite start, in the 332 s run
divided by the same in the 251 s run, lies between 1.23 and 1.40 for all
39 tests. No test moves alone. The spread is runner speed, not a cold root
disk export or contention landing in a different test.

## Per-start cost

A debug build on the homelab trial VM (2 vCPUs, nested KVM) logged each
disk `prepare` wrote during one full suite run (570.7 s):

| Component | Writes | Total | Mean |
|---|---|---|---|
| init disk (the 70 MB debug CLI) | 108 | 88.0 s | 0.82 s |
| root disk (`podman export`, `mkfs`) | 108, 27 cold | 77.6 s | 0.72 s |
| pi harness image | 37 | 51.0 s | 1.38 s |
| codex harness image | 3 | 13.4 s | 4.48 s |
| claude harness image | 2 | 9.3 s | 4.65 s |
| profile `share/` | 27 | 0.6 s | 0.02 s |
| caller mounts | 7 | 0.1 s | 0.02 s |

A warm start with no mounts reaches `ready` in 2.76 s; a cold root disk
adds 3.6 s. The 27 cold root disks are distinct image ids from build
tests and cannot be shared.

## Change

The init disk is written once per executable content under
`artifacts/init/`, and a pinned harness image once per version beside its
tree. `share/` and caller mounts stay per start: together they cost under
a second per suite.

## Result

Sequential (`--test-threads=1`) on the same VM: 758 s before, 671 s after.
The after run includes a +50 s regression in the secret scan, which read
all of `artifacts/`; it was narrowed to the init disks and pi's harness.

| Test | Before | After |
|---|---|---|
| pi::both_pi_config_levels_load_behind_a_route | 91.2 s | 66.5 s |
| runtime::cleanup_removes_only_pinfolds_garbage | 59.1 s | 57.0 s |
| box_::box_lifecycle_works_for_a_caller | 55.1 s | 51.4 s |
| box_::a_caller_can_tell_an_oom_kill_from_a_failure | 54.7 s | 40.8 s |
| image_warning::changed_image_inputs_prompt_a_rebuild | 53.4 s | 40.4 s |
| box_::a_caller_owned_box_launches_the_pinned_harness | 46.7 s | 24.2 s |

## Not done

- Init disks accumulate, one per executable, and are never pruned, as the
  macOS init already does.
- A staging file left by a killed writer (`.tmp-<pid>-…`) is not swept.
