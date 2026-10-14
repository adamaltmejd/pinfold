# Issue 89: supported image-mapping paths

The tested image-mapping workaround is insufficient for issue 89. The
corrected full workload finished with **68 passed, 4 failed in 1636.43
seconds**, including inventory timeouts. No production code changed.

The investigation found a supported way to avoid conversion at subsequent
keep-id container creation in a focused fresh-store diagnostic. Standalone
Buildah can ingest the base image with the same UID/GID maps that Pinfold's
keep-id containers use. Matching
the mappings on the subsequent COPY-only build preserved this benefit.
This does not require a Podman patch. The first full-workload trial exposed
a namespace permission failure during RUN instructions: **69 passed,
3 failed in 767.74 seconds**. Equivalent split mappings fixed that error,
but the corrected workload still failed under load.

This finding extends the earlier same-day storage-fix report. Its failed
build-flags-only experiment began with an unmapped base. Mapping the
initial ingestion is the missing distinction. Issue 89 is not resolved.

## Scope and identities

All experiments used disposable graph/run roots on the original
unprivileged `yard-sthlm` LXC: Podman 5.4.2, Buildah 1.39.3, native overlay
on ext4, kernel `7.0.14-19-pve`, four CPUs. No FUSE, security relaxation,
deadline increase, concurrency reduction or assertion change was used.
The active store and operational services were preserved.

Evidence base: `/home/yard/validation/pinfold-89-lxc-20261012/`.
The focused experiment is `buildah-store-map-probe-1/`; the workload trial
is `candidate-suite-mapped-1/`.

Candidate commit: `31ff6841aa78ff7b8557d271eb261846fc845d41`,
"Avoid inventory after acknowledged box teardown". Linux x86_64 musl
binary SHA-256:
`1396ed5badc3d7daa509077b928c50504a88498dcdff4febf025ceaa74f71dcd`.
Binary: `candidate-31ff6841/pinfold` under the evidence base. It includes
the bounded-inventory change. This experiment did not rebuild it.

Original base image ID:
`c5fc428fad13dbabcee7880a41a2715d48d17ee15b041aad3b1c3858b126e978`.
OCI archive SHA-256:
`08ad0be8b6e6275c58229c82517c95fb42cdede1e11b25b527f4b4fe60bf53ee`.
Podman binary SHA-256:
`6e4993e061eb88205003892252e2e13bbf49c12a6f67f519881b9b3c94e980e6`.
Buildah binary SHA-256:
`17ce870d84b45de68416990a2515e0e0a42152524211c4bf315b379cf03d374a`.

## Native shifting probe

A fresh-store debug `podman info` reproduced `Supports shifting: false`.
Its actual capability probe failed with:

```
mount_setattr .../overlay/compat3203063658/lower: operation not permitted
```

`native-shift-probe-1/debug.log`, `info.json` and `command.json` retain the
full observation. This identifies the failed probe and errno. It does not
identify the exact kernel permission branch. No feature cache in the
active store was removed and no host security setting was changed.

## Fresh-store mapping experiment

The diagnostic derived UID and GID maps independently from the actual
rootless namespace mappings, then checked them against the resulting
keep-id container. On this host each was:

```
0:1:1000
1000:0:1
1001:1001:64534
```

These are IDs relative to Podman's outer rootless namespace, not raw host
subordinate IDs. They are observed host-specific values, not constants
suitable for a production implementation.

