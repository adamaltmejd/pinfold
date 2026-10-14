# Issue 89: original LXC evidence review and candidate validation

The original report and supporting evidence were reviewed on `yard-sthlm`
under `/home/yard/validation/pinfold-89-lxc-20261012/`. They were preserved.
New observations are in that directory's `candidate-probe-1/`. Issue 89
remains unresolved. No comments, release, installation replacement, store
reset or security change was made.

## Candidate supplied

The binary and patch were copied to `candidate-70c18327/` on that host.
Their remote SHA-256 checksums match the local artifacts:

```text
commit  70c18327c811c7e9eae92778dcb365acec5e9ca8
parent  83ea53b564ccee915d57a6d515980375ba0fe8dc
binary  939f1e901063fd7a5f1e5370d197612033cf39c797a4756f8b392ed28959f079
patch   ab6a6254ad1df54f146bec0ee4d2f8ebfecc8fc590693a81581d709cebd0bef9
```

`pinfold-linux-x86_64` is a static x86-64 musl release build using locked
dependencies and Rust 1.98.1. Its version remains 0.2.2; the checksum
distinguishes this unreleased candidate from the official release.
`pinfold-70c1832-manifest.json` records the full provenance. The build
checkout was `6f2a0189b865d4ed0838ecb16959a35d9b37c2d9`, which differs from
the candidate only in archived documentation. Both have Pinfold source
tree `d5efd03a3a0ef9adc9abeadad07a07f41f90e5e8`. The patch applies on the
named parent, not directly on the older v0.2.2 source checkout.

## Review of the original workload evidence

The report's narrow conclusions are supported. The unchanged 72-test
workload passed its cold run and failed one prepared-run scenario. Neither
run reproduced the historical 30-second listing failures. The image/chown
holder chains and the separate build/layer-lock chain are supported by
actual descriptor and POSIX lock identities. The 16.617-second maximum is
a sampled process age, not a continuously measured hold duration.

Only the base image was prepared. Rebuilt workload images differ between
runs, so suite timing cannot establish an all-images-warm improvement.
Preparing that base mapping remained useful after temporary-container
removal. This is an operator preparation observation, not a concurrency
or throughput guarantee.

The descriptor observer retained only FDs whose inode matched a current
POSIX lock. Empty descriptor arrays do not prove an absence of files,
sockets or shared mappings. It recorded no syscall operands or memory
mappings. Those omissions prevent assigning the two uncorrelated prepared
listings, 2140436 and 2140999, to a specific wait mechanism.

The early cold observer also had invalid negative process ages for about
73 seconds. Its slow-process selection could omit early uncorrelated
listings. Correcting retained ages cannot recover observations not saved.
The ten retained cold slow listings have lock chains; that is not evidence
that every slow cold listing had one.

## Real cold stall on this LXC

A separate synthetic image of 1,500,000 empty root-owned files was imported
into the existing disposable native store. A real keep-id create prepared
its mapping. There was no artificial lock holder. The active store was
not selected. Two real Pinfold boxes were ready before the writer began.
The existing fresh maintenance stamp was retained; no broad orphan prune
was run. The candidate list used a separate fresh XDG state root.

This was a deliberate control stress probe, not another run of the
original 72-test workload. It generated substantial shared IO pressure:
full IO PSI avg10 reached 90.57 percent. Its latency is not a normal
throughput measurement or proof of the historical workload's cause.

| Observation | Result |
| --- | --- |
| Real cold mapping writer | exit 0 after 241.273 seconds |
| Direct candidate listing | exit 1 after 20.033 seconds |
| Failed listing stdout | zero bytes |
| Failed listing stderr | `runtime-timeout` |
| Ownership at listing failure | both records unchanged, both owners alive |
| Writer at listing failure | still alive |
| Listing after writer completion | exit 0 in 0.247 seconds |
| Preserved live box after recovery | same generation and owner, running |

Writer 2369127 held the real image-store inode `fc:08:533780`; child
2369189 was `storage-chown-by-maps`, observed in ownership adjustment and
overlay copy-up. Direct client 2369265 spawned runtime client 2369266.
The native comparison client was 2369267, a different process.

`lock-observations.jsonl:3-96` records 94 samples of runtime client 2369266
as a POSIX WRITE waiter on that inode. `userspace-observations.jsonl:29`
independently records its FD 7 with the same identity. At line 97,
writer+20.401 seconds, the candidate processes and waiter are gone while
the writer/helper continue. They remain absent in the next 954 samples.
The writer ultimately succeeds. Owner 2365982 and its generation remain
the same in the post-stall listing. PID start ticks were checked.

The live-owner check was completed before cleanup. A separate second box
was deliberately asked to stop for the EAGAIN diagnostic below.

## Caller capacity retention and admission recovery

