# Builder phase costs and fixture reuse

Date: 2026-10-29. Production source is unchanged from `3c9f9d6` through
the scheduling candidate `62102396505d8929df96af9d5583e29917f5f557`.
The owned disposable VM built the ordinary debug and release profiles:

| Executable | Bytes | SHA-256 |
|---|---:|---|
| debug musl | 75,866,776 | `a9e7604cb7d7122d080c6e8f76e9e1281e009dc4bcccd726ccad6fb50c78ce6e` |
| release musl | 7,037,360 | `ea636094c4a03182737a5d9c066a583fd8cf498fefcd275d4381d3275b07f73e` |

No production limit, cache policy or verification control changed for
these observations. Every successful sample records matching executable
hashes before and after its command.

## Unchanged-suite result

Ubuntu run `40881672764` at `6210239` passed **26 tests, 0 failed, 1 ignored**,
in 726.11 s. The wrapper recorded `e2e_exit=0` and 729,450,698,512 ns,
then correctly failed the unchanged 300-second budget. The fixture guard
change resolved both earlier `builder-in-use` failures. This is functional
suite evidence, not a passing CI/budget gate.

## Traced phase attribution

An external observer traced the real CLI, with separate initially empty XDG
roots for each build profile. It observed successful Firecracker exec and
process exit. The sequential lifetimes are assigned to discovery, build
and conversion by the normal production call order. They include boot,
teardown and tracing overhead, not guest BuildKit CPU or per-phase memory.

| Profile/case | Whole command | Before first VMM | Discovery VMM | Build VMM | Conversion VMM |
|---|---:|---:|---:|---:|---:|
| debug, cold default | 96.522 s | 28.188 s | 7.780 s | 33.760 s | 21.255 s |
| debug, changed tiny image 1 | 29.415 s | 9.624 s | 6.099 s | 6.506 s | 6.460 s |
| debug, changed tiny image 2 | 28.848 s | 9.500 s | 6.044 s | 6.028 s | 6.614 s |
| debug, identical image 2 | 22.283 s | 9.589 s | 6.187 s | 6.123 s | skipped |
| release, cold default | 93.977 s | 24.838 s | 7.617 s | 34.784 s | 21.088 s |
| release, changed tiny image 1 | 25.349 s | 6.093 s | 6.054 s | 6.163 s | 6.389 s |
| release, changed tiny image 2 | 25.335 s | 6.036 s | 5.928 s | 6.198 s | 6.585 s |
| release, identical image 2 | 18.705 s | 6.150 s | 6.136 s | 6.052 s | skipped |

The tiny recipes use scratch, one small COPY and distinct ENV values.
The third caller build repeats the second exactly. Both profiles produce
distinct IDs for the changed recipes and the same ID for the repeat;
the existing completed-root cache skips the converter only for that repeat.

Raw traces also identify an init-disk mkfs process lasting 3.707 s in the
second changed debug sample, compared with roughly 0.04 s for its small
input disks. These are traced process times. They do not establish the
untraced saving from a disk-cache change. Recorded host fsync sums were
small and can overlap other work; they are not added to the critical path.

## Untraced controls

Normal builds of four distinct tiny images used the same warmed XDG cache
in debug/release/release/debug order. All four built successfully, with
distinct image IDs and unchanged binaries:

| Profile | Command wall time | Child process-tree CPU time |
|---|---:|---:|
| debug | 15.546 s | 16.023 s |
| release | 14.537 s | 14.916 s |
| release | 14.484 s | 14.990 s |
| debug | 16.080 s | 16.608 s |

This small comparison shows about 1.3 s mean benefit for this workload,
not the much larger apparent benefit of traced init-disk work. It does
not justify changing the CI build profile as a solution to the budget gap.
No such change was landed. Traced VMM lifetimes must not be treated as
untraced phase timings or extrapolated into a promised suite saving.

A separate static C program, compiled with the already-required musl
toolchain and invoked through an ordinary scratch-image RUN, timed two
real `sync()` calls with CLOCK_MONOTONIC. It emitted timestamps
1,485,774,696 / 1,487,056,813 / 1,487,168,373 ns, then intentionally exited 1
so its output was returned. The calls took **1.282117 ms** and
**0.111560 ms**. The whole expected failed build took 13.463 s. This probe
does not explain the timing gap or justify deleting either production
flush barrier. Neither barrier was changed.

