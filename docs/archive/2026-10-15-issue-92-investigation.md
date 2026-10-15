# Issue 92: retained suite-budget investigations

Evidence snapshot, 2026-10-15. This is an interim record. Issue #92 remains open; final acceptance has not begun. Complete Cargo-command elapsed includes compilation, fixture compilation and image builds. Libtest elapsed is a separate metric. Instrumented diagnostics and isolated microbenchmarks are not acceptance.

## Frozen acceptance

After an independently reviewed measured correction, freeze uninstrumented source, workflows and ordinary worker selection. Run six fresh x64 jobs, interleaving three ordinary supported-cache and three cold-musl-target conditions, plus three ordinary-cache arm64 jobs. The explicit cold condition removes only restored target-triple artifacts; record the paths. Do not prebuild musl artifacts, changed-profile fixtures or images outside the timer.

Every complete `cargo test -p e2e --locked` command must pass every unchanged assertion and finish under 300 seconds. The worst ordinary-cache x64 invocation must also be at most 270 seconds. Publish all nine scheduled results, hardware/cache conditions and exits. Do not retry until fast, discard slow samples, subtract work, change worker settings after inspecting results, or lower thresholds. A failed series stays failed; a diagnosed correction requires a newly declared complete series. Run remaining applicable platform/release gates on final source. This finite engineering criterion measures headroom; it cannot guarantee every future hosted VM.

Controls, fixture isolation, the real 30-second production timeout, rootless keep-id and fresh full/documents builds remain. G9's stopped-owner down and competing claim stay ordered, including the post-down-timeout full wait. No checksum verification, debug assertion, overflow check or test assertion is removed.

## Baselines and G9 scheduling

Earlier retained libtest totals of 255.436 and 310.035 seconds on the release tree established variation, not a regression cause. Delay accumulated across compilation, runtime phases and final image/harness work.

