# Issue 89: first full candidate workload on the original LXC

The unchanged original Switchyard workload ran on `yard-sthlm` with
Pinfold candidate `70c18327c811c7e9eae92778dcb365acec5e9ca8`.
It failed: **69 passed, 3 failed, 828.59 seconds**, cargo exit 101.
This is not a clean compatibility pass or release confirmation.
No assertion, deadline, concurrency or security setting was changed.

## Identity and isolation

The source commit was `23e55b2d0f7d38bfd14a6ce11ba6dcc837cebf5e`, tree
`d90a980bce116925c8097d5aee74cf1b18e78e63`. Its checkout stayed clean.
The command was `cargo test -p e2e --locked`, with the original durable
inputs and no `RUST_TEST_THREADS` override. All 72 tests ran.

The candidate binary SHA-256 was
`939f1e901063fd7a5f1e5370d197612033cf39c797a4756f8b392ed28959f079`.
The fresh native-overlay graph/run roots and evidence are in
`/home/yard/validation/pinfold-89-lxc-20261012/candidate-suite-cold-2/`.
Private XDG state was `/tmp/pf89-cand-cold-2`. A current maintenance
stamp prevented the global daily orphan pass.

The initial store contained the original base image's two unmapped layers
and no containers. Native overlay had no FUSE program, no graph options,
no ID shifting and no metacopy. The base image configuration and rootfs
identities matched the original investigation. Mappings were cold.
Existing services and the active store were preserved.

## Invalid setup attempt

An earlier attempt, `candidate-suite-cold-1`, used a state path whose
proxy socket pathname was 116 bytes. Linux permits 107 pathname bytes.
Box startup failed before candidate workload behavior could be tested.
That attempt was interrupted through a pidfd after verifying the test
PID, start ticks, executable and working directory. Four surviving fixture
daemons were stopped individually after checking their fixture lock FDs.
Its scoped runtime inventory was empty. Logs and exit 101 were retained.

The corrected state path produced a 63-byte proxy socket pathname.
The invalid setup attempt is not evidence of a candidate regression.

## Failures

### Legacy planner migration

`g08_audit_reports::an_unbound_legacy_planner_requires_a_fresh_admitted_attempt`
failed at source line 333. Its real `yard attempt start Y-1` command
returned `internal`, with `store: database or disk is full`.
The project database was on `/tmp`.

### Packaged audit

`g08_durable_package::a_packaged_audit_refuses_missing_assets_then_publishes_in_a_corrected_box`
failed before the intended missing-entrypoint sabotage. The first image's
`install-image.sh` rejected its Node archive checksum. The later corrected
box phases were never reached.

The retained public source archive was intact: 62,798,625 bytes, SHA-256
`15fc043a51a5783f2f4b634561091658f5a7f0e3db03f997dca3a8033a33e45b`.
Failing layer
`a6b1c334325a1840f92d00ec28a58b608825125c71f14325ba0abac25c79290e`
contained 117 input files, of which 116 were empty. Its Node archive was
45,424,640 bytes, an exact prefix of the public source, with SHA-256
`628862fbb2a96654e3d30b8e7222a7400918dd92eccd88afe3ee6128d743bfe2`.
A separate complete COPY layer had the correct archive. No shared hardlink
or corrupt retained source was observed.

The fixture's image-build wrapper runs `cp -R` without checking its status
or enabling `set -e`, then invokes Pinfold. Yard's image-build error path
uses the JSON build log and omits separately collected wrapper stderr.
A failed host copy could therefore leave partial input and lose its error
detail. The failed host context was already removed. Host copy, Buildah
COPY and a later layer operation cannot be distinguished from this record.
The candidate does not change image-build copying.

### Claude review publication

`g08_review_is_a_publication::claude_api_credentials_stay_on_host_and_publication_requires_proof`
failed at source line 1806. The expected approval was instead a red review
attention: `the seat ended without publishing`, execution 3, sequence 18.
It did not fail a credential-leak assertion. Later credential assertions
and the login control were not reached.

This attention is raised after harness completion and successful box
teardown. It does not identify the harness's initiating failure. The
retained DB snapshot was earlier than the final event; its transcript was
removed by fixture cleanup, and raw harness stderr was not retained by
Yard. The exact failure remains unassigned.

