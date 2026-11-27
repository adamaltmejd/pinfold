# Runtime study: leaving Apple `container`, and Firecracker's place

Date: 2026-11-27. A study only; no spec or code change. Adam asked for it
after #506. A box never returns freed memory to macOS. A 22 GB release
build then needs a box restart on a 24 GB Mac.

Marks: [M] measured, [S] read from source, [I] inferred. The Mac numbers
come from one Mac (Mac17,2, 24 GiB, macOS 27.2.1), measured while a 14 GB
project box ran and swap was nearly full. Linux numbers come from the dev
VM (Debian 13, 6 vCPU, 7 GiB, KVM).

Raw material is outside the repository, in
`~/code/pinfold-runtime-study-2026-11-27/`:
- prototype sources and logs (`study-a/`, `study-b/`);
- the inventory of what pinfold takes from each runtime (`study-inventory/`);
- the image-chain sizing (`study-images.txt`);
- the static-link probe (`study-musl/`).

## Goals

1. **Mac memory.** Freed guest memory returns to macOS while a box runs.
2. **Shared code.** One core for Mac and Linux.
3. **Linux shares.** Host folders mounted live on Linux. Today they are
   refused there, by #112's Git-only design. This is Adam's decision.
4. **Common ground with wrap-style tools.** wrap builds on microsandbox,
   which is libkrun.

## Why `container` cannot meet goal 1

- [M] `container` 1.7.1 attaches no balloon device. The guest's
  `/sys/bus/virtio/devices` has no type 5.
- [M] A 4 GiB allocate-and-free leaves the VM's `phys_footprint` at
  4646 MB.
