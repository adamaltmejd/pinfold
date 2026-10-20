# Apple nested KVM probe

2026-10-20. Can an Apple silicon Mac be the arm64 Linux KVM host that
issue #121 needs to bring the Linux arm64 release back? Probed on this
Mac: Apple M5, macOS 27.0.1, Apple `container` 1.5.0. Scripts and raw
outputs are in `2026-10-20-apple-nested-kvm-probe/`. None of it is
pinfold code.

## Verdict

It works on this M5 (macOS 27.0.1) with one custom guest kernel. Apple
documents nested virtualization for M3 and later; M3 and M4 are
untested. `container run --virtualization` already starts the guest at
EL2. Apple's default guest kernel has `CONFIG_VIRTUALIZATION` unset, so
there is no `/dev/kvm`. The same kernel (6.18.15, Apple's embedded config) with
`CONFIG_VIRTUALIZATION=y` and `CONFIG_KVM=y` boots with KVM in nVHE
mode. A GICv3 vGIC is inferred: KVM enables the GIC system register
interface, and Firecracker's guests need GICv3 and boot. `/dev/kvm`
opens and `KVM_CREATE_VM` succeeds. Inside that box, Firecracker 1.17.0
aarch64 boots its CI guest kernel 6.1.188 and runs a command over vsock,
three times out of three.

The nightly Mac mini does not qualify. It is an M1 (Macmini9,1) on
macOS 27.0.1, and
`VZGenericPlatformConfiguration.isNestedVirtualizationSupported` returns
`false` there (`true` on this M5). That flag is the only evidence:
`container` is not installed on the mini, so no boot was tried there.
Apple's documentation for that property says nested virtualization "is
available for Mac with the M3 chip, and later", from macOS 15.0
(<https://developer.apple.com/documentation/virtualization/vzgenericplatformconfiguration/isnestedvirtualizationsupported>).
Bringing arm64 back needs an M3 or later Mac as a runner, and a check on
the chip actually used if it is not an M5.

Not covered: pinfold itself, its rootless VMM jail, or the Linux suite.
Apple's kernel config has `# CONFIG_SECURITY_LANDLOCK is not set`. The
jail #114 prototyped uses Landlock. If pinfold's jail keeps it, the
suite's kernel needs `-e SECURITY_LANDLOCK` as well. `CONFIG_USER_NS`,
`CONFIG_SECCOMP_FILTER` and cgroups are already on.

## Setup steps

1. Mac: M3 or later per Apple (tested on M5 only), macOS 15 or later,
   Apple `container` 1.5.0.
2. Extract the default kernel's config. `container system property
   list` names a Kata 6.18.35 kernel, but `kernels/default.kernel-arm64`
   links to `vmlinux-6.18.15-186`, which is what boots. Its config is
   embedded (`IKCFG_ST`):

   ```
   K=~/Library/Application\ Support/com.apple.container/kernels
   python3 -I ikcfg.py "$K/vmlinux-6.18.15-186" > apple-6.18.15.config
   ```
3. Build in a container: `container run --rm --cpus 5 --memory 8g -v
   DIR:/work debian:trixie-slim /work/build.sh`. It fetches
   `linux-6.18.15.tar.xz` from kernel.org (sha256 only, against
   `sha256sums.asc`; its PGP signature is not verified), applies the
   config, enables `VIRTUALIZATION` and `KVM`, runs `olddefconfig`,
   fails unless `CONFIG_KVM=y` survives, and copies out
   `arch/arm64/boot/Image`. Apple's `vmlinux-*` files are raw arm64 boot
   Images too, not ELF. 28 s setup, 212 s build on 6 vCPUs, 16 MB
   output.
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

Excluded runs. The first run's `boot_to_exec_ms=41106`
(`out/halfclose.txt`) is the retry loop running out against the
half-close bug below, not a boot time. A later run reached guest init at
8.57 s, with a 5.9 s gap after `devtmpfs: initialized`. Its host-side
poll ran blocking `socat` processes that each held stdin open for 10 s,
so its readiness time measured the poll, and the three clean runs above
did not reproduce the stall. Neither run is in the table.

For comparison, #114 measured launcher start to the guest's ready line
at a 963 ms median (ten runs, x86 labvm, nested KVM). The nearest
metric here is Firecracker start to vsock ready, 3.7–4.3 s, about 4x.
The two differ: #114's guest and init are its own, and ready there is an
agent's signal, not a socat listener. In the guest, CPU bring-up and
devtmpfs are where the time shows: each step that traps to L0 is slow
under nVHE nesting. The 600 ms exec time is the prototype's socat and
shell, not a tuned agent.

## Findings for the spec

- Firecracker drops a host-initiated vsock stream as soon as the host
  half-closes it (`out/halfclose.txt`: the handshake `OK` arrives, then
  the guest's writes fail with `EPIPE`). This is the same as #114's
  finding 4, now seen on aarch64 too. `socat ...,shut-none` avoids it.
- The suite on an arm64 Mac would run as L2 inside an Apple container
  VM. Start to ready is about 4x #114's x86 labvm figure (see Results for
the two metrics). That is a budget
  risk for the 5-minute suite, so measure it before the suite is
  admitted on this host.

## Method

- `ikcfg.py` extracts the gzip config after `IKCFG_ST` from a kernel
  image.
- `build.sh` is the kernel build above. `out/build.txt` holds the
  resulting options, timing and the Image's sha256.
- `probe.sh` runs in the L1 box. It prints the EL and KVM boot lines,
  opens `/dev/kvm` with three ioctls, fetches Firecracker
  `v1.17.0-aarch64` (sha256 only, against its `.sha256.txt`) and
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