The base was pulled from the original OCI archive using standalone
Buildah's global `--userns-uid-map` and `--userns-gid-map` flags, before
`pull`. The flags configure the store before layers are ingested. This
differs from Podman build flags that map only the working container.
[Buildah store initialization](https://github.com/containers/buildah/blob/v1.39.3/cmd/buildah/common.go#L57-L99).

Two separate stores compared an ordinary first Podman build with an
explicitly mapped first build. Both then made an ordinary derived build
and created a keep-id container from its output. The fixture used small
COPY-only layers; these timings are diagnostics, not a benchmark.

| Operation | Ordinary first build | Explicitly mapped first build |
|---|---:|---:|
| Mapped base ingestion | 2.986 s | 2.904 s |
| First build | 5.655 s | 0.483 s |
| Chown helpers sampled during first build | 2 | 0 |
| Subsequent ordinary derived build | 2.627 s | 2.914 s |
| Chown helpers sampled during derived build | 1 | 1 |
| Keep-id container creation | 0.085 s | 0.086 s |
| Chown helpers sampled during creation | 0 | 0 |

Layer metadata showed the canonical base and derived tops retained the
desired mapping. Ordinary builds created alternate unmapped layers for
their working containers. The explicitly mapped first build created no
such alternate. Subsequent keep-id creation used the mapped canonical
top without adding a converted layer. Helper absence is sampled evidence;
metadata and ownership checks supply separate observations.

Both trials passed checks of guest UID/GID, root-owned and user-owned
image files, a mode-0600 host file in a mode-0700 bind directory, guest
read/write access, and the ownership/mode of a guest-created host file.
The diagnostic containers were removed. Logs and stores remain.

Script: `pinfold89-buildah-store-map-probe.py` under the evidence base.
SHA-256: `8d22160c66a81439419f46d64762d468c428c1f07b172912e6b86bd155ab81d2`.

## Original workload treatment

Switchyard source remained at
`23e55b2d0f7d38bfd14a6ce11ba6dcc837cebf5e`, tree
`d90a980bce116925c8097d5aee74cf1b18e78e63`.
Command: `cargo test -p e2e --locked`, original 72 tests and default
concurrency. No assertion, deadline or workload source changed.

A fresh store began with the original base ingested using Buildah's global
mapping flags. Both initial layers had the desired maps. There were no
containers and no container warmup. A Podman wrapper applied those same
maps to every build. Other Podman operations retained their original
arguments apart from private graph/run roots. Pinfold used the unchanged
candidate binary and a private state directory with a current maintenance
stamp to prevent unrelated daily orphan maintenance.

This is a prototype of image-handling changes outside the binary. It is
not validation of a shipped Pinfold implementation. Sampling observers
and requester tracing also add overhead, so elapsed times cannot isolate
the performance effect from prior runs.

Both durable-package builds failed at their first RUN instruction, before
apt-get could execute. Crun reported `write file .../gid_map: Operation not
permitted` for child PIDs 4163185 and 4164554. The earlier COPY-only probe
did not exercise this namespace boundary. Its success must not be treated
as proof that mapped builds generally work. A third test failed at
`RUN echo main > /target-input`, with the same error for child PID 16334.
The immediate failure is not specific to apt or package installation.

The full command exited 101, with 768.356 seconds observed wall time and
a clean unchanged Switchyard checkout. Failed tests were:

- `a_packaged_audit_bounds_iterations_and_requires_native_publication`
- `a_packaged_audit_refuses_missing_assets_then_publishes_in_a_corrected_box`
- `a_ticket_binds_feature_inputs_repairs_and_lands_on_its_target`

Among 1,070 completed observed list/exec calls, none reported
`runtime-timeout`, EAGAIN or ENOSPC. Three incomplete records have unknown
outcomes. All 311 traced teardown-control requesters exited zero, without
receive-side EAGAIN. These are bounded observations, not proof that the
historical errors cannot recur.

The observer sampled 399 listing processes. The longest sampled age was
15.864 seconds, without a captured POSIX lock-wait chain. Seventeen of
the eighteen listings sampled at two seconds or longer lacked such a
chain; one listing reached 4.077 seconds with a captured POSIX waiter.
Removing image conversion does not establish that every remaining delay
is solved. The preserved observations need separate attribution.

Final private inventory was empty. Both remaining canonical image tops
had the expected maps, with no recorded alternate mapped layers. The
operational Yard PID/start identity and executable checksum, and installed
Pinfold checksum, matched the pre-run identities. Minimum free space was
5,502,664,704 bytes on `/tmp` and 46,847,897,600 bytes on the store
filesystem. `run-result.json`, `review-summary.json`,
`diagnostic-summary.json`, raw logs and observers remain in the trial.

## Minimal RUN comparison

After the workload finished, `run-map-probe-1/` compared the same minimal
COPY plus RUN fixture in three new private stores. The RUN printed its
identity, checked a fixture file's ownership, printed namespace mappings
and setgroups state, and wrote a completion file. All builds used
`--network=none`, no layer cache, and unchanged default OCI isolation.
No permission override or host-userns/chroot fallback was introduced.

| Treatment | Result | Elapsed |
|---|---|---:|
| Ordinary Podman load and ordinary build | exit 0 | 1.038 s |
| Mapped Buildah ingestion, explicit-map Podman build | gid_map EPERM, exit 1 | 0.809 s |
| Mapped Buildah ingestion, global-map Buildah build | gid_map EPERM, exit 1 | 0.988 s |

The positive control observed UID/GID 0 and the fixture's expected
`0:0:644` ownership/mode. Both mapped variants failed before executing the
RUN body. This reproduces the full-workload error independently of apt,
network access and concurrent workload pressure. Merely changing builder
frontends does not fix it.

Buildah's RUN path creates a user namespace using the configured mappings;
standalone Buildah does not inherently bypass this step.
[RUN namespace setup](https://github.com/containers/buildah/blob/v1.39.3/run_linux.go#L908-L931).
At this stage the exact kernel permission decision was unproven. Raw stderr,
descendant namespace/credential samples, storage metadata and build
leftover inventories are retained. No observed chown helper ran during
these short builds; this is sampled evidence only.

Script: `pinfold89-run-map-probe.py` under the evidence base.
SHA-256: `2d26df04c557018a1b0ceea1d87d8ece9a1113bac6fd56c1cbb3e8086a7fc37c`.

## Equivalent split mappings fix the RUN failure

The captured outer namespace has separate extents beginning at IDs 1 and
1000. The requested child extent `0:1:1000` crosses that boundary. Linux
requires each requested extent to fit within one parent extent, and its
map-write path returns EPERM when that translation fails.
[Kernel range translation](https://github.com/torvalds/linux/blob/v7.0/kernel/user_namespace.c#L1057-L1080).

Normal Podman container creation splits OCI mappings at these boundaries;
the examined Buildah RUN path copies supplied maps directly.
[Podman OCI setup](https://github.com/containers/podman/blob/v5.4.2/libpod/container_internal_linux.go#L500-L510),
[Buildah RUN setup](https://github.com/containers/buildah/blob/v1.39.3/run_linux.go#L908-L931).

`run-split-map-probe-1/` retained compact mappings for base ingestion and
split only the build mapping representation. The first extent became
`0:1:999` plus `999:1000:1`; the other extents stayed unchanged. Exhaustive
comparison confirmed the same ownership function for every mapped ID.
No security, isolation or network flag changed.

All three RUN cases then passed: ordinary positive control 0.454 seconds,
mapped Podman 0.711 seconds, mapped Buildah 0.657 seconds. Each observed
root UID/GID and the expected `0:0:644` fixture. This controlled change
strongly supports the parent-extent explanation for the reproduced EPERM;
it is not a traced kernel-branch observation.

Both mapped cases sampled one chown helper and added an alternate mapped
base layer. Their final canonical output retained the compact mapping.
Storage compares map arrays structurally, so equivalent split and compact
representations still differ for layer selection. Fixing RUN this way does
not eliminate the storage helper.
[Storage map matching](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/storage/store.go#L1682-L1746).

Script: `pinfold89-run-split-map-probe.py` under the evidence base.
SHA-256: `46185e6fc907f46cc430fd5a6cd15d356f45f9a05cfb2cb986e2ddc437b74ded`.

Source inspection also found direct, unsplit RUN map copying in Buildah
v1.45.1, and structural comparison in its vendored storage v1.64.1. An
upgrade alone is not a demonstrated fix for this mismatch. A narrower
runtime candidate would split only generated RUN OCI mappings against
parent ranges, leaving storage map arrays unchanged, as normal Podman
container setup already does. That candidate has not been implemented or
validated here.
[Buildah v1.45.1 RUN setup](https://github.com/podman-container-tools/buildah/blob/v1.45.1/run_linux.go#L897-L920),
[vendored storage comparison](https://github.com/podman-container-tools/buildah/blob/v1.45.1/vendor/go.podman.io/storage/store.go#L1822-L1836).

## Corrected full workload

`candidate-suite-mapped-split-1/` repeated the unchanged 72-test command in
a fresh store. Base ingestion retained compact keep-id mappings; only the
Podman build flags used the equivalent split mappings. The candidate
binary, source, assertions, deadlines and default concurrency were
unchanged. There was no container warmup.

Result: **68 passed, 4 failed in 1636.43 seconds**, cargo exit 101,
1637.665 seconds observed wall time. All three tests that failed on
`gid_map` in the first mapped run passed. The four failures were:

- `an_untracked_file_gets_no_review_and_is_named`: `yard init` failed
  because inventory exceeded the 20-second bound.
- `selected_failures_continue_without_rewriting_judgments`: observed
  red/ownership attention instead of stopped/limit.
- `claude_api_credentials_stay_on_host_and_publication_requires_proof`:
  observed red ownership attention, with capacity retained, instead of
  approval. This is not evidence of a credential disclosure.
- `a_proof_symlink_is_refused_by_name`: the forbidden symlink was correctly
  refused, but the allowed second case could not read its clone status;
  no expected decision arrived within the existing 600-second deadline.

There were 1,071 completed list/exec observer records and seven incomplete
records with unknown outcomes. Three completed records reported
`runtime-timeout`, exit 1, at 20.021–20.023 seconds: list candidates
274589 and 275251, and exec candidate 316462. Each stderr was 80 bytes,
SHA-256 `9605069c175a34902759b2a46db6523a57309bcd7fd550ac41290d8e3967bb99`.
Stdout was inherited, not counted by this observer. A further failed
teardown-control trace is separate from these list/exec records.

Of 309 teardown-control traces, 308 exited zero and one exited one.
No receive-side EAGAIN was recorded. No completed list/exec record
reported EAGAIN or ENOSPC. The historical EAGAIN remains unassigned.

Final private inventory was empty. Both remaining canonical image tops
had compact keep-id mappings; the original base also had a split-mapping
alternate. Installed Pinfold and operational Yard identities matched
before and after, and Switchyard source stayed clean. Minimum free space
was 5,489,188,864 bytes on `/tmp` and 44,652,539,904 bytes on the store
filesystem. Peak IO PSI avg10 was 95.07% some and 88.76% full. This was not
a controlled performance comparison; the result establishes failures in
the tested configuration, not a causal explanation for every elapsed-time
difference.

Bounded failure correlation found distinct wait paths:

- `g8-accept`: daemon 266165 -> candidate 274589 (start ticks
  116240333) -> runtime ps 274590. The 20.021-second timeout had no
  captured POSIX waiter. It instead waited on a nonprivate futex in
  `/dev/shm/libpod_rootless_lock_1000`, device/inode `00:52:2`, offset 64.
  The mutex owner is unproven. Execution 5 was observed ended/pass with
  `reserved=1`, `teardown=0` and ownership attention, confirming retained
  capacity at that sample.
- `g9-dirty`: daemon 275197 (start ticks 116242326) and candidate
  275251 (116242432) correspond to the 20.023-second init failure.
  Runtime ps 275252 had 33 samples, no captured POSIX waiter, and the
  same shared-mutex wait. No image/layer writer was correlated to this
  timeout in the captured evidence.
- `g14-proof`: daemon 313789 -> candidate 316462 (116263562) -> ps
  316463. At wall time 1791819347.680241, actual POSIX records show
  holder 316034 (116262389), WRITE on `fc:08:1883774` (`storage.lock`),
  and ps 316463 waiting on that identity. The holder itself waited in
  `F_SETLKW` on `layers.lock`, identity `fc:08:1883795`.
  The ultimate holder and the exact exec operation corresponding to the
  later clone-status refusal remain unproven.
- `g8-claude-api`: ownership attention and `reserved=1`, `teardown=0`
  for execution 6 were observed. No specific timeout chain was linked to
  its daemon 262581. The nearby g9 timeout must not be assigned to it.

The longest sampled listing, ps 274562 (116240323), reached 20.019 seconds
with the same shared-mutex wait and no captured POSIX waiter. A separate
ps 273777 (116238250) had 34 captured waiter samples on `storage.lock`,
held by 273545 (116238114), itself waiting on the same `layers.lock`.
The final layer holder was not identified.

The per-user shared mutex exists despite private graph/run roots. A
contemporaneous SQLite writer 273768 (116238250) held the database WRITE
lock, identity `fc:08:1883771`, while thread 273781 waited in fdatasync
against journal identity `fc:08:1887362`. It was not proven to own that
shared mutex. Image conversion alone does not explain
all observed delays. The next lock investigation must identify the shared
mutex owner and ultimate POSIX holder before choosing a fix for these
remaining paths.

## Existing stores and production integration

Changing build commands alone does not migrate an existing store. Image
ingestion reuses a layer with the same content-derived identity without
checking its UID/GID mapping. New child layers inherit their parent's
mapping. A repeated pull, a new tag or an ordinary uncached rebuild can
therefore retain the original unmapped chain.
[Layer reuse](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/image/v5/storage/storage_dest.go#L1024-L1059),
[mapping inheritance](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/storage/store.go#L1517-L1546).

The split RUN probe preserves isolation but still creates a storage
mapping variant, and the full workload failed. Do not adopt this wrapper
configuration as a production fix. A separate migration experiment could
consider a standalone Buildah build with global maps and `--squash`,
starting from an existing unmapped base in a disposable
store. Squashing includes base layers and should produce a parentless
output using the store maps if that flattened layer is newly created.
Reusing an existing identical layer retains the mapping limitation. This
is a hypothesis to validate, including preservation of a live container
using the old image.
[Buildah squash contract](https://github.com/containers/buildah/blob/v1.39.3/docs/buildah-build.1.md#L963-L968).

Automatically squashing every build is not justified. It rewrites the
whole image and changes layer sharing, cache behavior and image identity.
The current Pinfold contract promises cache use and identical-input image
identity. Standalone Buildah would also be a new declared dependency.

Cold Debian bootstrap, existing stores, arbitrary caller bases, scratch
and multistage builds, refreshed bases, and different UID/GID layouts
remain unvalidated. The workload's checked fixtures inherit the original
profile base, including the durable fixture's base-image argument. The
bounded-listing and teardown protections remain useful independently of
this image-handling approach. Historical EAGAIN and uncorrelated listing
delays are not resolved by the focused mapping result.

## Evidence retention

Each directory has an `evidence-sha256.json` manifest. Private graph/run
stores are retained but excluded from the manifests; recorded metadata,
logs, wrappers, observations and results are included. Final sampled
process inventory found no related trial runtime or fixture process.

| Evidence directory | Files | Manifest SHA-256 |
|---|---:|---|
| `native-shift-probe-1` | 3 | `a8f2377cd68cab333aa580ede42b809b7b956fef30f4d94067f277c39d8f9107` |
| `buildah-store-map-probe-1` | 126 | `997ce1405b1388916f2add670252ac8ce390fdb747a3a602c8ff3dbd8e4d88b1` |
| `candidate-suite-mapped-1` | 1626 | `2b63e162577f684177d64f898fba7b0752a53ad70d2d6bf7f73dddd57f4b39e0` |
| `run-map-probe-1` | 153 | `0a38a4dd2ef33ff874e778144159d284651f201b7c0ffbc00ed1d7cc2196af2b` |
| `run-split-map-probe-1` | 161 | `1b46bfd71948ad99d33350f1f9c3030acb86796b6df9f6e8b033a391e330d351` |
| `candidate-suite-mapped-split-1` | 1631 | `07ee0de44c7f210a89cbd6b73cd974f7646290d61ec5ce6b6ff4a43e1fd8f8ff` |

No active-store reset, runtime upgrade, production restart, release,
GitHub comment or issue closure was performed.

Workload driver scripts under the evidence base:

- `pinfold89-full-suite-mapped-1.py`, SHA-256
  `dc37feff7fda9497518c83e83b8010b59062e40aa6a896af53b5e6bf24a6b57a`.
- `pinfold89-full-suite-mapped-split-1.py`, SHA-256
  `3fcbce96aa2ee17ed18014a2b249161f6cf3d51e0fa3cb045a09809c43961ff7`.

Separate bounded triage: `split-failure-triage.md`, SHA-256
`966dd6bff1ab9c3ba32d96bc8b147eb600ec4b1fc722926a7dd44cf618a23e4e`.
