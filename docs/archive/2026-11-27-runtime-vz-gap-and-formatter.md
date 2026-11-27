# VZ balloon gap and the Mac disk formatter (2026-11-27)

This follows 2026-11-27-runtime-spikes.md. It does two things:
- explains spike 2's reclaim gap, from a probe run the same day;
- records the formatter decision that replaces spike 3's
  recommendation.

The raw material is in `~/code/pinfold-runtime-study-2026-11-27/vzgap/`.
Marks follow the study.

## The gap

**The likely rule** [I]: when the balloon inflates, VZ frees guest pages
that the host has already compressed or swapped. Pages still in host RAM
stay resident. The gap is the ballooned memory that was still in RAM at
the moment of the shrink.

The logs show aggregate accounting consistent with this. They cannot
show which pages VZ released, or fully rule out effects of inflation
rate or OOM history. "No gap" below means no positive excess over the
allowed memory, which is a lower bound on retained pages.

**Method.** 16 VMs at 4 or 6 GiB, measured with `footprint`, `vmmap`,
and a guest tool that reads `/proc/kpageflags` and `/proc/vmstat`.
`results.txt` holds the tables.

**What the gap is not:**
- **VZ outside guest RAM:** 8–29 MiB in every snapshot [M].
- **The guest:** its balloon page count matched the target at every
  steady snapshot [M]. During deflate-on-OOM the allowance rose to about
  2.2 GiB against a 1 GiB target, as expected.
- **Page size:** at most about 83 MiB sat in partly ballooned 2 MiB guest
  chunks [M].

**The evidence for the rule** [M]:
- **Shrink right after a burst:**
  - the footprint fell by about as much as the compressed amount fell
    (1297 against 1299 MiB, and 1711 against 1716);
  - estimated resident memory rose by 2 and 6 MiB;
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

**Real or accounting.** The stranded pages look real: they are neither
reclaimable nor volatile, and the host later compressed them while the
footprint stayed about the same [M]. Simulating host pressure needs root, so
no simulated pressure was applied. The test data was random, so it
compressed poorly. Real data compresses better, so stranded pages cost
less [I].

**Not explained:**
- Why VZ frees compressed pages but not resident ones.
- Spike 2's late footprint drops.
- The 16 KiB guest kernel's larger residual.

libkrun's REPLACE mode replaces the mapping [S], and its footprint fell
to 286 MB [M], so the platform allows freeing resident pages.

## What closes it

| Approach | Result |
|---|---|
| Reissue the same target | excess unchanged at 1319 MiB [M] |
| A partial pulse, to 512 MiB and back | recovered 230 of 1319 MiB [M] |
| Restore to full size, then shrink again | gap closed, 2 of 2 [M]; full size held about 5 and 18 s, with the stranded pages mostly compressed |
| Shrink 5 s after the free | no gap, no restore, in one manual trial [M]; that run also held the allocation 3 s longer |

**Controller design to test: shrink only after a delay** [I].
- The controller shrinks only once the guest's freed memory has been
  idle for a few seconds.
- If a shrink still leaves a gap, it restores the target once and
  shrinks again after the host has compressed the pages. How long that
  takes is unmeasured. Spike 2 saw full restores let the footprint climb
  to 4.28 and 4.66 GiB, so a restore has to be short.
- On a host under no pressure, the freed pages may stay in RAM, but
  then the memory is not needed. The controller acts only under
  pressure.

The figures above are from this pressured Mac, with random data. How
fast an idle Mac compresses pages, and how much real data saves, is
unmeasured.

## Mac disk formatter

Spike 3 compared two pure-Rust formatters; its agent favoured
`ferrosys`. Neither is taken. A formatter writes the on-disk format of
the persistent home and project disks, so its bug would silently lose
data. `ferrosys` is four months old and has one maintainer, according to
the spike agent's report.

Under A′, pinfold boots Linux VMs on the Mac anyway. So the real
`mkfs.ext4` runs inside a pinfold VM [I]:
- **Empty disks** (home, project, `/tmp`): a VM formats one template
  image per kind, once. Each new disk is an APFS clone of its template,
  which the gap probe already did for its scratch disks. `/tmp`, made
  new at every start, then costs a clone rather than a VM boot.
- **Populated images:** the converter VM formats them. Spike 5 formatted
  one output root this way; its other disks came from a container.
- **Linux** keeps host `mkfs.ext4`, which it already has.
- **Cost:** e2fsprogs in the builder root, and a template cache.

The filesystem stays ext4. btrfs is off in the guest kernel config
[S]. CoW gains
for worktrees are unmeasured, since Git worktrees already share objects.
If reflinks are ever measured to matter, XFS comes first: it is already
in the kernel (`CONFIG_XFS_FS=y` [S]), `mkfs.xfs` enables reflinks by
default [S], and its tools are mature.
