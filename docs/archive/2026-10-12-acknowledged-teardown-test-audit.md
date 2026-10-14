# Issue 89: acknowledged teardown without another inventory

`box down` now succeeds immediately when the observed owner explicitly
acknowledges completed teardown. Previously it could acquire the released
name lock and start a global runtime inventory after successful teardown.
Unrelated storage contention could then turn completion into failure.

`request_stop` returns true only for the owner's exact `done` reply.
An absent record, released claim, changed generation or `gone` reply does
not take the shortcut. Errors retain the existing recovery path.
Orphan removal still acquires the lifetime lock and checks runtime labels
and generation. The owner still acknowledges only after runtime removal
and host-state cleanup succeed. An acknowledgment can precede release of
the lifetime lock; callers' separate capacity verification is unchanged.

The implementation is 12 added and 7 removed lines across ownership and
box lifecycle code. The binding spec and guarantee 9 describe the change.
No runtime inspection API, dependency, CLI flag or timeout was added.

## Test audit

Retain the extension of `box_lifecycle_works_for_a_caller` in
`crates/e2e/tests/e2e/box_.rs`. The test diff is 114 added and 11 removed
lines, including moving the existing resume guard and retaining the
existing image-lock descriptor for the second scenario.

1. Guarantee 9 names acknowledged teardown without another inventory.
2. The comment names its sabotage: restore post-acknowledgment inventory.
   The restored path would wait on the held real image-store lock and fail.
   That sabotage was reviewed, not run by editing the binary.
3. Ordinary teardown does not expose this failure. The Linux scenario
   holds Podman's image-store POSIX lock and observes the real owner's
   removal child waiting on that device/inode. It stops the requester and
   confirms its stopped state before unlocking. The owner then completes
   teardown and exits. The test reacquires the image lock before resuming
   the requester, which must succeed with empty output while the lock is
   held. The original ten-second owner-request budget is preserved.
4. Expected success and empty output come from the CLI contract. Process
   parentage, stopped state and lock identity come from `/proc` and host
   metadata. The real owner's terminal event and exit prove teardown
   completed before resumption. A subsequent runtime inventory confirms
   absence after releasing the lock. Existing stopped-owner, cross-root
   replacement and foreign-container assertions remain in the same test.

No new test function, mock, runtime stub, sleep or binary test hook was
introduced. The lock is dropped before panic cleanup can inventory, and
the requester has a resume guard. Independent read-only reviews found no
production issue and accepted this assertion block against the audit.

## Validation

- Formatting, `git diff --check` and strict all-target Clippy passed locally.
- The macOS Apple-container lifecycle test passed: one test, 75.16 seconds.
  The initial inventory contained only the shared builder. No other
  Pinfold or Switchyard suite was observed during this test.
- Native-overlay Podman 5.4.2 on the original unprivileged `yard-sthlm`
  LXC passed the extended lifecycle test: one test, 72.96 seconds.
  Linux strict all-target Clippy and the real musl binary build passed.
- An earlier Linux attempt failed before exercising lifecycle behavior:
  concurrent Rust compiler launches collided while obtaining `rust-src`.
  A separate build then identified a missing musl C compiler. Both logs
  are retained. The successful attempt used the installed Rust compiler
  and a private extracted musl toolset; no system packages were installed.

Remote source and evidence are in
`/home/yard/validation/pinfold-89-lxc-20261012/ack-teardown-1/`.
The source base was `c5d0bdfa5cc02f1cb82dcae5ae79033f0c35aae9` plus patch
SHA-256 `1424e3d515a7a2686ea315f8f45c4f6be391660202a0d63a48e8fba3df35aab1`.
The tested Linux musl binary SHA-256 was
`d2eacdb2024f575454b2582c7eec209cfbb51058566f9aba090c250d545d8e85`.
The independently cross-built release binary SHA-256 was
`1396ed5badc3d7daa509077b928c50504a88498dcdff4febf025ceaa74f71dcd`.
The lifecycle test built and used the real debug musl binary, not the
cross-built release artifact.

The disposable runtime inventory was empty afterward, and no own test
process remained. Operational Yard retained PID 965905, start ticks
89301792 and its original binary checksum. Installed Pinfold retained the
official release checksum. Private fresh maintenance stamps prevented
daily cleanup from touching unrelated ownership records. No active store
reset, security weakening, release or issue publication occurred.

This is focused validation of the acknowledgment fix. The full Pinfold
suite and original 72-test Switchyard workload were not rerun for this
change. The prior failed workload results remain valid evidence. Native
storage contention, historical EAGAIN and issue 89 remain unresolved.
