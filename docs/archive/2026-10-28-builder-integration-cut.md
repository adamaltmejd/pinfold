# Linux builder integration cut

Date: 2026-10-28. Baseline: `ff6276cb832dfc78e4cb716857e33af1f00f12b8`.
Issue: https://github.com/adamaltmejd/pinfold/issues/230.
This is the pre-implementation cut record. It is not a current guarantee.

## Smallest runnable cut

Use one cohesive PR for the Linux image-service replacement. Its commits
are review boundaries, not independently landable features:

1. Install the reviewed bootstrap bytes through the normal verified cache,
   and consume them in a narrow builder job behind the existing VMM jail.
   Add the separately approved trusted guest role, cache ownership,
   cancellation and production proxy/audit. No arbitrary root-command RPC.
2. Finish the same build transaction through bounded OCI receipt, graph and
   label validation, a fresh offline converter, and opaque root receipt.
   Lease local bases. Publish immutable objects and the completed root
   before atomically moving the unique and latest refs together.
3. Switch Linux resolution, launch and maintenance to that transaction's
   catalog and completed roots. Remove the Podman adapter, native pull
   coupling, first-launch host export/extraction, preflight/runtime-dir and
   staging cleanup. Preserve the old store and managed disks.
4. Extend the existing guarantee scenarios and add builder isolation, with
   test-audit. Record runtime/boundary evidence and the final policy choice.

Each feature commit updates ARCHITECTURE.md. The final PR requires all
four boundaries; unused installer/catalog/job code is not a landable cut.
No live mounts, agent-control changes, automatic migration or publication
are included.

The coupling is concrete: `image/service.rs::service` selects Podman;
`image.rs::base_digest` constructs native pull commands;
`firecracker/vm.rs::prepare` resolves Podman directly, `root_disk` converts
at first launch, and `image_env` inspects Podman. Firecracker preflight
requires Podman. `clean.rs` protects box-used identities but lacks active
build-input leases. A build-only catalog would publish unlaunchable refs.

A converter-first alternative would remove host archive extraction and
first-launch conversion while retaining Podman as producer/store. It needs
explicit scope narrowing: it would not deliver neutral manifest identity,
local-base contexts or the requested catalog. A smaller structural removal
could move base refresh into the concrete image service and delete the
common service-program/CLI-shape coupling. Neither substitutes for this
replacement under the current scope.

## Artifact distribution

The preserved bootstrap is usable for local integrated evidence. Embed its
reviewed compressed and raw identities; the sidecar manifest is evidence,
never digest authority. The exact archive is 114,456,820 bytes with SHA-256
`e7dd1ab60072ee99ff387e21ac34791b1285c950f439f0d1ef0a5c54a41823e0`.
Its 1,073,741,824-byte root has SHA-256
`710189b13d814e4026e1095bd393dad99465e4042ced5dae069c59fd79cbaa80`.
Recipe identity is
`d985bd63260afaedcbaf90c3fbfc95f5d077b03cd13407137c3b4c4724ac5e0f`.

Use the existing artifact namespace and atomic install mechanism, keyed by
the archive digest. Verify the archive before extraction and the pinned
root size/hash before attachment; existence-only install markers do not
establish root identity. A normal documented offline archive cache allows
the preserved bytes to exercise the real consumer without a test flag,
environment override or replacement bootstrap. Include the builder in
artifact inventory and pinned-version cleanup.

Fresh public installation remains a distribution gate. No immutable public
asset exists. Do not invent a URL, infer identical rebuilt filesystem
bytes from recipe identity, or publish from this session. The smallest
publication plan follows `guest-kernel.yml`: explicit authorized main-only
manual dispatch, separate contents-write job, a new prerelease that is
never latest, fail rather than replace an existing tag, preserve manifest,
closure, signed metadata and checksums. Review the publisher response and
pin the emitted archive/root bytes in a follow-up. The installed CLI's
production download then uses that reviewed URL with the existing
512 MiB/five-minute contract. An offline-only candidate must report this
limitation and cannot claim fresh installation passed.

## Admission evidence

Exact runtime limits remain unapproved. The previous scratch model tested
512 MiB payloads; 8 GiB output and fifteen-minute jobs were proposals.
The measured 8 GiB cache and 3 GiB converter work disk show fit only for
the recorded trial sequence. Neither establishes arbitrary caller capacity.

Before selecting policy, record externally observed acceptance/refusal at
the header, JSON, descriptor, cumulative payload, disk and time boundaries.
Measure built-in default/documents/full and an explicit caller context.
Retain measured sizes, allocated backing bytes, timings, raw refusal and
audit records. State headroom and tradeoffs. Obtain Adam's final numeric
decision before landing; no existing download cap changes implicitly.

The receiver must reject malformed/truncated/corrupt frames and incomplete
graphs, external URLs, unexpected labels, unreferenced objects and
attestation ambiguity. No host archive extraction or guest writable-disk
inspection. Success requires a final success frame, completed conversion
and one atomic catalog generation. Interrupted work must preserve prior
refs and cannot leave a reusable partial root.

Runtime admission also needs real daemon and RUN allowed/denied controls,
no NIC/direct route, production audit failure, owner death/cancellation,
cache exclusivity/recovery, build/clean/rm races and base leases, secret
handling, agent reuse with builder absent, and old-store/disks preservation.
Check capability and ACL metadata in the converter; the trial left these
unproved. Extend guarantees 23/15/27 and add the justified isolation row.
No unit tests, mocks, test-only binary paths, retries or sleeps.

Final checks: format, locked all-target clippy with warnings denied, real
Linux/KVM and reserved exclusive Mac execution, and suite budget under
five minutes. Cross-builds and scratch evidence are not runtime gates.
Independent correctness/security and distinct lean/spec/test review must
cover the final fixes. No merge, tag, release or asset publishing here.

## Caller changes to report in the implementation PR

- Linux build egress becomes independently allowlisted, with explicit
  repeatable `--allow HOST` additions and no host registry credentials.
- Runnable manifest identity and completed build-time preparation replace
  Podman identity and first-launch conversion.
- Old-store-only refs stop resolving. Rebuild through `build` or
  `image build`; the old store and managed disks remain untouched.
- Name the final approved resource limits and stable refusal behavior.
- Apple image behavior stays as specified.
