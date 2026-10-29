# Linux builder lifecycle: bounded design review

Date: 2026-10-29. **Proposal only.** Reviewed source is
`638934b99ba82cadb044c4be34f7963d0b8fddfe`. No code, protocol, pin,
binding spec, resource policy or public configuration changes accompany
this review. No VM, test, dependency installation or publication was used.
The runtime PR remains blocked. Numeric policy remains unapproved.

This serves caller image construction and isolation, guarantees 23 and 44,
and the observed failure of the five-minute CI budget. The objective is
fewer repeated role starts while retaining host authority and the separate
converter. It is not permission to replace the current isolation model.

## Recommendation and remaining hold

**Retain the current draft with implementation and runtime admission
blocked.** The 300-second CI budget is already binding. This evidence
does not establish a credible path to that budget through the finite
candidate below. Its added transfer/state machinery may cost more than
it saves. This review supports no implementation approval request.

Keep **one finite builder VM per CLI build, running discovery then build,
followed by a fresh converter VM when needed** as a future comparator.
It is narrower than resident service or snapshot alternatives: retain
the pinned BuildKit binaries/native frontend and disabled VMM API, use a
bounded host-authorized vsock handoff, and end every guest with its CLI
request. The converter stays separate, cache-free and without egress.

That comparator would remove one VMM start per build but requires three
explicit changes: discovery/build share a guest kernel; selected OCI
data and host-generated source policy, store bindings and Job authority
metadata enter guest-staged storage instead of a new host-written
read-only disk; and the live handoff is charged to build's existing
deadline. Guest-side sealing is not equivalent to the physical read-only
device. Any future implementation, including an experiment, would need
separate approval for those changes after evidence justifies revisiting
the hold. Cross-build reuse, merged conversion, snapshots, a daemon,
wider privileges or a budget waiver are outside this comparator.

## Current mechanism and pinned sources

The binding rules are ARCHITECTURE.md's Linux image preparation section
(lines 763–853), Always applied and VMM jail (490–544), and verified builder
installation (912–937). They require selected inputs only, independent
converter storage and egress, bounded host admission, clean shutdown and
atomic publication. The image store has no registry or resident daemon.

