# Pinfold architecture and robustness review

Pinfold should retain its small Rust isolation core and external caller
interface. The highest-value work is stronger ownership during concurrent
startup and cleanup, bounded failure handling, and clearer separation of
configuration inspection from filesystem changes. Pi Durable merits an
external integration experiment after those boundaries are understood.
It should not become an orchestration subsystem inside Pinfold.

## Scope and verification

Reviewed snapshot `da17860c86dde43f2117b04fd251687d423dd7f6`, version 0.1.8,
on 2026-10-07. The checkout advanced from `c9eb9e1` during the review,
then another operation edited guarantee 15's test. An immutable copy under
`/private/tmp/pinfold-review-20261007` fixed the review target. Those
concurrent changes were not edited or included in the conclusions.

The pass read all production Rust modules, the E2E harness and test files,
the profile, maintenance scripts, workflows, README and binding spec.
The code-cleanup and test-audit skills supplied the review criteria.
This is an assessment, not a release cleanup or an implementation pass.

| Measure | Snapshot |
| --- | ---: |
| Production Rust under `src/` | 7,930 lines, 25 files |
| Build script | 59 lines |
| E2E Rust | 5,835 lines |
| Largest production file | `cli.rs`, 1,128 lines |
| Architecture | 867 lines |
| Direct production dependencies | 9, matching the spec |
| Guarantee rows | 32 |

`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`
and a debug build passed on the fixed snapshot. The first Clippy attempt
hit the sandbox's Zig cache permission limit; a separate build with Zig
caches in `/private/tmp` passed. Safe CLI checks used isolated XDG roots,
nonsecret fixture values and an empty `PATH`, preventing runtime access.

No container suite or new performance benchmark ran. The shared checkout
and runtime were in concurrent use or could not be established as
exclusive. Source-supported races below remain hypotheses until exercised
through real CLI/runtime boundaries. The October 5 archive reports a
194-second Mac suite on its release commit; that is historical evidence,
not a timing for this snapshot. No implementation was changed and no
external tickets were created by this review.

## Highest priority findings

### Cleanup needs ownership held through removal

`core/clean.rs:143` inventories dead boxes, then `:198-201` removes them
by reusable name without holding ownership of that generation. Two
pruners can interleave with a restart: one inventories dead `foo`, another
removes it, a new `foo` starts, then the first removes the replacement.

There is a more consequential version in `cli.rs:459-475` and `:512-520`.
Clean selects idle project homes and caches from a liveness snapshot,
then removes those paths after other cleanup operations. A project can
become live between selection and deletion. Its sessions and login state
are then exposed to deletion despite the live-project preservation rule.

Both are high-priority source-supported races, not reproduced failures.
Guarantees 8 and 15 should cover the replacement and concurrent-launch
cases. A second liveness check alone leaves the same race. Startup,
retirement and deletion need a shared ownership rule that remains valid
until the destructive operation finishes, including across state roots.

Related lifecycle cases deserve the same analysis:

- `core/box.rs:407-446` creates the state directory before locking and
  populating its PID file. `core/clean.rs:93-101` treats a missing or empty
  PID as alive forever. A crash in that window can strand the name.
- `core/box.rs:309-335` reads PID and lock liveness separately, then
  signals the saved PID. Its ten-second fallback can remove state while
  the old owner remains alive. Old-owner cleanup and name reuse need an
  explicit generation boundary. PID reuse is a risk requiring a concrete
  reproduction, not an established exploit from this review.

Do this as one narrow ownership design followed by module-sized changes.
It should remove duplicated, inconsistent ownership decisions rather than
add a general resource-management framework.

### Configurable storage must remain outside writable projects

`dirs.rs:151-158` accepts absolute XDG locations.
`pi/launch.rs:185-199` mounts the whole project writable.
`trust.rs:78-81` stores authoritative trust records below `XDG_STATE_HOME`.
Consequently `XDG_STATE_HOME=$PROJECT/.state` puts trust records inside
the guest's writable project. A guest can potentially change both a
project config and the record that authorizes its hash. Project-local
`XDG_CONFIG_HOME` similarly exposes custom profiles.

The path exposure follows directly from the code; an actual guest trust
bypass was not run. It conflicts with the external-state and unwritable-
profile controls. Validate canonical storage/mount overlap in the pi
layer, including symlink aliases and cache payloads that later execute on
the host. Reject an unsafe layout before materializing project state.
Core callers still own the mounts they explicitly choose.

### Guest environment transport can reconfigure the host runtime

`core/runtime/mod.rs:340-351` puts guest env values into the runtime
client's own process environment. Preflight, inspection and teardown use
the host environment instead. A guest `CONTAINER_HOST`, for example, can
make only `podman run` select a different endpoint. Podman explicitly
documents this variable as enabling remote mode. Configuration and XDG
variables provide other collisions. This is a source-supported
correctness and boundary concern, not a demonstrated host escape.

