# Performance and safety review, 2026-10-14

Pinfold is already small enough that a runtime replacement or a new daemon needs
stronger evidence than this review found. Keep its current architecture. Fix the
two reproduced lifecycle/protection defects first, strengthen three test
observers, then remove repeated work in the durable prototype and cleanup code.
The likely immediate savings are modest. No implementation speedup was measured.

## Scope and evidence

Reviewed the working tree at `198600b1751d889d354ed3657d8f839e8436e469`, branch
`codex/issue-89-bounded-listing`, with existing uncommitted changes. This is a
review of that tree, not certification of the separately published v0.2.4.
Another task changed the seccomp implementation/spec/test during this review.
Those changes were preserved. All product, test and living-document edits remain
outside this review; its deliverables are this archive, two local tickets and
the evidence directory.

The main session and three bounded reviewers read the production Rust modules,
the Rust E2E suite and helpers, the durable prototype and gates, profile assets,
build/release/runner scripts, README and the binding architecture. The
`code-cleanup` and `test-audit` skills guided the pass. Configuration keys, public
verbs/flags and all 37 current guarantees were traced to the spec and their
tests/gates in [the trace tables](2026-10-14-performance-and-safety-review/traces.md).
Traceability does not establish that every scenario passes.

Fresh measurements and reproductions used a real release build on macOS 27.0.1
(26A434), arm64, Apple Container 1.5.0, rustc 1.99.0. The binary reports 0.2.3;
SHA-256 is `ec5bbacd11b5944ac847544725ea242239cc402dcc9b58feadbeb8d6507a7c9b`.
The build preceded later test edits. No peer sandbox was installed or benchmarked.
Linux observations below come from current CI logs and separately identified
existing experiments. No Linux runtime test or full Mac suite ran in this review.

## Confirmed defects

### P1: an external hooks-path alias bypasses project protection

[`pi/git.rs:111–121`](../../crates/pinfold/src/pi/git.rs) treats an absolute
`core.hooksPath` outside the lexical project root as unreachable from the box.
It returns before resolving the host filesystem target. An external symlink can
instead point into the writable project.

In a disposable real Git repository, the configured external symlink resolved
to `project/hooks`. Real `pinfold pi --mode rpc` reached readiness, and
`pinfold box exec` overwrote `hooks/pre-commit` with exit 0. A subsequent host
`git commit --allow-empty` executed that hook and wrote a harmless marker outside
the project. With the equivalent direct in-project path, the guest write failed
with a read-only-filesystem error and the host marker was absent. Ordinary
project writes succeeded in both cases. Both owners exited 0 and both boxes
were removed.

This violates guarantee 11 and the explicit requirement to protect hooks as host
Git resolves them. It requires a pre-existing host alias and a subsequent host
Git operation; the probe does not establish immediate arbitrary host execution.
The exclusion for project scripts called by hooks does not exclude the directly
configured hook entrypoint.

Resolve the effective host location before classifying it as outside. Preserve
the refusal of symlinked protected paths and linked worktrees. Extend the existing
G11 test with the reproduced alias and its allowed controls. Do not weaken the
contract to match the implementation. See the
local bug ticket, landed as cfd2eb4, and
[observed results](2026-10-14-performance-and-safety-review/hooks.json).

### P2: a failed readiness write leaves a running box

[`cli.rs:249–277`](../../crates/pinfold/src/cli.rs) owns a live box before
printing `ready`. Closing the stdout reader before submitting a valid box spec
caused `println!` to panic with Broken pipe and owner exit 101. Native runtime
inventory still showed that exact box running. Explicit `box down NAME` exited
0 and removed it. The result was reproduced twice; the saved driver uses a
regular stderr file to avoid inherited-pipe reads obscuring the observation.

This is a cleanup/protocol defect. Owner death and recoverable orphans are
already part of the threat model; no network bypass was demonstrated. The
probe used a box with no egress. Use fallible readiness output and explicitly
await teardown on errors after `Box::up` succeeds, including other fallible
handoff work. A closed output pipe cannot carry terminal JSON. A generic async
destructor redesign is unnecessary. Extend the existing lifecycle test. See the
local bug ticket, landed as 7feaf89, and
[observed results](2026-10-14-performance-and-safety-review/closed-stdout.json).

### Existing Linux work remains separate

