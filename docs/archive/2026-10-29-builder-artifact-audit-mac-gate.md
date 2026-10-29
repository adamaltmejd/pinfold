# Current audit candidate: Mac gate receipt

Date: 2026-10-29. Tested clean head:
`54ba78de1900cf2c79fa2aaae4a202ea0914d4ad`.
This is a new result for the coherent candidate based on producer main
5c0ffa2. It supplements the pre-gate audit snapshot without rewriting it.

The parent confirmed the runtime worker had finished and released the
slot, and assigned independent correctness/security and lean/spec/test
reviews. Before starting, a process-name check found no active e2e suite;
disk space was 11617689600 bytes. No shared cache cleanup was run.

The single planned command was `cargo test -p e2e --locked`, unfiltered,
using the normal run-mac-e2e.sh Cargo runner, dedicated Cargo/Zig caches
and no compiler wrapper. No HOME, credential, production timeout or test
assertion was changed. No automatic retry or skipped live test.

Result: **24 passed, 0 failed, 1 ignored**, exit 0. The ignored case is the
existing production-five-minute response-deadline test, G34. Rust suite
time was **100.85 seconds**; full measured command time **101.816 seconds**.
The real live-login test passed. G36 is separate and was not run here.

Private receipts are retained in:

`/Users/adam/Code/pinfold/target/builder-artifact-audit-current-2026-10-29/`

| File | Bytes | SHA-256 |
|---|---:|---|
| mac-candidate.log | 4358 | 5a58de074f45c5d605facc6d6dc29da570659770169975a2852980c99d44c40e |
| mac-candidate.json | 342 | 71adfef34f75fa1ee396357ed5d55ebcb05a829f347439a76a8e58455aebb708 |

Both files are mode 0600. The timer binds the exact head, command, clean
initial state and result. The post-run process-name check found no e2e
process; the Mac reservation is released. No box was stopped to obtain
this result.

The older 3d6b106 run remains failed with its separate upstream-429
observation and intentional stop/137. Producer 784988b and runtime b70f323
passes remain separate evidence. This pass is not a rewrite of those runs.

This receipt is a later documentation-only commit. Any final metadata head
must retain the exact operative-file hashes from the tested candidate;
the parent reviewers must check that delta. No new Linux CI, CLI release
build, slow proof, push, merge, tag or artifact publication is implied.
The three test-proof tickets and explicit publication/gate dispositions
remain open as recorded in the current audit report.