[Four fixed baseline diagnostics, 38698195987](https://github.com/adamaltmejd/pinfold/actions/runs/38698195987), source `cbb3a53` on released `a05deeb`, retained all samples:

| Sample | Exposed CPU | Cargo s | Libtest s |
|---|---|---:|---:|
| 1 | AMD EPYC 7763 | 296.29 | 294.15 |
| 2 | AMD EPYC 7763 | 301.16 | 298.92 |
| 3 | AMD EPYC 9V74 | 305.85 | 303.92 |
| 4 | Intel Xeon 6973P-C | 258.90 | 257.04 |

All passed 34 tests, zero failures, one separately gated ignored test; their durable-recovery gates passed. Exposed affinity was four CPUs, rootless native overlay, shifting=false. Writes were 10.2–11.1 GiB. Sampled native Podman lock occupancy, excluding deliberate G9 holds, was 20.5/23.5/58.0/35.5 seconds. Host CPU throughput and storage stalls both varied. Counter overlap prevents a unique walltime decomposition; process maps/comm do not prove ownership conversion. A separate full Mac instrumentation run passed 33/0/1 in 302.27 seconds; retain that budget miss.

The runtime test lock serialized G9, G15, G37 and G8. G9 retained exclusivity for 34.6–39.5 seconds after its global-store-lock scenarios. The bounded correction atomically downgrades to a read guard at that boundary, preserving every sequence/assertion/deadline and fixture-drop protection.

[Four fixed downgrade diagnostics, 38700679755](https://github.com/adamaltmejd/pinfold/actions/runs/38700679755), `d046a90`, passed all suites and durable recovery. Cargo times: **289.73, 295.87, 297.19, 258.70 seconds**. Independent trace review observed both waiting readers entering at downgrade: repeated builds overlapped 7.90–9.23 seconds and file sharing 1.58–2.02 seconds inside the unchanged tail. Those intervals overlap each other. This supports the scheduling change but does not prove the final 270-second margin or assign the complete before/after wall difference to it.

## Rejected profile-build candidates

Both experiments used predetermined isolated ABBA orders, real builds and first launches, unchanged checked assets, frozen per-architecture base/source, and exact own-resource cleanup. They do not replace whole-suite safety gates.

| Experiment | Host | A / B / B / A build s |
|---|---|---|
| [Raw Debian asset-stage base, 38703029261](https://github.com/adamaltmejd/pinfold/actions/runs/38703029261) | x64 | 33.436 / 26.222 / 25.967 / 25.344 |
| Same | arm64 | 35.140 / 33.023 / 32.683 / 32.818 |
| [Podman jobs=2, 38703686717](https://github.com/adamaltmejd/pinfold/actions/runs/38703686717) | x64 | 36.732 / 34.095 / 65.171 / 36.022 |
| Same | arm64 | 32.769 / 28.972 / 29.080 / 30.290 |

All eight builds per experiment had equal 77-entry selected-asset manifests within architecture and successful cleanup. This proves selected-asset equality, not complete image/safety equivalence.

Raw asset-stage B did not beat warmed final A. First A wrote about 1.3–1.6 GiB more; cold pull/unpack/store work materially confounds the initial A/B mean. No useful warmed reduction was demonstrated. Reject shipping this candidate.

Jobs=2's x64 B3 added 31.076 seconds against B2, with only 1.870 extra host CPU core-seconds, 0.262 device-busy seconds and 0.063 IO-PSI-some seconds. The counters support waiting but do not identify network, locks or builder coordination. Native progress was not retained on successful builds. Do not call it an exact 30-second timeout or discard it. Arm's modest gain does not establish a reliable cross-host improvement. Reject shipping this candidate.

## Native versus FUSE

[Eight fixed storage cells, 38706059982](https://github.com/adamaltmejd/pinfold/actions/runs/38706059982), `ed78cb7`, used two fresh samples per architecture/backend. Complete copied vendor configuration selected the driver/helper before initializing empty default per-user stores. No store reset/relocation, guest /dev/fuse, capability change or keep-id change was introduced. Both cells installed the same packaged helper.

| Host | Native Cargo s | FUSE Cargo s |
|---|---|---|
| x64 | 312.72, 245.49 | 251.78, 341.43 |
| arm64 | 257.52, 243.57 | 259.15, 282.35 |

All eight suites passed 34/0/1 and all eight durable-recovery gates passed. FUSE reduced physical writes from roughly 10–11 GiB to 3.4–3.8 GiB, but timing remained variable and one FUSE x64 command exceeded 300 seconds. Reject FUSE as the demonstrated #92 solution. Native overlay remains the production configuration.

The slow native x64 cell had less sampled native lock occupancy and IO pressure than its faster native counterpart. Therefore storage contention alone does not explain all earlier variation. Host Pinfold CPU and compilation also varied. Visible ancestor throttle deltas and host steal were zero; hidden host limits remain unknown.

## Executable placement and profiler failures

[Three profiling-only cells plus same-byte staging ABBA, 38710043706](https://github.com/adamaltmejd/pinfold/actions/runs/38710043706): Cargo **311.48, 314.99, 385.62 seconds**, all suites 34/0/1. Profiling was unrun because the assumed versioned perf executable path did not exist, although matching packages were installed. No function attribution is available from this attempt.

All 24 post-suite real-box ABBA observations passed readiness/exec/down/cleanup checks with identical binaries. Original executable parent had 402 files/78 directories; isolated parent one file. Copy cost was 27–33 ms. Six block B-minus-A readiness differences were −9/−15/−36/−15/−99/−76 ms; total-time differences were −11/−24/−75/−30/+271/+18 ms and CPU changes mixed. This warmed serial fixture does not show a useful placement benefit. Reject adding staging machinery as a budget fix; it does not prove recursive admission scanning absent under all workloads.

The 385.62-second cell had IO-PSI-some/full 108.904/96.132 seconds versus about 12/7 in the other two, 174.472 device-busy seconds versus 28, and 72.689 seconds sampled native lock occupancy versus 8–11. Writes remained near 11 GiB. Delay accumulated across cleanup and later overlapping project/image work; full/documents build durations themselves were faster. These observations establish storage stalls and prolonged lock waits, but cannot be added or assigned entirely to one storage operation. The cleanup tail lacks finer attribution.

[Second profiling-only attempt, 38711743014](https://github.com/adamaltmejd/pinfold/actions/runs/38711743014): Cargo **314.68, 305.59, 259.40 seconds**, all suites 34/0/1. Recording ran, but the sanitized report parser rejected the installed perf output format; no usable symbol attribution was claimed. A public-zero-byte tool-only check subsequently established that format. Parser success on that fixture is not Pinfold runtime evidence. A corrected predetermined baseline profiling run was prepared separately; no result from it is claimed in this snapshot.

**Gate correction:** workflows 38710043706 and 38711743014 were profiling-only workflows and did **not** run G35 durable recovery. The earlier claim otherwise was corrected in [issue comment 6189109685](https://github.com/adamaltmejd/pinfold/issues/92#issuecomment-6189109685). The main E2E suite's 34/0/1 outcome never substitutes for that separate workflow gate. The four initial diagnostics, four downgrade diagnostics and eight storage cells above did include their recorded durable-recovery checks. Asset/jobs/staging microbenchmarks and the tool-only format check are not G35 evidence.

## CPU experiment and selected candidate

The third profiler attempt, [38714055544](https://github.com/adamaltmejd/pinfold/actions/runs/38714055544), passed the complete suite in 255.85 seconds. Recorder shutdown succeeded, but its report was unsuitable: multiple selected commands add an output column, and filtering only pinfold/pinfold-init misses Tokio blocking workers. Source review confirms harness preparation uses spawn_blocking; installed Tokio names those threads tokio-rt-worker. This failed diagnostic is not hotspot evidence. Raw samples were removed in all attempts.

Instead, [eight SHA2 development-profile cells, 38715150319](https://github.com/adamaltmejd/pinfold/actions/runs/38715150319), source 47e6546, compared the sole Cargo opt-level override using same-host real-update ABBA after each complete diagnostic suite. The update fixture served the same actual binary payload on every call. Every call verified byte identity, externally computed digest, inode replacement and installed version. All 32 calls passed. Source, input binaries and the suite executable's hash remained unchanged. Cargo reported host and target SHA2 optimization 0 versus 3, with identical features, debug assertions and overflow checks. A used a fresh target; B reused the suite build, so their build times are not a causal comparison. Cold compilation belongs in final acceptance.

| Cell | Cargo seconds | Mean A update user CPU | Mean B update user CPU |
|---|---:|---:|---:|
| x64 native 1 | 237.98 | 1.215 | 0.045 |
| x64 native 2 | 228.60 | 1.195 | 0.050 |
| x64 FUSE 1 | 288.87 | 1.985 | 0.055 |
| x64 FUSE 2 | 319.43 | 1.990 | 0.095 |
| arm64 native 1 | 245.21 | 0.950 | 0.075 |
| arm64 native 2 | 243.53 | 0.930 | 0.075 |
| arm64 FUSE 1 | 236.36 | 0.935 | 0.085 |
| arm64 FUSE 2 | 255.80 | 0.950 | 0.070 |

All eight suites passed 34/0/1 and all eight separate durable-recovery steps succeeded. The 319.43-second FUSE run is a budget miss despite green diagnostic CI. FUSE remains excluded. Both native x64 cells exposed AMD EPYC 9V45; these two favorable outcomes alone do not establish reliability across runners.

The controlled update experiment establishes a roughly 91–97% reduction in user CPU for that workflow. The source factor is development compilation of SHA2, not a changed checksum algorithm or bypass. It does not assign all suite variation to SHA2: measured storage stalls and native lock waits remain separate contributors, and total before/after suite observations used different runners. No function-level profiler claim is made.

The minimal candidate retains native storage and changes only the G9 guard boundary, SHA2's development optimization level and ordinary CI budget enforcement. No production Rust source, dependency version, assertion, expected value, fixture, deadline, release setting or security control changes. It adds no benchmark machinery to shipping code. Independent reviewers checked the exact guard/drop order, compilation profiles, binary provenance and all retained observations.

The instrumented SHA2 candidate passed the complete Mac command in 248.94 seconds (33/0/1), plus format and clippy. The final uninstrumented host check and nine-cell acceptance are recorded separately. The acceptance schedule is fixed before dispatch: three successive mixed rounds, each with one fresh ordinary-cache x64, one fresh cold-musl x64 and one fresh ordinary-cache arm64 runner. Later rounds run even after an earlier failure. This interleaves cache conditions rather than completing all ordinary-cache jobs first. The counts, strict 300-second limits and 270-second normal-x64 margin remain unchanged.

Issue #92 remains open at this evidence snapshot. No diagnostic result is final acceptance.
