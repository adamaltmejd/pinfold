# Issue 92 acceptance failure and follow-up diagnostics

Evidence snapshot before the second acceptance series. No acceptance or completion claim. The first series remains failed.

## First acceptance series

Production/test source: 30725e08e56e96b205b55e61bab3097a4bd1c93d. Driver: fc116806c5999cb5f484f0fea4631c7b6bc32086. Run: https://github.com/adamaltmejd/pinfold/actions/runs/38717694898.

| Round | x64 normal command seconds | x64 cold-musl command seconds | arm normal command seconds |
|---|---:|---:|---:|
| 1 | 242.131943771 | 293.905344892 | 238.609189826 |
| 2 | 286.495359223 | 302.248309888 | 241.912829808 |
| 3 | 274.833650170 | 285.717540062 | 249.176509863 |

All nine suites passed 34/0/1 and all nine G35 gates passed. Round 2 cold-musl exceeded 300 seconds; normal rounds 2 and 3 exceeded the fixed 270-second margin. Neither the successful assertions nor a favorable mean rescues the failed series.

The validation driver used reusable-job ID `cell`; production uses `linux`. The pinned rust-cache action includes this ID in both primary and restore keys. All nine cells explicitly missed cache; these were not observed warm-cache cases. A future driver must use `linux` and retain actual hit/miss evidence. This discrepancy does not invalidate the failed series or prove that it explains all timing variation. The correction changes no threshold, source, prebuild policy or automatic cache hashing.

## Native phase follow-up

Run: https://github.com/adamaltmejd/pinfold/actions/runs/38727311383. Source 36adf1495439f8e3f2714a546af51cdb1e846056 is the fixed shipping candidate plus reviewed observations. Four fresh native x64 full commands took 278.59, 278.26, 287.80 and 235.17 seconds. All suites and G35 gates passed. The three slower jobs restored fallback caches; the fastest restored the exact cache. Hardware also differed, so this comparison cannot isolate cache or CPU effects.

The slower jobs were already 18–24 seconds later when the full-profile test dispatched. Full builds took about 69–72 seconds versus 47; documents builds 54–57 versus 40. Two slow jobs showed high CPU pressure; the third showed substantial I/O pressure and longer native-store locking. No single lock or CPU brand explains every slow run.

Separate warmed, serial direct builds retained exact recipe hashes and successful cleanup. They are not full-suite measurements. APT-containing steps accounted for much of build time, but those steps include installation and upgrades; individual index fetches were about one second. Reusing package indexes was not admitted on this evidence. All attempted store `du` readings failed on mapped ownership and cannot establish store capacity.

## Rejected build mutex

Run: https://github.com/adamaltmejd/pinfold/actions/runs/38730086905. Frozen runner 20c12ed9dc9a000f536a0072e58f8ec546ad5c48 tested exact baseline A and a four-line Linux-only mutex B in order A/B/B/A on each of two hosts. All eight suites passed 34/0/1; all G35 gates and observers completed. Exact source hashes and baseline restoration were verified. Binary hashes were identical.

| Host | Initial A Cargo s | B1 Cargo s | B2 Cargo s | Final A Cargo s | B1 minus final A libtest s | B2 minus final A libtest s |
|---|---:|---:|---:|---:|---:|---:|
| Intel Xeon 8370C | 262.77 | 247.30 | 242.29 | 242.64 | +4.72 | +1.87 |
| AMD EPYC 7763 | 278.67 | 257.13 | 256.53 | 253.14 | +3.91 | +5.41 |

The declared comparison excludes initial A from gain claims because initial compilation and store state differ. Serializing the helper removed full/documents overlap and shortened individual builds, but the complete schedule was slower in every libtest comparison. Host CPU contention was not isolated. Intel B2's 0.35-second Cargo gain includes a 2.23-second harness-compilation advantage. Independent review confirmed rejection. No mutex is in the shipping candidate.

Initial A recorded 12.49–12.64-second first-build steps, versus about 0.05 seconds in final A. These are measured step differences, not an isolated explanation of the total elapsed gap. Warming and dispatch also differed; these runs had relatively low I/O pressure. Other retained slow hosts had substantial I/O pressure. The evidence therefore supports multiple measured costs rather than one universal storage-lock explanation.

## Fixed tmpfs diagnostic

Frozen diagnostic commit f3b23eb37b952215c2bf0ba449db65e8cce2f5b4; unchanged production/test source from 36adf1495439f8e3f2714a546af51cdb1e846056. Run: https://github.com/adamaltmejd/pinfold/actions/runs/38735522085. Only the diagnostic workflow and host setup/observer script differ from 36adf14. Four fresh x64 jobs compare two native disk controls with two native overlay stores on a capped 6 GiB tmpfs at the ordinary graphroot. All assertions, profiles, deadlines, keep-id and fixture isolation remain unchanged. Each job runs the full suite, G35 and actual G36; evidence lives outside target to survive cache restoration. Runtime fallback, capacity, incomplete evidence and cleanup failures disqualify a result. Inventory-unavailable is not clean even if an ordinary unmount succeeds.

