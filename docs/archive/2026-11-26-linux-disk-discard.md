# Linux disk discard, 2026-11-26

Issue #446, following 2026-11-23-linux-disk-reclaim.md. Question: with
Firecracker's `discard` on writable drives and a guest trim at stop, does
the host get a disk's freed blocks back, and what does stop cost?

Answer: yes. After each of five write-and-delete cycles the project
disk's image fell back to about 7 MB allocated at stop. Before the
change it stayed at 2.25 GB. Stop took about 0.4 s longer.

## Change

- `vm.rs`: every writable drive sets `"discard": true`. Read-only
  drives are unchanged. Firecracker 1.18.0 accepts the field in the
  `--config-file` form; it was added in 1.17.0 (upstream #6142) and
  works only for writable drives on the `Sync` engine, the default. For a
  regular backing file it punches holes.
- Guest init, at UNMOUNT: after ending box work and `sync`, a `FITRIM`
  ioctl on each writable attachment, then the unmounts. A failed trim is
  ignored. The tables below used one call over the whole range; the
  bound in "Trim budget" came after review.
- The jail needed nothing new. The hole punch passed Firecracker's
  seccomp filter and pinfold's Landlock rules (read and write on the
  writable disk).

The `discard` mount option was not tried. It would discard on every
delete during the run; one trim at stop already met the goal.

## Environment

- Host: the dev VM, Debian 13, Linux 6.12.105+deb13-cloud-amd64,
  x86_64, KVM, ext4 host filesystem.
- Firecracker 1.18.0 (pinned). Default profile image built on the VM.
- Two release builds for `x86_64-unknown-linux-musl` with Rust 1.99.1:
  "before" is main at abe5481c, "after" adds the change.
- Scratch `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` and
  `TMPDIR`; a fresh Git project with the default settings (isolated
  disk, 64 GiB logical size).

## Guest view

With the change, `/sys/block/vd*/queue/discard_max_bytes` is 134217728
(granularity 512) for the two writable disks (home, project) and 0 for
the read-only root, init and profile drives. The 2026-11-23 run saw 0
for all.

## Measurements

Each cycle: `pinfold -- sh -c 'dd if=/dev/urandom of=/project/big bs=1M
count=2048 && sync'`, then `rm` and `sync`, then `pinfold stop`, then
`pinfold start`. Allocated is `stat` blocks times block size of the
project disk's image. Stop time is the wall time of `pinfold stop`. Each
binary started from a new disk.

Before:

| Cycle | Written | Deleted | After stop | Stop (s) |
|---:|---:|---:|---:|---:|
| 1 | 2,154,754,048 | 2,154,856,448 | 2,154,856,448 | 0.17 |
| 2 | 2,247,290,880 | 2,247,393,280 | 2,247,393,280 | 0.27 |
| 3 | 2,247,544,832 | 2,247,647,232 | 2,247,647,232 | 0.25 |
| 4 | 2,247,802,880 | 2,247,905,280 | 2,247,905,280 | 0.21 |
| 5 | 2,248,040,448 | 2,248,142,848 | 2,248,142,848 | 0.21 |

After:

| Cycle | Written | Deleted | After stop | Stop (s) |
|---:|---:|---:|---:|---:|
| 1 | 2,154,754,048 | 2,154,856,448 | 7,286,784 | 0.64 |
| 2 | 2,154,905,600 | 2,155,008,000 | 7,520,256 | 0.61 |
| 3 | 2,155,139,072 | 2,155,241,472 | 7,753,728 | 0.63 |
| 4 | 2,155,372,544 | 2,155,474,944 | 7,987,200 | 0.55 |
| 5 | 2,155,606,016 | 2,155,708,416 | 8,220,672 | 0.62 |

First start allocated 7,077,888 bytes in both runs. An idle stop after
the cycles took 0.11 s before and 0.35 s after. That is about 0.4 s
added per stop, for two 64 GiB disks with almost no data. Stop time on
a fragmented disk is under "Trim budget".

Before the change this run grew by 93 MB over five rewrites; the
2026-11-23 run grew by 2.2 GB. After it, the image grew by about 230 KB
per cycle, and the deleted data came back each time.

A 100 MB file written before a stop had the same SHA-256 after the trim
and a new start.

## Trim budget

Review found two gaps. The trims run inside the host's 30 s unmount
deadline, and a stop past it leaves the disk unclean. And PID 1 keeps
only `CAP_SYS_ADMIN`, with no DAC override, so it could not open a disk
root the box user had made mode 0700, and skipped its trim.

Fragmented free space was measured on the dev VM with a 32 GiB disk:
write a file from `/dev/urandom`, punch holes (`fallocate` punch-hole)
in alternate stretches of it, then time the stop from closing `box up`'s
stdin until `up` exits. ext4 forgets what it trimmed at unmount, so each
stop discards every free extent again.

| File | Holes | Extent floor | Stop after punching (s) | Next idle stop (s) | Image after |
|---|---|---|---:|---:|---:|
| 1 GiB | 4 KiB | none | 21.9 | 20.3 | 548 MB |
| 1 GiB | 4 KiB | 1 MiB | 0.16 | 0.12 | 1,084 MB |
| 8 GiB | 1 MiB | 1 MiB | 1.47 | 0.68 | 4,303 MB |

