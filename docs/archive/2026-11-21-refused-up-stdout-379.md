# A refused `box up` that "never exits" (#379)

2026-11-21. macOS 27.2, Apple `container` 1.7.1, rustc 1.99.1. Guarantee
11's test, `disks_persist_and_attach_exclusively`, alone, through
`scripts/run-mac-e2e.sh` as the runner, against this branch's binary.

## Finding

The binary's refused path was not at fault. The refused `box up` printed
its `disk-in-use` line and exited. Its stdout pipe stayed open because the
other `box up`, the winner, held the pipe's write end, and passed it on to
its `container run`. The harness reads "still running" from stdout not
reaching EOF, so it reported a process that had already exited.

Cause: the test starts both `box up`s on two threads. On macOS, Rust std
1.99.1 makes a child's stdio pipes with `pipe()` followed by separate
`FD_CLOEXEC` calls (`library/std/src/sys/pipe/unix.rs`). A spawn on the
other thread between those calls inherits the descriptors. On Linux std
uses `pipe2(O_CLOEXEC)` and cannot leak this way.

## Reproduction, before the fix

One hang in 3 iterations (iteration 3); the loop was then stopped so the
fix could be built. The test failed after 302 s:

```
box up e2e-49079-attach-b did not end after {"box":"e2e-49079-attach-b","detail":"disk \"e2e-49079-attach\" is attached by box \"e2e-49079-attach-a\"","event":"refused","reason":"disk-in-use"}
e2e: BOX e2e-49079-attach-b DID NOT END CLEANLY: stdin EOF did not end box up within 120 s; trying box down; its roots are kept
e2e: BOX e2e-49079-attach-b DID NOT END CLEANLY: box up still running after box down; left as it is; its roots are kept
```

At 41 s, the test's processes (`ps`): the test executable 49079, the
winner `pinfold box up` 49124 (attach-a) and its `container run` 49217.
No `box up` for attach-b was listed, so it had exited.

Pipe ends (`lsof`), the loser's stdout:

```
e2e        49079  fd 9   PIPE 0x1ab3c019f1271cdb ->0x5a3be6a50e3141ae   (read end)
pinfold    49124  fd 10  PIPE 0x5a3be6a50e3141ae ->0x1ab3c019f1271cdb   (winner box up)
container  49217  fd 10  PIPE 0x5a3be6a50e3141ae ->0x1ab3c019f1271cdb   (winner's container run)
```

`sample 49124 5` (the winner): three threads, all parked. The main thread
is in `cli::up` → tokio `block_on` → `mio` `kevent`, holding its box as
expected; `tokio-rt-worker` and `box-diagnostics` idle. No stuck refused
process existed to sample.

After the failure, the test's unwinding brought the winner down; no box
or volume was left. The kept root was deleted by hand.

## Fix

The e2e harness spawns every child through one `spawn` helper that holds a
process-wide lock (`crates/e2e/src/lib.rs`), as pinfold's own
`spawn_guarded` does. The lock serialises process creation only; the two
`box up`s still race for the disk. No assertion changed.

## After the fix

The same loop, 40 iterations: 40 passed, none hung; 10 to 11 s each.
`cargo clippy -p e2e --all-targets --target x86_64-unknown-linux-musl`
is clean (zig as the C compiler), so the Linux-only rows compile.

## Not changed

`box up` passes descriptors it inherited without `FD_CLOEXEC` on to
`container run`. Here that only extended a caller's leak; a candidate
ticket, not fixed.

## Shared machine

Before the first run and after the last, `container ls -a`, `container
image ls` and `container volume ls` were identical: the user's project
box, its two volumes and every image tag untouched. Before running, the
harness's cleanup was read: disks are removed by name from the test's own
state root, and `box down` names only the test's boxes. The test's state
root also runs the automatic maintenance pass and an unfiltered `box
prune`, which on a host with an unrelated orphaned pinfold box could
remove it (#382); none existed here. The one kept root
was deleted; the build output was deleted afterwards.
