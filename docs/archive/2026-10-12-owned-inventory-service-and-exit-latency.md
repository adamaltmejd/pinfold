# Owned inventory service and exit latency

Issue #89 remains unresolved. These trials support a private, Pinfold-owned
Podman inventory service as the next prototype. They do not establish that
terminating such a service at an arbitrary point is safe, or implement its
production ownership and cancellation protocol.

The measured production change is narrower: remove an unnecessary 100 ms
wait after a runtime listing closes its output pipes. It is committed as
`cdf2c41970e82e41a453c5c55164d9a9bc3982ff`.

## Evidence and artifacts

All remote paths below are relative to
`yard-sthlm:/home/yard/validation/pinfold-89-lxc-20261012/`.

- `owned-api-probe-2/`: original candidate, owned-service cancellation and
  recovery, ten alternating native/API timing pairs.
- `owned-api-timing-trace-2/`: minimal parent syscall traces of the old
  candidate. `parent-16.trace` captures the exit delay.
- `owned-api-probe-4/`: exit-observer candidate, twenty alternating old/new
  timing pairs, cancellation and recovery. `summary.json` contains timings.
- `exit-observer-native-lock-1/`: native inventory timeout, waiter removal,
  preserved owner, recovery and exec with the exit-observer candidate.
- `candidate-exit-observer/`: binary, full-commit patch, and `identity.json`.

Each trial has `result.json`, stdout/stderr and `SHA256SUMS`. The larger
service probes also have process identities, lock observations and
`events.jsonl`. Their exact driver is saved as `driver.py`.

Candidate Linux x86_64 musl binary SHA-256:

`85a75dd43f5a197be7e6df1a24e32f45f3291838f19dcef0d0c3c7c710a3d5ca`

Full-commit patch `candidate-exit-observer/pinfold89-cdf2c419.patch` SHA-256:

`650da87e27b3dbacd4271925af45df14b6ec512573f5cc3e8e74077abfd9fcc5`

Probe 4 driver SHA-256:

`41790096f848176852acf97305b96d01e2eca97e09e2b2662f2f73d29137daea`

The binary was built from parent
`23786c6fae65bb7a2eda249bf8e13c92c0301ba0` plus the exact source diff later
committed as `cdf2c419`. Its recorded pre-commit diff SHA-256 is
`68811f1cbfb415144104dee7fbc9ecbf24a0f9caa125f44187eb7423342700df`.

## Scope and preservation

The trials used disposable native-overlay graph/run roots and the original
base image, with normal caller umask `0002`. Diagnostic directories were
private and API sockets mode `0600`. Only inventory used the API; create,
exec and removal used native Podman. The diagnostic controller owned each
service process group and retained its unreaped leader until termination.

Yard PID 965905 and its start identity were unchanged. Installed Pinfold
copies remained SHA-256
`7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`.
The active store and unrelated services were preserved. No FUSE changes,
security weakening, workload concurrency reductions, deadline increases or
test assertion changes were made. Nothing was released or published.

Private graph roots still share the UID's Podman shared-memory mutex file.
Before killing a blocked service, the controller verified its database
wait and read the shared-memory records to establish that it held no mutex.
It did not acquire, reset or modify those shared locks. This gate limits
what the cancellation result proves.

## Cancellation and recovery

Probe 4 held a real SQLite exclusive transaction on its private database.
The initialized service's listing threads repeatedly failed read locks on
the same verified database inode. An independent native create waited on
the holder. A live Pinfold owner and box remained running.

- Primary listing: exit 1, empty stdout, `runtime-timeout`, 20.022 seconds.
- Controller termination and reaping of the owned service: 6.024 ms.
- Two callers started two seconds later: exit 1, empty stdout, connection
  refusal after 18.640 and 18.639 seconds, before their own deadlines.
- All sampled service threads disappeared while the database holder
  remained locked. The owner, box init and writer belonged to other groups.
- A replacement service blocked during startup. The controller terminated
  it after 20.024 seconds; kill and reap took 6.263 ms.
- Releasing the holder let the independent writer succeed. Its total
  38.755-second lifetime includes the deliberately retained lock.
