# Issue 92: final suite-budget acceptance

The second complete acceptance series passed the unchanged acceptance criteria declared before the first series. This is a finite observed-headroom criterion, not a guarantee about every future hosted runner. The first series remains failed. No result was discarded or replaced.

## Frozen source and results

Shipping implementation: `4066e9122c0015b42b04b3b9c1cfe9bccc5bfb79`. Validation source: `c1ee6859acad4126aef72dae30292aaebdcd3e90`. Their diff adds exactly three temporary workflow files; product, test and ordinary CI source are identical. Those temporary workflows are excluded from the shipping patch.

[Acceptance run 38745035085](https://github.com/adamaltmejd/pinfold/actions/runs/38745035085) ran three sequential mixed rounds with default workers and four exposed CPUs. Every complete `cargo test -p e2e --locked` command had to take less than 300 seconds; each normal x64 command also had to take at most 270 seconds. Compilation, private-fixture compilation and image builds remained inside that command.

| Round | Condition | Whole Cargo s | Libtest s | CPU model reported | Actual Rust cache |
|---|---|---:|---:|---|---|
| 1 | x64 normal | 209.504710036 | 207.87 | AMD EPYC 9V45 | exact match |
| 1 | x64 cold-musl | 235.535468985 | 233.59 | Intel Xeon 8573C | exact match, then musl removed |
| 1 | ARM normal | 220.951890997 | 218.81 | not reported | exact match |
| 2 | x64 normal | 258.459172304 | 256.21 | AMD EPYC 7763 | exact match |
| 2 | x64 cold-musl | 265.791278712 | 263.56 | AMD EPYC 7763 | fallback restore, then musl removed |
| 2 | ARM normal | 251.234954395 | 246.72 | not reported | miss |
| 3 | x64 normal | 233.856803757 | 231.90 | Intel Xeon 8573C | exact match |
| 3 | x64 cold-musl | 270.604179381 | 268.31 | AMD EPYC 7763 | exact match, then musl removed |
| 3 | ARM normal | 232.545874221 | 230.17 | not reported | exact match |

All nine suites passed 34 tests, with zero failures and one separately gated ignored test. All nine separate G35 recovery gates explicitly passed with Bun 1.4.3. All source/final-source, cleanliness and runtime checks passed. The worst complete command was 270.604 seconds, leaving 29.396 seconds to the strict budget. The worst normal x64 command was 258.459 seconds, meeting the stronger 270-second requirement.

Full workflow logs distinguish seven exact cache matches, one fallback restore and one actual miss. The false cache-hit output alone was not treated as proof of a miss. The cold cells removed only `target/x86_64-unknown-linux-musl` after ordinary cache restoration and lint; the successful fixed workflow step establishes removal. The artifacts record pre-removal state. No prebuild, worker override, sampler or diagnostic span was added to acceptance.

Every x64 runtime used rootless native overlay at the ordinary graphroot, backed by the exact 6 GiB tmpfs cap. ARM retained its original disk backing. Native overlay diff, nodev, keep-id, runroot, non-transient storage and absence of a FUSE helper remained. No capacity or runtime-preflight failure occurred. Acceptance did not sample memory peaks; the separate diagnostic's sampled measurements are not complete memory bounds.

## Causes and selected changes

The retained investigations establish several contributors to timing variation. Host CPU throughput, storage stalls and native-store lock waiting varied. Cache and initial compilation affected early work; later full-profile build dispatch and overlapping builds affected the tail. The counters overlap, so they cannot provide an additive wall-time decomposition or identify every storage operation. CPU brand alone did not explain all slow runs.

Three measured changes were retained:

- G9 atomically downgrades its exclusive test-runtime guard after its real global-store-lock scenarios finish. Observations showed waiting readers doing useful work during the previously exclusive tail. The read guard remains alive through fixture cleanup. All assertions, deadlines and stopped-owner ordering remain.
- SHA2 alone uses optimization level 3 in development builds. Same-host real-update ABBA trials measured about 91–97% less update user CPU, while externally checking unchanged bytes, digest, inode replacement and version. This is a measured update cost, not an attribution of all suite variation. Debug assertions, overflow checks, dependency features and release settings are unchanged.
- Only disposable GitHub-hosted x64 CI runners mount a capped native store before Podman initializes it. The storage diagnostic observed fewer writes and less I/O pressure, but different host CPUs prevent an isolated 24–35-second speedup claim. The new acceptance provides operational evidence across AMD and Intel hosts. ARM, manual and self-hosted setup is unchanged.

The permanent budget wrapper measures the complete Cargo command and rejects child failure or elapsed time of 300 seconds or more. It allows the suite to finish and retain its safety results. No production Rust source, security control, assertion, expected value, profile recipe or fixture isolation changed. Full and documents profiles still receive independent fresh builds. Rejected worker, build-mutex, FUSE, asset-stage, builder-jobs and executable-staging changes are absent.

## Other gates and independent review

[Linux slow-proxy run 38745039509](https://github.com/adamaltmejd/pinfold/actions/runs/38745039509) passed actual G36 on both architectures at the validation source. Both emitted the passed event after checking the unchanged five-minute idle deadline through the private namespace boundary.

[Release-build run 38745043731](https://github.com/adamaltmejd/pinfold/actions/runs/38745043731) built and uploaded all three targets at that source: Linux x64 musl, Linux ARM musl and macOS ARM. This validation did not publish a release.

The same implementation passed the full Mac suite (33/0/1), actual G35, and the separate G34 slow gate (1/0; 319.77 seconds whole command). The Mac full-suite log retains 550.13 seconds libtest and 2311.76 seconds whole command. The power log records repeated deep sleep during that run. These are correctness results, not budget or speedup evidence. Mac source provenance comes from the retained invocation and clean-checkout record; local test logs do not embed a commit hash.

Format, clippy with warnings denied, shellcheck, shell syntax and workflow checks passed. Independent reviewers approved the complete shipping diff, assertion/guard preservation, storage setup, diagnostic conclusions, fixed acceptance driver, evidence analyzer, all nine actual outcomes and remaining gates. The evidence review verified actual cache restoration, Bun versions and the successful cold-removal steps. No blocking findings remain at this acceptance snapshot.

Earlier diagnostics, rejected changes and the failed first series remain in the preceding dated investigation and acceptance-and-storage records. Artifacts and full logs are retained under the issue's local evidence directory as well as the linked Actions runs. Ordinary PR and merged-main CI still apply to landing this patch; a later budget failure must be investigated rather than excluded from the evidence.
