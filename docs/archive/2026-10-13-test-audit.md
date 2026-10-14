# Bun switch and test audit, 2026-10-13

The durable example now runs under Bun 1.4.3. CI derives that version from
the profile image recipe. The separate Node runtime pin and setup steps,
Node update source and duplicate runtime-version file were removed.
Both Linux Bun archives matched the publisher's SHA-256 before repinning.
This was a requested manual upgrade; the nightly seven-day wait remains.

## Runtime boundary

The first real Mac gate reproduced an allowed writable controller alias.
Bun resolved a direct script path before placing it in `process.argv[1]`.
`--preserve-symlinks-main` did not retain the lexical path in a direct
probe. The Node-era guard therefore saw only the protected target.

The package's `start` and `resume` scripts now import through Bun's eval
loader. It leaves the supplied path in `argv[1]`. The controller parses
`argv.slice(2)` explicitly and refuses direct launches as `host-boundary`.
The existing writable-entrypoint-alias assertions remain. No runtime adapter,
test-only switch or new test was added. Node-compatible SQLite APIs remain.

Launches use the protected controller directory. Its Bun configuration
disables automatic `.env` loading and runtime dependency installation.
The typecheck explicitly uses Bun despite TypeScript's Node shebang.

## Assertion disposition

`examples/pi-durable/durable_recovery_preserves_boxed_execution.ts`,
lines 165–172 at base `0c2acc2`: `detector`, obsolete maintenance diagnostic;
delete the block, setup, comment and unused `utimes` import. [row 35]

The binding spec says `box list` skips daily maintenance and excludes it
from the automatic pass. `main.rs` implements that exclusion. The block
expected a successful list to emit a soft maintenance error, contradicting
the current contract. Guarantee 35 contains no maintenance-diagnostic
promise, so its row needed no edit. The succeeding boxed-command control,
writer refusal, crash recovery, unsafe replay and cancellation checks remain.

| Source | Lines |
|---|---:|
| Rust binary sources and build script | 9,361 |
| Rust e2e sources | 8,167 |
| Durable controller | 275 |
| Durable regression | 221 |

## Verification

The real Bun 1.4.3 Mac gate passed all host-boundary refusals, boxed command,
exclusive writer, crash recovery without unsafe replay and whole-box
cancellation. The full Mac suite passed 33 tests with one slow-gate test
ignored, in 331.30 seconds. Document conversion under the new profile Bun
pin passed. Frozen dependency installation, the Bun-only typecheck, Rust
build, formatting, Clippy, Ruff, actionlint and whitespace checks passed.

Candidate `dcb8b38` in the attached validation worktree matches the working
source changes. Its Linux upload was stopped by automatic approval review
pending explicit authorization for a temporary GitHub checks branch. No
remote upload occurred in that attempt. These Mac results do not establish
Linux runtime compatibility or release readiness.
