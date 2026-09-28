# Apple box loss at the proxy socket chmod

Third sighting of the loss the 2026-09-28 cleanup report lists under
Watch, and the first in a gate: Y-105/1's macOS e2e gate, 15:14 local,
`pi::a_caller_reads_the_effective_configuration_as_data`:

```
pinfold pi: container exec --user 0:0 pi-project-10e6b0519fe7-67857 chmod 666 /var/host-services/ssh-auth.sock: Error: container pi-project-10e6b0519fe7-67857 is not running
```

The replay (Y-105/2) passed the same suite.

## What the host logs show

From `/usr/bin/log show` over the window (zsh's `log` builtin shadows it):

- 15:14:13.04 launchd spawns the box's `container-runtime-linux` helper;
  its VM process is running at 13.10.
- Nothing logs the VM or helper stopping until 15:14:16.267, when
  launchd records `bootout initiated by: launchctl<-container-apiserver`
  and SIGTERMs the helper, 3.2 s after it started. No crash report, no
  jetsam.
- `container system logs` then shows only the consequence:
  `WaitHandler ... Connection interrupted`. The same line appeared once
  earlier today, at 13:14:42, also during a gate.

So the box was not seen to die before the chmod. The bootout is most
likely pinfold's own teardown after the exec failed. The exec's "not
running" is the API server's view, not the VM's.

## What did not reproduce it

A host loop starting `debian:trixie-slim` boxes with `container run -i
--rm --network none` (and `--ssh` with a listening socket), and running
`container exec --user 0:0 NAME chmod 666 /var/host-services/ssh-auth.sock`
the moment the box printed `ready`: 40 of 40 at 4 in parallel, 40 of 40
at 8 in parallel with `--ssh`. The gate ran about 26 tests at once,
builds included.

## Open

Not reproduced as a pinfold bug, so not a ticket. Candidate cause: under
load the guest's first output reaches `run`'s stdout before the API
server records the container as running. Revisit on the next sighting:
read `/usr/bin/log show` for the failing box's name within a minute of
the failure, before the log rotates, and look for the exec client's
request between the helper's spawn and the bootout.
