# Runtime branch audit integration

Date: 2026-10-29. Runtime parent is
`b70f32319c533007b927985f7c07061f10e9dddc`; merged audit parent is
`892e58d9fe4ca4d0af7ceff4924f1ccbdb34ca1f` (PR #250).
The merge preserves both histories.

The only conflict was in README's project setup paragraph. The resolution
retains the runtime branch's Linux build egress and explicit Podman-image
rebuild instructions, then takes main's corrected `git init` guidance.
Main's conditional AppArmor wording and removal of the obsolete full-profile
egress claim also apply.

Relative to the runtime parent, source changes move existing disk commands
from box help to top-level help and remove redundant doc comments in
`cli.rs`, `config.rs` and `trust.rs`. The guest-kernel workflow changes only
a comment. Twelve historical audit/evidence files are added unchanged from
main. This note records the integration separately.

The CLI source equals the runtime parent's bytes after removing doc-comment
lines and reversing the two known help-line placements. Config and trust
source equal the parent after removing doc-comment lines. No other source
file changes. Tests and assertions, dependency lock, init/core runtime,
builder pins, builder-root inputs and ARCHITECTURE.md have identical Git
objects. The local identity receipt records those objects. Formatting and
the staged whitespace check passed. No duplicate runtime suite was run for
this help/comment/documentation delta. The prior executable hash does not
claim to identify a newly built merged executable.

## Retained runtime-parent validation

The full macOS e2e suite passed at the runtime parent on 29 October:
24 passed, 1 ignored, 0 failed, in 93.00 seconds. The external wrapper
recorded a clean checkout, source identity, free disk space, timestamps and
matching before/after executable SHA-256:
`236990617b2d3c9499deac569c381c2160cb466258a0b8a2cb372cddab63d5f5`.
The whole wrapper took 100.03 seconds including the native prebuild.

Linux CI run `40843517833` at the same parent failed on the first default
build with `builder-unavailable`: the exact pinned archive was absent and
public distribution awaited approval. It reported 1 passed, 25 failed,
1 ignored in 1.26 seconds; later failures include a poisoned-lock cascade.
This provides no runtime or budget admission. The separate bootstrap
workflow passed but does not provide the exact pinned archive. No unchanged
Linux retry was requested.

## Receipts and open gates

Local evidence remains under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.

| File | SHA-256 |
|---|---|
| `mac-final-b70f323-1.json` | `07919a54e16b63975225a0e4b11e036155ce10d5b4d7e30b961e6765d7f41a22` |
| `mac-final-b70f323-1.log` | `e25065a83a613cbe62018528186edbf8fae0c291888a8ebaba243ebefea6e1a8` |
| `mac-final-check.py` | `c447fa778405fdb36bb92fb00e8921ad4773c4aa9f3c1e386f6560579dd43397` |
| `linux-pr252-b70f323.log` | `d587868d45bd566e743c2aef2f9bc981be73b210c47679e347c3d7a22d60f961` |

`main-audit-merge-identity.json` records the bounded source comparison and
unchanged objects. The runtime branch remains a draft. Exact-archive
publication, reviewed public download pin, normal fresh-cache installation,
full Linux validation and its budget, numeric policy, remaining boundary
evidence and operator admission remain separate gates. This merge does not
authorize artifact publication, a tag, release or runtime merge.
