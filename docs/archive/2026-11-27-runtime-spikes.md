# Runtime spikes (2026-11-27)

These are the five spikes that 2026-11-27-runtime-study.md asked for
before a spec proposal. Adam authorised them on 2026-11-27. He also
decided that Linux stays without shares, so Cloud Hypervisor is not
pursued.

Each spike ran prototype code outside the repository. The raw material,
including notes, sources, logs and per-second samples, is in
`~/code/pinfold-runtime-study-2026-11-27/spike{1..5}/`. Marks follow the
study: [M] measured, with a log there; [S] read from source; [I]
inferred. Mac runs shared the 24 GiB Mac with a live 14 GB box. Single
runs unless stated.

## 1. pinfold's init under VZ

- **Boot** [M]:
  - pinfold's init, built for aarch64 with the existing `cargo zigbuild`
    path, booted under Virtualization.framework with no source change.
  - The arm64 kernel came from Firecracker v1.18.0's
    `microvm-kernel-ci-aarch64-6.18.config`, with build.sh's three edits.
  - Init went through its boot handshake, `ENV`, one `exec` and
    `UNMOUNT`. The host side was a stub, not pinfold's host code.
- **Ready:** 357–389 ms after host process start, over 5 runs [M].
- **Kernel:** VZ needs nothing beyond that base config. It already
  enables virtio-pci, the console, blk, vsock, balloon, `PAGE_REPORTING`
  and ext4. Only the command line and make target change: `console=hvc0`,
  no `pci=off`, `root=` stated, and target `Image` [M].
- **Disks:** drive order matched Firecracker's (vda, vdb, vdc) for three
  disks [M].
- **vsock:** VZ's per-connection sockets are Unix stream sockets.
  Only the `CONNECT` handshake and the boot listener need VZ code [M].
- **Gap: reboot.** When PID 1 exits, the kernel panics and resets, and
  VZ restarts the guest instead of stopping [M]. Init must power off
  instead. Any other guest panic still restarts the guest, so the host
  must treat a second boot dial as a stop [I].

## 2. Balloon controller under VZ

This is a prototype of the study's design. The guest sent
`MemAvailable` and PSI every second. The workload was repeated 2–4 GiB
allocate, hold and free cycles in a 6 GiB guest.

- **Footprint between cycles, VZ process, MiB** [M]:
  - no controller: 2358, 3896, 3901, 3902; the baseline was stopped in
    cycle 4 at 11 GiB of host swapouts;
  - controller held active: 1146, 1718, 2353, 1516, 1291.
- **Reclaim is incomplete** [M]:
  - after a clean shrink, about 0.45 GiB stays above the target;
  - after deflate-on-OOM cycles, up to 1.6 GiB stays above it.
  - A 16 KiB-page guest kernel left more, about 1.7 GiB against
    0.44 GiB, so page-size mismatch is not the cause. The cause is
    inside VZ and unexplained.
- **Timing** [M]: the target is set within 1 s and reached in 1–2 s.
  The host footprint then falls over 8 to about 50 s.
- **Guest cost** [M]:
  - every burst under a shrunk target went through deflate-on-OOM;
  - allocation slowed to 0.85–1.66 s, from 0.33–0.70 s without the
    controller, with 0.04–0.76 s of stall per cycle;
  - no OOM kills.
  - The cost to the guest's page cache was not measured, because
    rereads came from the host's warm file cache.
- **Trigger** [M]: `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE` fired 1–2 times
  per run while the system level changed 2–7 times. In one run it
  fired 25 s late. Polling `kern.memorystatus_vm_pressure_level` each
  second saw every change. With a large box running, this Mac sat at
  warn.
- **Restoring the full target** let the guest refill past its earlier
  plateau, to 4.4–4.8 GiB [M].
- **Size:** about 190 lines [M].
- **Changes to the design:**
  - poll the level each second, with the dispatch source only as a
    wake-up;
  - relax the margin instead of restoring the full size;
  - have init stream stats, and boot with `psi=1`;
  - probe VZ's page-release path before a spec relies on reclaim.

## 3. ext4 formatter on the Mac

