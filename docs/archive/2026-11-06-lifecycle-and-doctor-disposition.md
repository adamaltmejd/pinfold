# Lifecycle and doctor investigation disposition

Adam authorized closing reports that the bounded investigation could not
substantiate. Issues 167, 202, 207 and 206 were closed as unconfirmed/not
reproduced, not fixed or duplicates. No control, guard or assertion changed.

## Doctor: issue 206

At source `423eacc8abc1a453e676e0ca80863ac429206395`, two real doctor
invocations used separate absent private XDG roots: runtime available and
absent from PATH. Apple Container was 1.6.1. Both exited 0 and reported the
fixture configuration. Complete project/config/state/cache snapshots were
equal; all XDG roots remained absent. Snapshots preceded cleanup. Native
API availability was checked separately. Source review found no current
writer. The historical cause remains unknown; G27 remains unchanged.

## Apple lifecycle: issues 167, 202 and 207

The recovered November 4 observer listed running containers only. Its empty
snapshots did not prove that a stopped/unstarted instance was absent. Its
raw receipts support cleanup, not late creation after completed prune.

The first November 6 observer lost a non-ready startup response; its cause
cannot be recovered from empty stderr. A receipt-corrected diagnostic then
identified an obsolete fixture tag as image-missing, before lifecycle work.
That later result is not retroactive proof of the earlier failure's cause.

A current immutable fixture was verified locally, without rebuilding:
`localhost/pinfold/profile-default:18e57c1d4022b190-58537`, image identity
`ab6b6a86fab216d13ed2caabbb95d88c2b5a9e8f32f35d97d7f2a13e99157699`.
Its default-recipe label matched the retained fixture's recipe. The
ordinary CLI came from source `75fb2f4d21d6ee454cc820dbb893e509f1c914f2`,
product-tree-identical to reviewed `21b0c5e`; SHA256:
`bea56ecc16b136ad92dd0feba38c5bc5e09a4823fb8eea2712902d88995c55c7`.

One actual scheduling attempt ran on Apple Container 1.6.1. The healthy
control started, wrote its marker and cleanly detached. After the second
startup client was paused, that native box already existed as stopped/unstarted,
with matching owner/generation. The intended pre-creation window was
missed. No target owner-death/prune phase ran. Scoped down and the single
disk removal returned 0; final native, claim and disk inventories were empty.
No repetition sought a better window. Later daemon publication is not ruled
out by that snapshot. No lifecycle product fix is justified by these data.

This does not reproduce 167's historical concurrent reattachment refusal or
identify 207's historical Pi-volume holder. Conservative liveness checks,
generation checks, claims and G11's runtime guards remain intact. The
separate source-level edges in 202's old comment were not empirically tested.

## Reopening evidence

- 206: capture the first changed owned tree, binary/runtime identity and
  complete before/after snapshots before teardown, cleanup or retry.
- 167: capture a dead/reaped writer, terminal native state, claim and
  lock/process holders when read-only attachment reports disk-in-use.
- 202: capture a matching generation appearing after completed prune;
  use all-state inventory, owner/client identity and monotonic timing.
- 207: capture the exact volume and native/process/open-file holders at
  the first in-use removal result, before another removal or cleanup.

Private raw receipts and reviewed observers are retained in the operator's
v0.5.3 preparation evidence. Closing these investigations authorizes no
publication or release-test waiver.
