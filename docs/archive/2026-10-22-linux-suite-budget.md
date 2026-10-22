# Linux e2e suite back under the 300 s budget (2026-10-22)

Issue #159, PR #165. Since #141 the Linux CI suite ran over CLAUDE.md's
300 s budget on Firecracker. Every test passed. Times are
`scripts/e2e-budget.py`'s `e2e_elapsed_ns`, from `cargo test` start to
exit, on `ubuntu-26.04` runners (4 vCPU).

**Result:** base b8bf5e9 took 339.8 s. With the changes, 5c95aea took
213.1 s and e546f8d, whose Linux code is the same, 211.4 s: about 110 s
less, 87 s under the budget. Every test passed in every run listed.

## Before

| Run | Commit | Seconds |
|---|---|---|
| 39752865752 | b8bf5e9 (base) | 339.8 |
| 39741118804 | ef4c9ea (#142 branch, same suite as base) | 301.8 |

#159 records 251, 291, 323, 340 and 345 s on identical code at #157.

## Where the time went

Measured with throwaway commits that print each test's lock wait, lock
hold and named phase marks to stderr. They were not merged.

- **A rebuild inside the timed window.** Cargo exports the test package's
  `CARGO_PKG_*` and `CARGO_MANIFEST_*` to tests. ring's build script
  reruns when they change, so `e2e::pinfold()`'s nested
  `cargo build -p pinfold` recompiled ring, rustls and pinfold while every
  test waited on it: 14.97 s in 39741118804, 0.03 s once removed
  (39747488652).
- **Writer drains.** The suite's lock is a fair tokio `RwLock`: a queued
  writer blocks every later reader. libtest starts tests in name order, so
  `box_::disks_persist_and_attach_exclusively` queued mid-run and waited
  34.5 s for readers while new ones queued behind it (39747488652).
  Shared work was 677 thread-seconds; the exclusive holds summed to 68 s.
- **The serial tail.** With every writer last (7a861e2), the exclusive
  holds ran alone at the end: 74 s in 39750194235, of which cleanup took
  38 s and disks 21 s.
- **One long build on the critical path.**
  `pi::both_pi_config_levels_load_behind_a_route` spent 61 s in
  `pinfold build --profile full` before its first prompt, which does not
  use that profile, and ended last of the shared tests (39751480274).

## Changes

1. The nested build drops `CARGO_PKG_*` and `CARGO_MANIFEST_*` (70aa3aa).
2. The tests that take the write guard all live in `runtime.rs`, so they
   start after every shared test (7a861e2).
3. The disks test takes the shared guard on Linux. It leaves no dead box
   that another test's maintenance could prune before it asserts, and
   runs no `clean`. It stays exclusive on macOS, where it failed shared
   (below). `full` builds during the first prompt and the settings run
   and is awaited before the selections (5c95aea, e546f8d).

No assertion, sabotage or scenario changed. Test bodies moved verbatim.

Rejected: private cache and state roots for the exclusive tests. They
isolate nothing that matters here: the box registry is
`/tmp/pinfold-<uid>` for every root (ownership.rs), each root's first
command runs a host-wide daily pass that prunes dead boxes and applies
image retention, and `clean` purges podman's build cache. Narrowing
cleanup's write span to start at its first host-wide hazard saves about
6 s; starting it later masks three of its sabotages.

## After

| Run | Commit | Seconds |
|---|---|---|
| 39748979547 attempt 1 | 7a861e2 | 309.4 |
| 39748979547 attempt 2 | 7a861e2 | 248.2 |
| 39750194235 | 7a861e2 + timing | 280.7 |
| 39751480274 | 7a861e2 + phase marks | 301.4 |
| 39752812679 | 5c95aea | 213.1 |
| 39753227123 | e546f8d | 211.4 |

## Other gates

- labvm (Debian 13, 2 vCPU, nested KVM, podman 6.1.4, `TMPDIR` on the
  root disk because `/tmp` is a 2 GB tmpfs): 39 passed, 1 ignored at
  7a861e2 (606.8 s) and at 5c95aea (528.2 s). e546f8d changes only a
  macOS `cfg`.
- Mac (Apple `container` 1.6.0): 35 passed, 1 ignored, 3 failed at
  e546f8d. The three (`every_build_reruns_its_steps`,
  `a_caller_builds_an_image_from_its_own_tree`,
  `cleanup_removes_only_pinfolds_garbage`) fail the same way on base
  b8bf5e9: 1.6.0 lists built images as `docker.io/pinfold/…` (#166).
- Mac, disks shared (5c95aea): after `container kill` the writer's owner
  exited but its disk stayed attached, so the read-only `up` was refused
  `disk-in-use`, not `disk-unclean`. Filed apart.
