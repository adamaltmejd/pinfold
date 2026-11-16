# Runtime transport rereview 5, 2026-11-16

Reviewed `e7fffec..6d91ebc`; working baseline `6d91ebc`. Both claims were
confirmed from the source before editing. Neither was rejected. The corrected
operator ruling replaces the fourth rereview's new-or-existing admission
exception. Earlier records are unchanged. Fix commits: `66ccf7d`, `fc90643`.
No dependency, suite test, binary hook, push, PR or release was added.

## 1. Unknown attachments refuse existing disks

The claim was valid. Tolerant disk::attachments omitted unreadable ownership
records. Writable admission did not check UNCLEAN, so an existing filled disk
could be attached without knowing whether a skipped record already held it.
The unsafe double-writable attachment was established by inspection; the old
binary was not allowed to mount the same live filesystem twice.

Disk admission now lists existing disks while holding all requested disk
locks. If any requested disk exists, the attachment snapshot requires complete
record decoding before creating, modifying or attaching a disk. A read/decode
failure refuses disk-in-use and includes the filename-bearing record error.
The prior deletion helper is renamed complete_attachments and serves this
guard too. Deletion, rollback and unfilled-disk recreation retain the same
complete-record requirement. A new-only request uses tolerant attachment
reads. Readable claims still pin only named disks until retirement; no VMM
liveness check was added to unrelated disk admission.

The spec now states the corrected existing-disk attachment/deletion rule,
the bad-record diagnostic and the new-only/reporting/recovery exceptions.
Diskless starts remain unaffected. The check also covers mixed new/existing
requests, read-only admission and up_existing/export before mutation; those
additional variants were inspected, not individually exercised this pass.

### Real-binary case and controls

```sh
# Linux, in the retained trial VM:
python3 docs/archive/2026-11-16-runtime-transport-rereview-5/claims.py \
  target/x86_64-unknown-linux-musl/debug/pinfold \
  localhost/pinfold/image-probe-commands:latest /tmp/pfrr5-before
# Apple, on the Mac:
python3 docs/archive/2026-11-16-runtime-transport-rereview-5/claims.py \
  target/debug/pinfold localhost/pinfold/profile-default:latest
```

[linux.log](2026-11-16-runtime-transport-rereview-5/linux.log) and
[mac.log](2026-11-16-runtime-transport-rereview-5/mac.log) retain the output.
A real A started writable with a seeded, filled 1 GiB disk. Before corrupting
discovery, a real guest command read DISK-CONTROL, synced, printed ACTIVE and
blocked reading its existing stdin channel. The observer made only A's owner
record mode 000. B's request to reuse that disk refused disk-in-use and named
the unreadable owner file. On Linux the entire backing file's SHA-256, size
and inode were identical before/after the refusal. On Apple the backing volume
was not hashed; its guest marker was read again unchanged.

While the record was still unreadable, the existing guest command accepted
its continuation input and returned DISK-CONTROL/STILL-ACTIVE, exit 0, on both
platforms. A's owner stayed alive. Another start using only a new disk reached
ready, exec returned NEW-ONLY-CONTROL and down succeeded. Disk list returned
both 1 GiB disk sizes and warned about the unreadable record instead of
failing. Its attachment list is advisory when a record cannot be read.

Restoring mode 0600 changed B's error from unknown attachments to the normal
named-holder conflict with live A. Repair does not release A's valid writable
attachment. After normal A down, the identical B request started, read the
original seeded marker and completed down. Both platforms exited 0 and
removed their own fixture disks.

### Operator recovery limits

For a permissions-only fault, the operator can restore the known ownership
file's readability with ordinary chmod 600, then use existing pinfold box
down NAME or box prune. Those commands still require the matching generation
and confirmed runtime removal/death; owner death or a missing identity alone
does not authorize deletion. chmod is an external permission repair, not a
pinfold recovery command.

For an irrecoverably malformed record, no existing pinfold command can repair
or force-remove it safely. A known-valid original must first be restored
externally. Without one, supported automatic recovery is unavailable. Do not
infer its attachments, generation or runtime death from its filename or a
dead owner PID, and do not delete the bad record to bypass ownership checks.

The Linux driver also exercised this limit. It started a real diskless owner,
saved its original published record, killed the owner and waited for actual
VMM death through pidfd, then replaced only that record with `{`. Targeted
down and same-name up failed decoding; prune reported the bad record and
preserved it. Restoring the saved original allowed existing box down to
perform checked retirement and recover the name. No repair command was added.

## 2. Name-lock errors protect one generation and continue

The claim was valid. leftover_socket_dirs propagated name_alive errors out
of shared CleanPlan measurement. It now reports the name/error, protects that
generation exactly as if it might be live, and continues measuring others.
No control is weakened: neither the unsafe lock nor its generation is repaired
or deleted. Other paths already handle these errors per candidate:
clean::boxes protects its project, prune catches name-lock errors, record-less
socket sweeping protects all readable generations, and run retention keeps
unknown owners. Those paths were inspected; only the measurement propagation
changed. The existing spec's tolerant measurement contract already applies.

### Real Linux case and before/after control

The same Linux driver started U, killed its owner and waited for VMM pidfd
death, then changed only its stable name lock to unsafe mode 0644. Its owner
JSON and VMM identity remained readable. Another real C was killed and its
VMM death independently observed, making C eligible for checked recovery.
No sleep supplied readiness or death authority.

The retained pre-fix real binary from 6d91ebc, run against the same fault,
exited 1 on clean --dry-run before printing measurements. Doctor printed
disk unavailable instead of a disk size category. Both named the unsafe
ownership lock. The fixed binary's doctor and dry-run exited 0, reported
the error per name and included the physical disk category (900.0 KiB).
Actual clean exited 0 and removed C's claim/state while preserving U's exact
record bytes, state, VMM identity and generation marker. The generation with
uncertain lock authority was protected, not treated as dead.

After correcting only U's lock to 0600, targeted down succeeded. The same U
name then started, exec returned LOCK-CONTROL and down succeeded. This case
ran on Linux; the common measurement path was compiled on Apple. An actual
open-time unreadable lock was not separately injected; unsafe-mode failure
was exercised and all name_alive errors share the same preservation branch.

## Checks and scope

[checks.txt](2026-11-16-runtime-transport-rereview-5/checks.txt) records
successful fmt and strict all-target clippy on macOS arm64 and Linux x64 musl.
Both real binaries were built with --locked. Linux commands used only
serialized labvm exec against the retained trial lease. All fixture boxes
and disks were removed; Linux had no remaining ownership records. No image
was built or published. Mac cases did not invoke image removal. Linux clean
ran ordinary image retention; the prepared execution image still started
afterward. The used prepared images and trial lease remain.

The full terminal/lifecycle matrix, native removal/IPC failures and storage
power-loss durability were not rerun. Earlier unrelated unverified cases
remain unverified. No crash-window durability claim is added here: guest
stdin readiness and pidfd supplied the external barriers used by these cases.
Final free disk: Mac Data 33 GiB; Linux 26 GiB.

Caller changes: unreadable ownership now refuses attachment of an existing
disk as disk-in-use, naming the record. New-only admission and advisory
reporting continue. Name-lock measurement errors now warn and preserve the
affected generation while allowing size reports and independent cleanup.