- **Call sites:** all five host `mkfs.ext4` sites (`create_disk`,
  `empty_disk`, `scratch_image`, `mount_image`, `input_disk`) need
  `-F -m 0`, with or without a journal. The populated sites also need
  `-N` and `-d` [S].
- **`ferrosys` 0.8.0** (MIT OR Apache-2.0):
  - covers all five unpatched;
  - its feature lines match mke2fs 1.47.2;
  - it writes to a caller-opened file, so mode 0600 and `create_new`
    are kept;
  - glue is about 60–80 lines, including a directory walker, since its
    own walker is Linux-only [M].
  - It has one maintainer and a large crate, built with only the `ext`
    feature.
- **`mkext4` 0.0.4** cannot omit the journal: `journal_blocks = Some(0)`
  panics. Three sites need that [M].
- **Verification:** `e2fsck -fn` passed all nine images, empty 64 GiB
  and populated, with and without a journal. `debugfs rdump` restored
  the trees exactly: modes, sha256 sums, symlinks, a hard link, and an
  htree directory [M]. No kernel mount was tried.
- **Speed and space:**
  - 64 GiB empty: 9–27 ms, 8.9 MB allocated;
  - the populated tree: 0.4–0.9 s, against mke2fs's 5–7 s on the x64
    VM [M].

## 4. Firecracker free page reporting

Runs used pinfold's pinned Firecracker 1.18.0 and kernel 6.18.57, with
sha256s verified against vmm.toml. They ran on the dev VM, outside
pinfold's jail. Three interleaved runs per arm.

**Host RSS, MiB** [M]:

| | peak | +5 s | +10 s | +20 s |
|---|---|---|---|---|
| reporting on | 4326 | 3068–3076 | 1156–1164 | 248 |
| balloon, reporting off | 4328 | 4328 | 4328 | 4328 |
| no balloon | 4328 | 4328 | 4328 | 4328 |

- **Boot time:** unchanged [M].
- **Kernel:** `VIRTIO_BALLOON`, `PAGE_REPORTING` and
  `BALLOON_COMPACTION` are already `=y` [M].
- **Firecracker's seccomp filter** allows `madvise` [S], and Landlock
  does not govern it [S].
- **Change:** a `balloon` object in `config()`, about 5 lines [I].
- **Status is ambiguous** [S]:
  - the v1.14.0 release notes call reporting a developer preview;
  - ballooning.md gives that label only to hinting.
- **Not returned:** guest page cache is not free memory, so it is not
  reported [S].

## 5. Mac builds on pinfold's builder under VZ

- **Builder:** an arm64 builder root, from the same Debian snapshot plus
  BuildKit 0.34.1 for arm64, booted under the spike-1 host. It used
  pinfold's init with the `builder` verb ungated: 7 lines added,
  9 removed [M].
- **Build:** row 44's Containerfile built in 7.8 s cold and 3.1 s warm
  [M].
- **Conversion:** a second VM with no proxy turned the output into a
  138 MB root that booted as a box [M].
- **Egress** [M]:
  - the RUN step saw only `lo`;
  - a direct connect failed at once with "Network is unreachable";
  - through a stub proxy on vsock, `api.github.com` was allowed and
    `example.com` refused, and allowing `example.com` let it through;
  - BuildKit's own pulls went through the proxy.
  - pinfold's real proxy was not run.
- **Size:** about 255–500 changed lines plus about 85 regenerated package
  rows. This is within the study's estimate [I].
- **New items:**
  - the job must use the embedded arm64 init, not a copy of the running
    Mach-O binary;
  - pinfold's proxy needs a small adapter for VZ's per-connection
    sockets [I].
- **The Apple-store snapshot fallback is dropped.** It keeps Apple's
  unproxied build egress, so it cannot meet row 44.

## Where this leaves A′

- **Linux** can return memory with about 5 lines of Firecracker
  configuration, independent of the Mac. That needs a spec line and the
  preview question settled.
- **Mac:**
  - Boot, builds, egress and disk formatting have no blocker.
  - Memory return, the reason for the move, works only in part. A
    controller brings the footprint between cycles down from about
    3.9 GiB to about 1.3–2.4 GiB.
  - It leaves 0.45–1.6 GiB above target, and it slows bursts.
- **Before a spec proposal:** VZ's page-release path is the open
  question. Then the controller trigger and margin need a second run.