The shared seccomp-profile selection race has an existing
[reproduction](2026-10-14-y153-runtime-policy-binding.md). The initial dirty tree
already moved the selected policy into each claimed box's state directory.
Concurrent work added guarantee 37 and its test. This review does not claim that
fix as new work or certify its runtime result. Preserve its per-box policy
ownership and external syscall observations.

Issue [#89](https://github.com/adamaltmejd/pinfold/issues/89) also remains open.
Bounded, truthful inventory failure does not resolve Podman storage conversion
contention. The existing [stock comparison](2026-10-13-stock-podman-contention-comparison.md)
and [native-ID-map probe](2026-10-14-native-idmap-capability.md) do not support a
simple runtime upgrade, storage flag or weaker UID mapping as a proven fix.

## Fresh performance baseline

Each variant started five real boxes using the cached default image, 2 CPUs and
1 GiB memory. Timing begins when the spec is submitted and ends at `ready`.
The private state root had a fresh maintenance stamp, excluding daily cleanup.
There were no image builds, model requests or external network requests. The
100,000-file fixture was created before timing. Variants ran sequentially with
warm/uncontrolled caches; this is not an isolated estimate of scanner cost.

| Operation | Samples | Median | Observed range |
|---|---:|---:|---:|
| Empty box, no egress, readiness | 5 | 0.642 s | 0.578–0.759 s |
| Empty box with proxy and empty allowlist, readiness | 5 | 1.040 s | 0.658–2.616 s |
| Writable 100,000-file tree with read-only `.git`, readiness | 5 | 1.055 s | 0.860–1.159 s |
| Empty box, normal teardown | 5 | 0.178 s | 0.128–0.181 s |
| `pinfold box exec … /bin/true`, empty box | 10 | 84.3 ms | 71.7–94.0 ms |
| Native exec through the same init wrapper | 10 | 62.2 ms | 51.1–69.7 ms |
| `pinfold box list`, empty-box fixture | 10 | 20.9 ms | 19.3–26.5 ms |
| Native runtime list | 10 | 15.2 ms | 14.1–16.2 ms |

Native/Pinfold command order alternated on the same running box. Native exec
still used Pinfold's init and exec wrapper; it bypassed host-side admission and
ownership work and is not a safe replacement interface. All 15 benchmark boxes
exited 0 after normal teardown. Raw samples, image identity and the exact driver
are in [benchmark.json](2026-10-14-performance-and-safety-review/benchmark.json)
and [benchmark.py](2026-10-14-performance-and-safety-review/benchmark.py).

The approximately 22 ms difference between exec medians and 6 ms between list
medians does not justify a resident controller or cached security checks. Use
the existing one-box-per-job lifetime and repeated exec where the caller can.
The startup results do not predict full Pi launch latency, cold image pulls,
model latency, CPU/RSS, sustained filesystem I/O or Linux startup. No p95 or
cross-tool speed ranking follows from these samples.

The Mac executable is 12,033,264 bytes (11.48 MiB); its embedded complete Linux
init executable is 3,677,352 bytes (3.51 MiB). Splitting out init could reduce
some embedded code, but that payload size is only an upper bound and no memory
or startup gain was established. It adds build/artifact work. Keep it behind a
measured need. The 6.3 GiB `target` directory is a development build footprint,
not installed Pinfold size.

## Small changes worth attempting

Each change should preserve assertions and use a separate bounded patch. The
table gives source-established work removal; it does not promise speedups.

| Order | Change | What disappears | Required boundary |
|---|---|---|---|
| 1 | `examples/pi-durable/prototype.ts:113,168`: remove inventories after successful awaited `box down` and owner close | Two subprocesses and two full runtime inventories per completed lifecycle; two lines | Retain the pre-down job-label authorization, exclusive checkpoint writer, generation checks and owner-close wait. Successful down already acknowledges removal. Run existing G35 on all required hosts. |
| 2 | `core/clean.rs:454–468`: choose the removable tag slice, then use one removal loop | Duplicate last-tag/general tag-removal path | Retain all-reference counts, last in-use tag, error propagation and distinct `in_use`/`ids` output. Preserve fresh ownership checks. |
| 3 | `core/runtime/podman.rs:61–64`: place empty resolver file in the already-owned box state directory | Separate shared-cache resolver directory/path | Preserve the read-only empty resolver and state ownership/teardown semantics. |
| 4 | `e2e/update.rs:367–410`: reuse `ChildOwner` and bounded readiness read immediately after spawn | Duplicate raw-child lifetime handling, estimated 15–25 lines | Preserve the fixture-installed executable and the real owner-starts-during-download race. A suite-binary owner would invalidate that proof. |
| 5 | Remove stale comment at `update.rs:272`; repair two spec cross-references | One misleading comment and documentation drift | Keep the executable-lock/older-owner guard. Include G37 in the Rust-coverage overview and remove the stale numbered root-exec reference. |

Start by measuring the prototype change with the same workload and alternating
baseline/candidate runs. A list costs about 21 ms in the empty local sample;
that is context, not a measured two-call improvement. None of the nine direct
dependencies was shown unnecessary. No general parser, walker, ownership engine
or profile-inheritance abstraction earned its added maintenance cost.

## Test audit and CI cost

No assertion block was justified for outright deletion. The positive controls,
generation checks, real-image races and external host observations distinguish
these tests from blanket-denial or timeout successes. Four G12 refusal-reason
blocks already have a [written ticket](2026-10-13-trust-refusal-test-ticket.md).
Do not duplicate it.

Three additional observer weaknesses should be repaired within existing tests:

| Observer | Weakness | Small correction |
|---|---|---|
| `box_.rs:5384–5393`, G9/17/22/26 cleanup helper | Directory-read errors become an empty directory; entry errors are dropped | Accept `NotFound` only. Fail other read/entry errors before asserting emptiness. Keep native runtime absence separately. |
| `box_.rs:3026–3034`, G23 image identity | Missing reference and missing latest can compare equal as `None == None` | Require both images to exist in native runtime inventory, then compare identities. Preserve nullable built-ID semantics. |
| `pi.rs:171–180`, G6 secret-in-argv check | A failed `ps` with empty stdout can satisfy absence | Require observer success before inspecting stdout. Preserve the independent fixture secret and allowed guest read. |

G5's `unshare` refusal also needs an attribution check: an EPERM from host policy
can conceal a missing Pinfold seccomp restriction. A successful `true` does not
establish that the same host would otherwise permit a user namespace. No
current-host masking was demonstrated. Keep the assertion and establish its
outside allowed control. Valid HTTPS injection alone cannot detect disabled
certificate validation; bare-LF/folded-header refusals are also not exercised by
the current G3 scenarios. These are coverage questions, not reproduced proxy
defects. Any added scenario must fit the existing guarantee and endpoint rules.

The state-discovery helper in `e2e/src/lib.rs:440–461` reads private
`projects/*/state.json` layout. Public `config` fields can locate project fixtures
without that coupling; they must not become the expected-value oracle for an
independent guarantee. This would improve maintainability, with no speed claim.

[Main CI run 38537343561](https://github.com/adamaltmejd/pinfold/actions/runs/38537343561),
commit `a1be9213efc3585d75bead7f2c2de4073167fc7d`, reports 33 passed, 0 failed,
1 ignored on each Linux architecture. The x64 suite took **313.43 s**, exceeding
the documented five-minute budget; arm64 took **258.25 s**. These include about
21.65/17.80 s of nested binary build time. They are suite times, not total job
times, and they do not validate the later dirty G37 work. Obtain per-fixture
timing before cutting test work. The production-deadline slow gates must retain
their real deadlines and isolated namespace prerequisites.

## Comparison with similar tools

These are architecture comparisons from current primary documentation accessed
2026-10-14. No claim about relative throughput or startup time is experimentally
established here. Isolation policies differ, so a faster command alone would not
establish a suitable replacement.

| Tool | Relevant design | Implication for Pinfold |
|---|---|---|
| [Anthropic Sandbox Runtime](https://github.com/anthropic-experimental/sandbox-runtime) | Seatbelt on macOS; bubblewrap on Linux; filesystem policy and proxied network access around host processes | Avoids Pinfold's Mac guest boot and image layer, but changes the kernel and filesystem boundary. Its current credential-proxy features also mean Pinfold cannot claim sole ownership of host-side credential injection. Consider only after matching the required safety properties. |
| [Codex sandbox](https://learn.chatgpt.com/docs/sandboxing) | Seatbelt on macOS and bubblewrap on Linux/WSL; spawned commands inherit restrictions; approvals are a separate control | A useful reference for protecting host-process execution. Pinfold's image, guest environment and box lifecycle are additional requirements; deleting them changes the product. |
| [Docker Sandboxes](https://docs.docker.com/ai/sandboxes/architecture/) | A microVM and Docker daemon per sandbox, persistent packages/images, workspace mounting and host credential injection | Closest broad workflow comparison on Mac. It carries more runtime machinery, but that is an architectural inference, not a measured footprint result. [Clone mode](https://docs.docker.com/ai/sandboxes/usage/) isolates workspace changes behind explicit Git transfer; adopting it would change Pinfold's same-files contract and add checkout/sync work. |
| [bubblewrap](https://github.com/containers/bubblewrap) / [nsjail](https://github.com/google/nsjail) | Linux namespace/process isolation building blocks; nsjail also exposes cgroup, rlimit and seccomp controls | Could replace some runtime overhead only by moving image, lifecycle, resource and policy responsibilities into Pinfold or its callers. Not a drop-in simplification. |
| [gVisor](https://gvisor.dev/docs/architecture_guide/intro/) | A userspace kernel mediates the application's kernel interface | Relevant if Linux host-kernel exposure becomes the priority. Its [performance guide](https://gvisor.dev/docs/architecture_guide/performance/) describes workload-dependent syscall/I/O costs; no basis here for calling it a speed upgrade. |

Pinfold's useful combination remains a small caller-owned box API, real guest
environment, host-enforced egress/credential policy and protected project
sharing. Preserve those concrete guarantees when comparing alternatives. A
future head-to-head study must match image cache state, writable/protected file
trees, credentials, egress policy, resource limits and teardown semantics; then
measure startup, repeated exec, representative builds, peak RSS and cleanup.

## Investigations and cuts deferred

- The proxy reads some protocol headers byte by byte. Measure a representative
  request/CONNECT workload before changing it. Buffered read-ahead must preserve
  body/TLS bytes, framing refusal, cancellation and timeout semantics.
- The durable prototype's nonexistent-path resolution may mishandle a dangling
  PATH symlink into a future guest-writable directory (`prototype.ts:35–42`).
  Reproduce through the real prototype before admitting a product bug.
- The slow CONNECT fixture calls host-wide `box prune` during cleanup
  (`scripts/connect_tunnels_share_activity.py:408`). Its private XDG/network
  namespace does not scope runtime inventory. Prefer generation-qualified
  fixture teardown while preserving same-name refusal safety. No unrelated
  deletion was induced in this review.
- The durable gate sends kills without awaiting all child closes and discards
  some cleanup statuses (`durable_recovery_preserves_boxed_execution.ts:209–220`).
  Preserve failed-fixture evidence until owned-runtime absence is confirmed.
- `doctor` appears to reach image listing via `config_report` even after a
  runtime failure, contrary to its skip wording. This remains a source question
  requiring a real CLI/process observation.
- Retain mount-alias admission, worktree refusal, ownership/generation locks,
  fresh runtime rechecks, exact environment, read-only rootfs, no-new-privileges,
  UID mapping, seccomp, TLS/SNI/address checks and update verification. Caching,
  bypassing or removing them would change safety rather than remove duplication.
- Retain managed no-cache builds (G24), the separate document/full profiles and
  required harness payloads. A feature's bytes are not evidence that it is unused.

## Verification and accounting

- `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`
  passed, including a repeat after the concurrent source/test changes.
- `cargo build -p pinfold --release --locked` passed with writable temporary Zig
  caches. Its 40.08 s elapsed time is not a clean-build benchmark.
- Real Mac box benchmarks, both bug reproductions and their controls completed.
  Exact fixture cleanup was checked; no global prune or service restart was used.
- 22 safe help commands exited 0. `pi --help` was excluded because it launches Pi.
- No full E2E, sabotage run, Linux runtime gate or release certification is claimed.

At the final source snapshot: 9,338 production Rust lines; 8,452 E2E Rust lines;
largest production file `cli.rs`, 1,125 lines; largest test file `box_.rs`, 5,454
lines; architecture 1,020 lines. The source manifest records hashes for drift
checks. No production lines were added or deleted by this review, so an audit
delete/add ratio is not meaningful. Two findings became new local bug tickets;
zero GitHub/Yard tickets were created. Existing tickets and dirty work were
preserved. This is not the mandatory pre-release landing/audit pass.

The evidence directory preserves the exact three observation scripts, raw
results, a calculated timing summary, source hashes and trace tables. Scripts
use this machine's binary path and cached artifacts and write results under
`/private/tmp`; review those paths and arrange exclusive runtime use before
replaying. They are archived experiments, not added E2E tests.
