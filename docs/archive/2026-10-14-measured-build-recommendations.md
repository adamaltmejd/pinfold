# Pinfold: measured build recommendations

2026-10-14. This follows the [whole-tool and peer review](2026-10-14-performance-and-safety-review.md).
The working ledger (not retained) contained
all twelve investigations, separate patches, raw observations and independent
reviews. Three agents investigated distinct parts; the main session serialized
builds and real runtime measurements.

## Recommendation

Land the two reproduced safety fixes first. Then take the read-only identity
index, build-time init digest and redundant Apple inventory removal. They remove
measured costs without changing the isolation model. Introduce the smaller
embedded init as a separately reviewed packaging change. Offer guest-local
build outputs through the existing box environment, where the caller accepts
their memory cost and disposable lifetime.

Keep the proxy implementation for now. Do not weaken mount admission, network
policy, resource limits, image identity checks, ownership or teardown to gain
speed. Do not add a persistent daemon, warm box pool or new isolation backend
based on these measurements.

## What the builds demonstrated

| Candidate | Observation | Decision |
|---|---|---|
| O1: index only read-only file identities | Owner peak physical footprint: 14.78 to 2.28 MB on 100,000 writable files; 16.05 to 3.41 MB with 10,000 additional read-only files | Recommend for memory. Startup timing was mixed. |
| O2: compute embedded-init digest at build time | Mac release binary: 12,033,264 to 8,334,624 bytes. Exact payload occurs twice in the baseline binary and once in O2. | Recommend. The size benefit is stronger than the small exec timing difference. |
| O3: reuse Apple's readiness observation | Twelve alternating pairs: no-egress median 659.65 to 640.21 ms; proxy-enabled 703.25 to 691.11 ms | Recommend the redundant-query removal; expect a modest gain. |
| O5: put disposable Cargo outputs in guest `/tmp` | Same real offline Pinfold build: shared outputs 16.98/17.38 s; tmpfs 10.37/10.98 s. All four binary hashes identical. | Recommend as an opt-in workflow, about 38% faster in this fixture. |
| O6: dedicated embedded init from shared source | Payload 3,677,352 to 498,128 bytes. Mac CLI with O2: 8,334,624 to 5,131,296 bytes. Empty Cargo-target cross-build 18.48 to 3.06 s. | Recommend a separate packaging change, after full cross-platform runtime gates. |

O1 memory is the Pinfold owner process, not its guest VM or all runtime clients.
The 79–85% reductions do not imply equivalent whole-machine savings. Eight
alternating pairs were measured for each large-tree fixture. The mixed fixture
was slightly slower in the second run; there is no general startup-speed claim.

O2 had 100 alternating `box exec /bin/true` pairs: medians 78.63 and 76.28 ms.
O6's build-cost comparison is one empty Cargo target per build, baseline first,
with an existing registry and Zig cache. It measures the nested Linux payload
build, not a clean end-to-end CI pipeline. Cold extraction and missing-payload
repair restored the exact externally hashed bytes. Real exec preserved the
fixture environment, a transport-prefixed variable and exit status 37.

O5 used the same writable source mount, Rust 1.99.0, 2 CPUs and 4 GiB. Only
output storage changed. The second pair reversed order. No-op warm builds were
0.20–0.22 s versus 0.11–0.17 s. The tmpfs outputs occupied about 428 MiB; its
prepared dependency cache occupied another 118 MiB. This fixture's `/tmp` had
2 GiB capacity. These builds were debug builds, not a claim about every workload.
The initial cached-image preparation failed because that image lacked a Cargo
cache; the failure and cleanup are retained. The successful experiment copied
only public crate archives and index into the no-egress guest.

For a suitable caller, use the existing box `env` entry
`"CARGO_TARGET_DIR": "/tmp/pinfold-target"`. Copy any deliverable that must
survive teardown into an explicitly writable export. Keep capacity and compiler
working memory within that caller's existing memory budget. No new Pinfold flag,
profile default or automatic cache migration is needed.

## Safety fixes and smaller consolidations

| ID | Result | Remaining gate |
|---|---|---|
| B1 | Effective external Git hook aliases into the project now refuse, including absent leaves and dangling aliases. Real Mac G11 passed with host-observed hook bytes, a harmless host hook and a genuine-outside control. | Linux G11. The existing refusal of linked worktrees remains. |
| B2 | Failed readiness output awaits teardown. A closed reader returns exit 1, leaves no native box or owned state, and the same name then starts normally. Real Mac G9 passed. | Linux G9. |
| O4 | Removed two inventories after acknowledged teardown. Existing label authorization and checkpoint ownership remain. Typecheck and real Mac G35 passed. | Prototype gate on both Linux hosts. |
| C2 | The empty resolver file belongs to the claimed state directory. The shared cache path and its setup disappear. | Linux no-egress, resolver and teardown observations; cross-compilation alone is insufficient. |
| C3 | The update fixture reuses the existing bounded child owner. It removes 24 lines without changing assertions or the installed-executable/alternate-XDG race. Real Mac G28 passed. | Linux update-race gate. |