`builder-root/pins.json` pins BuildKit 0.34.1, source commit
`f5b59a1f2fb25a0cb7072c398cab9aed7835da4e`; `vmm.toml` pins Firecracker
1.17.0 and guest kernel 6.18.57. The producer compiles the discovery helper
against that BuildKit source/vendor closure. `builder-root/discover/main.go`
uses its native Dockerfile conversion subrequest and records resolver
replies before empty-rootfs optimization. It does not solve RUN. Each call
uses explicit local/OCI stores and source policy, native syntax, fixed
platform and anonymous authentication. It does not parse FROM itself.
The [pinned gateway interface](https://github.com/moby/buildkit/blob/f5b59a1f2fb25a0cb7072c398cab9aed7835da4e/frontend/gateway/client/client.go)
provides image-config resolution, and the
[pinned client solve code](https://github.com/moby/buildkit/blob/f5b59a1f2fb25a0cb7072c398cab9aed7835da4e/client/solve.go)
binds local mounts, stores, policy and sessions to each solve. A common
daemon does not make those per-call authorities interchangeable.

Host `image/linux/job.rs` prepares the bootstrap/init once per transaction,
then calls separate discovery, build and optional conversion VMMs. After
discovery it validates exact resolver bindings, leases selected catalog
identities, copies only those complete layouts and pins external digests.
The selected layouts, host-generated source-policy.json and Job metadata
(phase, cache mode, labels and base/store bindings) all enter the new
read-only input disk; the guest reads its authority metadata there.
Each call receives its own such disk. The guest
`init/builder.rs` reads one job, starts one temporary BuildKit daemon,
completes that phase, stops work, syncs/unmounts and exits. Host receipt
validation, VMM reaping and backing-file sync precede cache-clean marking.
An intermediate acknowledgement is not that terminal receipt.

The current jail runs `--no-api`; boot disables PCI and the guest checks
fixed devices. Firecracker 1.17's
[runtime implementation](https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/src/vmm/src/rpc_interface.rs#L727)
does have drive-update/hotplug paths; this review does not claim otherwise.
They are unavailable through Pinfold's current jail/configuration.
Enabling an API or dynamic devices would add a separate control change.

## Alternatives

| Choice | Lifecycle and likely scope | Disposition |
|---|---|---|
| Keep separate roles | Current finite discovery/build/converter VMs and physical read-only inputs. Immutable init preparation might be reused separately without merging roles, but its measured cost is insufficient. | Preserve as baseline; admission stays blocked. No preparation-only budget claim. |
| Finite two-phase builder | Fresh VM for one request, discovery then build with host-authorized input transfer; fresh converter afterward. | Smallest candidate. Removes one start per build; changes input transport and the inter-phase kernel boundary. |
| All phases in one VM | Builder cache and audited egress have already entered the guest that would convert. | Reject. Unmounting a cache or clearing proxy variables does not remove raw-device, kernel or existing connection authority. It cannot satisfy “converter never attaches that cache.” |
| Fresh VM from a clean snapshot | Separate restored VMM for each role/request; no running resident service. | More machinery than the finite candidate. Requires API, trusted memory/state/disk artifacts, compatibility and fresh identity/entropy/transport setup. No measured saving here. |
| Cross-build resident builder | Guest/BuildKit process survives multiple CLI requests and clients. | Reject for this proposal. Changes the explicit no-resident-daemon and parent-lifetime contracts and requires a new reset/admission/supervision model. |

The temporary BuildKit daemon inside a finite request is already part of
the current implementation. Keeping it through two calls of that same
request is distinct from a service resident between CLI requests.

## Finite candidate: one request, two builder phases

This is a proposed state machine, not an adopted wire format. It adds no
public command, JSON field, image label, config option or platform choice.
The shared public base/profile rule and macOS behavior stay unchanged.

1. **Claim and freeze.** Keep the current per-cache exclusive ownership
   claim and fresh generation. Freeze caller context, recipe, labels,
   cache mode, selected profile identity and local metadata. Verify the
   pinned bootstrap on use. Boot one jailed builder with only its current
   bootstrap, init, read-only discovery input and owned cache/work disk.
   Start the existing request's proxy and audit. No unrelated image layers
   or host credentials enter it.
2. **Discover.** Run the existing pinned helper against a request-local
   session on the temporary BuildKit daemon. It receives only frozen
   metadata, not all local layers. Its bounded result is a discovery
   checkpoint, not terminal end/EOF or permission to build. Finish and
   reap the helper and close its client/session before proceeding. A
   successful metadata call is not a general proof that arbitrary guest
   descendants have reset safely.
3. **Host admission and input handoff.** The host accepts exactly one
   checkpoint for its current generation and expected phase. Preserve
   exact OCI store/image binding, selected profile checks, source-policy
   pinning and leases. Missing selected identities fail; retags do not
   redirect them. Only the host authorizes continuation. Transfer only
   the selected validated layouts, host-generated source policy, exact
   store bindings and build Job metadata (phase, cache mode, labels and
   bases) through the existing bidirectional vsock connection.
4. **Seal and build.** The trusted guest service receives fixed object
   kinds, store slots, lengths and digests into a fresh per-request staging
   directory on its owned writable work storage. The policy, store
   bindings and Job authority metadata are staged there with the OCI
   objects; they no longer arrive on a physical read-only build-input
   device. No guest-chosen path, owner, link or external URL is a host
   input. Reuse the current aligned
   input accounting and candidate object/JSON/frame bounds; charge context,
   recipe, metadata and selected content together. Verify complete receipt
   before starting build. Make the staging view read-only to normal
   BuildKit/RUN consumers, then use a distinct build call/session with the
   authorized source policy and exact stores. Run the native frontend
   afresh; do not resume discovery's callback/LLB or reuse its broad
   metadata-only store map as build authority. The initial metadata disk
   remains physically attached; changing call/session options does not
   revoke it. Only selected layers are added. This sealing remains
   guest-enforced and is the explicit immutable-input mechanism change,
   covering authority metadata as well as selected OCI bytes. Source
   pinning now depends on the guest protecting and consuming the staged
   policy and store bindings. Independent host proxy and output checks
   remain, but neither proves that the guest enforced those source pins.
5. **End the builder.** Stop/reap the daemon and all work, then sync,
   before declaring OCI ready or streaming output. The host still
   validates the ordinary stream's bounded graph, hashes, platform, labels
   and supported media. Preserve the final work-stop, sync/unmount,
   terminal receipts and EOF after streaming. Kill/reap the VMM and
   complete the existing host cache barriers. Hold ownership throughout.
   No builder guest remains alive
   during conversion or available for the next CLI request.
6. **Convert and commit.** Reuse only a fully verified completed root with
   the matching manifest/recipe/bootstrap. Otherwise start the current
   fresh converter with only the host-admitted read-only OCI layout and
   fresh separate work disk, no builder cache and no relay. Preserve the
   private ONBUILD-suppressed conversion view and original published OCI
   config. Validate the opaque root bytes. Publish immutable artifacts and
   both refs in the same atomic catalog generation. Keep leases until
   publication completes.

The candidate deliberately avoids mutating a mounted input image, exposing
all local layers before discovery, adding a filesystem service or enabling
Firecracker hotplug. The
[pinned vsock transport](https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/docs/vsock.md)
can carry bidirectional bytes; it does not supply this authorization,
framing, integrity or phase-state protocol for Pinfold.

## Control preservation and explicit changes

| Concern | Required behavior in the finite candidate |
|---|---|
| Request identity | Existing fresh generation plus a host-tracked expected phase binds every checkpoint, authorization, object and authority metadata to the frozen request. Reject stale, duplicate, reordered, skipped or unsolicited phases/results. Never put request identity in OCI labels: identical images must still share an ID. Guest output cannot select a different host request or advance host authority. |
| Reset/isolation | A fresh guest begins and ends with each CLI build. No cross-request process, credentials, live connection or kernel survives. Discovery and build deliberately share a kernel and daemon; only existing persistent cache crossed that boundary before. A guest-side process reset is not claimed to recreate the former fresh-kernel boundary. |
| Privileges/devices | Keep the present jailed unprivileged host VMM and trusted rootful guest capability bound; RUN remains without SYS_ADMIN, user namespaces or insecure entitlements. No API, NIC, host bind, agent disk or credential is added. Converter uses the same trusted numeric capability bound today; its protection is separate kernel/storage/transport, not a claimed lower mask. |
| Input storage and authority | Initial context/recipe remain on the frozen read-only discovery disk. Selected OCI bytes, host-generated source policy, exact store bindings and build Job metadata (phase, cache mode, labels and bases) arrive after authorization into guest-owned storage and are sealed there. This replaces physical read-only protection for both data and authority metadata with guest enforcement. Source-policy pinning and correct consumption of those settings therefore depend on that guest protection; independent host proxy/output checks do not prove source-pin enforcement. Staging consumes existing cache/work capacity; no new disk, tmpfs RAM reservation or capacity increase is assumed. Transfer/copy costs and ENOSPC effects must be measured. |
| Egress/audit | Discovery and build retain the same exact request-local host allowlist and production audit. No recipe can widen it. No host auth/registry credentials are introduced. An audit failure during admission, transfer, waiting or build ends the whole request. Converter retains its separate socket directory and no relay. |
| Cancellation/owner loss | Stop advances no phase. Cancel all handoff and receipt workers, close transfer, kill/reap the owned VMM, join workers and finish cleanup before releasing ownership. Parent-death enforcement remains. No guest heartbeat or delayed result can resurrect a cancelled generation. Pre-commit refs stay unchanged. |
| Deadlines | Preserve finite host-controlled discovery and build phase counts and the current candidate values. Charge the newly live admission/transfer wait to build's existing deadline, beginning when the host accepts discovery, rather than leave an idle live guest unbounded. Never extend it on guest traffic. Converter retains its separate deadline. This tightens the old host-preparation scope and requires an explicit decision; it is not a numeric-policy approval. |
| Cache cleanliness | Discovery checkpoint leaves the cache dirty. Retain selective unused mutable-cache pruning on handled recipe failure and retain ordinary layer cache. Only daemon/work teardown, sync/unmount, terminal failure/end/EOF, VMM reaping and host sync permit clean marking. Partial transfer, cleanup failure, death or deadline without those barriers forces cache replacement next time. Remove staged input before a clean terminal receipt; failure to do so is not clean success. |
| Output/commit | Keep all existing bounded host OCI/root checks and the host-admission round trip before a fresh converter. Never flatten the builder's unvalidated output in place. No intermediate phase publishes refs or reusable partial roots. Catalog rename remains commit; late cancellation/audit/durability/cleanup errors cannot reverse success afterward. |
| Leases/maintenance | Hold the same cache claim and selected image leases through terminal teardown, conversion and publication. Concurrent clean/rm may neither remove leased inputs nor lend mutable cache to a second builder. Completed images still launch after cache/bootstrap removal. |

The new phase staging/state logic changes the host/init protocol, which
lives in the CLI binary copied into `init.ext4`, not the published
bootstrap. An eventual implementation changes the consumer/recipe identity
and must prove compatibility with retained 093 bootstrap bytes. Review,
publish and repin new producer/bootstrap bytes only if helper, package or
other producer inputs actually change. No new host Go dependency, service,
framework or public setting is proposed.

## What the measurements can support

The accepted two-fixture receipt recorded 25 discovery, 25 build and 22
converter VMMs. The finite candidate would use **25 combined builder plus
22 converter starts: 47 rather than 72**, assuming the same outcomes and
completed-root reuse. The two-VM successful repeats still skip conversion;
the intentional failure never converts. A resident service could amortize
more initialization but would change the rejected cross-request contract.

The measured lifetimes were discovery 81.016 s, build 132.107 s and
conversion 168.539 s, including cold setup. Warm case lifetimes alone
totaled 332.160 s. These contain required work and are not startup timings.
Neither keeping one daemon nor restoring a snapshot can be credited with
removing those entire intervals.

The current code already prepares/verifies the bootstrap and init **once
per CLI build**, sharing those prepared inputs across role VMMs. Combining
discovery/build does not remove the measured 138.495 s warm pre-first-VMM
work. As a deliberately impossible ceiling for a change confined to the
discovery/build VMM intervals and inter-VMM gaps, deleting all of them from
the observed 624.629 s would still leave **395.214 s**, holding other
intervals fixed. Real discovery and build work must remain. This is a
scope comparison on one disposable host, not a prediction or lower bound
for CI or a redesigned system. It argues against promising the budget
from this candidate alone.

No snapshot or per-build phase-sharing speedup has been measured. New
input transfer, sealing, revalidation and guest capacity pressure may
offset avoided starts. The host is not the CI runner. The existing full
CI receipt remains 688.636 s against 300 s; neither gate is waived.

## Why snapshots and resident reuse are larger decisions

The [pinned snapshot design](https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/docs/snapshotting/snapshot-support.md)
treats state, memory and matching backing disks as trusted inputs and
does not provide their cryptographic authentication. Restore also resets
vsock connections. Pinfold would need API/jail changes, artifact identity
and integrity, CPU/kernel/device compatibility, immutable memory backing
and fresh ownership/transport after restore. A clean template must precede
caller input, mutable cache, credentials and BuildKit state. Clone entropy
and identifiers need explicit treatment under the
[pinned clone guidance](https://github.com/firecracker-microvm/firecracker/blob/v1.17.0/docs/snapshotting/random-for-clones.md).
A failed restore discards that VMM. This is not the present artifact model
and is not a fallback hidden inside the finite candidate.

A cross-build guest must additionally prove that no earlier request's
process, mount, writable storage, proxy tunnel, identity, daemon session,
capability or kernel compromise affects the next. Killing RUN or clearing
environment variables cannot prove that. It also needs an owner when the
invoking CLI exits and a multi-client admission/recovery interface. Those
are direct departures from no resident daemon and per-request ownership.
They are excluded from this recommendation.

## Failure cases and proof/admission plan

Implementation remains blocked on the binding budget and lacks support
from this evidence. If future evidence justifies revisiting it, reviewers
must resolve the shared kernel, guest-staged data/authority metadata and
deadline scope before seeking separate implementation approval. Any
adopted design updates ARCHITECTURE.md in the same implementation commit. A written proposal
does not change today's guarantee or make guest-side sealing equivalent
to the current disk control.

After separate approval, keep the proof narrow and use the real CLI:

1. Extend existing G23/44 scenarios for the changed mechanism, rather than
   add a parallel test suite. Show zero-layer local config, local COPY,
   multi-stage/argument-selected inputs, external digest pinning, unchanged
   build IDs, real no-cache/profile reruns and deferred ONBUILD. Exercise
   selected-image deletion/retagging and lease-held maintenance at the
   handoff. Verify that only selected layers enter the builder.
2. Hold real operations at readiness signals and exercise cancellation,
   owner death, audit failure and production deadline expiry during
   discovery, host selection, partial input transfer, build output and
   final cleanup. Assert reason tokens, unchanged refs and no live VMM;
   show the allowed ordinary build afterward. No sleeps, retries, shorter
   production timeouts, mock runtime or test-only binary flag.
3. Exercise live descendants or open output handles, unused mutable-cache
   exhaustion, input staging ENOSPC, cleanup failure and clean-marker
   interruption. Preserve the failure/end/EOF and host backing-file
   barriers; show cache recovery without manual intervention and ordinary
   layer-cache retention where currently guaranteed.
4. Retain the fresh converter's host-validated OCI boundary, graph/media/
   hash/platform/label admission, ONBUILD suppression and filesystem
   metadata fidelity. Preserve root-reuse validation, converter exhaustion,
   concurrent leases, pre/post-commit EIO/cancellation and builder-absent
   completed-image launch. G15's sibling/shared-ID retention stays intact.
5. Review duplicate, truncated, oversized, stale, reordered and skipped
   handoff messages against the proposed state machine. Where the pinned
   ordinary producer cannot generate a hostile wire case through a user
   seam, record that execution-proof gap. Source inspection is not a live
   negative trial; do not invent a runtime stub or testing back door.
6. Compare exact source/executable identities with the accepted native
   event method on the two affected fixtures. Measure actual starts,
   transfer bytes/cost, phase/host intervals and cache headroom. Reject the
   candidate if complexity or net cost outweighs the removed start. Only
   a justified changed-source candidate proceeds to the required full
   Linux/Mac checks and unchanged CI budget. No repeated unchanged suite.

Final admission still requires resource-policy approval and the earlier
unclosed negative/capacity/lease evidence. The prototype's performance,
functional checks and changed-control proof are separate gates. Nothing
here approves runtime merge, CLI release or a final numeric allocation.

## Evidence basis

Read the current spec and consumer/guest source at the reviewed head;
the actual producer helper and pins; and the upstream versions linked
above. The accepted process-event receipt is recorded separately in
`2026-10-29-linux-kernel-phase-accounting.md`; its raw bundle SHA-256 is
`81cb626f1d2eab1026902276bd6ede5aa7a6e4d57a6d7e6b0345695f8c139495`.
The earlier full CI result and static counts are in
`2026-10-29-linux-budget-critical-path.md`.
