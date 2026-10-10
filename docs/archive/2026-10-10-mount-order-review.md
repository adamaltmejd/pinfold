# Y-12 runtime mount order coverage repair

Astra's P2 review found that the second admission scenario reversed the
mount list and left that order in place for accepted runtime startup.
That lost the existing guarantee 22 scenario where the read-only child is
listed before its writable parent. An adapter applying mounts in input
order could then pass the runtime check.

Both refused admission orders remain. After removing the duplicate export,
the existing fixture swaps its first two mounts back before runtime launch.
The hook-write assertion again detects input-order application. Its expected
refusal comes from the read-only mount contract; the runtime exit and host
fixture remain the observers. The existing project write is the control.
Guarantee 22 again names the original runtime order. No new test function
or production change was added.

The operator supplied these real Mac startup medians in seconds:

| Fixture | Baseline | Candidate |
|---|---:|---:|
| Small | 0.7353 | 0.6779 |
| Loose | 0.7260 | 0.7364 |
| Packed | 0.6951 | 0.6878 |
| 100,000-file tree | 0.7217 | 0.9791 |

The measurements used the same debug toolchain, cached image and Pi harness,
three runs each, with warm uncontrolled caches. The measured source was
`fe4ef2f0`; subsequent production changes are comments only. These are
operator-reported host measurements. The operator will record full
methodology and gate outcomes separately. Copied-function timings were
not rerun.

Worker checks passed with `CARGO_HOME=/usr/local/cargo`: locked offline
build, format check and locked offline all-target clippy with warnings
denied. Runtime suites remain the host landing gate.
