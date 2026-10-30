# Follow-up: graceful Apple box shutdown

Guarantee sought: allow an Apple box a bounded graceful shutdown before forced removal, while retaining the box as the unit of lifetime. This is proposed work, not implemented or release-validated behavior.

The upstream revisit condition in ARCHITECTURE fired: [apple/container#1941](https://github.com/apple/container/issues/1941) closed as fixed on 2026-10-22; [#1778](https://github.com/apple/container/pull/1778) merged at `1ebb9d07d1cfc8c221553457cf7b0f32c3a2d858` that day. Its signal-encoding fix does not establish which supported runtime versions expose a suitable native API or how pinfold should use it. The two future-work lines move here; every current process-supervision requirement remains unchanged.

Scope: core/runtime/apple.rs teardown. Review native API support, the minimum supported runtime version, signal/exit handling and bounded cancellation before selecting an implementation. Keep forced cleanup and fail-closed ownership. Do not introduce a host-to-process signaling path contrary to the current box-lifetime contract. Any changed guarantee needs its corresponding architecture edit and real CLI scenario.

Admission evidence: demonstrate orderly box shutdown through real runtime signals and an external durable-write observer, plus a nonresponsive workload that still reaches forced removal within the declared deadline. Preserve existing cancellation, ownership and exit-code cases. No mock, test-only binary flag or general supervisor framework.

Disposition for 0.4.0: written follow-up. Graceful shutdown is not added to this release.
