# Runtime transport rereview 4, 2026-11-16

Reviewed `8142b3d..e7fffec`; working baseline `e7fffec`. All three claims
were confirmed against the source before editing. None was rejected.
Resource-local authority is the operator's replacement for the third
rereview's global strict inventory rule. Earlier records are unchanged.
Fix commits: `83df00e` and `ee07c12`; the staging clarification accompanies
this record. No suite test, dependency, push, PR or release was added.

## 1. Decisions consult relevant claims

The claim was valid. disk::attachments read every record and then strict
global Runtime::list. An unreadable VMM identity unrelated to the requested
disk failed that inventory. clean::boxes shared the same strict inventory
with measurement. Other global retention reads could also stop clean before
independent recovery.

Disk attachment reads now use the retained ownership records directly.
A readable record pins its named disks until published retirement and owner
release, without consulting VMM identity. State-root membership resolves
absent paths instead of dropping their claims. An unresolvable path retains
the named disks conservatively. Undecodable records produce warnings during
attachment display/admission. Physical disk deletion, failed-start disk
rollback and unfilled-disk recreation require decodable records because an
undecodable record could name any disk. These checks run under disk locks.

Linux Runtime::list is advisory and reports individual bad records or
unknown identities. The duplicate list_display method is removed. Strict
target inspect, target down and durable retirement still authorize deletion.
No missing or unreadable identity is treated as proof of death.

Clean continues past builder-cache and image-retention errors, reporting
what was kept. Each selected project disk removal still calls guarded
disk::remove; a failure is reported, other candidates are attempted, and
the disk failure is returned after independent work. Undecodable ownership
keeps run records, but measurement independently counts their physical bytes.
This also changes the run-record size category to include retained records.
Socket measurement reports individual decode errors and continues.

### Inventory and deletion consumers checked

The search covered Runtime::list/inspect, list_display, attachments,
ownership::records/record_results, remove_disk and CleanPlan callers.

| Consumer | Authority after this change |
| --- | --- |
| cli::list | Advisory display; reports unknown and skips undecodable records. |
| cli::present, exec/stat/fetch | Strict target inspect on Linux; native target lookup on Apple. |
| Linux finish_start | Strict own-claim readiness; unchanged. |
| Apple up name collision, wait_until_running, down absence check, default inspect | Native container inventory; unrelated ownership files do not decide native presence. |
| disk::attachments, disk list, fresh/filled disk claim, export and project crossing | Readable records pin only named disks. Decode errors are reported; VMM identity is irrelevant. |
| disk::remove, Claimed::undo_locked, unfilled recreation | Strict record decoding, retained named attachments and disk locks. Unknown VMM liveness does not block unrelated disks. Undecodable records refuse physical deletion. |
| clean::boxes and CleanPlan::measure, doctor, dry-run | Advisory inventory and tolerant measurement. Nonterminal or unknown target rows protect their projects. |
| clean::prune_boxes/remove_orphan | Advisory candidates followed by name lock, strict target generation/down and retirement publication. Per-target failure is reported and other candidates continue. |
| clean::prune_leftover_states | Per-record target recovery continues. Record-less socket sweep still needs complete record decoding; unreadable records cannot authorize an unidentified generation's deletion. |
| clean::in_use_images | Native/advisory inventory plus readable unretired claims and Linux image leases. Decode failure preserves images and is reported without aborting independent clean work. |
| runs::retired | Strict record retention; decode failure preserves run files. Measurement still reports their bytes. |
| builder purge | Own cache lock and own strict claim/retirement; bad builder record preserves cache and is reported without stopping other cleanup. |

### Real Linux case and controls

Command, with the static musl real binary and prepared image:

```sh
python3 docs/archive/2026-11-16-runtime-transport-rereview-4/resources.py \
  target/x86_64-unknown-linux-musl/debug/pinfold \
  localhost/pinfold/image-probe-commands:latest
```

[resources.log](2026-11-16-runtime-transport-rereview-4/resources.log) retains
the observations. A real box A wrote and synced its 1 GiB disk. The observer
killed its owner and waited for actual VMM death through pidfd, then made its
VMM identity mode 000. This left A's own record readable and liveness unknown
to pinfold. GDB stopped an actual diskless builder after claim publication
and before socket preparation; the observer killed its owner at that barrier
and replaced that fixture's owner JSON with `{`. The original record named
no disks and had a durable not-started identity. No sleep supplied a barrier.