- A fresh service became ready in 80.386 ms. Listing found the original
  owner generation; a real exec returned `alive`. Normal owner teardown
  succeeded, then native and API inventories were empty.

Probe 2 independently passed the same sequence with the original candidate.
Its primary listing took 20.022 seconds and service kill/reap took 6.114 ms.

The controller kills the service **after Pinfold has returned its timeout**.
This demonstrates removal and recovery, not cleanup before publishing the
failure. Likewise, blocked startup was a direct service-process trial,
not a Pinfold CLI startup-timeout implementation.

The separate native lock trial held the real image-store POSIX lock.
The new candidate returned exit 1, empty stdout and `runtime-timeout` in
20.022 seconds. No waiter remained on that lock while it was still held.
The owner survived; listing and exec succeeded after release. Normal
teardown left an empty private inventory.

## Latency defect and fix

When both listing pipes reached EOF before `try_wait` observed the child's
exit, Pinfold called a cancellable empty-fd poll with a 100 ms interval.
The trace records `try_wait` returning no status, entry into that poll,
SIGCHLD at `1791836721.815027`, and the next `wait4` at
`1791836721.915138`: a 100.111 ms gap. The displayed first poll segment was
interrupted after 95 microseconds; that is not the total delay.

The fix lazily starts a `waitid(WNOWAIT)` observer only in this EOF race.
A channel wakes the caller at child exit under the original absolute
deadline. The main thread remains the sole reaper. Failure cleanup kills
the owned process group, joins the observer, then reaps. No dependency,
global polling interval or listing deadline changed.

Twenty alternating pairs in probe 4 gave these real CLI timings:

| Operation | Old median | New median | New range |
| --- | ---: | ---: | ---: |
| Native inventory | 39.913 ms | 40.323 ms | 37.783–43.537 ms |
| API inventory | 121.355 ms | 21.676 ms | 20.263–23.995 ms |

With the new candidate, exec using native inventory had a median of
183.655 ms; exec using API inventory had a median of 164.734 ms. Both
performed the actual exec through native Podman. These are warm,
single-box measurements, not whole-workload throughput results.

The warm service started in 64.403 ms. Its resident memory rose from
49,000,448 to 53,039,104 bytes; its own CPU increased by 0.10 seconds
during the mixed warm sequence. The service was alive during native
comparisons too, so these are not paired total-resource measurements.

## Validation and diagnostic corrections

Source review found no blocking issue with the observer or reaping order.
Formatting and all-target Clippy passed for macOS and Linux musl. The Linux
release build and the real CLI checks above passed. No full Pinfold suite,
Apple runtime suite or unchanged 72-test Switchyard workload was run for
this source change. The latter is still required before a throughput claim.

The macOS build needed a writable Zig cache. Linux Clippy needed the
installed cargo-zigbuild compiler wrapper; plain cross-Clippy lacked the
musl compiler. The disposable build VM was acquired, but its bootstrap
package upgrade held the package-manager lock. The existing local cross
compiler produced the artifact instead. The VM was released to idle.

Probe 1 omitted the required listing label and stopped before contention.
Probe 3 attempted old-binary exec into a box created with the new binary's
different mounted path; the runtime correctly reported that path absent.
Both cleaned up normally. Probe 4 compares both binaries for listing and
uses the matching binary for exec. No assertion was weakened. The first
timing trace included extra syscalls and did not catch the race; the
minimal second trace did.

## Follow-up work item: owned inventory service

Admission: preserve bounded, truthful inventory failure while avoiding
per-call storage initialization. The current experiments justify a small
prototype, not production adoption.

Before integrating it, establish safe termination while the service owns
shared mutexes on a disposable host or separate UID. The unheld-mutex gate
used here cannot justify arbitrary termination on the operational UID.
Then validate one-generation ownership, startup and election races,
concurrent callers, stale sockets, and cleanup before reporting failure.
The controller must never terminate a shared external Podman service.

Acceptance also requires fail-closed scheduler capacity retention and the
unchanged original 72-test workload at default concurrency and caller
umask, with matched native/API timing and resource measurements. Preserve
live owners, writer success and recovery. Keep #89 open: natural cold
stalls beyond the deadline and the historical daemon EAGAIN remain separate
unresolved evidence requirements.