An idle stop of the new disk took 0.09 to 0.12 s. Without a floor, a GiB
of 4 KiB holes cost about 20 s at every stop, about 150 us a discard.
An earlier 8 GiB run of 4 KiB holes, with 1 GiB ranges and the 20 s
cutoff but no floor, was stopped with `box down`. It exited with `box
teardown deadline exceeded` after its own 10 s wait, and the disk was
left unclean. Why the stop did not end clean was not established; at
about 20 s a range, a second range started just before the cutoff would
pass the host's 30 s.

So the trim now:

- issues `FITRIM` 128 MiB at a time, one ext4 block group, and starts no
  range after 20 s from UNMOUNT, which includes the 10 s for box work to
  end; the unmounts keep the rest of the host's 30 s (ranges were 1 GiB
  until the second review, below);
- sets `minlen` to 1 MiB, so one range sends at most 128 discards. The
  1 MiB holes above cost about 0.18 s per GiB on the first stop and
  0.08 s after;
- uses a descriptor on each writable root that init opens at mount, while
  it still holds every capability. The exec service, forked after, closes
  its copies; PID 1 closes them after the trims, before unmounting.

Free space in pieces under 1 MiB stays allocated on the host. Deleting
whole trees mostly frees large extents: four copies of the image's
`/usr` (17,136 files, 763 MB) took the image from 6.8 MB to 837 MB, and
after `rm -rf` and a 0.22 s stop it held 37 MB.

## Second review: range size and `box down`'s wait

Review found that a range started just before the 20 s cutoff could run
past the host's 30 s, and that `box down` waited only 10 s for an owner
whose teardown may now take most of the unmount's 30.

Ranges went from 1 GiB to 128 MiB. With the 1 MiB floor a range is then
at most about 128 discards instead of 1024. `box down` on Linux now
waits 40 s: the unmount's 30 s plus the 10 s it waited before, for the
rest of teardown. Claim acquisition keeps its 10 s. On macOS nothing
changed.

Same dev VM, 32 GiB disk, release build of the change. Each run writes
8 GiB of zeros, punches holes with `fallocate` in a stride across the
whole file, syncs, then times `box down` until `box up` exits. Then a
read-only box attaches the disk, and a writable box with no work is
stopped by closing stdin.

| Build | Holes / kept | `box down` (s) | Image after | Read-only up | Next idle stop (s) |
|---|---|---:|---:|---|---:|
| 1 MiB floor | 1 MiB / 4 KiB | 4.05 | 104 MB | ready | 2.45 |
| no floor | 4 KiB / 4 KiB | 20.50 | 8,167 MB | ready | 22.91 |

The first row is the densest layout the floor still trims: about 1,020
discards per GiB, twice the archive's 1 MiB-hole run. It cost about
0.5 s per GiB on the first stop and 0.3 s after, so about 0.06 s per
128 MiB range. A 64 GiB disk at that density would take about 32 s, so
the cutoff, not the disk's end, would stop that trim; the rest stays
allocated until a later stop.

The second row repeats the earlier failed case, 4 KiB holes over 8 GiB
without a floor, with a temporary build that sets `minlen` to 0. Each
range then sends about 16,000 discards and takes about 2.8 s. The trim
reached about 430 MB before the cutoff. `box down` exited 0 after
20.5 s, past the old 10 s wait, and the disk was clean: the read-only
box came ready. The idle stop's 22.9 s is the cutoff plus one range
started just before it, still about 7 s inside the 30.

An idle stop of a box with two fresh 64 GiB disks took 0.26, 0.32 and
0.27 s, so 512 ranges a disk cost no more than the single whole-disk trim in "Measurements" (0.35 s).

## Test

Row 11's test (`disks_persist_and_attach_exclusively`) was extended: on
Linux the recovery run writes and deletes 256 MiB on the 1 GiB disk, and
after its stop the image's allocation must be at least 240 MiB below its
allocation with the file written. The write itself must grow the image
by at least 240 MiB, not 256: the first run on the VM grew it by
268,419,072 bytes, 16 KiB short of the file, since the file reused
blocks already allocated.

On the VM the extended test passed in 43 s (default image cached). With
the `discard` field removed from `vm.rs` it failed at the reclaim check,
269,426,688 bytes with the file written and 269,467,648 after the stop.

After review the test samples allocation after the delete as well, while
the box is live, and requires the drop across the stop; a disk mounted
with continuous discard would fail it. Before the stop the box makes the
disk's root mode 0700.

With 128 MiB ranges the 256 MiB file cannot sit in one range, so a trim
that stopped after its first range would reclaim at most 128 MiB, under
the 240 MiB the check needs. The disk stays 1 GiB. On the VM the test
passed (210 s, cold default image). With `trim` returning after its
first range it failed at the reclaim check, `269467648 -> 269467648`.
With the `discard` field removed, in a separate build from the first
sabotage's, it failed at the same check with the same numbers: neither
trimmed any of the file's blocks.

## Cleanup

`pinfold stop`, `disk rm --all` under the scratch roots, the scratch
directory and the clone deleted, after each round. In the second round
every box of the scratch roots had ended with `down` before the
deletion; the one Firecracker process seen then belonged to another
session's checkout on the same VM and was left alone.
