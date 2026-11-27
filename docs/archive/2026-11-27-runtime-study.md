# Runtime study: leaving Apple `container`, and Firecracker's place

Date: 2026-11-27. A study only. It changes no spec or code. Adam asked
for it after #506.

The problem:
- A box never returns freed memory to macOS.
- The registry-research-toolkit release build was OOM-killed at its box's
  14 GB limit. That build peaked at 12–22 GB when run on the host (its
  agent's report).
- A larger limit then holds that memory until the box restarts.

Result: A′ (below) is the next thing to prototype. It is not yet a
decision to switch runtimes. The open items are listed at the end.

Marks:
- [M] measured, with a log in the raw material;
- [R] measured this session by a research agent, with no log kept;
- [S] read from source or documentation;
- [I] inferred.

Mac numbers come from one Mac (24 GiB, macOS 27.2.1). It was running a
14 GB project box with swap nearly full. Linux numbers come from the dev
VM (Debian 13, 6 vCPU, 7 GiB, KVM). Single runs unless stated.

Raw material lives outside the repository, in
`~/code/pinfold-runtime-study-2026-11-27/`:
- `study-a/` and `study-b/`: prototype sources and logs;
- `study-inventory/`: what pinfold takes from each runtime;
- `study-images.txt`: the image-chain sizing;
- `study-musl/`: the static-link probe.

## Goals

1. **Mac memory.** Freed guest memory returns to macOS while a box runs.
2. **Shared code.** One core for Mac and Linux.
3. **Linux shares.** Host folders mounted live on Linux. They are refused
   today by #112's Git-only design, which also rejected guest writes to
   host files. This is Adam's decision.
4. **Common ground with wrap-style tools.** wrap builds on microsandbox,
   which is libkrun.

## Why `container` cannot meet goal 1

- [R] `container` 1.7.1 attaches no balloon device: the guest has no
  virtio type 5. A 4 GiB allocate-and-free left the VM's
  `phys_footprint` at 4646 MB.
- [S] [containerization#1000](https://github.com/apple/containerization/pull/1000)
  would add a target-size balloon. It was open and unreviewed when read,
  and `container` has no call to drive it.
- [R] `container` runs each VM in Apple's sandboxed
  `com.apple.Virtualization.VirtualMachine` XPC service. [M] The VZ probe
  confirmed that service is sandboxed (`sandbox_check` = 1).

## Options

- **0. Stay.** `container` on the Mac, Firecracker on Linux as
  configured today, with no balloon.
- **A.** Virtualization.framework (VZ), driven from Rust, on the Mac.
  Cloud Hypervisor (CH) on Linux.
- **A′.** VZ on the Mac. Firecracker on Linux, with its balloon and free
  page reporting turned on.
- **B.** libkrun on both.

| Goal / property | 0 | A | A′ | B |
|---|---|---|---|---|
| Mac memory returns | no [R] | only when a balloon target is set; a controller is [I] | as A | yes, without a controller, in REPLACE mode [M] |
| Linux memory returns | no (no balloon configured) | yes, by free page reporting [M] | Firecracker reporting: [S] in 1.14+, unmeasured with pinfold's kernel | [S] only, not run on Linux |
| Shared code | today's split | init, transport, disks and images; two VMM backends [I] | as A | one VMM backend; host confinement stays per OS [I] |
| Linux shares | no | possible, through virtiofsd [M], untested in pinfold's jail | no | built in [S] |
| Mac VMM process | Apple XPC service, sandboxed [M] | the same service [M]; same confinement [I] | as A | pinfold's own process under Seatbelt [M]; `sandbox-exec` is marked deprecated [S] |
| Linux VMM controls | Firecracker seccomp and pinfold jail | CH seccomp and `--landlock` [M]; inside pinfold's jail [I] | as 0 | pinfold jail; seccomp [S] none found in the 1.19.8 tree |
| New dependencies | none | `objc2`, `objc2-virtualization`, `block2`; CH; virtiofsd for shares; an ext4 formatter | Mac part of A | libkrun (about 107 crates [R]); an ext4 formatter |

Share behaviour depends on the operation:
- **Rename and replace** appear about 1 s late on Apple shares (October
  probe).
- **Ownership** in the guest follows the last uid to look a path up, on
  VZ [M].
- **libkrun:** rename and replace appear about 5 s late [M], and
  in-place edits and `chmod` at the first sample, about 0.3–0.4 s [M]. A
  zero-timeout patch exists, but its result has no log.
- **Spec:** the spec promises no visibility delay. Staleness matters only
  if a proposal needs a bound on it.

## Measurements

### VZ from Rust (prototype about 200 lines, `objc2-virtualization` 0.3, ad-hoc signed)

| Measure | Result |
|---|---|
| VZ start | 107 ms; 171 ms with the host process sandboxed [M] |
| First guest command over vsock | 235 ms [M] |
| vsock 64 B round trip p50 / p99 | 21.4 / 44.1 µs and 31.3 / 77.7 µs (two runs) [M] |
| Balloon features offered | MUST_TELL_HOST, DEFLATE_ON_OOM; no stats queue, hinting or reporting [M] |
| 4 GiB touched then freed, plus `compact_memory` | footprint stays 4332–4336 MB [M] |
| Balloon target 1 GiB, run 1 | 263 MB at the first sample, +8 s [M] |
| Balloon target 1 GiB, run 2 | 3030 MB at +0.3 s, 246 MB at +2 s; host free 33% to 71% [M] |
| Run 3: alloc 2 GiB under a 1 GiB target, then free | footprint 2216 MB, flat at +5 and +15 s; setting the same target again gave 1026 MB [M] |
| virtio-blk discard | 256 MiB punched back to 16 KB [M] |
| Host process killed | VM process gone within 1.5 s [M] |

Run 3 bears on any controller:
- The guest can grow past the target (deflate on OOM).
- The balloon then stays deflated until the target is set again.
- The result is bounded by the target, not by guest use.

Whether a controller keeps reclaim reliable under a real workload is
untested.

### Cloud Hypervisor v55.0 on the dev VM

Host VmRSS of the CH process, in MiB. The guest touched 4 GiB, then freed
it. One run per row; x64 guest, CH's own test kernel.

| Configuration | boot | peak | +10 s | +20 s | +30 s |
|---|---|---|---|---|---|
| private memory, free page reporting | 258 | 4360 | 1250 | 298 | 290, after a manual compaction [M] |
| private memory, no reporting | 258 | 4360 | 4360 | 4360 | 4360 [M] |
| shared memory, reporting, virtiofsd attached | 242 | 4343 | 2063 | 463 | 271 [M] |

- **Shared memory**, which virtiofsd needs, adds about 1.2 s to boot and
  makes first touch about 10× slower [M].
- **Confinement:** CH ran rootless with seccomp, `--landlock`, no network
  device and a read-only disk [M].
- **Not run:** a vsock round trip, arm64, and CH or virtiofsd inside
  pinfold's own jail.
- **Fragmentation:** a fragmented long-lived guest may keep pages that
  reporting never sees [I].

### libkrun 1.19.8 on the Mac

`phys_footprint` in MB. 6 GiB guest; it touched 4 GiB, then freed it. One
run per row.

| Mode | baseline | allocated | +3 s | +15 s | +30 s |
|---|---|---|---|---|---|
| DEFAULT (`MADV_FREE`) | 269 | 4373 | 4373 | 4368 | 4368 [M] |
| REPLACE, libkrunfw kernel | 270 | 4372 | 3868 | 1531 | 286 [M] |
| REPLACE, Apple's 6.18 kernel | 266 | 4373 | 3851 | 1528 | 286 [M] |

- [M] Process start to exit took 0.109–0.124 s with the libkrunfw kernel
  (10 runs).
- [R] Network:
  - with TSI on, the default, the guest reached 1.1.1.1;
  - with TSI off, connect returned ENETUNREACH;
  - a read-only share refused writes even after a guest remount.

  The launcher code is kept; result logs are not.
- [R] libkrun linked into a static x86_64-musl binary (2.9 MB), which was
  built but not run. pinfold's own kernel is then needed, through
  `krun_set_kernel`.
- It was not built on Linux: the dev VM has no Rust toolchain or
  libclang.

## Security changes against today

**A and A′, Mac**
- The VMM stays in Apple's sandboxed XPC service [M].
- pinfold can sandbox its own client process, but must allow sandbox
  extension issuing [M]. VZ hands file access over from it.
- The VM now dies with its owner [M]. Today the Apple guest survives
  owner death (row 8), so row 8's Mac clause changes.
- Mac builds would move onto pinfold's builder. They need a Mac
  counterpart to row 44: direct egress refused, the proxy used, and the
  egress log kept. They also need host confinement for the builder VM.
- Existing Apple volumes and project homes need a path across, or a
  documented manual reset. Under the no-users rule a reset is acceptable.

**A, Linux**
- CH has its own seccomp and Landlock [M].
- Shares add virtiofsd as a second process to jail. The October design
  rejected live mounts for that daemon surface and their mount controls.

**B**

Each of these is a threat-model change:
- The Mac VMM moves from Apple's service into a pinfold process,
  confined by Seatbelt [M]. Its Mach and IOKit allowances are not
  narrowed, and the tool is deprecated [S]. Whether that boundary is
  weaker than Apple's is [I].
- No seccomp filter was found in libkrun [S]. The spec's "Firecracker's
  own seccomp filters" would need pinfold's own filters, or a spec change.
- FUSE names are not validated [R]. A compromised guest kernel would then
  reach all the VMM's jail allows, without needing a VMM bug [I].
- TSI is on by default. Its absence needs a positive-control test.

## Shared costs (any option that leaves `container`)

**Images** (`study-images.txt`)
- Port the Linux image service to arm64: about 260–490 changed lines and
  about 75 regenerated package rows.
- It deletes about 400 lines of Apple build code.
- These pins double: the builder root, the kernel, BuildKit, Go, and the
  Debian package list. These stay shared: Firecracker, Debian snapshot
  metadata, and BuildKit's source and vendor pins.
- One alternative clones the ext4 snapshot Apple already unpacked, in
  about 60–80 lines. It keeps Apple's unproxied builds and depends on an
  unverified private store layout.

**ext4 on the Mac**
- pinfold calls `mkfs.ext4` in five places.
- A pure-Rust formatter (`mkext4` 0.0.4) would cover them in about
  200–300 lines.
- To prove:
  - a sparse 64 GiB empty disk;
  - a `-d`-style populated tree, links included;
  - a clean e2fsck.

**Kernel**
- An arm64 guest kernel from pinfold's config: about 80 lines, CI
  included.
- The candidate arm64 config has virtio-fs off, which matters only for
  shares.

**Signing**
- An ad-hoc `codesign` with the entitlement in the macOS release job:
  about 10 lines plus a plist.
- Self-update renames the verified binary unchanged, so the signature
  survives [S].
- The e2e harness and the build-from-source steps need the same signing
  step.

**Re-proof**
- A Mac switch re-proves rows 1, 5, 8, 26, 48 and 50, and needs row 44's
  Mac counterpart. A Linux switch re-proves rows 1, 5, 11, 43 and 44.
- The 14-attachment limit is 17 virtio devices less init, root and vsock
  (`plan.rs`), with `/tmp` counted in. Another VMM's device limits must be
  checked.

## Size

| Piece | Lines |
|---|---|
| VZ backend | about 800–900 itemized, plus init changes of about 80 |
| Balloon controller | about 150–250 [I], no breakdown yet |
| Firecracker to CH, parity | about 250–350 changed Rust lines; kernel config, pins, CI and docs extra |
| Linux shares through virtiofsd | about 200–300 [I] |
| libkrun backend, Seatbelt, seccomp | about 500–800, 200–300, 200–400 [I] |
| Image chain to arm64 | about 260–490 |
| ext4 formatter | about 200–300 |
| Deleted with `container` | `apple.rs` 1,363, `apple/image.rs` 386, about 350–450 of `pid1.rs`, Apple spec text |

## Recommendation

Prototype A′ next. Leave CH, and with it Linux shares, as Adam's
separate decision.

1. **Mac: VZ from Rust.**
   - It is the only option measured to return memory while keeping
     today's Mac VMM process.
   - It moves the Mac onto pinfold's own init, transport, disks and
     image service (goal 2).
   - The reclaim it showed was manual. A controller has to make it
     automatic and dependable.
2. **Linux: keep Firecracker.** Measure its balloon and free page
   reporting with pinfold's kernel first. If that holds, Linux gets
   goal 1 without a VMM change.
3. **Linux shares (goal 3).** Adam decides. If yes, CH is the measured
   route here, and other routes were not studied. Without shares, CH's
   only measured gain over Firecracker is reporting, which Firecracker
   claims too.
4. **B is not the next prototype.** Each reason carries its own evidence
   mark above:
   - it moves the Mac VMM out of Apple's service;
   - it lacks seccomp;
   - unvalidated FUSE names widen what a guest-kernel compromise reaches;
   - its share timeouts are fixed in 1.19.

   Goal 4 alone does not outweigh these.

### Balloon controller design (for the spike)

- **When it acts:** on host memory pressure, not on a timer.
  `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE` reports warn, critical and normal.
- **What it reads:** init reports `MemAvailable` and
  `/proc/pressure/memory` over the existing vsock transport.
- **On warn or critical:**
  - it sets each box's target to its use plus a margin;
  - it skips a box under its own memory pressure;
  - it sets the target again after any deflate on OOM, because run 3
    showed no automatic re-inflate.
- **On a sustained return to normal:** it raises targets back to the
  full size.
- **Deflate on OOM** remains the guest's safety valve. A target set too
  low can still cost the guest page cache or stall it.

### Spikes before a spec proposal

- pinfold's init booting under VZ with an arm64 kernel built from
  pinfold's config.
- The controller above, driving reclaim repeatedly under the
  registry-research-toolkit release build, with footprint, swap and
  guest stalls recorded.
- `mkext4`: a sparse 64 GiB empty disk and a populated tree, both passing
  e2fsck and mounting in that guest.
- Firecracker's balloon and free page reporting, measured with pinfold's
  kernel on the dev VM.
- Mac builds on pinfold's builder under VZ, meeting row 44's egress
  controls.