- [S] [containerization#1000](https://github.com/apple/containerization/pull/1000)
  would add a target-size balloon to every VM. It is unreviewed, and
  `container` has no call to drive the balloon.
- [S] Earlier balloon PRs (#893, #894) were closed unmerged.
- [M] `container` already runs each VM in Apple's sandboxed
  `com.apple.Virtualization.VirtualMachine` XPC service. That is today's
  Mac VMM boundary.

## Options

- **0. Stay.** `container` on the Mac, Firecracker on Linux.
- **A.** Virtualization.framework (VZ) driven from Rust on the Mac. Cloud
  Hypervisor (CH) on Linux.
- **A′.** VZ on the Mac. Firecracker stays on Linux, with free page
  reporting turned on.
- **B.** libkrun on both.

| Goal / property | 0 | A | A′ | B |
|---|---|---|---|---|
| Mac memory returns | no [M] | yes, through a pinfold balloon controller [M] | as A | yes, automatic in REPLACE mode [M, one run per kernel] |
| Linux memory returns | FC 1.18 free page reporting, developer preview [S, unmeasured] | yes, automatic, about 20 s [M] | as 0 | yes, from source [S], not run |
| Shared code | today's split | init, transport, disks and images are shared; two VMM backends | as A | one VMM backend; jail and seccomp versus Seatbelt stay per OS |
| Linux shares | no | possible: virtiofsd as a second jailed process [M] | no | built in [S] |
| Mac VMM boundary | Apple XPC sandbox | Apple XPC sandbox, unchanged [M] | as A | pinfold process under Seatbelt, which is deprecated [M] |
| Linux VMM controls | FC seccomp and pinfold jail | CH seccomp, `--landlock` and pinfold jail [M] | as 0 | pinfold jail only; libkrun has no seccomp [S] |
| Share host-to-guest staleness | about 1 s (Apple probes) | about 1 s, the same VZ virtio-fs [M] | as A | about 5 s; 0 needs a patch or the 2.0 API [M] |
| Read-only shares | guest mount flag | guest mount flag [I] | as A | host-enforced EROFS, survives remount [M] |
| New dependencies | none | `objc2-virtualization`; CH, and virtiofsd for shares | `objc2-virtualization` | libkrun, about 107 crates; zstd and bzip2 C code |
| API stability | `container` 1.x | VZ public API since macOS 11; CH stable | as A | 1.19 stable; the needed share settings are only in 2.0, pre-release [S] |

## Measurements

### VZ from Rust (prototype about 200 lines, `objc2-virtualization` 0.3.2, ad-hoc signed)

| Measure | Result |
|---|---|
| VZ start | 86–141 ms [M] |
| First guest command over vsock | 158–252 ms [M] |
| vsock 64 B round trip, p50 / p99 | 21–39 / 44–110 µs [M] |
| Balloon features offered | MUST_TELL_HOST, DEFLATE_ON_OOM only; no stats, hinting or reporting [M] |
| 4 GiB touched then freed, plus `compact_memory` | footprint stays 4332–4336 MB [M] |
| Balloon target set to 1 GiB | footprint 246–263 MB within about 2 s; host free 33% to 71% [M] |
| Guest allocates past the target | allowed (deflate on OOM); no automatic re-inflate until the target is set again [M] |
| virtio-blk discard | 256 MiB punched back to 16 KB [M] |
| Host killed | VM gone within 1.5 s [M] |

VZ needs a pinfold balloon controller of about 150–250 lines [I]. It
takes guest memory statistics from init over vsock, because the device has
no stats queue. It sets the target to use plus a margin, and sets it again
after any deflate.

### Cloud Hypervisor v55.0 on the dev VM (host VmRSS of the CH process, MiB; 4 GiB touched then freed)

| Configuration | boot | peak | +10 s | +20 s | +30 s |
|---|---|---|---|---|---|
| private memory, free page reporting | 258 | 4360 | 1250 | 298 | 290 |
| private memory, no reporting | 258 | 4360 | 4360 | 4360 | 4360 |
| shared memory (needed by virtiofsd), reporting, virtiofsd attached | 242 | 4343 | 2063 | 463 | 271 |

Each row is one run [M]. Shared memory adds about 1.2 s to boot and
makes first touch about 10× slower [M]. CH ran rootless with seccomp,
`--landlock`, no network device and a read-only disk [M]. The vsock round
trip and arm64 were not run.

### libkrun 1.19.8 on the Mac (6 GiB guest, 4 GiB touched then freed; `phys_footprint`, MB)

| Mode | baseline | allocated | +3 s | +15 s | +30 s |
|---|---|---|---|---|---|
| DEFAULT (`MADV_FREE`) | 269 | 4373 | 4373 | 4368 | 4368 |
| REPLACE, libkrunfw kernel | 270 | 4372 | 3868 | 1531 | 286 |
| REPLACE, Apple's 6.18 kernel | 266 | 4373 | 3851 | 1528 | 286 |

Each row is one run [M]. Process start to exit took 0.06–0.12 s [M].

- **Network.** With TSI on (the default), the guest connected to
  1.1.1.1 [M]. With `krun_disable_implicit_vsock` plus an explicit vsock,
  connect returned ENETUNREACH [M].
- **Static link.** libkrun linked into a static x86_64-musl binary
  (2.9 MB, built not run) [M]. It then needs pinfold's own kernel through
  `krun_set_kernel`.
- **Linux.** Not built on Linux: the dev VM has no Rust toolchain or
  libclang.

## Security changes against today

- **A and A′.**
  - The Mac VMM stays in Apple's sandboxed XPC service, the same as today.
  - pinfold can sandbox its own client process with Seatbelt, but must
    allow sandbox-extension issuing [M].
  - The VMM jail stays Linux-only, as now.
  - The balloon controller is new code and a new way to fail: a target
    set too low makes the guest deflate on OOM, which costs time rather
    than safety.
- **A, the Linux side.**
  - CH brings its own seccomp and Landlock [M].
  - Linux shares would add virtiofsd as a second process to jail. The
    October design rejected live mounts for exactly that daemon surface.
- **B. Each of these is a threat-model change:**
  - The Mac VMM moves from Apple's XPC sandbox into a pinfold process,
    confined by a deny-default Seatbelt profile [M]. The profile's
    Mach and IOKit allowances are not narrowed.
  - There is no seccomp. "Firecracker's own seccomp filters" in Always
    applied would need pinfold's own filters, or a spec change.
  - libkrun does not validate FUSE names [S]. A guest kernel compromise
    reaches everything the VMM's jail allows, without needing a VMM bug.
  - TSI is on by default. Its absence needs a positive-control test.

## Shared costs (any option that leaves `container`)

- **Images** (`study-images.txt`).
  - Port the Linux image service to arm64: about 260–490 changed lines and
    about 75 regenerated package rows.
  - It deletes about 400 lines of Apple build code.
  - Every pin except Firecracker doubles.
  - Mac builds then get the same egress controls as Linux.
  - Keeping `container build` and converting its output costs as much or
    more, and keeps Apple's uncontrolled build egress.
- **ext4 on the Mac.** pinfold calls `mkfs.ext4` in five places. A
  pure-Rust formatter (`mkext4` 0.0.4) covers them in about 200–300 lines.
  Not yet proven: sparse 64 GiB output and a clean e2fsck.
- **Kernel.** An arm64 guest kernel from pinfold's own config, about 80
  lines plus CI. Not yet proven to boot pinfold's init under VZ.
- **Signing.** An ad-hoc `codesign` with the entitlement in the macOS
  release job, about 10 lines. Self-update keeps the signature because it
  renames the binary unchanged [S]. The e2e harness and the
  build-from-source steps need the same step.
- **Re-proof.** A Mac switch re-proves guarantee rows 1, 5, 8, 26, 48 and
  50. A Linux switch re-proves rows 1, 5, 11, 43 and 44.
  - Behaviour with no code behind it needs re-measuring: flush semantics,
    vsock reset versus timeout, drive order, discard.
  - The 14-attachment limit's origin is not recorded in the spec. #506's
    archive calls it the virtio device count. Treat it as VMM-dependent
    until measured.

## Size

| Piece | Lines |
|---|---|
| VZ backend | about 900–1,100 new, plus a balloon controller of about 150–250 |
| Firecracker to CH (parity) | about 250–350 changed |
| Linux shares through virtiofsd | about 200–300 |
| libkrun backend, Seatbelt, seccomp | about 500–800, 200–300 and 200–400 |
| Image chain to arm64 | about 260–490 |
| ext4 formatter | about 200–300 |
| Deleted with `container` | `apple.rs` 1,363, `apple/image.rs` 386, about 450 of `pid1.rs`, Apple spec text |

## Recommendation

Do A′ in this order. Leave CH, and with it Linux shares, as a separate
decision.

1. **Mac: VZ from Rust.** It meets goal 1 with a measured result. It keeps
   today's Mac VMM boundary. It moves the Mac onto pinfold's own init,
   transport, disks and image service, which serves goal 2.
2. **Linux: keep Firecracker.** Measure its free page reporting on the dev
   VM first, because it is marked developer preview. If it holds, Linux
   gets goal 1 with no VMM change.
3. **Linux shares (goal 3).** Adam decides. If yes, CH is the route:
   - about 250–350 lines to reach parity;
   - plus virtiofsd, which brings back the daemon surface #112 removed.

   Without that decision, CH buys only what Firecracker's reporting would.
4. **B is not recommended.**
   - It would weaken today's Mac boundary: a deprecated Seatbelt profile
     around our own process, in place of Apple's sandboxed service.
   - It drops seccomp.
   - It widens what a guest-kernel compromise reaches.
   - It depends on a pre-release API for share settings.
   - Goal 4 does not outweigh these. wrap's own engine (microsandbox) adds
     no VMM hardening of its own [S].

Spikes before a spec proposal:
- pinfold's init booting under VZ with an arm64 kernel built from
  pinfold's config;
- `mkext4` producing a sparse 64 GiB disk that passes e2fsck and mounts in
  that guest;
- Firecracker 1.18 free page reporting measured on the dev VM;
- a balloon controller design: policy, period, margin, and its race with
  deflate-on-OOM.
