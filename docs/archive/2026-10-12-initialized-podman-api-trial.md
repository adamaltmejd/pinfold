# Initialized Podman API trial

Investigation on the original unprivileged `yard-sthlm` LXC on 2026-10-12.
The acceptance requirement is no material slowdown in normal use or overall
workload throughput, while retaining truthful inventory and bounded failure
cleanup. This is an external real-runtime experiment, not a production
transport change. No installed service, runtime configuration or Pinfold
binary was changed.

## Focused build contention and normal listing

`api-layer-probe-1` used a fresh native-overlay graph/run root and a private
Unix socket. Its Podman 5.4.2 API service was initialized before the writer.
The same 100,000-file input as the prior layer-lock probe was used. A real
build held `layers.lock`; a create using a prepared small image held
`storage.lock` while waiting on that layer lock. The driver then launched
one local and one API listing.

| Measurement | Fresh local CLI | Initialized API |
|---|---:|---:|
| Contended listing | 1.473419 s | 0.025960 s |
| Warm listing median, 10 alternating pairs | 38.97 ms | 19.76 ms |
| Warm listing range | 32.14–42.37 ms | 19.18–21.71 ms |

The API returned while an actual POSIX snapshot still showed the complete
chain: local listing 1245826 (start ticks 117480271) waited on storage
inode `fc:08:528769`, held by create 1245817 (117480267); that create waited
on layer inode `fc:08:528790`, held by build 1245712 (117479442).
The build and create completed successfully in 17.522867 and 1.531519 seconds.

Both paths returned the retained container with identical Pinfold-consumed
fields. After creation, both returned the new container as well. The probe
checked types and equality for `Id`, `Names`, `Labels`, `ImageID`, `Image`,
`Created` and `State`. Exact-name cleanup followed successful writer
completion. Final inventory was empty and every launched process was reaped.

Service startup through a successful nonempty inventory took 78.65 ms.
Sampled service RSS was 48,592 KiB before warm measurements and 49,672 KiB after
recovery. A one-second idle sample recorded no CPU tick increase at 100 Hz.
These are own-service process measurements, not system-wide resource costs.
Warm local samples also ran with the service present. Ten pairs screen
obvious listing overhead; they do not establish a tight performance bound
for other commands or the complete workload.

The script used `waitid(WNOWAIT)` for exit timestamps, retaining each child
PID until main-thread collection; required process-group signaling preceded
reaping. Its SHA-256 is
`92cd603c14748e8b62eafca3450de2e72c355fe0cb29e6547e380321ce4887f3`.

## Cancellation review

The API's performance benefit does not by itself preserve Pinfold's
no-runtime-waiter guarantee after timeout. In Podman 5.4.2, the HTTP handler
passes its request context to `ContainerList`, but that implementation
calls `ps.GetContainerLists` without forwarding the context. Listing then
enters `ctr.Batch` and a blocking shared mutex. The SQLite listing path
uses `Query`, without the request context, and configures a 100-second
database busy timeout. Killing the remote CLI does not provide an
interruption path for those server-side waits.

