# Cleanup fixture name and backing-file observer

Date: 2026-10-29. Test-only follow-up to
`dc45ab6e251e57346d13adc012144412fc9b4a7e`, scoped to guarantee 15.
The operator's Mac invocation passed the caller test and failed cleanup
at the unlabeled image's positive existence check, before cleanup ran.
The native scratch build had returned success. The result was one passed,
one failed, exit 101, 27.02 s test time and 28.542 s wrapper time. The
production executable's before/after SHA-256 matched
`8458dd1438a1f3b92d34ddefe195528a19e37e50e5720d26f9dc0d0bf8b5ea57`.

The fixture used an unqualified native image name, while `image_id`
compares inventory names exactly. The previous fixture used an explicit
localhost reference. Restore a task-owned
`localhost/pinfold/e2e-cleanup-unlabeled-<pid>` repository without changing
the scratch recipe, labels or existence/survival checks. Runtime name
normalization is the source-supported explanation, not an observed
inventory result: the read-only inventory command was denied with
`Operation not permitted`. It was not retried. No product normalization
helper or external-image protection was changed.

The Linux root observer previously selected the first ext4 file in the
image directory. An older completed representation could satisfy that
choice while the root actually used by the live box disappeared. Obtain
the existing box's VMM pid and stat its open descriptors. Match their
device/inode identities to regular files in that image's host directory,
rejecting missing or ambiguous matches. Candidate symlinks are not followed;
only disappeared transient descriptors are skipped. The live-rm assertion
requires the observed path to retain that same identity. After teardown
and removal, that actual backing path must disappear. This does not claim
that every historical representation or hard-link alias was collected.

Expected file identity comes from the kernel's live VMM, rather than
Pinfold's root-selection code. The named sabotage remains premature root
deletion during a live lease or leaving its backing path after final
removal. No guest filesystem is mounted or read, and no build or box is
added. The five case builds and the separate Apple external fixture remain.

`runtime.rs` changes from 981 to 1,014 Rust lines. Production source and
G44 are unchanged. Static reviews and whitespace checks completed; no
Cargo command, compiler, formatter, test, VM or public push was run for
this follow-up. Both fixes require the operator's changed-source platform
validation before claiming success.

The failed Mac receipt was copied without changing the original files
to `/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`:

| File | SHA-256 |
|---|---|
| `mac-critical-five-dc45ab6-failure.log` | `068857e1478907ef0aef1d88c83ca2aa36b3e4708c95009584e6be836caeb5c3` |
| `mac-critical-five-dc45ab6-failure.json` | `b15a36f957c9dddfa8b4f53e6dce4dd416c8a7c4693a620a4388c0c9d9385242` |