## Bounded fixture candidates

A normal warmed build of a profile derived solely with
`FROM localhost/pinfold/profile-default:latest` took **31.584 s**. Its
binary was the debug hash above. G26 tests profile seeds, skills, routes
and login authority, not image construction. It can select the existing
default image in both profile-config writes and remove that unrelated
derived build and cleanup. Its only remaining default-image preparation
uses the existing mutex, so it can use the shared runtime guard.

G17 needs a real stale managed tag before deleting the recipe. On Linux,
scratch provides that input without carrying the default toolchain into
the tag. The later successful explicit-default-image control is unchanged;
Apple keeps its old base. The exact profile refusal reason distinguishes
the intended missing-recipe guard from a later image startup failure.

Candidate `7c4a37ce2653139b5553f8b887a7a420a7500f3d` makes only these fixture
changes. No assertions, expected values, production code, helper or
guarantee changes. Rows 17/26 retain their named sabotage and outside
fixture/host/runtime expectations. The per-call saving above is measured;
the final suite saving and budget admission require their own CI result.

The affected G17/G26 tests passed together at `7c4a37c`: **2 passed** in
60.57 s on macOS and **2 passed** in 173.21 s on Linux, including their
normal default-image setup. The executed test file SHA-256 is
`84a9c82116c216a774e0e54ec54e377c55617289c0be6bb470b0a4cfd1c7f1cc`.
Linux invocation hashes before/after match the debug hash above. The native
Mac executable matches before/after at
`8458dd1438a1f3b92d34ddefe195528a19e37e50e5720d26f9dc0d0bf8b5ea57`.
The first Mac attempt lacked sandbox access to Apple's container service
and stopped before the scenarios. Read access was then verified and the
passing run used the required service permissions. Both attempts are kept.

The final wording-only correction calls the stale tag selected for launch,
rather than claiming an empty scratch image must complete a usable launch.
It changes no assertion or fixture input from the executed test file.

## Evidence

Receipts, raw traces, observer sources, normal CLI stdio and recipe inputs
are retained under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
The bundles exclude guest-writable filesystems and disks.

| File | SHA-256 |
|---|---|
| `build-phase-profile-6210239.tar.gz` | `5dd889c821b93dd19c353233809d718f49d35956ea9075b631b1934cc009e9ca` |
| `build-phase-release-6210239.tar.gz` | `ae5838b400e19c080f226c67fdbf9039ad867a405c0b75a870fb17457303284f` |
| `untraced-build-ab-6210239.tar.gz` | `f344503709f22e0f7e6cda537df9b241e712b9c037dd3059fcbee87e0f6cc69c` |
| `guest-sync-profile-6210239.tar.gz` | `288890916835b77376aa07539cd1cec9d6a19772059600b4663ac769aa2c0702` |
| `login-fixture-build-profile-6210239.tar.gz` | `cdcce58f7d2cfb5ef1a1cd00dedefe3bb5d958b60dd09ce16dfc307ea5cb825e` |
| `linux-pr252-6210239.log` | `c73e60c0d995500d6df0c0d8f0d5858625a380a14363e458c89656bfc89adced` |
| `mac-fixture-reuse-7c4a37c-2.json` | `2f344b84c56d56e272dcf9600b9e10fd11f2a7f847f0a8e29e68bbfb9cf4384c` |
| `mac-fixture-reuse-7c4a37c-2.log` | `108ca0eb40f8922c9629cdb2bc1412e1d6c88857c783da44e9c6a70100e52fdd` |
| `linux-fixture-reuse-7c4a37c-1.log` | `32430f7ba700aaa53b23c72925d1752d547a25a54f9b5a12495f5c4fc3e40b48` |
| `phase-profile-executables-6210239.tar.gz` | `01cb82ffcc304eb57f788b9b30f7feb720f96b8122d03656ab4147fd0f0c1aef` |

The last bundle preserves both exact measured executables locally. Final
process inspection found no remaining pinfold, Firecracker or curl fixture
process. All observations and platform test logs were copied out before
releasing the owned VM.

Resource-policy approval remains deferred. No measurement here approves
general image capacity, concurrent memory headroom, a larger timeout or
suite budget, runtime merge or CLI release.
