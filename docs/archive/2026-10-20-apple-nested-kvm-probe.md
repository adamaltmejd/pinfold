# Apple nested KVM probe

2026-10-20. Can an Apple silicon Mac be the arm64 Linux KVM host that
issue #121 needs to bring the Linux arm64 release back? Probed on this
Mac: Apple M5, macOS 27.0.1, Apple `container` 1.5.0. Scripts and raw
outputs are in `2026-10-20-apple-nested-kvm-probe/`. None of it is
pinfold code.

## Verdict

It works on an M3 or later Mac, with one custom guest kernel.
`container run --virtualization` already starts the guest at EL2. Apple's
default guest kernel has `CONFIG_VIRTUALIZATION` unset, so there is no
`/dev/kvm`. The same kernel (6.18.15, Apple's embedded config) with
`CONFIG_VIRTUALIZATION=y` and `CONFIG_KVM=y` boots with KVM in nVHE mode
and a GICv3 vGIC. `/dev/kvm` opens and `KVM_CREATE_VM` succeeds. Inside
that box, Firecracker 1.17.0 aarch64 boots its CI guest kernel 6.1.188
and runs a command over vsock, three times out of three.

The nightly Mac mini does not qualify. It is an M1 (Macmini9,1), and
Apple's Virtualization framework offers nested virtualization only on M3
and later, on macOS 15 and later. Bringing arm64 back needs an M3+ Mac as
a runner.

Not covered: pinfold itself, its rootless VMM jail, or the Linux suite.
Apple's kernel config has `# CONFIG_SECURITY_LANDLOCK is not set`, and the
jail #114 prototyped uses Landlock, so the suite's kernel needs
`-e SECURITY_LANDLOCK` as well. `CONFIG_USER_NS`, `CONFIG_SECCOMP_FILTER`
and cgroups are already on.

## Setup steps

1. Mac: M3 or later, macOS 15 or later, Apple `container` 1.5.0.
2. Extract the default kernel's config. It is embedded (`IKCFG_ST`):
   `python3 -I ikcfg.py ~/Library/Application\ Support/com.apple.container/kernels/vmlinux-6.18.15-186 > apple-6.18.15.config`.
3. Build in a container: `container run --rm --cpus 5 --memory 8g -v
   DIR:/work debian:trixie-slim /work/build.sh`. It fetches
   `linux-6.18.15.tar.xz` from kernel.org (checked against
   `sha256sums.asc`), applies the config, enables `VIRTUALIZATION` and
   `KVM`, runs `olddefconfig`, fails unless `CONFIG_KVM=y` survives, and
   copies out `arch/arm64/boot/Image`. Apple's `vmlinux-*` files are raw
   arm64 boot Images too, not ELF. 28 s setup, 212 s build on 6 vCPUs,
   16 MB output.
4. Boot with it: `container run --virtualization --kernel Image ...`.
   `/dev/kvm` is `crw------- root root` in the box.

## Results

`out/probe.txt` is the last full run; `out/fc-console.txt` is the guest
console of its first boot.

L1 box (custom kernel, `--virtualization`, 4 vCPU, 4 GiB):

```
CPU: All CPU(s) started at EL2
kvm [1]: nv: 568 coarse grained trap handlers
kvm [1]: IPA Size Limit: 40 bits
kvm [1]: GICv3: no GICV resource entry
kvm [1]: disabling GICv2 emulation
kvm [1]: GIC system register CPU interface enabled
kvm [1]: Hyp nVHE mode initialized successfully
KVM_GET_API_VERSION 12
KVM_CAP_ARM_VM_IPA_SIZE 40
KVM_CREATE_VM fd 4
```

"No GICV resource" only disables GICv2 emulation. Firecracker's arm64
guests use GICv3, which works.

L2 guest (Firecracker, 2 vCPU, 512 MiB, read-only ext4 root, init runs
`socat VSOCK-LISTEN:1000,fork EXEC:/bin/sh`). The host side sends
`CONNECT 1000` to Firecracker's vsock socket and then
`uname -a; head -3 /proc/cpuinfo; echo vsock-ok; exit`:

| run | Firecracker start to vsock ready | exec round trip | guest init at |
|----:|---------------------------------:|----------------:|--------------:|
| 1   | 3955 ms | 600 ms | 2.70 s |
| 2   | 4296 ms | 595 ms | 2.85 s |
| 3   | 3711 ms | 598 ms | 2.49 s |

For comparison, #114 measured about 950 ms per boot on x86 labvm with
nested KVM. In the guest, CPU bring-up and devtmpfs are where the time
shows: each step that traps to L0 is slow under nVHE nesting. One
earlier run reached init at 8.57 s, with a 5.9 s gap after `devtmpfs:
initialized`. It ran while the host polled with blocking `socat`
processes, and three clean runs did not reproduce it. The 600 ms exec
time is the prototype's socat and shell, not a tuned agent.

## Findings for the spec

- Firecracker drops a host-initiated vsock stream as soon as the host
  half-closes it (`out/halfclose.txt`: the handshake `OK` arrives, then
  the guest's writes fail with `EPIPE`). This is the same as #114's
  finding 4, now seen on aarch64 too. `socat ...,shut-none` avoids it.
- The suite on an arm64 Mac would run as L2 inside an Apple container
  VM. Box boots are about 4x slower than on x86 labvm. That is a budget
  risk for the 5-minute suite, so measure it before the suite is
  admitted on this host.

## Method

- `ikcfg.py` extracts the gzip config after `IKCFG_ST` from a kernel
  image.
- `build.sh` is the kernel build above. `out/build.txt` holds the
  resulting options, timing and the Image's sha256.
- `probe.sh` runs in the L1 box. It prints the EL and KVM boot lines,
  opens `/dev/kvm` with three ioctls, fetches Firecracker
  `v1.17.0-aarch64` (checked against its `.sha256.txt`) and
  `firecracker-ci/20261016-5732b8331e1e-0/aarch64/vmlinux-6.1.188`, and
  turns the box's own Debian trixie userland, which has socat, into the
  guest root. It then boots three times, polls until a bare `CONNECT`
  answers `OK`, and times one exec. sha256: firecracker
  `fe726e0b...`, vmlinux `01455a3d...`.
- `out/host.txt` has the controls. The default kernel with
  `--virtualization` starts at EL2 with no `/dev/kvm`. Without the flag,
  it starts at EL1. It also has the Mac mini's chip.
- Disk: the host had 31 GiB free at the start, already below the 60 GiB
  floor. The build ran in the throwaway container's own rootfs. No images
  or volumes were added to Apple's store: 28 images before and after.
