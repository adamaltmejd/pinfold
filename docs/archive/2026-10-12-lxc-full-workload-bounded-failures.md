# Issue 89: bounded failures in the original LXC workload

The second valid full candidate run failed: **70 passed, 2 failed,
1094.68 seconds**, cargo exit 101. It reproduced native storage stalls
beyond the candidate's 20-second inventory bound. This confirms bounded
failure under the original workload, but is not a clean compatibility
pass. Issue 89 remained open when checked after the run.

The first valid run's **69 passed, 3 failed, 828.59 seconds** remains
recorded in `2026-10-12-lxc-full-workload-first-candidate-run.md`.
Its failures were not erased by this result. All three of those tests
passed in this run; that does not establish their earlier failure causes.

## Identity and execution

Candidate commit: `70c18327c811c7e9eae92778dcb365acec5e9ca8`.
Binary SHA-256:
`939f1e901063fd7a5f1e5370d197612033cf39c797a4756f8b392ed28959f079`.
Switchyard source commit: `23e55b2d0f7d38bfd14a6ce11ba6dcc837cebf5e`.
Source tree: `d90a980bce116925c8097d5aee74cf1b18e78e63`.
The checkout remained clean.

The command was the unchanged `cargo test -p e2e --locked`, with the
original durable inputs and default test concurrency. All 72 tests ran.
No assertions, deadlines, production code or security settings changed.
The host was `yard-sthlm`, using Podman 5.4.2 and native overlay in the
original unprivileged LXC. Disposable graph/run roots, initial inventories,
identities and logs are under
`/home/yard/validation/pinfold-89-lxc-20261012/candidate-suite-cold-3/`.

This was a fresh store with the original base image's two unmapped layers
and zero containers. No mapping warm-up was performed. Private XDG state
was `/tmp/pf89-cand-cold-3`; a current maintenance stamp prevented the
global daily orphan pass. No FUSE program or overlay option was added.

The external observer sampled processes, locks, descriptor identities,
syscalls and fixture rows. An external list/exec wrapper inherited stdin,
stdout, process group and open descriptors, including Yard's host lock.
It relayed stderr while recording only its checksum, byte count, selected
error tokens and the real child's exit status. Owning `box up` stayed a
direct exec. Down/prune requester traces retained raw syscall operands,
not argument strings or read buffers. Instrumentation adds overhead;
these timings are not an uninstrumented performance benchmark.

## Actual listing timeouts

Two `box list` calls returned exit 1 with `runtime-timeout`:

| Candidate PID | Runtime PID | Candidate start ticks | Seconds | Fixture |
| --- | --- | --- | --- | --- |
| 3062859 | 3062860 | 114980660 | 20.023662 | g8-noreg |
| 3070116 | 3070117 | 114984429 | 20.023657 | g8-killed |

Both stderr records were 80 bytes, SHA-256
`9605069c175a34902759b2a46db6523a57309bcd7fd550ac41290d8e3967bb99`.
PID/start-tick ancestry and independent daemon lock identities establish
the fixture associations.

The first runtime initially waited on storage lock `fc:08:670660`, then
image lock `fc:08:670669`. Writer 3062602 held the image lock while child
3062699 performed `fchownat` amid journal waits. Immediately after the
timeout, the caller, runtime child and waiter record disappeared while
the same writer still held that image lock. This establishes waiter
removal without killing that writer.
Writer ancestry belongs to the durable-bound fixture's daemon 3033651,
independently identified through its daemon lock descriptor.

The second runtime waited on image lock `fc:08:670669`, held by writer
3069765, whose child 3069835 performed `fchownat` amid `rq_qos_wait` and
`jbd2_log_wait_commit`. Writer ancestry leads to the durable-package
fixture's daemon 3034360. Later sampling does not establish that writer's
exit status or continuous survival.

Stdout was inherited and was not counted externally per invocation.
These records do not establish empty stdout or writer success. The
earlier separate stress probe's empty-output, writer-success, live-owner
and capacity proofs remain separate evidence.

## Failed tests

