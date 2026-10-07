# Linux validation follow-up

The first Linux run of candidate `3aacad4c77c8fb81277f1a2c4c76d87357e721fa`
failed: x64 passed 31 ordinary tests and failed two in 208.39 seconds;
arm64 passed 30 and failed three in 162.77 seconds. Guarantee 34 was
ignored as intended. Both jobs passed formatting and strict Clippy.
The [run](https://github.com/adamaltmejd/pinfold/actions/runs/37642602006)
contains the original observations.

## Observer corrections

Guarantee 26 looked for `codex-app-server` in `ps`'s `comm` column. Linux
truncates that kernel name to 15 bytes; the executable's name is 16 bytes.
The real helper reached the stalled HTTP fixture, but the observer failed
to identify its PID. The Linux observer now uses the parent PID and the
actual executable symlink in `/proc`. Cancellation, deadline, fixture EOF
and helper-reaping assertions remain.

Guarantee 24 compared all new dangling image IDs in a shared store. A
concurrent caller-image build can legitimately use cached intermediate
images. The fixture now marks its source with a unique label before its
RUN instruction. Runtime inventory filters by that inherited fixture
label before comparing ID sets. Reusing a cached RUN still repeats the
random stamp; leaking its intermediate layer still adds a fixture-owned
ID. Other tests' builds and cleanup cannot affect the attribution.

Guarantee 27 observed the entire XDG cache root. Podman's native image
lookup created `containers/short-name-aliases.conf.lock`; the Pinfold
subtree was unchanged. The original maintenance contract protects
Pinfold state and artifacts, not every application's native bookkeeping.
The guarantee now says Pinfold-owned state, config and cache trees
explicitly. All three complete subtrees, including absent versus empty
roots, are observed. There is no runtime prewarming or filename exclusion.

Current upstream code explains the Podman observation: local image lookup
calls [short-name resolution](https://github.com/containers/common/blob/main/libimage/runtime.go),
whose [alias lookup](https://github.com/containers/image/blob/main/pkg/shortnames/shortnames.go)
uses a [cache directory and lock](https://github.com/containers/image/blob/main/pkg/sysregistriesv2/shortnames.go).
The runner's actual before/after tree is the evidence for these two CI
environments. Qualifying user image references solely to avoid that lock
would change reference-resolution semantics.

Sol 6.1 implementers and an independent Sol 6.1 reviewer handled the
corrections. They change the observers and clarify their scope; they do
not change production isolation, runtime selection or image resolution.
The pull request's checks record validation of the corrected commit.

## Shared cleanup fixture

The next Mac run passed 32 ordinary tests and failed guarantee 15 in
181.74 seconds. Its deliberately orphaned box had already been reclaimed
by another test's automatic maintenance before the positive-control
inventory. The earlier fixture retained a zombie PID to make other XDG
roots consider it alive. Host-wide generation ownership correctly removes
that distinction.

Guarantee 8 also needs an intact orphan to inspect egress and cross-root
liveness before pruning. A single standard read/write lock now covers
the shared runtime for the lifetime of every test. Guarantees 8 and 15
hold it exclusively; the other tests hold shared guards and remain
parallel. Guards are declared before fixtures, so fixture teardown ends
before the guard is released. Helpers acquire no additional guards.

This replaces the partial dead-box mutex. The cleanup scenario's intended
second caller runs an ordinary box-list command before the orphan exists,
so its initial maintenance cannot remove that same scenario's fixture
ahead of the measured cleanup. The previous `artifacts` call no longer
does this because artifact inspection is now read-only. Unnecessary
prewarming in the other tests is removed.

The original positive controls, exact orphan observations, dry-run/cache
assertions and sabotage claims are preserved. No retry, sleep, runtime
stub or production maintenance exception was added.

The corrected full Mac suite passed 33 ordinary tests in 163.96 seconds,
with the slow test ignored as intended. Formatting and strict all-target
Clippy passed. The log is
`/private/tmp/pinfold-mac-runtime-guard-20261007.log`. Production source and
the Durable example are unchanged from the already validated candidate;
the earlier slow-gate and prototype observations still apply.
