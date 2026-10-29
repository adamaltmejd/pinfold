# Builder lifetime and maintenance observations

Date: 2026-10-29 local time. Raw timestamps are UTC on 28 October.
The production source is `c4bcd4d`; later commits add evidence only.
The actual executable was rehashed after both existing test runs:
`fdefe37d46cade9d2e4b85c55b2f48ba63b0ad0e94cbd84fcd68090779e602b7`.
The raw `linux-final-binary.sha256` was collected at 23:44:26 UTC after
these runs, before the base trial. Its file SHA-256 is
`5a5b2533a0ba0b3ba7bf2c67850c3dc2089e30af187ac3dece20082dea26cb21`.
The two test logs contain no per-invocation executable hash or source
snapshot. The post-run check corroborates the retained executable but is
not independent attestation of each earlier invocation's bytes.

| Existing test | Actual result |
|---|---|
| G15 `runtime::cleanup_removes_only_pinfolds_garbage` | Passed, 313.82 s. Exercises retention while an image is running; cleanup of stale state; live project and disk preservation; exact-name removal, shared tags and in-use images; Linux root preservation/removal. |
| G11 `runtime::disks_persist_and_attach_exclusively` | Passed, 88.31 s. Exercises actual file persistence, competing writers, read-only sharing, removal refusal, owner/VMM death and interrupted seeding recovery. |

These are targeted Debian trial runs, not the full Ubuntu suite. G15's
local duration alone exceeds five minutes. No CI budget pass or waiver is
inferred. The first G15 shell command could not find Cargo and started no
test; the subsequent command used the installed toolchain. The actual
test ran once. No assertion or production source changed for these runs.

The existing tests do not hold a selected local base during a build.
A separate finite normal-CLI trial built one uniquely named base from the
existing full profile, then built a child from its immutable reference.
The child's actual allowed API audit entry established active execution
after local input selection. The recipe then blocks on a FIFO; the audit
entry alone does not establish that the FIFO instruction had been reached.

While the child was active, exact-name `image rm` returned removed with
`ids=[]` and the actual base identity in `in_use`. It retired a free tag
but preserved the base's final reference and immutable data. The identical
second command again reported the image in use and changed no references.
SIGINT then returned the child as failed/exit 1 with `cancelled`; the VMM
pidfd became readable. Public references stayed unchanged by cancellation.
The identical removal command then returned the base ID in `ids`, no
`in_use`, and removed its remaining reference and catalog identity.
Other repositories' references stayed unchanged. Cleanup of the two exact
owned names succeeded. The complete manual driver took 67.720 s.

This proves last-reference protection during selected-base use and release
after cancellation. It does not prove successful child completion, the
initial lease-acquisition/removal race, concurrent clean/publication or
host PID-1 reaping. The fresh trial VM also cannot establish preservation
of a pre-existing real Podman store. No permanent old store or disk was
touched. These boundaries remain explicit rather than being inferred from
passing maintenance tests.

Evidence is copied out under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
The base bundle includes exact driver/recipe, raw output, production audit,
catalog snapshots, executed binary identity and kernel death observations.
Exact production source/binary remain in the successful lifecycle bundle.

| File | SHA-256 |
|---|---|
| `linux-row15-current-2.log` | `bbd34017f4702071688764e0dcea517f234fad85ab05d01e55c61bca6742e186` |
| `linux-row11-current-1.log` | `e2ab88290a9407ff64b26e24925869cc5ce554a5f488263b94a5274bc92f6837` |
| `base-lease-success.tar.gz` | `0f32c178aa0aa6fe119a8ba3b7250979fc657515ddcca2f3fcbc1d18477eb644` |
| Base-lifetime driver | `d126f3ad2b6d4c8df0430114945f9e959306c11fcf649f918c00d3b29e76c610` |

Full Linux and Mac gates, distribution and numeric policy remain open.
