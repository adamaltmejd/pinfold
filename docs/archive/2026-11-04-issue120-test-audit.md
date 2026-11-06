# Issue 120: Apple nested user namespaces

Approved contract: all Apple agent boxes disable nested user namespaces.
The fixed kernel argument is `sysctl.user.max_user_namespaces=0`. Before
signals, relay setup or readiness, guest init reads
`/proc/sys/user/max_user_namespaces` and requires the trimmed value to be
exactly `0`. An unreadable or different value fails startup. The operator
accepted the compatibility cost for workloads that need nested namespaces.

The check is after subcommand dispatch: setup and other helpers keep their
existing behavior, and Firecracker boot uses its existing separate path.
The Apple guest init is compiled for Linux, so the check has no macOS
compile-time guard. No capability, dependency, pin, fixture, timer or
product option is added.

Baseline receipt: `/private/tmp/pinfold-120-baseline.jsonl`, read during
implementation. The operator identified Apple container 1.6.1. The real
CLI box reported guest kernel 6.18.15, user-namespace limit 1503, uid 501
and empty capability sets. A descendant successfully created a distinct
user namespace. Normal down and owner exit both returned 0. This proves
the original gap, not candidate behavior.

Test admission:

1. Extend guarantee 5, `nothing_can_gain_privileges`. Its existing
   credential, capability, setuid, rootfs and kernel assertions remain.
   No guarantee row or test is added.
2. The comment names omission of Apple's boot argument. With the
   readiness check retained, a nonzero kernel default refuses startup.
   Linux's existing sabotage remains omission of its zero sysctl write.
   Removing only the new readback check is not caught by the normal
   zero-state scenario. No sabotage was run.
3. Existing Apple checks observe empty capabilities and the host uid,
   which the baseline showed still permit a nested namespace. The
   extended block reads the effective kernel limit, then keeps a shell
   parent alive while its descendant attempts `unshare -U true`. It uses
   no ID-mapping option that could refuse for another reason. The same
   box successfully runs a plain child as its positive control. This
   extends the existing Linux block to both runtimes without a new fixture.
4. The approved spec supplies effective zero. The guest kernel supplies
   the proc readback and ENOSPC namespace-creation refusal; the existing
   error assertion matches its standard `No space left on device`
   diagnostic. Child exit status supplies the successful control. No
   expectation is copied from product policy logic or its argv.

Changed lines against `2f4aa95590a4898827eeb7d42d11a23602fdd054`:
test +17/-11 (28 changed lines); binary +11/-0 (11 changed lines).
The specification also resolves the Apple user-namespace open question.

Implementation was source-only. The readiness error branches are
source-reviewed; the normal-zero G5 scenario does not exercise an
unreadable sysctl. No sabotage was run. The global timing and
assertion-pruning audit remains separate.

Separate observation: the baseline also reports `NoNewPrivs: 0` for Apple
work and PID 1, while Always applied states `no_new_privs`. This change
does not address that discrepancy.

The reviewed candidate passed a real Apple CLI trial on the same kernel
and immutable image: effective limit 0, descendant `unshare -U` exit 1
with ENOSPC, ordinary child success, and Git and the supplied Pi harness
startup success. Capabilities and host uid/gid stayed unchanged; down and
owner exit were 0. Receipt: `/private/tmp/pinfold-120-candidate.jsonl`.
Compared with baseline, the trial explicitly supplied the Pi harness,
HOME `/tmp`, normal 1 GiB memory and the existing host artifact cache.
It is not the exact G5 test or universal application compatibility proof.
Candidate CLI SHA256:
`14f12611c93cc93e39245f4a36b5e7d426372ffbe208bb1cb07c89f1a0f7d00b`.
Format and locked all-target Clippy passed. The full Mac suite passed
26 tests, failed none and ignored the separate five-minute response gate,
in 102.46 seconds; this includes the exact G5 scenario. Log:
`/private/tmp/pinfold-120-mac-suite.log`. Linux CI remains a required gate. Apple documents the argument in its supported minimum
[1.6.0 command reference](https://github.com/apple/container/blob/1.6.0/docs/command-reference.md).
The [kernel parameter documentation](https://docs.kernel.org/admin-guide/kernel-parameters.html)
states that boot sysctls apply before init; invalid values only reach the
kernel log, which is why readiness checks effective state.
