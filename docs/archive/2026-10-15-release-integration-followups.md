# v0.2.5 integration follow-ups

2026-10-15. The combined PR #91/#93 candidate `a494641` passed all
assertions, but the x64 suite took 305.43 seconds. It did not meet the
300-second budget. Arm64 took 217.81 seconds. Both durable-recovery gates
passed. These results did not authorize release.

## Pi prompt composition

Tracked in [#95](https://github.com/adamaltmejd/pinfold/issues/95).

A manual real-Pi RPC check found an incompatibility introduced by the
Ponytail 5 pin. With a routed host model fixture, `/ponytail lite` returned
`handled`, but the next model request omitted Ponytail's active rules.
The initial `off` request and the later `lite` request both contained the
operating context and the same 6,279-character system prompt.

Pi 1.2.0 treats a returned complete system prompt as `forceSystemPrompt`.
Our operating-context extension returned one before Ponytail 5 added its
structured section. Pi's final projection discarded that later addition.
The pinned upstream sources and the real requests agree on the cause.

`e25e75f` writes the unchanged operating facts into their own structured
section. G14 now checks the upstream full-mode marker across its existing
default/full/default sequence, alongside its existing allowlist checks.
No security control moves into the image or extension. Independent review
approved the API use, external expected value and named sabotage. The
spec's G14 row describes the additional scenario.

## Remove a repeated fixture build

The G14 custom profile changes mounted configuration, not image contents.
It previously rebuilt the complete default image and then separately
obtained the suite's default image. `8d327a3` inherits that already-built
image using an ordinary Containerfile. It retains the distinct profile
build, its cleanup, the full-profile build, every launch and every assertion.
The implementation and test-audit reviews found no weakened proof.

Two alternating real Mac CLI build pairs measured the operation before
changing it. Each used isolated XDG state and a separately named profile;
all four builds succeeded and their image tags were removed afterward.

| Pair | Complete default copy | Derived from existing default |
|---|---:|---:|
| 1 | 10.413 s | 3.152 s |
| 2 | 10.735 s | 2.114 s |

This is a measured component reduction on Mac, not an isolated Linux
whole-suite effect or a product startup claim. Linux release acceptance
still requires the complete final suites to meet the budget. Compilation
and image builds remain inside the suite timer.

## Verification scope

The release audit now has two written tickets: #94 for deferred trust-reason
proof and #95 for the reproduced integration defect fixed here.

The earlier whole-tree audit and its #94 trust-refusal follow-up remain
applicable. These two additional commits received independent review.
Proxy, runtime, init, pinned artifacts and Containerfiles are unchanged
from `a494641`; only the operating-context resource, G14 and its spec row
changed. The final release report records the full suites, recovery,
slow-gate applicability, release builds and downloaded-artifact verification.

Final implementation `8d327a3` passed formatting and strict Clippy. The Mac
suite passed 33 tests, failed none and ignored its separate slow gate in
233.04 seconds. A real Pi RPC session now handles `/ponytail lite` and
changes the next model request from no active rules to the upstream lite
marker. Mac durable recovery passed again. Native inventory afterward
contains only the pre-existing buildkit service.

G34 passed at `a494641` in 311.36 seconds. G36 passed on both Linux hosts
in run [38684323106](https://github.com/adamaltmejd/pinfold/actions/runs/38684323106).
Independent review confirmed that neither gate executes Pi or Ponytail,
and their mechanisms and fixtures are unchanged. The rebuilt binary does
embed the changed inert resource, so these are retained behavioral results,
not claims that the final binary bytes were executed by those earlier runs.

Fresh final Linux suite and recovery results remain release gates. The
release report records them and the published-artifact checks.
