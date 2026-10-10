# Y-12 host validation and startup cost

2026-10-10. This records the operator's review and real Mac measurements for
an additive startup refusal of observed read-only/writable regular-file
aliases. It does not close Y-9/Y-10 or enable linked worktrees. The earlier
trusted-host topology question was withdrawn; no new caller responsibility
or exception to the threat model was approved.

## Startup measurements

Baseline: `9d50d05ca32285155e20120bc2b41d3fa293a1f7`.
Measured candidate: `30f9520ca5699ff17cc13f328410116dd8f86ba0`.
Both used the same Rust 1.98.1 dev build on macOS 27.0.1 (26A434), arm64,
Apple Container 1.5.0. These are real CLI launch-to-ready timings, not a
standalone copy of the scanner. The candidate's embedded Linux init was
built by the normal build script. No binary instrumentation was added.

Each spec used the cached default image, cached Pi 1.1.0 harness, two CPUs,
1 GiB memory, no egress, a writable fixture repository and its read-only
`.git` overlay. Image identity after the runs was
`sha256:4c897f619d3290a049bba7707df4cea9f31fc3d7304ad4d41fd7094cc927c224`.
The harness was mounted but no agent/model was launched. A normal source
file write after ready was observed on the host. Closing owner stdin ended
every run with exit 0. No benchmark-labeled boxes remained afterward.
No protected-file writes or runtime bypass probes were performed.

Three baseline runs per fixture preceded the candidate runs. An initial
candidate (fe4ef2f0) was measured first; after the path/cancellation repairs,
three runs per fixture measured the final candidate above. Creation
was outside timing. Caches were warm/uncontrolled; the trees were freshly
created, the image and harness already cached, and no cache flush or
randomized/interleaved trial was used. First launches also include
per-binary init preparation. The table reports medians, not a latency bound
or a precise isolated scanner cost. Small negative differences are noise.

| Fixture | Ordinary files | Git files | Baseline seconds | Candidate seconds | Difference |
|---|---:|---:|---:|---:|---:|
| Small | 20 | 46 | 0.7353 | 0.7413 | +0.0060 |
| Loose objects | 5,000 | 5,045 | 0.7260 | 0.6782 | -0.0478 |
| Packed objects | 5,000 | 30 | 0.6951 | 0.6549 | -0.0402 |
| Large working tree | 100,000 | 27 | 0.7217 | 0.9737 | +0.2520 |

The ordinary-file control adds one file after the first baseline launch.
The loose and packed fixtures each contain 5,022 real Git objects. The
packed fixture has one pack and no loose objects. The large working tree
has one committed file and 99,999 untracked files. It is not a million-file
repository or a cold disk benchmark. No Linux latency claim follows.

Raw readiness samples, baseline then candidate, in seconds:

- Small: `[1.357734, 0.711468, 0.735325]`;
  `[0.985177, 0.741310, 0.611791]`.
- Loose: `[0.758127, 0.681126, 0.726011]`;
  `[0.791319, 0.678207, 0.664955]`.
- Packed: `[0.697331, 0.695114, 0.686813]`;
  `[0.704529, 0.654937, 0.654523]`.
- Large: `[0.771167, 0.721721, 0.719944]`;
  `[2.006181, 0.973677, 0.943700]`.

The temporary driver is `/private/tmp/pinfold-admission-benchmark.py`.
The fixture manifest and result files are under
`/private/tmp/pinfold-admission-benchmark-s9uz_u6v` (manifest separately at
`/private/tmp/pinfold-admission-benchmark-fixtures.json`). These observations
do not justify a new cache or scan shortcut. The startup increase in the
large warm fixture is approximately a quarter second.

## Operator test audit

The changed assertion blocks extend guarantees 11, 17 and 22; no test function
was added. Refused inputs are host-created static aliases. Expected bytes,
file identities and external object links come from the host fixtures.
The allowed version of the same fixture supplies real startup, Git reads
and ordinary-file writes. Existing direct-write runtime assertions cannot
observe this new admission behavior. Each added scenario names omission of
the check, an extra export, or a blanket link-count decision as its sabotage.
No sabotage was executed.

The operator corrected a cleanup observation that selected every Pinfold
owner and a project-ID lookup before fixture state existed. Astra then
identified that the accepted runtime scenario had lost its original
read-only-child-before-writable-parent order. The candidate was returned
for repair to preserve that existing security regression coverage.

A later review found that lexical `..` components caused false alias
classification and that cancellation was not checked while draining queued
empty directories. Both were returned as guard/fail-safe correctness issues.
The operator also rejected a proposed general last-export-wins rule:
duplicate caller destinations must retain their existing early refusal.

A separate baseline CLI observation on the same Mac accepted a read-only
Git destination spelled `REPO/.git/../.git`, listed before writable REPO.
The box reached ready and read the fixture config exactly; its owner exited
0. It performed no protected writes. This confirms an existing accepted
spelling rather than relying solely on a static runtime assumption. The
small driver is `/private/tmp/pinfold-parent-path-compat.py`.


## Landed result

Yard attempt 7 landed Y-12 at
`30f9520ca5699ff17cc13f328410116dd8f86ba0` after operator approval of that
exact head and proof
`0decfadcea4202206b628ea6f155db41ac6f12d269691e2be6645f20b78c5ef1`.
Sol 6.1 medium implemented it. Astra medium's final review had no findings.
Formatting, all-target clippy with warnings denied, and the secrets gate
passed. No Opus agent was used.

The host landing gate passed on the candidate:

- macOS: 33 passed, 0 failed, 1 ignored, 220.86 seconds.
- Linux x86-64 and arm64: both jobs passed in
  [CI run 38053360908](https://github.com/adamaltmejd/pinfold/actions/runs/38053360908).

The new refusal cases are passive host-admission checks. These Linux
results do not repeat or establish the previously blocked runtime bypass
probe. Existing runtime assertions were retained. No release was issued
or installed by this work.

Disposition: retain the added row-11 and row-22 admission blocks, their
allowed controls, and row-17's normalized duplicate-destination scenario.
The row-17 block uses the existing early-refusal observer and successful
startup control. The mount-order regression found in review was repaired.
Final test sources contain 8,171 lines across nine Rust/Python files;
binary sources contain 9,203 lines across 29 Rust files, including build.rs.
The baseline counts were 8,043 and 9,091 respectively. No test function was
added and no sabotage was run.

The reproduced pre-existing alias layout is now refused before guest
startup. Y-9/Y-10 remain open for broader alias and concurrent-mutation
questions. Worktrees remain refused. The bounded mitigation is not a claim
of complete inode-wide enforcement for every layout throughout a run.
