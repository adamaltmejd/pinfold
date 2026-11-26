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
  ioctl over the whole range on each writable attachment, then the
  unmounts. A failed trim is ignored.
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
a full or fragmented disk, where a trim sends more discards, was not
measured. The trim runs inside the 30 s unmount deadline; a stop that
exceeds it leaves the disk unclean.

Before the change this run grew by 93 MB over five rewrites; the
2026-11-23 run grew by 2.2 GB. After it, the image grew by about 230 KB
per cycle, and the deleted data came back each time.

A 100 MB file written before a stop had the same SHA-256 after the trim
and a new start.

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

## Cleanup

`pinfold stop`, `disk rm --all` under the scratch roots, the scratch
directory and the clone deleted. No Firecracker process remained.
