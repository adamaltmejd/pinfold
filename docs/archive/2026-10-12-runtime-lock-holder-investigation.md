# Runtime lock holders in issue #89

Investigation on 2026-10-12, following
[the mapped-image experiments](2026-10-12-supported-image-mapping-paths.md).
Issue #89 remains unresolved. This investigation changes no production
code, runtime configuration, workload assertion or deadline.

## Historical shared-mutex owner hint

The saved `candidate-suite-mapped-split-1/userspace-observations.jsonl`
contains more information than the earlier report used. Listings 274562,
274590, 275252 and 275266 passed expected futex value `0x80042cc2` while
waiting on `/dev/shm/libpod_rootless_lock_1000`, identity `00:52:2`,
offset 64. Under the verified glibc 2.41 x86-64 robust mutex layout,
masking the two flag bits yields owner TID **273602**.

The expected value describes the word supplied at wait entry. It is not
a read of the current mutex word or proof of continuous ownership.
Saved snapshots associate TID 273602 with Podman PID **273584**, process
start ticks **116238124**, parent 272604. At wall time 1791819141.183535,
that thread was in `fdatasync` on FD 7, which resolved to the trial's
`storage/db.sql-journal`, identity **`fc:08:1887356`**. Sampled kernel waits
included `rq_qos_wait`, `folio_wait_bit_common` and, at 1791819141.739354,
`jbd2_log_wait_commit`.

This supports database journal I/O as an explanation for the earlier
shared-mutex delays. It does not establish the exact duration of mutex
ownership. It also does not assign the unrelated contemporaneous writer
273768 or its journal `fc:08:1887362` as the mutex owner.

Exact source lines, array positions, timestamps and the source checksum
are preserved in `historical-mutex-owner-hint.json` under the remote
evidence base below. Its SHA-256 is
`1c8c92abaeadc5e8a5b31adbaee4d92d2df5a45258f4b136d669ccb20b825b8b`.
Relevant source lines are 1303 and 1361–1363; source SHA-256 is
`01f26ac290e62afb0fba351203a203f6f60a6307f46d3302d9bee415ef08a357`.

## Read-only observers

The new POSIX observer follows actual waiter-to-holder records recursively,
including a holder that is itself waiting. Edges retain lock-record IDs,
device/inode identities and process start ticks. It records holder
descendants and distinguishes `storage-chown-by-maps` from other helpers.
Open descriptors alone are not treated as proof of a wait edge.

The shared-mutex observer reads the existing segment using read-only
`pread`. It validates the host layout and records mutex words, owner
TID/TGID, start ticks, PID namespace, mapped segment identity and owner
syscalls. It checks owner identity and mutex bytes again after sampling.
No mutex is acquired, released, reset or modified by the observer.
Snapshots remain non-atomic; ABA and cross-PID-namespace ambiguities are
explicit limitations.

