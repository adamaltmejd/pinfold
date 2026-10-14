# Issue 89: full workload and storage-fix probes

Issue 89 remains open. The acknowledged-teardown candidate completed the
unchanged original Switchyard workload with **70 passed, 2 failed in
890.72 seconds**. The image/storage contention still breaks execution.
No production implementation changed during this investigation.

Two separate native-overlay experiments found a useful distinction:

- An already initialized Podman API service returned an accurate inventory
  during a real cold ownership conversion in 0.111 seconds. A fresh local
  CLI listing against the same store waited 22.446 seconds.
- Explicitly matching the build's UID/GID maps to keep-id did not preserve
  those maps in the final image. Subsequent creation still converted
  ownership. Adding those build flags is not a solution on this runtime.

## Identity and scope

Host: original unprivileged `yard-sthlm` LXC, Podman 5.4.2, native overlay
on ext4, kernel `7.0.14-19-pve`, four CPUs. `Supports shifting: false`.
All stores and service sockets used here were disposable and private.
The active store, installed binary and operational Yard were preserved.

Candidate commit: `31ff6841aa78ff7b8557d271eb261846fc845d41`,
"Avoid inventory after acknowledged box teardown". It includes the earlier
bounded-inventory candidate. The Linux x86_64 musl release binary SHA-256:

`1396ed5badc3d7daa509077b928c50504a88498dcdff4febf025ceaa74f71dcd`

The binary is on the host at
`/home/yard/validation/pinfold-89-lxc-20261012/candidate-31ff6841/pinfold`.
The focused lifecycle validation is recorded separately in
`2026-10-12-acknowledged-teardown-test-audit.md`.

Switchyard source remained clean at
`23e55b2d0f7d38bfd14a6ce11ba6dcc837cebf5e`, tree
`d90a980bce116925c8097d5aee74cf1b18e78e63`.
The original command was `cargo test -p e2e --locked`, with original
assertions, deadlines and default concurrency. The cold store began with
the exact original base image, two unmapped layers and no containers.
There was no container warmup. The workload builds additional images.

Evidence base on the host:
`/home/yard/validation/pinfold-89-lxc-20261012/`.
The valid run is `candidate-suite-cold-5/`; raw logs, CLI result records,
lock/descriptor observations, fixture observations, control traces,
checksums and detailed `triage.md` remain there. Observers add overhead;
this is not a controlled performance comparison with prior runs.

## Invalid setup attempt retained

`candidate-suite-cold-4/` was interrupted after a diagnostic established
that the candidate binary's parent directory also contained the earlier
disposable storage tree. Pinfold scans the directory it mounts for init.
Opening a remapped layer directory there returned EACCES before container
creation. `permission-probe.trace` records the exact failed open.

Only that test harness and its identified fixture daemons were signaled.
The logs and setup-failure record were retained. The corrected run used
the identical binary in a directory containing only the binary, plus a
new cold store. No product code or assertion changed for this setup error.

## Full-workload failures

1. `a_packaged_audit_bounds_iterations_and_requires_native_publication`
   observed zero controller requests instead of twelve. Its implementation
   harness failed before the controller scenario. Candidate 3697042's
   `box exec` timed out in runtime `ps` 3697044 after 20.022507 seconds.
   It waited on `storage.lock`, identity `fc:08:809296`, held by 3697012.
   That holder was itself waiting on `layers.lock`, `fc:08:809317`.
   The ultimate layer-lock holder was not captured. This is not evidence
   of an iteration-limit bug.
2. `a_seat_killed_after_publishing_has_published` failed waiting for the
   initial planned attempt, before its publication/kill scenario.
   Candidate 3696440's `box exec` timed out after 20.021146 seconds.
   Runtime `ps` 3696441 waited on `images.lock`, `fc:08:809305`, held by
   3696393. Its direct child 3696411 repeatedly executed `fchownat` with
   journal/IO waits. This observer did not record the child's argv role.
   The waiter disappeared after timeout while the same writer continued.
   The implementation ended failed; its reservation remained until
   teardown, then was released. The test's existing 600-second event
   deadline produced the final failure.

Three further captured `box list` timeouts belonged to `g8-gate-error`,
which ultimately passed. All five captured timeouts returned exit 1 and
`runtime-timeout` after approximately 20.02 seconds. Each stderr was
80 bytes, SHA-256
`9605069c175a34902759b2a46db6523a57309bcd7fd550ac41290d8e3967bb99`.
Stdout was inherited rather than counted by this observer. These records
do not replace the separate empty-stdout and live-owner preservation proof.

All 321 traced teardown-control requesters exited successfully. There was
no receive-side EAGAIN; 1,285 immediate read EAGAIN records were polling.
No recorded traced call exceeded eight seconds. This run does not resolve
the historical EAGAIN report or prove its exact original syscall.

The observer captured 1,127 completed CLI records and four incomplete
records; the latter have unknown outcomes. Across 1,608 resource samples,
temporary space remained at least 5,540,372,480 bytes and store-filesystem
space at least 51,576,913,920 bytes. Peak IO PSI avg10 was 84.96% some and
77.65% full. No captured CLI record reported ENOSPC.

One long listing, 3714319 in passing `g9-edit`, had no captured POSIX
waiter and reached sampled age 19.923 seconds. Its main thread slept in
`clock_nanosleep`; concurrent actual SQLite write-lock holders waited in
journal `fdatasync`. This supports a database-contention hypothesis, but
without a userspace stack the exact sleep cause is unproven. Several
shorter delays also lacked a proven POSIX wait chain. Avoiding storage
initialization does not establish that these delays disappear.

## Initialized-service comparison