Source: [API handler](https://github.com/containers/podman/blob/v5.4.2/pkg/api/handlers/libpod/containers.go#L101-L122),
[ABI listing](https://github.com/containers/podman/blob/v5.4.2/pkg/domain/infra/abi/containers.go#L1054-L1058),
[batch locking](https://github.com/containers/podman/blob/v5.4.2/libpod/container_api.go#L928-L944),
[shared mutex](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.c#L516-L532),
[SQLite configuration](https://github.com/containers/podman/blob/v5.4.2/libpod/sqlite_state.go#L70-L81)
and [query](https://github.com/containers/podman/blob/v5.4.2/libpod/sqlite_state.go#L820-L828).

The separate live probe below confirms continued server-side lock attempts
after client cancellation.

## Live cancellation probe

`api-cancel-probe-1` used a fresh store and one stopped retained container.
Only its privately launched service was traced, recording `fcntl` and
`clock_nanosleep`, without payload or argv capture. The driver acquired a
real SQLite exclusive transaction on its own `graph/db.sql`, with no data
writes. This is an injected real-lock diagnostic, not a reproduced cold
stall or a test of Pinfold's exit code.

The remote CLI remained blocked for 20 seconds. The driver terminated and
reaped that client, PID 1371145, at wall 1791832177.940986. Stdout was empty.
It retained the database lock until wall 1791832179.973567. Between those
events, service TID **1371136**, start ticks **117545581**, made **20**
further `fcntl(F_SETLK)` read-lock attempts, all returning EAGAIN. Sampled
FD 3 identities link those calls to database **`fc:08:708713`** and its
SQLite locking range. This is direct continued lock-attempt evidence,
not an inference from sleeping threads alone.
These were repeated nonblocking lock attempts, not a continuously queued
`F_SETLKW` waiter. After rollback, the same thread acquired and released
the lock, then received SIGPIPE while handling the abandoned connection.

Rollback released the holder normally. A subsequent API listing preserved
the retained container's exact identity. Exact removal succeeded and the
final API inventory was empty. Only own processes were stopped. The
expected SQLite EAGAIN in this diagnostic is not an attribution of the
historical daemon EAGAIN failure.

This confirms that client-only cancellation can leave work waiting inside
the service. The probe used one abandoned request; it did not measure
accumulation from repeated abandoned requests or reproduce the shared-mutex
variant. Script SHA-256:
`025e80f3ba75bb7638a725f5879cd76deda01ac3f4d458d539e0340ddd887f17`.

## Full-workload method

`candidate-suite-api-1` ran the original Switchyard command
`cargo test -p e2e --locked`, with all 72 tests, original assertions and
deadlines, default concurrency, and the original durable inputs. Its source
commit was `23e55b2d0f7d38bfd14a6ce11ba6dcc837cebf5e`, tree
`d90a980bce116925c8097d5aee74cf1b18e78e63`.

The unchanged Pinfold binary was candidate
`31ff6841aa78ff7b8557d271eb261846fc845d41`, SHA-256
`1396ed5badc3d7daa509077b928c50504a88498dcdff4febf025ceaa74f71dcd`.
A shell wrapper routed only `podman ps` to the private API socket. All
other runtime operations used the local CLI against the same private
store. The original base image was loaded normally, without mapped-image
ingestion, build mapping flags or container warmup. A private maintenance
stamp prevented the unrelated global orphan pass.

The API service was initialized against the empty container inventory
before starting the workload. Startup took 72.14 ms. The driver recorded
list/exec outcomes without retaining argv or stderr payloads, raw teardown
syscalls, host pressure, disk availability and service CPU/RSS. Read-only
POSIX and shared-mutex observers ran alongside it.

The initial fixture observer required the test process's cwd to equal the
source root. Cargo used `source/crates/e2e`, so it missed fixture-state
records. A supplemental read-only observer, bound to the actual direct
child PID 1246261 and start ticks 117484946, began about 349.53 seconds into
the run. Early fixture-state history is unavailable. Command outcomes and
timings were recorded independently and were unaffected. The workload was
not restarted or altered for this diagnostic defect.

The exact executed driver is retained as `candidate-suite-api-1/driver.py`,
SHA-256 `09bec4f508d5681b42688ae30d2a7431ca8372a977635b68a0cfe88ebc389dfe`.

## Full-workload result

**70 passed, 2 failed in 571.36 seconds**, cargo exit 101. Measured workload
wall time was 571.941 seconds, excluding the 0.072-second service startup.
Both failures were in `g08_durable_package`:

- `a_packaged_audit_bounds_iterations_and_requires_native_publication`
  observed zero controller requests where twelve were expected.
- `a_packaged_audit_refuses_missing_assets_then_publishes_in_a_corrected_box`
  did not observe the expected `runner_unavailable` diagnostic.

Both recorded execution 1 as failed. The next sampled interval, about
0.5 seconds later, recorded `reserved=0`, `teardown=1`. These samples do
not establish a capacity leak. The initiating failure was not assigned to
a specific candidate invocation; nearby errors must not be attributed
solely by their timestamps.

There were 1,122 completed observed list/exec calls and six incomplete
records with unknown outcomes; their processes were gone at postcheck.
No completed record reported `runtime-timeout`, EAGAIN or ENOSPC.
All 674 observed `box list` calls exited successfully. Across these calls,
median latency was 135.22 ms, the empirical 95th percentile was 186.25 ms,
and maximum latency was 4.273732 seconds. These include Pinfold and observer
overhead and are distinct from the focused runtime-only measurements.

Service CPU increased by 5.04 seconds over the run at 100 ticks/second;
sampled peak RSS was 57,778,176 bytes. This excludes client processes and
other runtime processes. Minimum available space was 5,466,804,224 bytes
on `/tmp` and 39,224,037,376 bytes on the store filesystem. Final local and
API inventories were both empty; the source checkout remained clean.

This is not a matched throughput comparison. Earlier runs had different
failure paths and host conditions. No speedup percentage or whole-workload
no-regression conclusion follows from comparing their total times.

## Decision

Do not adopt the API-only substitution as a production fix. It bypasses
the two demonstrated storage-initialization wait paths and improves the
focused warm listing measurement, but client timeout does not remove the
server-side database lock retries. The full workload also lacks a clean pass.
Changing the transport now would weaken failure cleanup without resolving
all observed failures.

The prepared local/API comparison driver was not used for another full
baseline run after this correctness blocker was confirmed. A performance
acceptance comparison belongs after cancellation and the durable-package
failures are addressed. That comparison must include cold service startup,
normal create/exec/teardown/build latency and total throughput at unchanged
concurrency. Service-startup-during-contention and running-box preservation
under API failure remain unvalidated. No production code or spec changed.

## Evidence and preservation

Evidence base on `yard-sthlm`:
`/home/yard/validation/pinfold-89-lxc-20261012/`.
Each manifest includes logs, recorded metadata and script identities;
stores and input archives/contexts are excluded.

| Directory | Artifacts | Manifest SHA-256 |
|---|---:|---|
| `api-layer-probe-1` | 72 | `eec7bd3d71f465ce47368380be78723c00a4b023e4fa3b5ac8c09483af00d2d0` |
| `candidate-suite-api-1` | 1486 | `748a5e105565f1bf49e421fd4125ccbeba33220eccad2e1972e8df9dd31e47ed` |
| `api-cancel-probe-1` | 32 | `882d72323752675829c69384d67ec2fe8f8c74b7f8f3479abcb688248aa381af` |

Final postchecks found no process with a trial path in its arguments.
Operational Yard remained PID 965905, start ticks 89301792, executable
SHA-256 `05ae25832964703c81525e7c5f5be7a8148954517cfddf69ef9846953bf1ebb2`.
Installed Pinfold remained SHA-256
`7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`,
and Podman remained SHA-256
`6e4993e061eb88205003892252e2e13bbf49c12a6f67f519881b9b3c94e980e6`.
Active stores and unrelated services were preserved. Nothing was released,
posted to GitHub or closed; issue #89 remains unresolved.
