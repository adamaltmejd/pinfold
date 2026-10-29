# Full-cache recovery test audit

Date: 2026-10-29. Source-only candidate audit. No test, build, VM,
sabotage, source push or publication was run for this audit.

## Guarantee and change

Extend existing guarantee 44,
`linux_build_steps_stay_inside_the_builder_role`, and strengthen guarantee
23's existing failed-RUN scenario. No test function,
Rust helper, dependency, runtime stub or binary test flag is added.
`fill_cache.c` is a normal workload copied through the caller's context
and executed by RUN, not an alternate builder or a forged result.

The real CLI reproduction filled a normal BuildKit cache mount, received
ENOSPC, then failed the following plain COPY build while retaining the
cache as clean. The existing SIGINT case cannot cover that path: an
interruption already leaves the cache unclean. The ordinary failed RUN
case in guarantee 23 leaves usable space, so it cannot cover a completed
failure that exhausts persistent storage.

Keep SIGINT followed by its existing successful role build. Then add the
full-cache failure and a second ordinary role build. That ordering keeps
the two controls independent: an intervening cancellation could discard
the exhausted cache and hide the reuse bug. The final existing launch
uses the second build and still checks its actual role observations with
the builder unavailable.

## Assertion blocks and outside expectations

| Test lines | Block | Outside source |
|---|---|---|
| `box_.rs:1999-2030` | An ordinary cache-mount RUN fails with exit 1, event failed, actual fixture ENOSPC output, and unchanged public references. | The finite workload writes up to 8 GiB into the candidate's 8 GiB cache, which also contains metadata and base data. It emits `cache-space-exhausted` only when an actual filesystem operation returns kernel errno ENOSPC. The before/after reference comparison observes host-written public state. |
| `box_.rs:2031-2035` | The following ordinary role recipe builds successfully without cache-mount cleanup, host cache deletion or another interruption. | The guarantee requires recovery for this known-valid caller recipe. Its kernel/HTTP/file expectations come from the approved role, endpoint policy and fixture. The final existing box launch reads the second completed image. |

The reason assertion does not inspect BuildKit's native ENOSPC wording or
introduce a product reason token. The expected marker exists in the
compiled workload, not its Dockerfile command, so a failure's quoted
recipe cannot satisfy the assertion. `fill_cache.c:7-52` handles partial
writes and EINTR, and reports other errors or completing all bounded
writes with a different exit and no marker. The test compiles it statically
with the already-required musl toolchain; no guest compiler is added.
The two successful-build checks serve different preceding failures and
are not interchangeable. Existing SIGINT, VMM death, role, allow/deny,
audit, cache cleanup and bootstrap-absent launch checks remain.

## Binary branch and sabotage

`init/builder.rs:103-122` selectively prunes BuildKit `exec.cachemount`
records after failed non-converter work. It does not infer exhaustion
from a post-teardown zero-space threshold: the real candidate failed to
classify ENOSPC with that predicate. Its raw post-stop counters were not
retained, so no exact remaining-space quantity or timing is inferred.
The complete shutdown/end/EOF, VMM reap and host backing-sync barriers
remain required before any cache can be marked clean.

The test comment at `box_.rs:1846-1849` names the sabotage: skip pruning
`exec.cachemount` records after failed work. That retains the full
filesystem, and the second ordinary build fails. The
sabotage was reviewed, not executed.

## Healthy cache preservation

Guarantee 23's existing failed recipe now mounts an empty cache at
`/caller-cache` on Linux (`box_.rs:1677-1689`). It still emits the giant
invalid-UTF-8 line, final caller marker and failure. On Mac the mount prefix
is empty, so the recipe is unchanged. No build, assertion, helper or
dependency is added to this scenario.

The existing before/after-image checks and unchanged-input identity
assertions remain at `box_.rs:1693-1737`. The expected marker comes from
the caller's context; the output bound comes from the spec. Image-ID
agreement is the guarantee's explicit two-outputs-agree contract for
identical fixture inputs, not an expected value copied from pinfold's
cache logic. Its RUN obtains fresh random bytes from the guest kernel if
the cached layer is lost.

The named sabotage at `box_.rs:1677-1679` is blanket pruning after failure:
that deletes the regular cached RUN layer, changing the repeated image's
ID. The old identity assertions already guarded that outcome. The fixture
change ensures a real matching `exec.cachemount` record exists while
healthy regular-cache preservation is checked, rather than exercising an
empty filtered prune. It adds no second assertion block for the same
contract and leaves no-cache and profile-rebuild checks unchanged.

The fixture exercises block exhaustion in a persistent cache mount.
It does not separately establish inode exhaustion, every form of regular
layer-cache exhaustion, memory peaks or concurrent capacity.
The 8 GiB allocation remains candidate policy; this test does not approve
it. The added negative and positive require two extra real builds and up
to 8 GiB of writes, so the full CI five-minute budget remains a material
gate. The operator must record the actual candidate execution and final
line references before push. No pass is inferred by this source audit.
