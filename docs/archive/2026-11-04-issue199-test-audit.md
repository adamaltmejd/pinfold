# Issue 199: Linux work stays OOM killable

Approved contract: Linux exec work starts at `oom_score_adj` 1000 and may
lower it to -999. Neither proc OOM score interface lets unprivileged work
or its descendants acquire score-based immunity. The exec service stays
at -1000. The VM memory limit and Apple behavior are unchanged. This is
not a general guest-liveness guarantee under arbitrary memory pressure.

Before dropping privileges, the exec service writes modern -999, then
legacy -17. Either failure fails startup before listeners are published.
The modern write sets the inherited floor. The deliberately deprecated
legacy write protects the service without lowering that floor. No new
capability, role, dependency, kernel pin or protocol is introduced.

Baseline receipt: `/private/tmp/pinfold-199-baseline-trial.jsonl`, supplied
by the operator and read during implementation. Against `4505677`, the
real CLI started a 256 MiB box. Its uid-1000 workload observed service
parent -1000 and MemTotal 234092 KiB, lowered its own score from 1000 to
-1000 successfully, read it back, and restored 1000. No memory pressure
was applied. The owner exited 0 after clean teardown. This reproduces
score-based immunity; it is not candidate validation.

Primary sources for the pinned guest kernel 6.18.57:

- [proc score writes](https://github.com/gregkh/linux/blob/v6.18.57/fs/proc/base.c#L1040-L1158):
  modern writes check the saved floor and privileged modern writes update
  it. Legacy writes check the current score and leave the floor alone.
  Legacy 15 maps to 1000; -17 maps to -1000.
- [fork inheritance](https://github.com/gregkh/linux/blob/v6.18.57/kernel/fork.c#L1607-L1608):
  both score and floor are inherited.
- [OOM selection](https://github.com/gregkh/linux/blob/v6.18.57/mm/oom_kill.c#L190-L212):
  a score of -1000 excludes a process from victim selection.
- [Mapping admission](https://github.com/gregkh/linux/blob/v6.18.57/mm/mmap.c#L514-L525):
  `MAP_NORESERVE` avoids upfront commitment under the pinned guest's default
  overcommit policy; touching pages still consumes physical memory.

Test admission:

1. Extend Linux guarantee 20 in
   `a_caller_can_tell_an_oom_kill_from_a_failure`. Preserve its existing
   cross-platform stat and startup-score assertions and Linux MemTotal
   assertion. No test or VM is added.
2. The test comment names setting the privileged modern score back to
   -1000: the modern immunity attempt then succeeds. Omitting the legacy
   write fails the service-parent check. The existing comment names the
   wrong OOM counter sabotage. No sabotage was run.
3. Startup score 1000 does not prove that work cannot undo it. A static
   ordinary workload checks allowed modern -999 and legacy 15 writes in
   the same process, then EACCES and unchanged scores for modern -1000
   and legacy -17 at the inherited floor -999.
   Its fork+exec descendant inherits -999 and repeats the controls. A
   separate single-process run lowers to -999 and touches a finite 1 GiB
   mapping in the same 256 MiB box. The host expects exit 137 and an
   increased OOM counter, then a fresh probe must succeed. No competing
   guest observer can become the victim instead.
4. The approved spec supplies startup 1000, floor -999, protected service
   -1000 and the 256 MiB limit. The pinned Linux ABI supplies EACCES,
   unchanged denied writes, legacy mapping and inheritance. The host
   fixture supplies the finite 1 GiB pressure ceiling; process status supplies exit 137;
   kernel-backed stat supplies the OOM count. Assertions do not match
   stderr prose or derive expectations from product logic.

The fixture is compiled with the existing static musl-gcc flags and
exposed through an ordinary read-only mount. A new disk cannot combine
`from` and `readonly`; no preparation box or core change was added. The
fixture uses its argv[0] for descendant exec and has no historical mode,
duplicate uid/MemTotal check or duplicate startup-1000 check. New real CLI
execs use the existing bounded-command helper with a 60-second deadline.

The independent test audit removed the easier legacy refusal at 1000;
the refusal at -999 covers the same permission check at its hardest boundary.
The legacy 15 positive control stays. The adjacent init doc comment now
states startup behavior without claiming universal victim ordering.

Changed lines against `4505677`: `box_.rs` +39/-8 and `oom.c` +113/-0,
160 test changed lines; binary +7/-6, 13 changed lines including comments.
The global timing and assertion-pruning audit remains separate.

The first candidate trial passed the permission and inheritance probe but
failed pressure: a single reserved 1 GiB mapping returned ENOMEM before any
page touches. Its receipt is retained as
`/private/tmp/pinfold-199-candidate-trial.jsonl`. Independent reviews admitted
the workload-only MAP_NORESERVE correction. No product control, guest
memory limit, capability or kernel setting changed.

The corrected real CLI candidate trial passed in the same disposable Linux
host: uid 1000, both interface refusals returned EACCES with unchanged
scores, allowed adjustments succeeded, and the descendant inherited the
floor. Pressure exited 137; the guest OOM counter rose 0 to 1; a fresh
probe succeeded. The service remained -1000 before and after; the limit
remained 268435456 bytes. Down and owner exit were 0. This trial used the
reviewed external workload with extra diagnostics, not the final Rust G20
test. Receipt: `/private/tmp/pinfold-199-candidate-pressure-fixed.jsonl`.
Baseline CLI SHA256:
`680981f9f52ec289106ca13dcc45445af4c5a124cecccf00ccbca5d0cb379da4`.
Candidate CLI SHA256:
`4b0eff7650ac9377a17a9dd60428653d0432a57b23f8b6359979d033da5f2c67`.
VM release succeeded and reported idle. Universal guest liveness and
other OOM policies are outside this evidence.

Format and locked all-target Clippy passed. Full Mac suite and ordinary
Linux CI remain separate required gates. No sabotage was run.

The first full Mac run was stopped with SIGTERM for host disk pressure;
it is incomplete, not passed. Native teardown and removal of its detached
test volume recovered space. User-authorized unused-image pruning then
reclaimed 10.22 GB without deleting persistent Pi disks. The resumed cold
Mac suite passed 26 tests, failed none, and ignored the separate real
five-minute response gate, in 297.40 seconds. Log:
`/private/tmp/pinfold-199-mac-suite-resumed.log`. Ordinary Linux CI remains
pending; the Mac result does not execute the Linux-only G20 additions.
