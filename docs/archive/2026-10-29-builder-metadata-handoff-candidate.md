# Builder metadata handoff candidate, 2026-10-29

This is an uncommitted candidate based on public head
`50ca0b88d11e838e4cf27dc7e1fd8941409f2068`. It has not run in a VM.
No performance result or runtime admission follows from this source review.
The published `09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`
bootstrap is unchanged and does not contain the new helper. The authorized
private proof may build the exact producer, pin its actual bytes in the
unpushed candidate and use the ordinary verified offline archive path.
Separate exact publication approval is required before publishing or adopting
public artifact bytes. No unbuilt artifact was pinned.

## Change and authority

One finite builder VM performs native discovery and build. Conversion remains
a fresh, independent VM without cache or egress. The host no longer synthesizes
metadata-only OCI layouts for the entire local catalog. The separate discovery
boot and its completed-output receipt are removed.

The initial input contains context, recipe, aliases, opaque manifest identities
and original store IDs. It has no unconsulted image config, environment, labels
or content. The helper keeps native named-context behavior. It requests config
bytes through its stdout/stdin and trusted init relays those requests over the
existing framed connection. There is one outstanding helper query at a time.
The host validates the exact canonical store and frozen alias/manifest binding,
records the consultation before replying, and returns the exact verified config.
The returned digest is the manifest identity, not the config blob's digest.
The existing JSON, item, name, input and deadline bounds remain in use.

The lower native metadata API can only replay an already served local binding.
It cannot open a new host lookup or fall back to the removed stores. The shown
epoch path repeats the image loaded through the recorded config callback.
Unreachable ordinary stages are pruned by the native frontend. Consultations
later absent from the final LLB still count: current select_bases already
included them. The host ledger, rather than a guest claim, selects local inputs.
This is not an independent provenance attestation.

The helper exits and is reaped before init sends the external discovery receipt.
That receipt permanently closes host metadata lookup. The host admits its union
with all recorded local consultations under the previous count and JSON bounds.
It leases and validates the selected complete contexts, then constructs the
selected job and source policy privately. The new native build session receives
only those contexts and the pinned policy. Managed registry fallback is refused.

The backing device is physically read-only to Firecracker from boot. Its host
file is created and sized once, before the VMM starts. The formatter sees only a
different task-owned staging inode. Its process exits and is reaped before the
host opens that result with O_NOFOLLOW and requires a regular, nonempty,
sector-aligned file no larger than the reservation. The retained destination FD
is positioned at zero and receives exactly that many bytes, with short/growing
source rejection and cancellation/deadline checks. No later create, truncate,
rename, replacement or resize touches the backing file. Its fresh zero tail is
untouched. Identity/length checks are additional checks, not the size invariant.
The host syncs and closes its only population writer before continuation.

Init leaves the slot unopened and unmounted before host continuation. Afterward
it opens the device read-only, issues BLKFLSBUF using existing SYS_ADMIN, closes
that FD and mounts it read-only. The helper is gone and there is no healthy
earlier reader or mapping. This does not reset a compromised kernel, and a guest
acknowledgement cannot establish host completion or complete invalidation.
The selected job and policy never reside on guest-writable storage.

No midpoint marks the cache clean. Final daemon/work teardown, sync, unmount,
end/EOF, VMM reap and host backing sync remain required. Selected leases remain
owned by the host through reaping and publication. Failure while preparing the
private selected tree cannot expose its bytes on the still-blank live slot.
Outer jail, capabilities, proxy/audit, cancellation, output admission and atomic
catalog publication remain unchanged. No public flag or configuration changed.

## Reservation derivation

The reservation is derived rather than a new accepted payload limit. The
512 MiB InputBudget charges at least 4096 bytes for every copied entry, even an
empty file or directory. At most 131072 entries can be charged. Add three
uncharged structural roots: 131075 entries and at most 524300 KiB. The selected
tree uses only two such roots, so this bound is conservative.

Apply input_disk's existing formula:

