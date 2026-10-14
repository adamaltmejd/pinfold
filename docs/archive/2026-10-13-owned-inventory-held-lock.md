# Owned inventory service: held-lock termination

Podman recovered after an initialized inventory service was killed while
holding a container mutex. This supports continuing the prototype. It does not prove
arbitrary service termination safe, or resolve issue #89.

## Environment and identity

The trial ran in the disposable labvm, using rootless Podman 5.4.2,
glibc 2.41, native overlay, and the original image
`c5fc428fad13dbabcee7880a41a2715d48d17ee15b041aad3b1c3858b126e978`.
The VM kernel was `6.12.107+deb13-cloud-amd64`. It was a VM, not Yard's LXC.
The entire UID and host were disposable; no operational UID's shared mutex
segment was used for this fault injection.

Candidate commit: `cdf2c41970e82e41a453c5c55164d9a9bc3982ff`.

Candidate binary SHA-256:
`85a75dd43f5a197be7e6df1a24e32f45f3291838f19dcef0d0c3c7c710a3d5ca`.

Original image archive SHA-256:
`08ad0be8b6e6275c58229c82517c95fb42cdede1e11b25b527f4b4fe60bf53ee`.

Evidence archive:
`yard-sthlm:/home/yard/validation/pinfold-89-owned-lock-20261013/pinfold89-owneddeath-evidence.tar`.

Archive SHA-256:
`3889c32bd7a0ef38a6fa0876771f9fcc84f78715695c987599b86b29c4aa6121`.

The archive contains the exact controller and debugger scripts, gate
observations, stdout/stderr, `evidence/result.json`, and the earlier failed
setup's evidence. Controller SHA-256:
`f0ad3f66a9a0c0b3a5559d34884a97df8442671141146847ee8ce247f62a877b`.
Debugger script SHA-256:
`3a293161fa36e83cdcf3261f5cd99b987d9ad573f5aad03e59bac29c153bda68`.

## Fault boundary

A real Pinfold owner started a live box. Native exec succeeded, then a
separate initialized Podman service returned a successful API inventory.
Only inventory used that service. Lifecycle mutations stayed native.

GDB ran as the service's parent inside an existing rootless user namespace.
The controller did not weaken ptrace settings, edit shared memory or insert
a replacement runtime. After the positive control, the debugger armed an
entry breakpoint on `pthread_mutex_unlock`, restricted to this container's
actual mutex. The lock ID came from Podman's database. The pointer was
matched to the mapped segment's device, inode and calculated offset.

The stopped service PID was 11913, owning TID 11922, lock ID 0, offset 64
in the 82,488-byte `/dev/shm/libpod_rootless_lock_1000` segment. Its robust
mutex word and owner field both identified TID 11922. Repeated read-only
snapshots matched while GDB held all service threads stopped.

The controller opened a pidfd for that verified service, sent SIGKILL,
then let GDB observe and reap its death. It never selected a process by
name or used a stale PID to terminate a replacement.

## Observations

- Service termination and debugger completion were observed in 17.022 ms.
- The mutex word changed from `11922` to `0x40000000`, the kernel's robust
  owner-death flag. The stale owner field still named TID 11922.
- The interrupted real Pinfold listing returned exit 1, empty stdout and
  connection refusal. Its observed lifetime was 0.695 seconds.
- A native Pinfold listing succeeded. Afterwards the mutex word and owner
  field were both zero, consistent with recovery and normal unlock.
- The original owner process, box init start identity and ownership-file
  hash remained intact. Real box exec again printed `alive`.
- A new native container create succeeded after recovery. A replacement
  API service returned the original live box. This writer was not queued
  across the kill; it is a post-recovery mutation check.
- Normal teardown removed the original owner claim. Both native and API
  inventories were empty. The replacement service exited normally.

These observed durations include controller waiting and are not latency
benchmarks. The debugger gate was immediately before unlock, after the
batch body. It did not interrupt a partially completed database update.
The stripped executable did not yield a complete named Go backtrace;
the mutex pointer, mapping identity, lock ID and owning TID establish the
held-lock gate directly.

Podman's [acquisition helper](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.c)
handles `EOWNERDEAD` by calling `pthread_mutex_consistent`. The observed transition and successful native
operation exercise that recovery behavior without manually acquiring or
resetting the mutex. This prevents permanent mutex abandonment in the
tested case; it does not repair arbitrary partially modified application
state.

## Setup correction and preservation

The first attempt placed the executable beside the private storage tree.
Pinfold's mount-alias checks encountered an inaccessible image directory
under that executable parent and refused startup. No box remained and no
lock fault was injected. A thread-aware trace identified the denied open.
An initial fully followed trace itself prevented `newuidmap` from using
its normal privileges; the decisive trace detached executed helpers.

Moving the candidate into a separate executable-only directory fixed the
diagnostic layout. Security checks, assertions and production code were
unchanged. The failed setup and tracing outputs were retained.

Evidence was copied out and checksummed before labvm release returned
`idle`. Yard's existing service and installed Pinfold were preserved.

## Remaining work item

Before production adoption, cover death during healthchecked-container
state updates and cold/refresh startup. Ordinary listing can call
[`HealthCheckStatus`](https://github.com/containers/podman/blob/v5.4.2/libpod/healthcheck.go),
which can [synchronize state and write the database](https://github.com/containers/podman/blob/v5.4.2/libpod/container_internal.go);
an inventory-only endpoint is not necessarily read-only internally.
The current runtime requests all containers, so foreign healthchecked
containers also matter. Existing-segment startup and fresh-segment
initialization have different failure windows.

Production ownership, concurrent startup, stale sockets and service
generation races remain unimplemented. Cleanup must finish before failure
is published, and scheduler capacity must stay reserved when runtime
state is uncertain. Whole-workload measurements are recorded separately.