A standalone observational driver used the original Switchyard source's
public e2e fixture API, real Yard binary, real Pi, real boxes and existing
scripted host model. No original test, assertion or source was changed.
Its disposable project had one execution slot and two lanes. A real host
gate held the slot; the second worker waited. Once the real mapping lock
was observed, the gate was released.

The gate completed and its host effect lock was externally proved free.
Its ended execution, id 3, still had `reserved=1` and ownership attention.
The second worker had made no model request. Captured real inventory
calls show exit 1, empty stdout and `runtime-timeout`, including a
20.027-second call. This excludes a still-running host gate as the reason
capacity remained held. Further failed inventory probes remained recorded.

After storage recovered, real inventories succeeded. The gate reservation
became zero and the second real worker reached the model. These establish
fail-closed capacity retention and subsequent admission recovery.

The whole diagnostic driver did not pass. It exited 101 when its final
watch, waiting for second-worker completion, rejected an earlier expected
Y-1 ownership error event at sequence 26. Retention and admission recovery
had already been recorded. Second-worker completion was not validated;
its persisted final outcome was failed. The first landing also recorded
an ownership error. Do not claim successful completion of either workflow
or replace the failed driver result with its successful observations.

The driver's `ManuallyDrop` retained its DB and avoided broad fixture
pruning. Its own daemon was subsequently stopped through a pidfd after
verifying PID/start ticks, executable and the unique fixture daemon lock.

## Separate EAGAIN reproduction

The official v0.2.2 binary, verified by its release checksum, requested
teardown of the second diagnostic box while the same writer held storage.
Filtered syscall tracing recorded no read/receive contents, argv or
environment. Strace and its library were extracted locally to the
diagnostic directory; no system package was installed.

The requester connected to the expected generation-control socket, set
`SO_RCVTIMEO`, then `recvfrom(fd=4, count=7)` returned EAGAIN after
10.216838 seconds. The caller exited 1 after 10.255 seconds with the same
plain `Resource temporarily unavailable (os error 11)` text.

Owner 2366346's direct runtime-removal child 2372798 was a POSIX WRITE
waiter on `fc:08:533780`. A thread was in `fcntl_setlk` with syscall 72 and
command 7 (`F_SETLKW`). It remained blocked after the requester timed out.
The writer and mapping helper continued. The storage writer later succeeded
and the temporary box was removed.

This establishes a real storage-stall-to-teardown-socket-timeout EAGAIN
path on the original LXC. It does not assign the historical untagged
prepared-suite G8 error to this path or exclude a different syscall there.
That attribution still needs PID-tagged control-operation and failed-syscall
evidence when the original workload fails. No production change was made
to teardown's existing deadline or error handling.

## Shared mutexes and unresolved listing delays

Podman 5.4.2's inventory takes container locks through `ListContainerBatch`.
Its default SHM manager uses process-shared pthread mutexes, rather than
POSIX record locks. These are a plausible explanation for uncorrelated
futex waits, not a demonstrated cause of the two historical delays.

Graph/run root isolation does not isolate the per-user shared segment.
Live operational Yard descendants mapped `/dev/shm/libpod_rootless_lock_1000`,
device `00:52`, inode 2. Podman selects that segment independently of the
graph/run roots. Normal new-object allocation takes a free slot under the
segment mutex, so this sharing does not itself prove lock-ID collisions.
No copied database, SHM replacement, renumbering or lock-type change was
used. Shared allocator and host IO activity remain possible confounds.

Primary sources: [inventory](https://github.com/containers/podman/blob/v5.4.2/pkg/ps/ps.go),
[container batch](https://github.com/containers/podman/blob/v5.4.2/libpod/container_api.go),
[manager selection](https://github.com/containers/podman/blob/v5.4.2/libpod/runtime.go),
[shared mutex implementation](https://github.com/containers/podman/blob/v5.4.2/libpod/lock/shm/shm_lock.c).

`pinfold89-userspace-observer.py` is available in the new evidence directory.
It was used during this probe. It records first sightings, all FD
identities, epoll membership, syscall operands and the mapping containing
each futex address. It distinguishes anonymous/private futex observations
from addresses in the shared segment. It does not read memory, buffers,
credentials or environment, decode an unverified mutex ABI, or terminate
a process for a stack dump. These captures support a future recurrence
investigation; they do not repair missing historical observations.

## Cleanup and limits

The disposable runtime inventory is empty. Only this probe's boxes,
writer container, synthetic image and tar archive were removed. Diagnostic
metadata, source and reports remain. The active Pinfold binary is still
the official release checksum; operational Yard remains PID 965905 with
the original executable checksum. No active-store command, global prune,
service replacement or reset was performed.

The candidate's required control checks were exercised on the original
LXC. Its full 72-test workload was not run. The historical 30-second
failures, their final holders, the original G8 EAGAIN syscall and the two
uncorrelated listing causes remain unresolved. This is not release
certification and does not justify closing issue 89.
