# Measured implementation test audit — 2026-10-14

Base `a1be921`; final implementation `0a50a74`. This is the bounded approval
pass for the selected implementations, not a new whole-tree release audit.
The module commits and independent reviews are linked from the implementation
tracker. No guarantee or assertion was removed. No sabotage was executed.

## Dispositions

- B1, G11: retain the new real-CLI external hook alias scenarios. Existing
  direct-path tests never entered the former absolute-outside early return.
  Missing descendants, dangling targets and unresolved parent traversal are
  the hard cases. The host hook fixture, real Git execution and host bytes
  supply expectations. A genuine outside hook path and the original writable
  project/read-only hook controls succeed. Source review names the sabotage.
- B2, G9: retain the real closed stdout reader before the spec is sent, with
  stdin still held open. Exit 1, native absence and host-state absence come
  from outside the binary. The following normal run reuses the name and spec.
  The old stdin/signal cases never exercised readiness-write failure. Keep
  the absence helper correction: only NotFound can mean an absent directory.
- C3, G28: reuse ChildOwner immediately after spawning the actual installed
  fixture executable under alternate XDG roots. The readiness assertion is
  moved, not changed. Remove the duplicate UpdateOwner cleanup implementation.
- O6, G28/G31: copy the new production init crate into the existing private
  source-copy fixture. Four lines; no assertion, seam or expected-value change.
- O1/O2/O3/O4/C2: no test assertion changes. Their existing guarantees remain
  the runtime acceptance boundary. O5 documents existing forwarding only.
- B3: G15 already repeatedly builds FROM scratch through the real CLI. Keep
  that scenario and its assertions; skip only virtual scratch registry work.

Binary crate changes (including manifests): +195/-83 lines. E2E changes:
+147/-51 lines. The code deletion/addition ratio is 134/342 = 0.392, excluding
lockfile, docs and the two-line example deletion. Added regression coverage
and the small executable entrypoint mean this is not a source-line reduction.
Largest production Rust file: cli.rs, 1,135 lines. Largest Rust file including
tests: box_.rs, 5,274 lines. ARCHITECTURE.md: 1,017 lines. All ten selected code
mechanisms are implemented; no unresolved implementation ticket was opened.

## Actual host validation before the first push

Formatting and Clippy with warnings denied passed on the final source.
The uninterrupted full Mac suite passed 33 tests, failed 0, ignored the
separate G34 slow test, and took 225.85 seconds. The durable G35 gate and its
TypeScript check passed separately. The G35 source/binary predates only B3,
which changes base-image resolution and is not used by that running-box gate.
Linux suites, the slow gates and release builds are separately recorded on the
associated PR; this audit does not substitute compilation for runtime evidence.

The first Mac run reached 33/0/1 in 808.53 seconds but required intervention,
so it was not accepted. A test-owned Apple `image pull ... scratch` client
stalled in G15. Its process listing showed 45 minutes elapsed; that wall-clock
observation differs from the Rust runner's monotonic duration. Only that client
was terminated. The existing code ignored the failed pull result, continued
building the empty image, and the suite finished. core/image.rs was identical
to main (SHA-256 2e8305d174a56a0a1cee282a3d45b3cbf3bbf6570d69f35b50d64abad1176a6f).
B3 removes the unnecessary lookup/pull; an independent reviewer approved its
exact lowercase match and spec sentence. The uninterrupted rerun above passed.
No runtime service was restarted and no global prune was used to clear the stall.

The primary tracked working diff remained SHA-256
`d553421a48a119d7d6eef39fa85b7aff691441b0dca05800c9ef2624008cc9c7`.