O6 exposed one integration failure: the update fixture copied the host CLI's
source but omitted the new init crate. A four-line source-copy correction fixed
the fixture without weakening assertions. G28 and G31 then passed. This correction
is recorded separately from C3 so the packaging dependency is explicit.

## Ideas not selected

**O7, proxy buffering: defer.** A real route fixture received and verified 600
serial requests and their exact bodies; all 600 audit decisions matched. With
256-byte heads, medians were 0.71–0.78 ms. With 1 KiB heads, 1.10–1.17 ms. A
7.5 KiB stress case was 2.69–2.81 ms. Split means two guest writes, not guaranteed
packet segmentation. These are complete local request timings, not isolated
parser timings or a comparison with a buffered candidate.

The mixed workload consumed about 0.48 seconds of owner CPU. The original
observer mislabeled Mach ticks as nanoseconds; the raw record is preserved and
the conversion was corrected using the same host's 125/3 timebase. No requests
were rerun for that correction. See the retained observer report and correction
script. A proxy rewrite is not justified by the demonstrated normal latency.
Reopen on a representative workload that identifies proxy CPU or header latency
as material. Any candidate must retain buffered bytes across HTTP body and TLS
transitions and pass the existing refusal and slow-deadline gates.

**C1, single tag-removal loop: reject.** The candidate removes two lines but
replaces direct branches with boolean-to-length arithmetic. That is insufficient
maintenance benefit. Its patch is retained for review and excluded from the
recommended bundle.

Peer comparisons remain architectural. No head-to-head performance benchmark
against Docker Sandboxes, sandbox-runtime, bubblewrap, nsjail or gVisor was run.
Their differing isolation contracts do not justify relaxing Pinfold's controls.

## Actual final builds and validation

The combined review candidate includes B1/B2, O1/O2/O3/O4/O6, C2/C3, the O6
fixture correction and matching guarantee evidence. C1 is excluded. O5 is a
caller configuration choice; O7 has no production patch.

| Target | Release bytes | Result |
|---|---:|---|
| macOS arm64 | 5,130,336 | Built; selected real Mac gates below passed |
| Linux arm64 musl | 3,684,520 | Cross-built; native runtime gate not run |
| Linux x64 musl | 4,169,016 | Cross-built; native runtime gate not run |

The binaries are retained in `target/performance-review-2026-10-14/` in the
primary workspace. Exact commands, hashes and compiler output are in
`evidence/final-builds.json`. The x64 Zig link succeeded with a warning about an
ignored deprecated linker optimization setting; no source warning was reported
by the required Clippy check.

Passed: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
twelve selected Mac guarantee tests (G3, G4, G5, G6, G7, G9, G11, G17, G20,
G22, G28, G31), and the separate G35 durable recovery/typecheck. Tests used the
real binary, native boxes and existing external observers. No mock runtime or
test-only production flag was introduced.

Before landing: reconcile these patches onto the intended current baseline;
run the full Mac suite, both Linux suites, the applicable slow proxy gates and
the prototype gate on both Linux hosts. In particular, C2 requires Linux
runtime evidence and O6 requires complete init/relay lifecycle coverage. The
current evidence supports recommendations and reviewable builds, not a release.

## Tracking and handoff

The reference is HEAD `198600b1751d889d354ed3657d8f839e8436e469` plus the ten
pre-existing dirty files saved in `evidence/baseline-working-tree.patch`.
The primary tracked diff remained byte-identical. This matters: the baseline
includes another task's bounded-listing and seccomp work. It is not silently
claimed as part of this optimization.

The retained managed checkout is
`~/.codex/worktrees/pinfold-measured-candidates/pinfold`.
Separate patches and `patches/recommended-candidates.patch` are relative to that
saved baseline, not plain HEAD. Three agent worktrees were submitted for
recoverable archival after their evidence and patches were captured.

Nine bounded handoff tickets (ledger not retained)
name the mechanism, evidence, affected guarantee and exclusions. They remain
local while the source baseline and cross-host gates are reconciled. The Yard
board and its unrelated parked Y-10 were left unchanged; no workers were
automatically dispatched. No candidate was installed, committed, merged or
released.