`api-probe-1/` used a new native store, a private Unix socket and a Podman
service process started before the writer. No system/user service was
installed. A real stopped container with a unique label supplied the
nonempty inventory control. A separate cold image contained 250,000 empty
root-owned files and had no mapped layer before creation.

The probe started both listings only after the real keep-id creator held
the image-store WRITE lock. It observed the local listing's actual POSIX
wait record and the creator's `storage-chown-by-maps` descendant. A fresh
snapshot after the API call exited still showed the writer holding the
same lock.

| Operation | Seconds | Exit |
| --- | ---: | ---: |
| Cold keep-id create | 22.794 | 0 |
| Contended fresh CLI inventory | 22.446 | 0 |
| Contended initialized API inventory | 0.111 | 0 |
| Subsequent CLI inventory | 0.067 | 0 |
| Subsequent API inventory | 0.064 | 0 |

Both contended inventories preserved the retained container's exact ID and
label. After writer completion, both paths saw the newly created container.
The service was stopped and both exact probe containers removed; final
inventory was empty and no launched process remained.

This demonstrates bypassing repeated storage initialization for this
inventory path. It does not validate Pinfold exec, cancellation, service
restart, startup during contention, running-box ownership or the entire
workload. It does not remove the ownership conversion or the locks needed
by storage mutations.

The source agrees: the API server injects its existing runtime into the
request, and the list handler calls ContainerList on it.
[Server context](https://github.com/containers/podman/blob/v5.4.2/pkg/api/server/server.go#L93-L98),
[list handler](https://github.com/containers/podman/blob/v5.4.2/pkg/api/handlers/libpod/containers.go#L101-L122).

## Explicit build-mapping comparison

`mapping-build-probe-1/` compared two separate fresh native stores with
the same original base image and a small COPY-only Containerfile.
The explicit maps were derived from the actual rootless namespace and
checked against the subsequent real keep-id container maps. Both trials
verified guest root/user file ownership, access to a host 0600 file inside
a 0700 bind directory, and ownership/mode of a guest-created host file.
Both exact containers were removed afterward; evidence and stores remain.

In both trials the final image's top layer had no UID/GID map and no mapped
layer. Creating the output with keep-id added a mapped layer and ran an
observed ownership-conversion helper. The explicit build also ran two
such helpers while building. This is a mapping result, not a benchmark:
the fixture is small and does not exercise RUN instructions.

The result is consistent with Buildah committing through containers/image,
whose storage destination calls PutLayer without the build's mapping.
[Commit path](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/buildah/commit.go#L382-L427),
[storage destination](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/image/v5/storage/storage_dest.go#L1250-L1270).
The experiment establishes the final mapping loss, not the unique internal
line responsible for it.

## Recommended next work

The smallest measured next experiment is real Pinfold inventory and exec
through a private initialized Podman service, with the existing lifecycle,
cancellation, owner-generation and capacity guarantees tested under cold
contention. A resident runtime is a design change and must not be adopted
from this listing-only result. SQLite and mutation contention remain.

The underlying storage work should investigate preserving the requested
mapping through image commit and narrowing global locking around cold
mapping preparation. Pinfold must not edit Podman storage metadata to
simulate either change. Exact-image warming merely moves the same locked
work; it does not solve concurrent builds. Native idmapped overlay would
avoid recursive ownership conversion, but this host reports it unavailable.
Permissions and storage configuration outside the disposable probe
directories were not changed.

Upstream PR 28393 was still open when checked. It concerns propagating
mappings through pull, and does not establish a fix for locally built
images. [Upstream proposal](https://github.com/podman-container-tools/podman/pull/28393).

No FUSE change, weaker control, concurrency reduction, deadline increase,
assertion change, active-store reset, issue comment, closure or release
occurred. The failed runs remain evidence; issue 89 is unresolved.

## Final verification and checksums

All four scoped stores had empty inventories afterward. No matching
candidate or probe process remained. Operational Yard retained PID 965905,
start ticks 89301792 and SHA-256
`05ae25832964703c81525e7c5f5be7a8148954517cfddf69ef9846953bf1ebb2`.
Installed Pinfold retained SHA-256
`7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`.

Paths below are relative to the remote evidence base; values are SHA-256.
Manifests cover diagnostic artifacts, not entire container storage trees.

| Artifact | SHA-256 |
| --- | --- |
| `candidate-suite-cold-5/evidence-manifest.json` (1,691 files) | `6162bbd6bfc3a9de1a5cfc9b5ca1950f32e4308b1b5603f88473894b455e6e2a` |
| `api-probe-1/result.json` | `655a67736d09fca81e75e87166a9bc819736e801c0b664f0fafcf6244695332a` |
| `api-probe-1/evidence-sha256.json` | `3fac8cbb91b544e5bcd5270131411a7303297737454eea0c8dc81143cc475679` |
| `pinfold89-api-probe.py` | `435723551b5ef50385bd2bb98e6c5bfcfd8e2a669501a439ff001d433313abdd` |
| `mapping-build-probe-1/result.json` | `24c418700cd6c7c2e596a885507037e2fed3ff9fe676b9d1493e06ff08ffe0d4` |
| `mapping-build-probe-1/evidence-manifest.json` (90 files) | `777ae688b140d7548b09dda869506fbe81a091f5d44ef3b60a0ead100500a786` |
| `pinfold89-mapping-build-probe.py` | `3809d58a878759b283e65c57005d37c1bbdbcb8b14bd86518bc167dc6bab32af` |
| `verification-after-root-probes.json` | `f3b283b878d3d2dbd30b874e8853cfdc03e72dd2e04729e1e51d72fbf2d47045` |