The predetermined consideration condition is both tmpfs commands at most 270 seconds, worst tmpfs faster than best disk control, and all gates and cleanup successful. This is not acceptance and does not prove ordinary disk-backed performance fixed. Fresh-host hardware/cache differences remain confounds. All four suites passed 34/0/1, all four G35 and G36 gates completed, observers completed, and source/final-source plus actual binary hashes matched. Both tmpfs stores normally unmounted after verified zero running containers.

| Backing and sample | Cargo s | Libtest s | CPU model | Rust cache |
|---|---:|---:|---|---|
| disk 1 | 278.47 | 276.13 | AMD EPYC 7763 | restore-key hit |
| disk 2 | 279.38 | 277.04 | AMD EPYC 7763 | exact hit |
| tmpfs 1 | 243.89 | 241.80 | Intel Xeon 8370C | restore-key hit |
| tmpfs 2 | 254.21 | 251.92 | Intel Xeon 8573C | exact hit |

The fixed consideration condition passed. Sampled tmpfs allocation maxima were 1,965,285,376 and 1,983,934,464 bytes out of 6,442,450,944. Minimum observed host MemAvailable was 11,475,312,640 and 11,355,996,160 bytes. These are sampled observations, not complete memory-safety bounds.

During the suite window, controls recorded about 10.2–10.4 GiB host writes versus 1.15–1.51 GiB for tmpfs. I/O pressure and sampled Podman lock waiting were lower with tmpfs. These host-wide observations are consistent with less storage work, but cannot be added into elapsed savings or attributed to a specific operation. Both controls ran on AMD; both tmpfs hosts ran on Intel. Cache status matched within each numbered pair, but CPU and underlying disk differed. No 24–35-second causal speedup claim is made.

Independent review confirmed the actual gates, provenance, native storage/capacity and cleanup. It supports a minimal candidate for the separate frozen nine-run acceptance, not shipping or closure. AMD tmpfs performance remains unobserved by this trial. This snapshot precedes acceptance of the selected candidate.


## Candidate selected for the second acceptance series

Independent code review approved a 55-line addition to the existing Linux runner setup, against shipping candidate 30725e0. It selects tmpfs only for GITHUB_ACTIONS=true, RUNNER_ENVIRONMENT=github-hosted and x86_64. ARM, manual and self-hosted paths retain their existing setup. The complete vendor storage configuration, systemd session and runtime flags stay unchanged.

Before the first Podman invocation, it refuses aliased paths, rootless storage overrides, existing mounts, nonempty stores, and mismatched ownership or permissions. It mounts the ordinary graphroot with a fixed 6 GiB cap, then verifies the mount identity, ownership and capacity. After normal runtime initialization it requires the expected graphroot/runroot, rootless native overlay, nodev, non-transient storage, no FUSE mount program, native overlay diff and tmpfs backing/cap. A mismatch fails setup. The mounted store lasts for the disposable hosted VM, including the unchanged slow-proxy gate; no early unmount or live-host cleanup mechanism is added.

GitHub documents RUNNER_ENVIRONMENT and the fresh-VM lifecycle in its [variables reference](https://docs.github.com/en/actions/reference/workflows-and-actions/variables) and [hosted-runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). The implementation is limited to that environment. It changes no product guarantee or test assertion, so the architecture guarantees table remains unchanged. No new dependency is introduced.

The second validation driver corrects the cache job ID to linux and retains evidence outside target so a cache restoration cannot erase setup records. The timed command, default worker count, cold-musl condition and thresholds remain unchanged. No timing sampler or diagnostic spans are included. Host and final acceptance results will be recorded separately when complete.


## Candidate review and host checks

Independent reviews verified the unchanged assertions and product source, the tmpfs diagnostic evidence, the candidate setup diff, and the corrected acceptance driver. The driver review found and corrected a tool-provenance issue: Bun setup requires successful recipe-version extraction; G35 requires successful Bun setup. G35 still runs after a suite or budget failure when pinned tool setup succeeds. A failed tool setup retains its not-run result.

The host suite passed 33 tests, with zero failures and one ignored slow test. Libtest reported 550.13 seconds and the whole command 2311.76 seconds. The macOS power log records repeated deep sleep during this run, followed by full wake; these are retained sleep-affected timings, not a budget success or a speedup sample. The earlier unchanged diagnostic-source pre-push Mac suite passed 33/0/1 in 230.28 seconds libtest and 232.36 seconds whole command; it is separately retained and does not replace the later run.

Cargo fmt, clippy with warnings denied, shellcheck, shell syntax and workflow static checks passed. The local actionlint version does not recognize the already-used ubuntu-26.04 runner labels; only those label diagnostics were excluded. The actual Mac G35 gate passed with Bun 1.4.3. The final remaining slow/build/platform gates and the nine-run acceptance remain required.

Test-audit disposition: the selected storage addition changes no test or assertion; no block was deleted, replaced or weakened. Existing G9/G37 real-store locks and G36 namespace behavior remain intact. The shipping candidate has 8,572 Rust lines in the e2e crate beside 9,449 in the product crate. Temporary timing instrumentation, host observers, mutex experiment and validation workflows are not part of the shipping patch.
