# G17 shared runtime guard and test audit, 2026-10-30

This consolidation starts from `9256aed91bb9131a4650c0142060ebc6e8b31f9d`.
Its release-binary Ubuntu run 41028179076 passed 26 tests, failed none and
ignored one. The body took 333.00 seconds; the wrapper took 336.271598769
seconds with e2e_exit 0. The unchanged 300-second budget failed. The bootstrap
job passed. The log shows the optimized release prebuild took 32.59 seconds
and the helper's release query 0.03 seconds, with the matching AppArmor path.
The whole job took 430 seconds. These are prior-head observations, not this
change's results or isolated scheduling costs. No unchanged rerun or further
fixture trimming follows from them.

Commit 62102396505d8929df96af9d5583e29917f5f557 gave Linux G17 an exclusive
runtime guard to protect its dedicated shared-cache profile build. The dated
2026-10-29 scheduling records describe that need and the real competing-build
failures. They also retract the earlier G23 downgrade: G23's later builds
still require exclusive protection.

Commit c6cc1bea1de4ad6e97b299bab9d079b568674d4a removed G17's dedicated
profile build and ImageCleanup guard. The whole current test uses only the
existing default_image setup, serialized by its per-cache set under a mutex,
private config/files/state, scoped refusal observers and uniquely named boxes.
It performs no clean, prune, image removal or uncoordinated image build.
Ordinary shared tests already use the same default setup and daily maintenance.

Only G17's three-line Linux conditional is removed. Both platforms use the
existing shared_runtime guard at the same first local position. Its scope
still outlasts every fixture and drop. No lock API or framework is added.
G23, G12, G26, maintenance, dead-owner and disk-writer guards are unchanged.

## Candidate audit

Row 17 keeps every assertion, refusal reason, fault observer, positive control
and fixture. Its sabotage comments still name weakened spec validation, recipe
selection and name claims. Nested inputs, the stale managed tag and the two-up
race remain the hard cases; the winner still answers exec after the loser
exits. Expected values still come from the supplied specs, host state, fixture
bytes and the spec's refusal tokens. No block is added, removed or rewritten,
and no new guarantee or table edit is needed.

Test source changes are zero added lines and three removed lines. Binary source
changes are zero. All other tests and guards, fixtures, release selection,
AppArmor setup, timer, budget, artifacts, pins, dependencies, caps and protocols
are unchanged. No timing improvement or new pass is claimed.

The worker performs source review only, with no formatting, build, test, VM or
push. The operator owns appropriate checks and real release-binary Ubuntu
functional and full-budget validation. Runtime merge, numeric approval, a CLI
release and a budget waiver remain unapproved.
