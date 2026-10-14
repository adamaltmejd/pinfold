# Inventory workload comparison

This investigation uses the unchanged original 72-test Switchyard workload
to compare native inventory with an initialized private Podman service.
It also captures natural storage-lock stalls beyond Pinfold's inventory
deadline. API inventory passed 72/72; native inventory passed 69/72.
This supports the prototype, but does not establish a clean throughput
improvement or complete production ownership and cancellation. Issue #89
remains unresolved.

## Identities and scope

Pinfold candidate: `cdf2c41970e82e41a453c5c55164d9a9bc3982ff`.
Linux binary SHA-256:
`85a75dd43f5a197be7e6df1a24e32f45f3291838f19dcef0d0c3c7c710a3d5ca`.
The candidate removes the previously measured 100 ms child-exit delay.
It does not implement ownership of a persistent Podman service.

Switchyard source: `23e55b2d0f7d38bfd14a6ce11ba6dcc837cebf5e`.
The command was `cargo test -p e2e --locked`, with default concurrency,
unchanged tests and assertions, and original caller umask `0002`.

Each run used its own native-overlay graph/run roots on Yard's original
unprivileged LXC. Both loaded the original base-image archive, SHA-256
`08ad0be8b6e6275c58229c82517c95fb42cdede1e11b25b527f4b4fe60bf53ee`.
This prepares the base image, not all images rebuilt by the workload.
Only `podman ps` uses the API in the comparison; lifecycle mutations and
exec remain native. Both runs use the same CLI and lock observers.

Evidence root:
`yard-sthlm:/home/yard/validation/pinfold-89-lxc-20261012/`.
Run directories are `candidate-suite-local-cdf2-1/` and
`candidate-suite-api-cdf2-1/`. The driver is
`pinfold89-paired-workload-cdf2.py`, SHA-256
`d75732afaf3e6de01a4906dc080b9bbbb67c54ebbd74319ccea96f5379f8f5d8`.

## Native result

The native run completed naturally: 69 passed, three failed, with
1,558.900 seconds of driver wall time. Cargo exited 101 and the source
remained clean. The final private container inventory was empty.

Failures:

- `a_publication_without_a_registration_does_not_count`: daemon attention
  reported that a box was not down within 60 seconds.
- `a_packaged_audit_bounds_iterations_and_requires_native_publication`:
  the controller observed zero calls where the scenario required 12.
- `a_gate_errors_start_reruns_that_gate`: a later scenario's FIFO handshake
  did not arrive within its existing 600-second wait. This was not an
  indefinitely hung cleanup; all 72 tests produced terminal results.

The durable failure's initiating cause is unproven. Its execution ended
failed and released capacity. Caller umask was `0002`; the earlier
diagnostic umask error was corrected, but the failed image's executable
permissions were not independently captured.
The three timed-out exec calls share one daemon ancestry, but retained
observations do not bind it to this fixture. The fixture's execution
transcripts were deleted without capture, preventing a causal assignment.

There were 656 completed `box list` calls: median 0.122 seconds, p95 0.675,
maximum 20.024. Six returned `runtime-timeout`. Three additional `box exec`
calls timed out in their internal inventory. No captured list/exec stderr
contained EAGAIN or disk-exhaustion errors. This does not assign the
historical daemon EAGAIN to a command or syscall.

Native evidence manifest `evidence-sha256.json` covers 1,450 files,
85,627,606 bytes; manifest SHA-256:
`9b6edd059472de7cdeedf7114b0c96d3ba6cd99e064163c359372d146d72e384`.

## API result

The API run passed all 72 tests, including all three native failures.
Driver wall time was 912.597 seconds, or 912.725 including service startup.
Cargo exited 0, source remained clean, and both final native and API
container inventories were empty. The private service exited 0 after
graceful TERM. Its recorded service, Cargo, test and observer identities
were absent on the final check.

| Observed result | Native inventory | API inventory |
| --- | ---: | ---: |
| Tests passed | 69/72 | 72/72 |
| Driver wall, including service startup | 1,558.900 s | 912.725 s |
| Completed `box list` calls | 656 | 687 |
| Listing median | 121.518 ms | 62.161 ms |
| Listing p95 | 675.278 ms | 226.727 ms |
| Longest listing | 20.024 s | 4.395 s |
| Direct listing runtime timeouts | 6 | 0 |
| Exec calls timing out in inventory | 3 | 0 |

