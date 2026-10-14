# Podman cancellation and durable workload diagnosis

Investigation on 2026-10-12. Source review and disposable native-overlay trials.
No production binary, runtime configuration, test assertion or deadline changed.

The two previously unexplained durable failures came from an inherited file-mask
change in the experimental driver. Restoring the caller mask made both original
tests pass. The independently reproduced API cancellation blocker remains.

## Recommendation

A small Pinfold wrapper around the shared Podman API cannot cancel the work
already running inside that service. The correct reusable-service fix crosses
Podman context propagation, SQLite busy handling, and shared mutex acquisition.
It is not a one-line conversion from Query to QueryContext.

First measure a driver-only cancellation reproducer with the exact installed
driver/SQLite build, separately at statement preparation and row stepping.
Then an upstream-oriented Podman candidate can be narrowly scoped to ordinary
container inventory. A Pinfold-owned inventory-only service is a possible
process-isolation alternative, but needs lifecycle and concurrent-client design
and testing before it is simpler or faster overall.

## Current upstream still has the gap

`git ls-remote https://github.com/containers/podman.git refs/heads/main`
returned **803f524fffba7972433a4b5fb445c352a46b2b1f**. The findings below use
that immutable revision, not a moving main URL.

- [ABI ContainerList, lines 1146–1150](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/pkg/domain/infra/abi/containers.go#L1146-L1150)
  explicitly ignores its context and calls the context-free listing function.
- [Listing implementation](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/pkg/ps/ps.go#L31)
  still obtains database state and uses Container.Batch for each container.
- [AllContainers, lines 829–867](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/libpod/sqlite_state.go#L829-L867)
  still uses Query without a request context. Its database busy timeout remains
  100 seconds at lines 69–80.
- [SHM acquisition, lines 23–46 and 516–530](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/libpod/lock/shm/shm_lock.c#L23-L46)
  still reaches blocking pthread_mutex_lock without request cancellation.

This proves the relevant main-branch code path is unfixed at that revision.
It does not prove there is no proposed PR or downstream patch. Targeted public
search did not locate a fix; search absence is not the basis for the conclusion.

## SQLite: why QueryContext is insufficient by itself

Podman 5.4.2 vendors go-sqlite3 **v1.14.24**;
[current Podman vendors v1.14.52](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/vendor/modules.txt#L402-L404).

In the installed-version source,
[QueryContext](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/mattn/go-sqlite3/sqlite3_go18.go#L26-L28)
passes context into the driver. However,
[prepare, lines 1811–1828](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/mattn/go-sqlite3/sqlite3.go#L1811-L1828)
accepts context without inspecting it and synchronously calls C preparation.
In [Next, lines 2165–2195](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/mattn/go-sqlite3/sqlite3.go#L2165-L2195),
cancellation invokes sqlite3_interrupt but then waits for the C operation to
finish. The driver itself notes an interrupt race between SQLite calls.

The vendored SQLite
[pager_wait_on_lock, lines 61031–61047](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/mattn/go-sqlite3/sqlite3-binding.c#L61031-L61047)
retries the lock through its busy handler. The
[default busy callback, lines 181481–181524](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/mattn/go-sqlite3/sqlite3-binding.c#L181481-L181524)
sleeps and retries according to the configured timeout. Neither that loop nor
that callback checks the interrupt flag. Thus, for this rollback-journal lock
path, merely calling sqlite3_interrupt does not supply prompt interruption of
the retry loop. This is a source conclusion, not a measured driver-only result.

Current driver improvements do not close these two gaps:
[prepare still ignores context, lines 2057–2072](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/vendor/github.com/mattn/go-sqlite3/sqlite3.go#L2057-L2072).
Current [row cancellation, lines 2554–2584 and 2663–2699](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/vendor/github.com/mattn/go-sqlite3/sqlite3.go#L2554-L2584)
tracks the active row operation to prevent stale cancellation from interrupting
later work. Its mechanism remains sqlite3_interrupt. Current SQLite retains
the same relevant
[pager loop, lines 63628–63646](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/vendor/github.com/mattn/go-sqlite3/sqlite3-binding.c#L63628-L63646)
and [busy callback, lines 189102–189145](https://github.com/containers/podman/blob/803f524fffba7972433a4b5fb445c352a46b2b1f/vendor/github.com/mattn/go-sqlite3/sqlite3-binding.c#L189102-L189145).

SQLite documents [interrupt as connection-wide](https://sqlite.org/c3ref/interrupt.html),
and permits only [one busy handler per connection](https://sqlite.org/c3ref/busy_timeout.html).
A custom [busy handler must not modify the invoking connection](https://sqlite.org/c3ref/busy_handler.html).
Do not implement cancellation by resetting a live shared connection's handler
or interrupting it after it has been returned to the pool.

## Smallest correct upstream-oriented candidate

These are implementation proposals, not tested fixes:

1. Thread the request context through ABI listing, list helpers, container
   enumeration and every database operation reached by the supported filters
   and options. Check cancellation between containers. Stop before returning a
   partial inventory. Options that reach storage or runtime synchronization
   require their own audit; passing a context into the initial query is not
   sufficient for every API option.
2. Make database lock waiting genuinely cancellable, including preparation.
   Prefer a driver-supported per-operation cancellation-aware busy handler,
   with scoped connection ownership, bounded sleep segments and cleanup that
   cannot affect the next borrower. This likely needs go-sqlite3 work in
   addition to Podman work. Normal successful queries should incur no sleeps.
3. An alternative wholly within Podman is a separately owned read-inventory
   connection/pool with busy waiting disabled and context-aware retry of the
   entire read on SQLITE_BUSY. Discard partial results before retrying. Leave
   writer connection timeouts unchanged. This adds a database access path and
   needs careful snapshot/connection validation; do not choose it only because
   it avoids a driver patch. It also does not bound arbitrary kernel I/O.
4. Add context-aware shared-lock acquisition for inventory. Preserve the
   immediate uncontended try-lock path. On contention, use bounded native waits
   or context-aware retry without leaving a goroutine blocked in Lock after the
   caller returns. Preserve robust owner-death recovery and other lock errors.
   A successful pthread lock must remain on the acquiring OS thread through
   unlock. Failure/cancellation must balance LockOSThread/UnlockOSThread.

The existing [Go SHM wrapper](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.go#L214-L259)
and [C robust-mutex handling](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.c#L23-L49)
are the required invariants for step 4. Simply moving blocking acquisition to a
goroutine and selecting on ctx.Done recreates the abandoned-waiter problem.
Cancellation after acquisition must release the lock before returning. The
other lock backends need explicit coverage rather than a silent blocking fallback.

Avoiding container locks entirely for a freshly loaded, immutable inventory
snapshot could remove overhead, but is a separate semantics change. Podman's
listing already tolerates slightly outdated state, yet every field and filter
must be audited before concluding that Batch can simply be deleted.

## Pinfold-owned service alternative

An exclusive Pinfold inventory service process could be terminated and reaped
on deadline, eliminating its abandoned SQLite or mutex work without modifying
Podman. Only inventory would run in that process; builds, creates and container
owners would remain elsewhere. Reuse between requests could retain warm-path
speed. This is a feasible design direction, not established safe implementation.

Required work includes ownership/generation verification, private Unix socket
permissions, startup readiness and startup deadline, concurrent callers, failure
fan-out, shutdown/reaping and service replacement. A timeout of one request
would terminate other concurrent inventories in the same process; those must
fail truthfully and retain capacity, never be interpreted as an empty result.
One worker per request loses amortization; a worker pool adds lifecycle cost.
Serializing all listings could reduce throughput and is not an acceptable
unmeasured workaround. Restarting after a timeout can hit the original storage
initialization stall again.

Podman does not provide a restricted inventory-only API mode. Its
[service documentation](https://docs.podman.io/en/latest/markdown/podman-system-service.1.html#security)
states that socket access exposes all runtime functionality. Pinfold must keep
the socket outside boxes and route only its own inventory calls there; calling
this service read-only would be inaccurate. Starting the service itself still
initializes normal Podman state. Killing it during startup or a lock-owning
window requires real recovery tests, including unrelated writer completion.

This alternative is Pinfold work; proper request cancellation inside an existing
shared Podman service is upstream Podman/driver work. Neither requires weakening
isolation controls. Neither has yet demonstrated unchanged total workload speed.

## Required evidence before adoption

- Real exclusive SQLite holder: cancel during preparation and separately during
  row stepping; service lock attempts stop before holder release. Reuse the
  connection afterward and prove no late cancellation affects the next request.
- Real shared-container mutex holder: cancel while contended; no residual server
  waiter/thread remains. Holder succeeds and subsequent listing succeeds.
- Lock release racing with cancellation, concurrent inventories, canceled client
  death, service death/replacement if using owned process isolation.
- Ordinary create/exec/teardown/build latency, aggregate throughput, cold startup,
  warm inventory, CPU/RSS, and existing workload at unchanged concurrency.

The earlier live probe proves the existing API keeps attempting SQLite locks
after client death. It does not benchmark any proposal above, prove a cold stall,
or explain the historical Pinfold EAGAIN failure.


## Correction to the earlier API workload experiment

The earlier initialized-API report recorded 70/72 tests passing and left two
durable-package failures unexplained. The API diagnostic driver introduced a
material workload-environment difference: its global `os.umask(0o077)` was
inherited by Cargo, Yard and Git. Earlier full-suite drivers did not set it.
This was an instrumentation error. Private evidence permissions should not
have changed the workload's file-creation mask.

The focused command below selected the two original tests at source commit
`23e55b2d0f7d38bfd14a6ce11ba6dcc837cebf5e`, with unchanged assertions,
deadlines and default test threading. Pinfold remained candidate
`31ff6841aa78ff7b8557d271eb261846fc845d41`, binary SHA-256
`1396ed5badc3d7daa509077b928c50504a88498dcdff4febf025ceaa74f71dcd`.
Only `podman ps` used the initialized private API; other commands used the
local runtime against the same private graph/run roots.

```
cargo test -p e2e --locked --test guarantees g08_durable_package::
```

`durable-diagnostic-api-1` reproduced both failures in **180.48 seconds**.
An external observer copied bounded, sanitized fixture transcripts during
`Machine::Drop`, before the original prune and fixture deletion. It validated
the live test PID/start ticks and captured only the two known fixture roots.
No tests were edited. Both stderr transcripts were exactly 1,256 bytes with
SHA-256 `d175c3eda0bd058aefa5b220850961330e05385c95816be61f060b1670142675`,
matching the previously unexplained full-run stderr hashes. Both report
`EACCES` spawning `/opt/yard/durable-audit/launch`. The outer tool then accesses
missing `child.stdio[3]`, masking the launch failure with a second error.

Switchyard creates the image context using Git clone/checkout. Pinfold passes
that context directly to Podman. The image copies the launcher without
normalizing modes. A separate checkout probe measured identical launcher
contents under both masks, SHA-256
`611f69bb454fda76f79df78d2ca21feb0a254bd9e51517b26f8a04f85faf208a`:

| Inherited mask | Launcher mode | Parent directory mode |
|---|---|---|
| `0077` | `0700` | `0700` |
| Normal SSH caller `0002` | `0775` | `0775` |

A small native-overlay image probe copied those actual checkouts and ran the
launcher as UID 1000, with no container network, no capabilities and a read-only
root filesystem. The image destination directory was **0755 in both cases**;
the copied launcher was root-owned **0700 versus 0775**. The restrictive case
exited **126, permission denied**. The normal-mask case successfully entered
the launcher and exited **78, runner_unavailable**, as expected without the
package installation. Both probe images and containers were removed, and final
inventory was empty. This proves the COPY/execute permission mechanism. It
does not retrospectively inspect the original failed images, which normal
cleanup had already deleted.

`durable-diagnostic-api-2` restores the recorded incoming mask only on Cargo's
subprocess. The driver and capture retain private permissions. Actual image
build contexts in this run had mode **0775** for both the launcher and its
parent. This is a corrected experiment, not a Pinfold or Switchyard patch.

The corrected focused run **passed both tests in 328.66 seconds**, cargo exit
0; measured wall time was 329.025 seconds. The source remained clean and the
private API service exited normally after the tests. Both tests exercised
all their original scenarios, including the corrected-image and publication
positive controls. Both local and API final inventories were empty.

This closes the diagnostic question about the two durable failures: the
restrictive experimental mask caused the launcher permission failure, and
restoring the caller environment resolved those failures. The previous 70/72
result is not evidence that API routing caused these execution failures.
It remains a recorded outcome of that flawed experiment, not a corrected
72-test result. The entire suite has not been rerun with the corrected driver.

The longer successful focused run executes scenarios that the earlier failure
never reached. It cannot be used as a performance comparison. A separate small
COPY probe also ran in another disposable store during part of the corrected
run; timings were diagnostic only. The original test concurrency and deadlines
were unchanged.

The Podman cancellation result remains independently valid: that probe injected
a real lock into its own store and directly observed server lock attempts after
client death. The historical daemon EAGAIN remains unassigned. Neither is
explained away by this workload-permission correction.


## Retained evidence and final checks

Evidence base on `yard-sthlm`:
`/home/yard/validation/pinfold-89-lxc-20261012/`.

| Trial | Manifest artifacts | Manifest SHA-256 |
|---|---:|---|
| `durable-diagnostic-api-1` | 56 | `d129914837d7f2a950d2a287d0155e66fe410720e0978f42ba2fd4147e1a2c1f` |
| `durable-diagnostic-api-2` | 101 | `065d027e769018955eb148f60141a5c643347baefbffe40447decb1884a5f78c` |
| `durable-checkout-modes-1` | 13 | `724a52094e22a47282d54c524caee0e7bb92aa059eb8914c356f8a58807c4dd0` |

Manifests cover retained diagnostic scripts, logs and metadata, excluding
runtime graph/run stores and copied Git checkouts. The first focused run
completed all 12 observed CLI calls: four successful lists, six successful
execs and two failed execs. The corrected run completed all 38 observed calls:
14 successful lists and 24 successful execs. Neither had incomplete records,
classified timeouts, EAGAIN, ENOSPC or capture errors.

Final postchecks found no process referring to the trial paths. Operational
Yard remained PID 965905/start ticks 89301792, executable SHA-256
`05ae25832964703c81525e7c5f5be7a8148954517cfddf69ef9846953bf1ebb2`.
Both installed Pinfold paths remained SHA-256
`7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`;
Podman remained `6e4993e061eb88205003892252e2e13bbf49c12a6f67f519881b9b3c94e980e6`.
Active stores and unrelated services were preserved. Nothing was released,
published to GitHub or closed. Issue #89 remains unresolved.
