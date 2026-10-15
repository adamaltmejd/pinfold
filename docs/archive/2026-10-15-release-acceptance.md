# v0.2.5 release acceptance

2026-10-15. Production and test source is `1e3e25f489c003109a16cadd6d5b7a8c92fb45e4`.
This record adds documentation only. The release combines PR #91 and the
exact PR #93 head `df38f9d7a88c0ccc8d57daec1c8b4a4c6841e0d0`.
The earlier cleanup, trace and integration records describe the independent
whole-tree audit, implementation reviews and pin refresh.

## Suite budget investigation

Every run below passed all assertions. Each full Linux suite passed 34,
failed zero and ignored the separate slow gate; durable recovery passed too.

| Run and source | Configuration | x64 seconds | arm64 seconds |
|---|---|---:|---:|
| [38684327457](https://github.com/adamaltmejd/pinfold/actions/runs/38684327457), `a494641` | ordinary CI | 305.43 | 217.81 |
| [38686258341](https://github.com/adamaltmejd/pinfold/actions/runs/38686258341), `1e3e25f` | ordinary CI | 311.51 | 243.99 |
| [38687403822](https://github.com/adamaltmejd/pinfold/actions/runs/38687403822), `842d2e8` | four workers | 268.08 | 228.86 |
| same controlled matrix | eight workers | 278.68 | 234.70 |
| [38688755454](https://github.com/adamaltmejd/pinfold/actions/runs/38688755454), `dc23534` | four workers, phase output | 252.94 | not run |

`842d2e8` changes only the diagnostic workflow matrix, job name and explicit
test-worker argument relative to `1e3e25f`. Production code, tests, assertions,
deadlines and timed build work are identical. The predetermined four-worker
cells are acceptance evidence for that final source. Eight workers were
slower on both hosts and were rejected. No concurrency change is shipped.

This public repository uses GitHub's standard Ubuntu x64 and arm64 runners,
documented as four CPUs. Rust's default test concurrency follows available
parallelism; the workflow and setup script supply no override. Four is the
documented ordinary configuration, not a claimed speed improvement.
Sources: [GitHub runner specifications](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and [Rust test execution](https://doc.rust-lang.org/rustc/tests/index.html#--test-threads-num_threads).

`dc23534` adds temporary static-label phase timings and uncaptured output,
with no assertion, production-code or deadline change. It measured:

| Operation | Seconds |
|---|---:|
| Initial in-suite musl build | 13.48 |
| G14 custom image inherited from the suite default | 0.354 |
| G14 fresh full-profile build | 52.418 |
| G14 model prompts, custom/default/full/default | 7.706 / 3.890 / 5.763 / 2.644 |
| G32 fresh documents-profile build | 41.126 |
| G32 PDF conversion / rendering | 0.171 / 0.132 |
| Changed-profile production fixture compilation | 17.073 |
| G9 stopped-owner down / subsequent claim | 10.578 / 10.513 |

Fresh profile image builds dominate the measured final stages. They exercise
different bundled integrations and remain. The initial musl build took
20.44 seconds in the earlier 311.51-second run, illustrating variation even
outside profile builds. These observations do not isolate the cause of the
whole-suite difference or promise a maximum runtime on future hosted VMs.

The measured fixture reductions are recorded in the October 14 fixture-cost
report and October 15 integration follow-up. Compilation and image building
remain inside the timer. No assertion, production deadline or trust boundary
was weakened. Independent test and release reviewers accepted the controlled
four-worker results for release. #92 remains open for the observed budget
variation; these results are not a claim that the overrun is eliminated.

One proposed further optimization was rejected on independent review:
overlapping G9's stopped-owner callers would lose its explicit proof that
the claim remains held after down times out. The unchanged targeted Mac
test passed in 53.54 seconds; the overlap was never applied. Consolidating
19 repeated native image inventories was also not admitted without a
meaningful measured cost. Temporary diagnostic changes are not released.

## Other gates and remaining work

Final implementation `8d327a3` passed formatting, strict Clippy and the full
Mac suite: 33 passed, zero failed, one separate slow gate ignored, 233.04 s.
Source is identical to `1e3e25f` except documentation. Mac durable recovery
and real Ponytail RPC off-to-lite composition passed. G34 and both G36
results at `a494641` remain applicable to their unchanged mechanisms and
fixtures, as independently reviewed and qualified in the integration record.
They are not represented as executions of the final binary bytes.

The audit opened two tickets: #94 remains a pre-existing trust-refusal
reason proof gap; #95's reproduced prompt-composition bug is fixed. #89
remains open for stock Podman image-lock contention and #92 for suite-time
variation. No control-removal
decision or implementation finding remains unresolved for this release.

All three release builds must succeed before publication. Published asset
download, checksum and execution evidence belongs to the final release
verification bundle, not this pre-publication record.

Before this record: 2,032 added / 299 deleted lines since v0.2.4; delete/add
ratio 0.1471. Largest Rust file: E2E box, 5,501 lines. Largest production
file: cli.rs, 1,135 lines. Production Rust: 9,489 lines. ARCHITECTURE.md:
1,023 lines. Two audit/integration findings became written tickets.
