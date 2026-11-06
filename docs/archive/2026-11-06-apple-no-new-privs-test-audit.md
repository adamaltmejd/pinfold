# Apple work keeps no_new_privs

Always applied already requires `no_new_privs`. The real Apple baseline
and namespace-candidate receipts report `NoNewPrivs: 0` for work and PID 1
with empty capability sets. No escalation or host escape was demonstrated.
Receipts: `/private/tmp/pinfold-120-baseline.jsonl` and
`/private/tmp/pinfold-120-candidate.jsonl`. Main `423eacc` retains those
init paths; this fixes an existing contract defect.

Generic guest init sets the flag before threads or readiness. `init exec`
also sets it before restoring workload environment and executing argv:
Apple runtime exec is launched independently of PID 1. Either setter
failure exits 1 before its protected action. Both calls are compiled for
the Linux guest, including the init embedded by the Mac CLI. Helper,
Firecracker boot and builder dispatch remain separate. Linux work already
inherits the flag from its exec service; the common wrapper repeats the
idempotent setting. Capabilities, dependencies, pins and timers stay intact.

Test admission:

1. Extend G5, `nothing_can_gain_privileges`; add no test or fixture. The
   guarantees row now names the observed flag without changing the
   existing Always applied contract.
2. The comments name omitting each setter. Missing `init exec` setup makes
   Apple work report 0 despite protected PID 1; missing generic setup
   leaves PID 1 at 0 despite protected exec work. No sabotage was run.
3. Empty capabilities and the host uid did not detect the recorded defect.
   Reuse G5's existing PID-1 status read. Its existing retained shell now
   runs grep as a fork+exec descendant before unshare; the host checks
   that descendant's flag instead of adding a direct-work twin. The plain-child positive
   control and namespace-refusal reason remain. No box or CLI exec is
   added, and no existing assertion is removed.
4. The existing contract supplies expected 1. The guest kernel's proc
   status supplies each observation. Flag inheritance across fork and
   exec is the kernel ABI, not a copy of product logic. No expected value
   comes from Pinfold output, its command line or its implementation.

Changed lines against `423eacc8abc1a453e676e0ca80863ac429206395`:
test +20/-1 (21 changed lines); binary +12/-0 (12 changed lines).

Implementation was source-only. No compilation, tests, probes, CI,
commits or pushes were performed during this preparation. The setter
failure branches remain source-reviewed; normal G5 does not force a
kernel rejection. Linux CI remains a required gate. The final suite-wide
assertion and timing audit is separate.

Both independent source reviews passed. Format and locked all-target
Clippy passed. The real Apple candidate reported work-descendant and PID 1
NoNewPrivs 1, unchanged empty capabilities and host uid/gid, effective
namespace limit 0 and ENOSPC. Git and the supplied Pi harness started;
down and owner exit were 0. Receipt:
`/private/tmp/pinfold-nnp-candidate-2026-11-06.jsonl`. This trial is not
the exact G5 test or universal compatibility proof. Candidate CLI SHA256:
`bea56ecc16b136ad92dd0feba38c5bc5e09a4823fb8eea2712902d88995c55c7`.
The full Mac suite then passed 26 tests, failed none and ignored the
separate five-minute response gate, in 96.00 seconds, including exact G5.
Log: `/private/tmp/pinfold-nnp-mac-suite-2026-11-06.log`.
