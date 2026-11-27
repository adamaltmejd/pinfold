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
- **Ready:** `ENV_SET` came 357.6–388.6 ms after a clock started in
  the host's `main`, over 5 runs [M].
- **Kernel:** VZ needs nothing beyond that base config. It already
  enables virtio-pci, the console, blk, vsock, balloon, `PAGE_REPORTING`
  and ext4 [S]. Only the command line and make target change:
  `console=hvc0`, no `pci=off`, `root=` stated, and target `Image` [M].
- **Disks:** drive order matched Firecracker's (vda, vdb, vdc) for three
  disks [M].
- **vsock:** VZ's per-connection sockets are Unix stream sockets. This
  is recorded in the notes, not a log. Only the `CONNECT` handshake and
  the boot listener would need VZ code [I].
- **Gap: reboot.** Under `panic=1 reboot=k`, a PID 1 exit panics and
  resets, and VZ restarts the guest instead of stopping [M]. Init must
  power off instead. Other panics would likely restart the guest too, so
  the host should treat a second boot dial as a stop [I].

## 2. Balloon controller under VZ

This is a prototype of the study's design. The guest sent
`MemAvailable` and PSI every second. The workload was repeated 2–4 GiB
allocate, hold and free cycles in a 6 GiB guest.

- **Footprint between cycles, VZ process** [M]:
  - no controller: a plateau of about 3.8 GiB; the baseline was stopped
    in cycle 4 at 11 GiB of host swapouts;
  - controller held active: 1.1–2.3 GiB.
- **Reclaim is incomplete** [M]:
  - after a clean shrink, 0.41 GiB stays above the target with a 4 KiB
    guest;
  - in the run that followed host pressure, up to 1.85 GiB stayed above
    it.
  - A 16 KiB-page guest left 1.65 GiB in one matched pair. Matching page
    sizes did not fix it in that trial. Where the gap sits, host or
    guest, is unexplained [I].
- **Timing** [M]: the target is set within 1 s and reached in 1–2 s.
  The host footprint then settles over roughly 1–53 s.
- **Guest cost** [M]:
  - every burst under a shrunk target went through deflate-on-OOM;
  - allocation slowed to 0.85–1.66 s, from 0.33–0.70 s without the
    controller, with 0.04–0.76 s of stall per cycle;
  - no OOM kills.
  - The cost to the guest's page cache was not measured. Rereads were
    likely served from the host's file cache [I].
- **Trigger** [M]: `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE` fired 0–2 times
  per run. In one run it came 25.7 s after the polled level changed; in
  others it came first. Polling `kern.memorystatus_vm_pressure_level` at
  1 Hz saw more changes than the source, but cannot show it saw every
  one. With a large box running, this Mac sat at warn.
- **Restoring the full target** let the host footprint climb past the
  earlier plateau, to 4.28 and 4.66 GiB [M].
- **Size:** about 190 lines [I].
- **Workload:** synthetic 2 GiB bursts plus file I/O. The
  registry-research-toolkit release build was not run.
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
- **Evidence:** this spike kept its images, not its logs. The checks
  below are the agent's report.
- **`ferrosys` 0.8.0** (MIT OR Apache-2.0):
  - covers all five unpatched [I];
  - its feature lines match mke2fs 1.47.2;
  - it writes to a caller-opened file, so mode 0600 and `create_new`
    can be kept [I]; the prototype itself wrote 0644;
  - glue is about 60–80 lines, including a directory walker, since its
    own walker is Linux-only [I].
  - It has one maintainer and a large crate, built with only the `ext`
    feature.
- **`mkext4` 0.0.4** cannot omit the journal: `journal_blocks = Some(0)`
  panics [S]. Three sites need that.
- **Verification:** `e2fsck -fn` passed all nine images, empty 64 GiB
  and populated, with and without a journal. `debugfs rdump` restored
  file lists, modes, sha256 sums and symlinks. The images hold the hard
  link and an htree directory, but `rdump` does not prove hard-link
  identity. No kernel mount was tried.
- **Speed and space:**
  - 64 GiB empty: 9–27 ms, with 8.66 MiB of extents without a journal
    and 10.67 MiB with one [M for the extents];
  - the populated tree: 0.4–0.9 s, against mke2fs's 5–7 s on the x64
    VM.

## 4. Firecracker free page reporting

Runs used pinfold's pinned Firecracker 1.18.0 and kernel 6.18.57, with
sha256s verified against vmm.toml. They ran on the dev VM, outside
pinfold's jail. Three interleaved runs per arm.

**Host RSS, MiB** [M]:

| | peak | +5 s | +10 s | +20 s |
|---|---|---|---|---|
| reporting on | 4326 | 3068–3076 | 1156–1164 | 248 |
| balloon, reporting off | 4326–4330 | 4326–4330 | 4326–4330 | 4326–4330 |
| no balloon | 4326–4330 | 4328–4330 | 4328–4330 | 4328–4330 |

- **Boot time:** no clear difference across three runs per arm [M].
- **sha256:** the agent reports matches; the output was not kept.
- **Kernel:** `VIRTIO_BALLOON`, `PAGE_REPORTING` and
  `BALLOON_COMPACTION` are already `=y` in the pinned kernel's IKCONFIG
  [S].
- **Workload:** one synthetic 4 GiB allocate-and-free.
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
  BuildKit 0.34.1 for arm64. It booted on spike 1's kernel under the
  spike's own launcher. It used pinfold's init with the `builder` verb
  ungated and two architecture literals changed: 7 lines added,
  9 removed [S].
- **Build:** an adaptation of row 44's Containerfile completed in 7.85 s
  cold and 3.09 s warm; the warm run also changed the allowlist [M].
- **Conversion:** a second VM with no proxy turned the output into a
  138 MB root that booted as a box [M].
- **Egress** [M]:
  - the RUN step saw only `lo`;
  - a direct connect failed with "Network is unreachable" (its latency
    was not logged);
  - through a stub proxy on vsock, `api.github.com` was allowed and
    `example.com` refused, and allowing `example.com` let it through;
  - BuildKit's own pulls went through the proxy.
  - pinfold's real proxy was not run.
- **Not covered:** builder confinement on the Mac, host admission of
  arm64 output, and init powering off at job end; PID 1 exit still
  panicked.
- **Size:** about 255–500 changed lines plus about 85 regenerated package
  rows, against the study's 260–490 [I].
- **New items:**
  - the job must use the embedded arm64 init, not a copy of the running
    Mach-O binary;
  - pinfold's proxy needs a small adapter for VZ's per-connection
    sockets [I].
- **The Apple-store snapshot fallback is dropped.** It keeps Apple's
  unproxied build egress, so it cannot meet row 44 [I].

## Where this leaves A′

- **Linux:** free page reporting returned memory on a synthetic run
  outside pinfold's jail. Enabling it is likely a few lines [I] and
  independent of the Mac. It needs a spec line and the preview question
  settled.
- **Mac:**
  - No blocker was found for boot, builds or egress. Each was shown
    only in part: a stub host, a stub proxy, unmounted `ferrosys`
    images.
  - Memory return, the reason for the move, works only in part on a
    synthetic workload. A controller brings the footprint between cycles
    down from about 3.8 GiB to 1.1–2.3 GiB.
  - It leaves 0.41–1.85 GiB above target, and it slows bursts.
- **Before a spec proposal:** VZ's page-release path is the open
  question. Then the controller trigger and margin need a second run.