There is a potentially relevant inventory observation. Runtime listing
2794552 waited on image inode `fc:08:657943`, held by 2794528, with a child
in `fchownat`. Its maximum sampled age was 19.533 seconds. Its candidate
parent 2794551 was `box exec`, under this Claude fixture's daemon 2788216.
Both disappeared before the next sample, about 0.55 seconds later.
Neither the caller exit status nor its timeout token was captured.
A candidate timeout is possible and cannot be asserted or excluded.
The suite log contains no `runtime-timeout` diagnostic.

## Temporary storage

`/tmp` is a separate tmpfs of 8,266,735,616 bytes. The graph store's
filesystem still had approximately 59 GB free after the run. Checking
only the graph-store filesystem before the run missed this separate limit.

After the run, `/tmp` initially had about 1.8 GB free. During review it
independently recovered to about 6.6 GB free; this investigation did not
remove unrelated files. Each durable input set is about 253 MB. Two
overlapping fixtures' input and build-context copies alone can consume
about 1.01 GB, before other temporaries.

Temporary-space exhaustion is a strong explanation for the SQLite error
and partial copy. No contemporaneous filesystem occupancy or failed copy
syscall was recorded, so this is not proof of the physical allocation
failure. The Claude publication failure must not be assigned to it.

## Listing wait mechanisms

The observer captured 300 runtime inventories. Fourteen reached a sampled
age of at least two seconds. Eleven had captured POSIX waiter chains;
the remaining three have distinct userspace evidence.

Listings 2756226 and 2756227 reached about 3.24 seconds. Their main threads
were repeatedly in `clock_nanosleep`. Their futexes used private anonymous
mappings. Both opened SQLite database inode `fc:08:657913`, concurrently
write-locked by 2755887, whose thread 2755901 was flushing the journal.
No kernel waiter record connected either listing to that range lock.
SQLite busy-handler sleep is supported as a hypothesis, not a proved
call-site assignment.

Listing 2809661 reached 2.206 seconds. Its main thread entered nonprivate
`FUTEX_WAIT` in `/dev/shm/libpod_rootless_lock_1000`, device `00:52`, inode
2, offset 144. Matching Podman and host glibc layout place that offset at
lock 2's robust mutex word. Expected value `0x802adf1f` is
`FUTEX_WAITERS | 2809631`, matching an observed writer thread flushing the
SQLite journal. This is a source-supported owner-TID correlation at wait
entry, not a direct current-memory observation or a continuous hold time.

Sources: [Podman mutex initialization](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.c),
[segment layout](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.h),
[glibc robust mutex](https://github.com/bminor/glibc/blob/glibc-2.41/nptl/pthread_mutex_lock.c).
The shared segment remains common to the user's stores. These new
observations do not recover the two historical delays' missing data.

## Evidence and limits

The evidence directory holds the complete host log, run result, identity
records, raw PID/start-tick/FD/syscall/futex/lock samples, read-only fixture
row observations, daemon identity mappings, filtered requester traces,
partial fixture snapshots and checksums. Separate SQLite/WAL file copies
are not guaranteed transactionally consistent snapshots.

Down/prune requester traces ended successfully with no captured socket
receive EAGAIN. Nonblocking read EAGAIN in inventory drains is expected
and is not a teardown failure. Other caller operations, including the
near-bound `box exec`, were not result-traced. The original EAGAIN failure
was not reproduced or assigned by this run.

After the run, the disposable inventory was empty and no candidate or
fixture daemon remained. Source stayed clean. Operational Yard remained
PID 965905, start ticks 89301792, with its original binary checksum.
Installed Pinfold remained the official release checksum. The failing
build layer and all diagnostic evidence were retained. No active store
reset, unrelated cleanup, issue comment, issue closure or release occurred.

The earlier separate stress probe still establishes bounded listing,
waiter cleanup, live-owner preservation and fail-closed capacity retention.
This full workload does not establish a clean candidate pass. A further
unchanged run was prepared after space recovered, with filesystem
occupancy and non-owning list/exec result capture to resolve the specific
evidence gaps. The failed result remains part of the validation record.