`a_seat_killed_after_publishing_has_published` failed during `yard init`,
before its intended killed-seat scenario. The CLI reported the actual
`runtime-timeout` from candidate 3070116. Its native image-lock chain is
directly established above. This current startup failure does not assign
the original investigation's historical EAGAIN failure.

`a_gate_errors_start_reruns_that_gate` failed at source line 1066. Its
fixture waits for a real `box down` completion and requires exit 0. The
observed completion was exit 1. Requester trace `control-3062626.log`
belongs to this fixture's daemon 3046910. Candidate 3062639 killed its
inventory child 3062644 after approximately 20 seconds, then exited 1.
That child waited on layer lock `fc:08:670687`, held by writer 3062602,
while itself holding the storage lock that blocked other listings.
The trace retained no stderr token; attribution is supported by the
observed inventory operation, lock wait, deadline and child termination.
No socket receive EAGAIN was recorded. The failure occurred before the
test's intended listing-executable removal and later recovery assertions.

Fixture cleanup also had an exit-1 prune, `control-3073602.log`, whose
inventory child 3073606 was killed after approximately 20 seconds.
It waited on storage lock `fc:08:670660`, held by runtime 3069675 under
the durable-bound fixture's `box exec`; that holder itself had a thread
waiting on the layer lock. Down's orphan removal and prune both inventory
before removal, so these failures can occur before reaching removal.
It must not be silently counted as successful cleanup. A separate scoped
inventory after the workload succeeded and was empty.

The noreg fixture separately reported unknown ownership and retained
implementation execution 1's reservation across the first listing timeout.
Observation 1109 shows an ended candidate with `reserved=1`, `teardown=0`
and red ownership attention. Observation 1191, after that timeout, shows
`reserved=0`, `teardown=1`, no ownership attention and review execution 2
running. These rows and the identified caller belong to the same fixture.
They support retention and later recovery; they do not replace the earlier
separate probe's explicit maximum-capacity admission proof. The noreg test
ultimately passed.

## Resources, recovery and limits

There were 1,812 filesystem samples. Minimum available `/tmp` space was
4,577,701,888 bytes; minimum graph-filesystem space was 50,256,990,208 bytes.
No sampled filesystem exhaustion accompanied this run's failures.
Peak sampled IO pressure avg10 was 97.11 percent some and 91.11 percent
full. This is shared-host evidence, not an assignment of all pressure to
the workload. The prior run's disk-full and truncated-copy causes remain
qualified by its missing contemporaneous occupancy evidence.

There were 1,091 completed list/exec observer records and five incomplete
ones. Incomplete records do not establish child outcomes. Nonzero guest
exec statuses also occur in intentional negative scenarios; they are not
automatically candidate defects. No completed observed list/exec call
recorded EAGAIN or ENOSPC. Immediate nonblocking pipe read EAGAIN is
expected and is not the historical socket failure.
The final requester trace census contains 318 files, three successful
socket receives and no socket receive EAGAIN. Its only exit-1 traces are
the down and prune inventories described above.

After the run, no own fixture or candidate process remained, the disposable
inventory was empty, and source remained clean. Operational Yard retained
PID 965905, start ticks 89301792 and its original binary checksum.
Installed Pinfold retained the official release checksum. The active store
and unrelated services were preserved. No issue comment, issue closure,
release, unrelated cleanup or active-store reset occurred.

Raw observations, filtered traces, fixture observations, partial snapshots,
CLI result records, post-run checks and checksums remain in the evidence
directory. Separately copied SQLite/WAL files are not guaranteed consistent
snapshots. Sampled lock observations do not establish continuous hold
durations. Per-user shared Podman mutexes and concurrent host IO remain
confounds. This run does not recover the original uncorrelated delays.

The candidate bounds the caller's wait and exposes failure truthfully.
The unchanged caller workload still assumes success at startup and this
teardown handshake. Follow-up work must distinguish caller handling of
unavailable inventories from native image-chown contention. Preserve
fail-closed capacity ownership and the failed workload record. Increasing
deadlines, reducing concurrency or weakening assertions would not confirm
the intended behavior. Issue 89 is not resolved by these runs.
