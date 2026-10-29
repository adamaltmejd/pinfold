# Host catalog SHA-256 implementation, 2026-10-30

This module cut starts from
`ddb6de0d4c8d8b29823df767d8991016953fce80`. The operator's native musl release
for that head has SHA-256
`6f7e18c450099ed87b1e38356be2fcc440acb73f5f41095642b6afe1c5921d6c`.
Its real default-profile warm/repeat observations were 10.872642006 and
9.983453041 seconds. These are prior-head samples, not this cut's results.

CPU-only profiling after the existing build-run readiness retained 290 samples
with zero lost samples. The operator reports 279 samples (96.21%) in sha2's
software SHA-256 compression and about 2.93 CPU seconds captured. Profiler
shutdown returned -15; its retained data and report remained valid. This
observation covers that sampling window. It is not a controlled wall-clock
comparison, complete build attribution or a predicted speedup.

The only code change substitutes the already directly linked Ring SHA-256
context for Sha2 in host catalog.rs::copy_checked, matching artifacts.rs's
existing builder-file implementation. Final digest bytes still pass through
the same lower-hex function and exact comparison. Digest-shape validation,
the 64 KiB buffer, remaining-byte accounting, read_exact/write_all/update loop,
cancellation checks, all other size/media/graph checks and public error tokens
are unchanged. Every previously hashed byte is still read and hashed.

There is no metadata cache, skipped read or representation fast path. Guest
init and its Sha2 implementation remain untouched. No package, feature,
dependency, compiler, artifact pin, root, helper, protocol, test or cap changes.
Ring was already a direct dependency; only its existing binding dependency
description expands to name host image-store hashing. Managed cached-build
defaults and explicit --no-cache behavior are unchanged.

The changed catalog.rs source SHA-256 is
`d6029c37bfc82b624cbc20a9377b140f46ad40f9c9275a986ae920e8d2e350e3`.
The worker runs formatting, whitespace and exact-diff checks only. Native
compilation, real default-profile observations and integrity-negative/restore
proof remain with the operator. No worker build, VM, suite, public push or
publication runs, and no performance result is claimed for this cut.