With both faults present, B started with two new disks, exec returned ONETWO,
and down exited 0. Disk list returned all three 1 GiB sizes and A's own named
attachment. Doctor and dry-run exited 0, reported both errors and showed disk
allocation matching independently summed host st_blocks within display
rounding. Actual clean exited 0 and removed a separately killed, verified-dead
C's claim, state and sockets while preserving A, the bad builder and both
remaining full-disk byte snapshots.

With only the builder JSON repaired, disk rm of A's disk refused disk-in-use
and preserved its full backing bytes. Removal of an unrelated disk succeeded
despite A's unreadable VMM identity. Corrupting only builder JSON again made
the other unrelated disk removal fail; full SHA-256, size and inode remained
identical. Repairing both faults allowed targeted down and removal of all
fixture disks. The final run exited 0; an earlier run without the explicit
own-disk refusal also passed. No new image was published by this case.

## 2. Apple failed preparation cleans up

The claim was valid. Every attempted helper error returned before
preparation_done/retire/undo, even after helper stop/removal succeeded.
The helper now distinguishes an unconfirmed leftover from an ordinary
preparation failure. A spawn error has no helper to remove. After a spawned
helper, successful removal or strict native absence allows ordinary
retirement and rollback. Otherwise a typed error preserves the claim and
disks for checked recovery. The owned child handle remains available for
wait/kill/reap cleanup. The last 64 KiB stderr tail remains memory-only and
escaped. No diagnostic file was introduced.

The spec preserves the failed-start cleanup promise and names the exception:
stop/removal cannot confirm an existing runtime is gone. Disk deletion still
obeys item 1's ownership gate. In particular, a different wholly undecodable
record can prevent physical rollback deletion even after the failed helper
is confirmed removed; it does not require preserving that retired claim.
That combined fault was inspected, not exercised.

```sh
python3 docs/archive/2026-11-16-runtime-transport-rereview-4/apple-cleanup.py \
  target/debug/pinfold localhost/pinfold/profile-default:latest
```

[apple-cleanup.log](2026-11-16-runtime-transport-rereview-4/apple-cleanup.log):
the actual preparation helper refused a generated Pi directory beneath a
seeded symlink, reporting Not a directory. The fixture used an unused
synthetic value and unreached loopback origin, never a real login. The failed
up exited 1; no owner record, state or new disk remained pinned. After fixing
the seed, an invalid native label caused failure before the main container
was created; claim and new disk were again absent. Removing only that label
let the same name and disk run real preparation/up, exec PREPARATION-CONTROL,
down and disk rm, all exit 0. No Mac image was created or removed.

## 3. Specify private pre-start staging cleanup

The claim was valid as a documentation mismatch. Existing Sockets::create
removes its own fresh staging after failed publication before any VMM can
start. The spec now states this exception. Other generation-resource deletion
still requires confirmed removal and published retirement.

```sh
python3 docs/archive/2026-11-16-runtime-transport-rereview-4/staging.py \
  target/x86_64-unknown-linux-musl/debug/pinfold \
  localhost/pinfold/image-probe-commands:latest
```

[staging.log](2026-11-16-runtime-transport-rereview-4/staging.log): GDB stopped
the real binary immediately before the no-replace rename. The observer created
a foreign publication destination with sentinel bytes. A function-return
breakpoint observed staging absent, retiring=false and identity=not-started;
the foreign bytes were unchanged. Up refused runtime-path and cleaned its
claim. Removing only the fixture collision let the same name start, exec and
down successfully. This was an exercised publication failure, not a simulated
runtime or an inspected-only crash window.

## Checks and limits

[checks.txt](2026-11-16-runtime-transport-rereview-4/checks.txt) records builds,
fmt and strict all-target clippy, all successful on macOS arm64 and Linux x64
musl. Linux used only serialized labvm commands on the retained trial lease.
Source verification and final gates include both source fix commits.

Only inspected this pass: native stop/removal or wait/pipe failures, combined
Apple preparation failure with a different undecodable owner, clean --unused
project-disk refusal, and storage power-loss durability. Item 1's corruption
and recovery cases ran on Linux; item 2 ran on Apple; item 3 ran under Linux
GDB. The full terminal/lifecycle matrix was not repeated. Earlier unrelated
unverified cases remain unverified. No added suite tests or binary hooks.

All fixture boxes/disks were removed; the Linux host had no remaining owner
records. Prepared images and the trial lease remain. Free disk at completion:
Mac Data 33 GiB, Linux 25 GiB. No push was performed.

Caller changes: bad unrelated ownership is reported without aborting healthy
admission, inventory/size reports or independent recovery; disk deletion stays
blocked by undecodable ownership. Ordinary Apple preparation failures now
clean their claims/new disks after confirmed helper removal. The private
staging clarification changes no runtime behavior or public output schema.