The segment has 2,048 mutexes. Offset 64 is semaphore 0; the allocation
mutex is at offset 8. Private graph/run roots still use the per-user
shared segment. The source layout and robust owner encoding were checked
against [Podman 5.4.2's shared-lock header](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.h),
[its implementation](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.c),
[glibc's x86 mutex layout](https://github.com/bminor/glibc/blob/glibc-2.41/sysdeps/x86/nptl/bits/struct_mutex.h)
and [robust locking implementation](https://github.com/bminor/glibc/blob/glibc-2.41/nptl/pthread_mutex_lock.c).

## Focused cold-image probe

`lock-chain-probe-1` used a disposable native-overlay store on the original
unprivileged LXC. It imported a root-owned image containing 100,000 empty
files, alongside a small retained container. A keep-id create started
first. Only after observing its actual storage WRITE lock and an
ancestry-linked `storage-chown-by-maps` child did the driver start a
COPY-only build and a fresh listing. No artificial lock holder or injected
pause was used. This is a focused diagnostic, not the 72-test workload or
a controlled performance comparison.

| Operation | Seconds | Exit |
|---|---:|---:|
| Baseline listing | 0.063940 | 0 |
| Cold create | 5.886096 | 0 |
| Contended listing | 5.817800 | 0 |
| Contended build | 6.071215 | 0 |
| Recovery listing | 0.063885 | 0 |

Eight process samples from wall 1791825008.563029 to 1791825013.907316
show listing **756125**, start ticks **116830626**, and build **756124**,
same start ticks, waiting on `images.lock`, identity **`fc:08:1489375`**.
The holder was create **756106**, start ticks **116830619**. Its direct
child **756116**, start ticks **116830624**, was positively identified as
`storage-chown-by-maps`; helper threads were observed in `fchownat` and
filesystem creation/lookup waits. The trigger also recorded the create
holding `layers.lock`, identity **`fc:08:1492168`**.

This proves the direct listing-to-image-conversion chain. It does not
reproduce the historical `storage.lock` holder waiting on `layers.lock`.
Both writers succeeded. The retained container's identity was present in
the contended and recovery results. Exact-name cleanup occurred only after
both writers finished; final private inventory was empty.

The shared-mutex observer collected 65 sweeps over 6.51 seconds and six
held-mutex samples, with no sampling-cap truncation or set waiter bits.
During this probe's cleanup, mutex offset **104** had stable owner TID
**756841**, TGID **756828**, TID start ticks **116831241**. That same thread
was observed in `fdatasync`/`jbd2_log_wait_commit` against the probe's
database journal. Stable mutex reads bracketed the journal-I/O sample,
subject to the non-atomic observation limits above. No listing was captured
waiting on that mutex.

A stable sample at offset 184 included journal I/O from a different
validation store, `podman-shifting-profile/storage`. That activity is not
assigned to this listing stall. The host was not otherwise idle. No sampled
listing had a shared-mutex wait; missing samples do not prove that none occurred.

## Focused layer-lock chain

`lock-chain-probe-2` used another fresh native-overlay store and the same
100,000-file tar. A retained keep-id container first prepared the small
base's mapping. A build then used `FROM` that small base and `ADD cold.tar /`.
After its COMMIT progress and an actual layer WRITE lock were observed,
the driver started a create using the prepared small base. Only after that
create held `storage.lock` and waited on `layers.lock` did it start listing.
At most three runtime commands overlapped. There was one attempt, with no
injected lock holder, pause, retry or increased file count.

The recursive observer captured the full chain in JSONL line 18, from
the lock snapshot at wall 1791825533.895547 to the completed process
sample at 1791825533.921036:

| Process | Start ticks | Actual wait/hold |
|---|---:|---|
| Listing 765658 | 116883168 | Waits in `F_SETLKW`, FD 7, on `storage.lock`, `fc:08:539703` |
| Create 765649 | 116883162 | Holds that storage lock; waits in `F_SETLKW`, FD 8, on `layers.lock`, `fc:08:539728` |
| Build 765555 | 116882374 | Holds that layer lock |

The ultimate holder's build role was captured. Its sampled threads showed
private futex, epoll and nanosleep states; no descendant helper or specific
storage syscall was identified. Do not attribute this sample to ownership
conversion. One full chain sample proves the mechanism, not continuous
ownership throughout the operation or the identities in the historical run.
It agrees with storage initialization's nested locking in the
[store implementation](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/storage/store.go)
and [layer-store initialization](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/storage/layers.go).

The build completed in 17.094243 seconds, the create in 1.520435 seconds,
and the contended listing in 1.465641 seconds. All exited zero. Baseline
and recovery listings both took about 0.064 seconds. The retained identity
was preserved, exact-name cleanup followed successful writer completion,
and final private inventory was empty.

## What remains

Image ownership conversion is a proven blocking path. Preparing compatible
images can avoid that work for those images, but the prior full mapped
workload still failed. The new historical owner hint and live mutex sample
also implicate database journal I/O as a separate path requiring attention.
Neither result justifies calling image flags a complete fix.

The second probe identifies a real build as the ultimate holder in a new
two-level chain, even though the created container's image mapping was
prepared. The ultimate holders in the historical chains remain unknown;
the old observer did not retain their records. Historical EAGAIN remains
unassigned. Neither focused probe exercises Pinfold's 20-second failure path.

The evidence separates two directions: avoiding repeated runtime storage
initialization for inventory, and reducing image/storage work. The earlier
API probe supports the first direction for an image-conversion stall only;
it must also be checked against this build chain and shared-mutex/database
contention before a production transport change is justified. Image
preparation alone cannot remove these other paths. Existing bounded-listing
and acknowledged-teardown changes remain useful failure containment; they
are not a cure for every runtime wait.

## Evidence and preservation

Remote evidence base on `yard-sthlm`:
`/home/yard/validation/pinfold-89-lxc-20261012/`.
Each focused directory's `evidence-sha256.json` includes driver and observer
checksums; graph/run stores and input tar archives are excluded.

| Directory | Artifacts | Manifest SHA-256 |
|---|---:|---|
| `lock-chain-probe-1` | 45 | `c52fbbbc36e9b7b4b89c31f560abea60cac921053939447be0a42d24499e7bea` |
| `lock-chain-probe-2` | 42 | `b2e2d31f672b1b644e8bd85ba3156abe7953319bfc85993996015ec533f072c5` |

Postcheck found no remaining process with a trial-path argument. Operational
Yard remained PID 965905, start ticks 89301792, executable SHA-256
`05ae25832964703c81525e7c5f5be7a8148954517cfddf69ef9846953bf1ebb2`.
Installed Pinfold remained SHA-256
`7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`;
Podman remained SHA-256
`6e4993e061eb88205003892252e2e13bbf49c12a6f67f519881b9b3c94e980e6`.
The controller stopped and reaped only its own read-only observers after
the driver completed. Active stores and unrelated services were preserved.
No software was published and no GitHub comment or issue closure was made.