All 687 completed API listings succeeded. The 464 completed exec calls
included 20 nonzero results, but none flagged runtime timeout, EAGAIN or
disk exhaustion. A successful full suite includes intentional failures;
nonzero exec results are not independently a regression. Whole exec
lifetimes include guest work and are not empty-exec latency measurements.

The service started in 128.134 ms. Sampled peak RSS was 58,974,208 bytes,
and its own CPU increased by 5.58 seconds between workload boundaries.
Those figures exclude remote clients, native mutations and other workload
processes. They do not measure total resource savings. Minimum available
space was 5,417,848,832 bytes on `/tmp` and 6,934,269,952 on the trial volume.

Live inspection bound daemon identities to the review and durable fixtures.
The ownership-unknown messages came from deliberate fixture sabotage;
subsequent SQL showed completed cleanup and released capacity. The durable
scenario reached `audit_limit`. Final passing assertions are the deciding
evidence. Token-only transcript observations and process/SQL metadata are
retained as `pinfold89-api-live-*` files, without raw transcript payloads.

API evidence manifest `evidence-sha256.json` covers 1,522 files,
36,421,743 bytes; manifest SHA-256:
`72ab6e11edeb7e9f525837b1faef6c0754efeccbd9986e3f97ce74af906b8adc`.

## Natural stalls and recovery

All six direct listing timeouts returned exit 1 and `runtime-timeout`
after 20.021–20.024 seconds. Actual POSIX lock records and descriptor
device/inode identities identify their storage-lock chains.

The first three native `ps` processes (755622, 758839 and 762775) waited
on `images.lock`, device/inode `fc:08:525028`, held by native writer
755544. Its child 755608 was `storage-chown-by-maps`; sampled threads
performed ownership changes and waited for filesystem journal work.
The same writer and helper survived multiple listing cancellations.
The fourth listing also waited behind an image-lock writer. The last two
waited on `storage.lock` held by 766814, which waited on `layers.lock`
held by 766833 with ownership-conversion child 769028.

A subsequent listing, candidate 777842, descended from the same daemon
737916 and waited on the final holder chain, then succeeded after 8.153
seconds. The following
listing succeeded in 0.084 seconds. All six timed-out native `ps` PIDs
were absent on the final check.

Fixture snapshots show `g8-gate-error` execution 4 retaining `reserved=1`,
`teardown=0`, and ownership attention across the timeout interval. It
changed to `reserved=0`, `teardown=1` immediately after successful listing.
The snapshots do not independently bind daemon 737916 to that fixture,
so the cross-source linkage is temporal correlation.

These observations do not complete the combined validation contract.
The observer forwarded stdout without recording it. Sampling does not
prove waiter removal before failure publication. Writer survival does
not prove successful completion. No before/after box-init identity and
ownership-file comparison binds these natural stalls to a preserved
live box. Earlier controlled probes cover several of those facts, but
cannot substitute for evidence from these particular incidents.

## Comparison limits and preservation

Another workload using `podman-shifting-profile` ran concurrently on the
same UID. Shared-memory records show activity from both private stores;
the native run also experienced substantial I/O pressure. This prevents
an isolated speedup claim or assigning a test failure to that other
workload. Sequential run order and host caches are additional confounders.
The failed native run did not execute all later scenarios within its
failed tests. These wall times therefore do not compare equal successful
work, and no percentage throughput improvement is claimed.

No FUSE configuration, security setting, workload concurrency, deadline
or assertion was changed. No active store was reset or pruned. The
operational Yard process and installed Pinfold identity were preserved.
Yard remained PID 965905, start ticks 89301792. Installed Pinfold remained
SHA-256 `7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`.
The separate held-mutex fault injection ran on disposable labvm and is
recorded in `2026-10-13-owned-inventory-held-lock.md`.

## Implementation boundary

Native inventory already kills `podman ps` at its deadline. Native and
API listings reach the same container-state synchronization, and both
initialize libpod. Interrupted state writes and startup are therefore
largely shared baseline risks. The held-mutex experiment does not prove
arbitrary crash safety, but an exhaustive new Podman crash-safety campaign
is not a prerequisite to starting a small owned-service implementation.

The new risks are specific: one cancellation affects every request in a
service generation; client disconnection alone leaves server work alive;
and service ownership outlives individual callers. Production adoption
requires evidence for concurrent requests, simultaneous cancellation and
restart, stale-generation rejection, supervisor death, and cleanup before
reporting failure. Reuse must remain bound to the intended runtime/store
and private socket. Startup and coordination must fit the existing deadline.
Isolated performance validation and the missing natural-stall evidence
remain separate requirements.