`524300 + floor(524300/10) + floor(131075/4) + 16384 = 625882 KiB`.

Round up to 4 KiB: 625884 KiB, or **640905216 bytes**. The existing inode request
is actual entries plus 1024, bounded here by 132099; its entry overhead and the
existing fixed filesystem reserve are retained. The reservation arithmetic is
checked. The formatter uses the actual tree and the original flags and formula;
only its resulting file length is admitted for copying. There is no host ext4
parser or sparse/extent-copy optimization.

The selected payload budget retains the prior context/recipe baseline, then
charges selected complete layouts and final job/policy. It does not deduct a
new fixed reserve or charge the initial alias job twice. Before metadata replies,
context plus unique consulted config bytes are checked against that same input
budget. The initial alias job and discovery policy keep their existing bounds.

## Validation and remaining proof

gofmt, cargo fmt --check and git diff --check passed. The attempted Linux check was
`RUSTC_WRAPPER='' cargo check --all-targets --locked --target x86_64-unknown-linux-musl`.
It stopped in ring's build script because this Mac lacks
`x86_64-linux-musl-gcc`; it did not establish that changed Linux Rust sources
compile. No compiler workaround or dependency change was added. The Go helper
has not been compiled. No VM, suite, workflow or producer build ran.

Proposed proof stays within the existing five selected builds and G44; no test
matrix or additional product build is added by this candidate. Preserve their
success, real failure, cancellation, publication and cache-recovery observations.
Use existing recipe slots to exercise native consulted-local binding and
unconsulted-local exclusion, with native arguments/ONBUILD/empty-rootfs cases
placed only where existing fixtures permit. The repeated epoch metadata path
needs exact-helper proof, rather than inferring success from source inspection.

A narrow external runtime observer must establish the same VMM and backing
inode/length across the midpoint; mkfs touching only staging; writer closure and
sync before continuation; early read cache priming followed by quiescence and
invalidation; and failure/cancellation before continuation preventing RUN and
publication. Healthy and rejected paths both need observation. This proof must
not add a production test flag or trust a guest acknowledgement. Measure the
added copy cost and removed boot on the exact producer/consumer pair afterward.
No few-seconds or 300-second suite claim is made here.

## Exact candidate source identities

SHA-256 after formatting, before this note:

| File | SHA-256 |
|---|---|
| builder-root/discover/main.go | 28c4e5b5ed7fa0d6682031c5b33ea02c0460033227d495180b3cd9bc8a8c28fe |
| crates/pinfold/src/core/image/linux/catalog.rs | 48833562ea5586f396e2ee3e4176c75fbd272fb893a74c6ae5b086aecdf9bbda |
| crates/pinfold/src/core/image/linux/job.rs | ffe6dd85e7f5c410d422650586ea5624e9fafb1e187d88d8c005e6e9bde72abd |
| crates/pinfold/src/init/builder.rs | d2b7934f5d1c539b28036656f9c6bb4b885a77e584d9545a2997b961a25a58c8 |
| crates/pinfold/src/init/firecracker.rs | cb513cd2542a24fa2703dde00485ed7eb08aa598f730e104e1b0ed91d632243f |
| docs/ARCHITECTURE.md | 345cd8c08a90981bad6a013abf74c21309540bcc854d63e634fbc3b2de789281 |

Unchanged artifact/compiler inputs: builder.toml
`651a4445c115b3d310094bb97e703c991d373a4b597d98050ac682a5d9eeb8db`;
builder-root/pins.json
`9c3028025b6aa3ca8c31d02c3a26940ac3ddb97bfc5b5700ff6737ee0ece57ea`;
Cargo.toml
`b3cf1df072afe6d61e18534446a00603a13f191dc6ff45aec46cb77449d47f9c`;
Cargo.lock
`1b1e51b06ad1fedeac1e15cecf7e18d52f83d8fe41463e964e420ad61e71102c`;
crates/pinfold/Cargo.toml
`9a7d79683a3297241a88cffebcbc87b80f67e2ada3f5a2719a57fbda5cbd3281`.
