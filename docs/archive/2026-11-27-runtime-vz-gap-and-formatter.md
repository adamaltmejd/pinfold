# VZ balloon gap and the Mac disk formatter (2026-11-27)

This follows 2026-11-27-runtime-spikes.md. It does two things:
- explains spike 2's reclaim gap, from a probe run the same day;
- records the formatter decision that replaces spike 3's
  recommendation.

The raw material is in `~/code/pinfold-runtime-study-2026-11-27/vzgap/`.
Marks follow the study.

## The gap

**The rule.** When the balloon inflates, VZ frees guest pages that the
host has already compressed or swapped. Pages still in host RAM stay
resident. The gap is the ballooned memory that was still in RAM at the
moment of the shrink.

**Method.** 16 VMs at 4 or 6 GiB, measured with `footprint`, `vmmap`,
and a guest tool that reads `/proc/kpageflags` and `/proc/vmstat`.
`results.txt` holds the tables.

**What the gap is not:**
- **VZ outside guest RAM:** 26–29 MiB in every snapshot [M].
- **The guest:** its balloon page count matched the target exactly,
  every time [M].
- **Page size:** stranding from mixed page sizes is at most 83 MiB at
  2 MiB grain [M].

**The evidence for the rule** [M]:
- **Shrink right after a burst:**
  - the footprint fell by exactly as much as the compressed amount fell
    (1297 against 1299 MiB, and 1710 against 1716);
  - resident memory did not move;
  - 1.1–1.5 GiB was left above target.
- **Shrink 5, 20 or 60 s after the free:** by then the host had
  compressed nearly all of it (67, 33 and 12 MiB still in RAM), and no
  gap was left.
- **Stepwise inflation** still left 1107 MiB, so inflation rate is not a
  factor.
- **Deflate-on-OOM** is not a separate mechanism:
  - after three OOM cycles on already-compressed memory, no gap was
    left;
  - a re-shrink seconds after each burst left about 1.0 GiB every
    cycle, and the amount did not grow.

**Real or accounting.** The stranded pages are real: they are neither
reclaimable nor volatile, and the host later compressed them while the
footprint stayed the same [M]. Simulating host pressure needs root, so
no simulated pressure was applied. The test data was random, so it
compressed poorly. Real data compresses better, so stranded pages cost
less [I].

**Not explained:**
- Why VZ frees compressed pages but not resident ones.
- Spike 2's late footprint drops.
- The 16 KiB guest kernel's larger residual.

libkrun's REPLACE mode does free resident pages, so the platform allows
it [M].

## What closes it

| Approach | Result |
|---|---|
| Reissue the same target | no change, 1319 → 1319 MiB [M] |
| A partial pulse, to 512 MiB and back | recovered 230 of 1319 MiB [M] |
| Restore to full size, then shrink again | gap closed, 2 of 2 [M]; the stranded pages were already compressed both times |
| Shrink only once freed memory has been idle about 5 s | no gap, no restore [M] |

**Controller design: shrink only after a delay.**
- The controller shrinks only once the guest's freed memory has been
  idle for about 5 s.
- If a shrink still leaves a gap, it restores the target once and
  shrinks again after the host has compressed the pages. That takes
  5–30 s at full guest size.
- On a host under no pressure, the freed pages may stay in RAM, but
  then the memory is not needed. The controller acts only under
  pressure.

The figures above are from this pressured Mac. On an idle Mac, how fast
the host compresses pages, and so how long the controller must wait, is
unmeasured.

## Mac disk formatter

Spike 3 compared two pure-Rust formatters and recommended `ferrosys`.
That is replaced. A formatter writes the on-disk format of the
persistent home and project disks, so its bug would silently lose data.
`ferrosys` is four months old and has one maintainer.

Under A′, pinfold boots Linux VMs on the Mac anyway, and spike 5's
converter already wrote a root image there. So the real `mkfs.ext4`
runs inside a pinfold VM:
- **Empty disks** are formatted by a small VM at creation. One boot,
  about 0.4 s [I], only when a disk is created.
- **Populated images** are formatted by the converter VM.
- **The same path can serve Linux,** so there is one formatter on both.
- **Cost:** e2fsprogs is pinned in the builder root.

The filesystem stays ext4. btrfs is off in the guest kernel. CoW gains
for worktrees are unmeasured, since Git worktrees already share objects.
If reflinks are ever measured to matter, XFS comes first: it is already
in the kernel (`CONFIG_XFS_FS=y`), reflinks are on by default, and its
tools are mature.