Separate guest env transport from runtime-client configuration while
preserving names-only secret argv and the prohibition on written secret
files. Verify native pipe/input support on both runtimes before choosing
an implementation. Refusing conflicting names is a smaller alternative
but changes the accepted spec and needs an explicit contract decision.

Acceptance should launch a local box with a guest-only runtime control
variable, observe its value inside, and confirm the ordinary local
list/down path manages that same box.

Source: [Podman environment and remote mode documentation](https://docs.podman.io/en/latest/markdown/podman.1.html).

### Bound stalled proxy and helper operations

`core/proxy.rs:508` and `:614` can block indefinitely writing to a guest
that stopped reading. The tunnel's opposite direction shuts the upstream
socket at `:645`; that does not interrupt a write on the client socket.
Workers and connection slots can remain occupied. Bound writes and ensure
idle cancellation releases both directions, while retaining the shared
activity rule for a legitimately one-way active tunnel.

`init.rs:95-107` has a related half-close problem: proxy EOF closes only
the client's write half, then joins a thread still reading from that
client. Repeated clients retaining their upload side can accumulate relay
tasks. Preserve the legitimate upload-half-close case while fixing this.
Neither saturation scenario was executed during this review.

The known Y-2 issue remains: `core/login.rs:153` waits indefinitely for
helper output; kill/reap follows only after the exchange returns. Include
deadline, output bound, cancellation, helper reaping and lock release in
that ticket. Initial synchronous resolution can also delay pre-ready
signal handling.

Separately, `core/login.rs:95-102` places the supposedly host-wide login
lock under the caller's XDG state root. Different state roots sharing one
Codex login therefore do not share that lock. Use a stable per-user lock
identity independent of Pinfold's chosen state root. Keep credential
storage and refresh owned by Codex. Concurrent refresh failure is still
to be reproduced with the real helper and a controlled refresh endpoint.

### Image identity must describe the image actually started

`core/box.rs:189` resolves an image and `:203` copies its labels.
`core/runtime/mod.rs:377-381` later starts the original mutable reference,
while `core/box.rs:271` reports the earlier ID. A concurrent tag movement
can produce a ready record and copied labels for the wrong image.

Freeze the resolved identity through startup using a runtime-supported
mechanism, or reconcile identity and labels from the actual created box.
Do not assume Apple accepts Podman's image-ID spelling: the existing E2E
code explicitly records that limitation. Verify both runtimes with a tag
replacement scenario, then extend guarantees 9/23.

### Define what happens when audit logging fails

`core/proxy.rs:708` discards errors opening/writing the egress log. Traffic
can continue without the decisions promised by the spec. Define a visible
logging-failure outcome and whether it terminates the owner or stops new
egress. Continuing silently does not satisfy the current contract.
The source behavior is clear; host-induced log failure through a real
box remains the acceptance check. This is a policy clarification with
implementation, not a logging cleanup.

## Smaller reproduced findings

All checks below used the real built CLI without a runtime executable.

| Finding | Evidence | Narrow follow-up |
| --- | --- | --- |
| Nested env objects ignore unknown fields | `env.VALUE={"from":"REVIEW_VALUE","misspelled":true}` reaches a `runtime` refusal; the same unknown key at top level returns `spec`. `core/plan.rs:282-289`. | Reject unknown nested fields and name the key. Extend guarantee 17. |
| Invalid UTF-8 overrides silently disappear | `PINFOLD_CPUS=invalid` produces a config error; a present value containing byte `ff` reaches runtime lookup. `config.rs:159-179` uses `.ok()`. | Distinguish absence from invalid encoding. Refuse without printing the value. Extend guarantee 16. |
| Valid long project names fail derived paths | A host-valid 245-character basename makes `pinfold allow` return `File name too long`. `pi/state.rs:57-68`. | Bound the cosmetic prefix while retaining the path hash; define existing-state implications. |
| Doctor extracts cached profile files | With empty PATH and fresh XDG roots, doctor creates the embedded share's manifest/package/extension/skill. `cli.rs:580`, `core/profile.rs:91,145`. | Make metadata loading pure; materialize shared files only for operations that need them. Clarify guarantee 27 to cover the whole cache. |

Doctor's last finding contradicts its own “nothing is created” comment.
The present test specifically observes state and an old artifact; it does
not establish that every profile-cache path remains untouched.

Another source-supported caller inconsistency is `box list`:
`cli.rs:364` applies caller label filters without the existing
`clean::pinfold_box` predicate. A foreign container sharing that label can
appear as a box with a null owner, although exec/stat/down exclude it.
Reproduce with a real foreign container and extend guarantee 9's existing
foreign-container scenario before making the narrow predicate change.

## Leanness and module boundaries

The core is already reasonably small and its nine dependencies match the
spec. The runtime trait has two substantial implementations. Profiles
already provide configuration, toolchains, seeds and shared files without
requiring code in the security boundary. Keep those choices.

The useful consolidations remove actual ownership or policy duplication:

1. Complete Y-6: one curl download primitive for artifacts and updates.
   Artifact downloads currently lack the updater's curl-config disabling,
   HTTPS redirect restrictions and size bound. Preserve different limits
   for a release check and a large harness download.
2. Complete Y-3: let the protected-directory owner clean up on scope exit,
   including partial preparation and plan-construction failure. Remove
   the explicit caller cleanup path, preserving existing/nonempty dirs.
3. Separate profile metadata from extraction. This makes config/doctor
   observation simpler and removes hidden writes from a read path.
4. When profile creation next changes, move its filesystem operation from
   `cli.rs:994` onward into the existing profile module. Keep CLI parsing
   and serialization at the edge. `CleanPlan` currently belongs in CLI by
   an explicit spec exception; moving it blindly into core loses the
   core/pi boundary.

Most production modules had no worthwhile standalone deletion after the
recent cleanup. Splitting files solely by line count, changing every
proxy operation to async, or adding plugin/provider abstractions is not
supported by this review.

## Speed and storage

The best measured lead remains [issue 78](https://github.com/adamaltmejd/pinfold/issues/78).
`core/image.rs:260` still refreshes external Apple bases without specifying
a platform. The issue records 20.5 GiB of unrelated-architecture Debian
and Rust snapshots on October 4. That attribution still needs a clean
Pinfold build reproduction; it is not a promised saving on this machine.
Restrict future pulls to the actual build platform while respecting
explicit Containerfile platform choices.

Two other concrete mechanisms deserve measurement:

- The profile ADDs both architectures' archives, then removes them in a
  later RUN. Earlier layers still retain those archives. Measure final
  layer bytes and total builder storage separately. An artifact stage
  that copies only installed outputs into the final image can eliminate
  archive-bearing final-image layers; platform selection can avoid
  unused downloads. Preserve checksums and current tools.
- `core/runtime/mod.rs:424-429` keeps the entire build log in memory, even
  on success, although failures expose only 40 trailing lines. Drain into
  a bounded tail instead. Define a byte limit as well as a line limit so
  one giant line cannot defeat the bound. This is a resource robustness
  improvement; measure peak RSS before claiming a performance benefit.

Before startup optimization, measure cold/warm artifact installation,
image inspection, profile preparation, runtime readiness, first exec and
first routed request separately. Compare interactive launches with
many-short-tool-call orchestration. The existing suite time cannot locate
these bottlenecks. Keep explicit builds and the current cache contract
until measurements justify changing it.

## Configuration

Pinfold already exposes the important choices: profiles/images, resources,
mounts, exact env, allowlists, routes and login adapters. More general
runtime selection or switches that disable controls would weaken the
product's contract.

Make existing configuration easier to inspect before adding knobs.
`cli.rs:704` currently makes all `pinfold config` JSON depend on runtime
image inspection. Returning host-derived policy/trust with an explicitly
unavailable image status would help callers diagnose a missing runtime.
This changes the caller contract and requires a spec update. Distinguish
an unavailable observation from a negative answer.

For unattended deployments, resource policy deserves a separate design:
log growth, artifact downloads, process limits and disk exhaustion. The
current absence of a disk cap is an explicit open question. A retention
age alone is not a storage bound. Add validated configuration only for
policies that have a real operational requirement and enforceable runtime
semantics.

## Tests and confidence

Keep the E2E approach, external observers and positive controls. Thirty-two
named guarantee tests do not imply coverage of every clause in the spec.
The most valuable changes extend existing scenarios:

- Guarantee 3 needs an accepted raw Content-Length request with a real
  body through the same transport as malformed cases. Rejecting all
  Content-Length bodies could otherwise pass the current block.
- Guarantee 24 compares whole-store dangling-image counts. Concurrent
  cleanup can hide a new leak. Attribute observations to the scenario or
  give that observation exclusive store ownership.
- Guarantee 15's shared-runtime fixture race is already Y-8, and was
  being edited separately during this review. Different XDG roots do not
  isolate a host's container/image store. Preserve assertions and
  coordinate at that actual boundary.
- Guarantee 13's separation block observes host seed files. Add guest
  reads/writes that prove each box received its own project home.
- The ordinary shared-image helper accepts a stale local default image.
  Build the candidate's embedded profile once for checks that depend on
  current image contents. Guarantee 32 already builds a fresh image and
  is a useful example; this concern does not invalidate that test.
- Host supervision needs bounded readiness/completion waits and cleanup
  of failed scenarios. Preserve signal-based readiness and avoid retries.
  Remove inherited `PINFOLD_*` configuration before applying scenario
  settings, preserving required runtime and live-login inputs.
- Guarantee 28's release fixture serves the original executable, then
  compares installed bytes with that original. Atomically reinstalling
  the wrong source could pass. Serve distinct valid executable bytes.
  Also start a real working owner while a fixture holds the download,
  proving the final installation lock/recheck rather than only the
  initial busy guard. These strengthen existing Y-4.
- Guarantee 29 permits a four-second observation for a one-second request
  contract. Observe request lifetime at the host fixture with a stated
  scheduling tolerance so a three-second timeout cannot pass silently.

The trace found all seven config schema keys and public parsed CLI flags
documented, and all 32 guarantees mapped to named tests. Y-7 still has
caller-path gaps: CPU/memory/protect environment overrides, unknown TOML
keys, profile-level Containerfile refusal, direct exec TTY/workdir flags,
caller-image no-cache and successful attach box selection. Extend scenarios
where those clauses matter; a count of tests cannot resolve these gaps.

The five-minute idle timeout cannot be fully exercised inside a suite
whose entire budget is below five minutes. Resolve that validation policy
explicitly, for example with a separate slow gate. Do not add a test-only
timeout path to the binary.

## Pi Durable integration

Upstream was checked at `7fb59f995b0a1db552001a8577b234e4105d7179`.
Its API remains experimental. It persists conversations and tasks and
offers an `ExecutionEnv` boundary; interrupted tools rerun only when
declared replay-safe. Storage has one process owner and no cross-process
locking. These are useful orchestration capabilities, but not box
recovery or exactly-once shell execution.

Keep interactive `pinfold pi`. Prototype a separate trusted host adapter
that owns `box up`, consumes ready/down events and routes all
model-directed filesystem/process operations into boxes. Keep durable
checkpoints on host-owned storage outside the writable project. Do not
expose an unrestricted host `NodeExecutionEnv` to the model or open the
same SQLite store from both host and Apple guest.

The current interface has two substantial costs:

1. `ExecutionEnv` covers more than four coding tools: binary readers,
   directory iteration, metadata, file operations, watches, execution,
   output delivery and cleanup. A conforming adapter is a maintained
   component, not a trivial subprocess wrapper. Environment identity must
   represent the actual shared file namespace, including conversations
   intentionally sharing mounts.
2. Upstream `Shell.exec` promises that cancellation kills only that
   command's processes. Pinfold explicitly makes the box the unit of
   lifetime and never signals a process inside it. Stopping the host
   `box exec` client is not proof that guest descendants stopped. Either
   provide a narrowly specified per-command supervisor in the guest and
   test it, or deliberately expose coarser box cancellation in a smaller
   tool adapter. Do not advertise full ExecutionEnv conformance with
   different semantics.

Initial acceptance should be a failure experiment, not a coding demo:

1. Run a mutating command and record an external side-effect marker.
2. Kill the durable owner before the result is committed. Determine
   whether the previous guest command can still execute before recovery.
3. Reclaim/remove the previous box generation, recreate isolation, and
   resume the conversation without repeating unsafe work automatically.
4. Cancel a command with descendants. Prove they stop and an independent
   command survives if per-command cancellation is promised.
5. Fork/restart conversations and verify intended filesystem sharing,
   stable identities, checkpoint ownership and absence of host tool access.
6. Run upstream environment conformance checks plus Pinfold's actual
   runtime checks on both platforms before publishing a full adapter.

Success would justify an optional, separately pinned integration package.
It would not justify embedding SQLite, a task scheduler, or agent-specific
application state in Pinfold's Rust core. Running an entire Durable app
inside a box is a simpler separate experiment, but guest-writable
checkpoints cannot be trusted as host approval or recovery authority.

Sources: [Pi Durable README](https://github.com/earendil-works/pi/blob/7fb59f995b0a1db552001a8577b234e4105d7179/packages/durable/README.md),
[ExecutionEnv contract](https://github.com/earendil-works/pi/blob/7fb59f995b0a1db552001a8577b234e4105d7179/packages/durable/src/env/index.ts).

## Recommended sequence

1. Reproduce and address ownership/removal and writable-storage boundary
   risks. Include cross-state-root operation in the design.
2. Fix reproduced input bugs, finish Y-2/Y-3, and bound proxy/helper/build
   resource retention with focused E2E scenarios.
3. Complete Y-6 and separate pure configuration loading from extraction.
4. Reproduce issue 78, then measure profile layers and startup stages.
5. Prototype the external Durable adapter against the interruption and
   cancellation criteria before adding it to the supported interface.

Existing Y-2 through Y-8 are recorded in the October 5 archive; refresh
their acceptance criteria instead of filing duplicates. New source-level
bug candidates need CLI reproductions before admission under AGENTS.md.
