# Linux disk reclaim, 2026-11-23

Issue #129. Question: when a box writes and then deletes files on a
pinfold disk on Linux, does the host get the blocks back?

Answer: no. Deletion, `sync`, stop and start did not shrink the image's
allocated host bytes. The box cannot trim. Host allocation grew unevenly
across rewrites, so it can climb past live data toward the disk's logical
size (64 GiB, the default, for the measured disk). Removing the disk is
what frees the space.

## Environment

- Host: the dev VM, Debian 13, Linux 6.12.105+deb13-cloud-amd64, x86_64,
  KVM. Host filesystem ext4, mounted `rw,relatime,discard`.
- pinfold 0.6.3 at 869a4740 (main), release build for
  `x86_64-unknown-linux-musl`. It was cross-built on the Mac with
  `cargo zigbuild`, because the VM's rustup had only 1.99.1 and the repo
  pins 1.100.0.
- Firecracker 1.17.0, pinned guest kernel 6.18.57 (from `doctor`).
- Default profile image, built on the VM by `pinfold build` in 168 s.
- Scratch `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` and
  `TMPDIR` under one directory in the VM's home; a fresh Git project
  configured with `config edit` (isolated disk, the Linux default).

## Disk and drive configuration

- Creation (`runtime/firecracker/mod.rs`): `set_len` to the logical size,
  then `mkfs.ext4 -E lazy_itable_init=1,lazy_journal_init=1,...`. The
  image is sparse: 68,719,476,736 logical bytes, about 7 MB allocated
  at first start.
- Drive config (`runtime/firecracker/vm.rs`): each drive sets only
  `drive_id`, `path_on_host`, `is_root_device` and `is_read_only`.
  Nothing about discard.
- Guest: `/project` is `/dev/vdd`, ext4, mounted
  `rw,nosuid,nodev,relatime,noinit_itable`. No `discard`.
- Guest: `/sys/block/vd*/queue/discard_max_bytes` and
  `discard_granularity` are 0 for every virtio disk. The device offers
  no discard.
- Guest: the box user is uid 1000 with all capability sets empty, no
  `sudo`; `su` fails.

## Measurements

The disk is the project disk, `disks/pi-proj-…-project.ext4`. Logical
size stayed 68,719,476,736 throughout. "Allocated" is
`du --block-size=1`; `stat -c '%b*%B'` agreed in every row. Each write was
`dd if=/dev/urandom bs=1M count=2048` then `sync`; each delete was
`rm` then `sync`, all through `pinfold -- sh -c`. After every delete,
`df` in the guest reported 2,105,344 bytes of filesystem usage.

| Step | Allocated bytes | Change |
|---|---:|---:|
| 1. first start | 7,114,752 | |
| 2. wrote 2 GiB | 2,154,782,720 | +2,147,667,968 |
| 3. deleted | 2,154,885,120 | +102,400 |
| 4. `fstrim -v /project` in the box | 2,154,885,120 | 0 |
| 5. `pinfold stop` | 2,154,885,120 | 0 |
| 5b. `pinfold start` | 2,154,885,120 | 0 |
| 6. cycle 2: wrote 2 GiB | 2,247,294,976 | +92,409,856 |
| 7. cycle 2: deleted | 2,247,397,376 | +102,400 |
| 8. cycle 3, same boot: wrote 2 GiB | 4,218,908,672 | +1,971,511,296 |
| 9. cycle 3: deleted | 4,219,015,168 | +106,496 |
| 10. stop, start, cycle 4: wrote 2 GiB | 4,219,179,008 | +163,840 |
| 11. cycle 4: deleted | 4,219,281,408 | +102,400 |
| 12. cycle 5, same boot: wrote 2 GiB | 4,353,634,304 | +134,352,896 |
| 13. cycle 5: deleted | 4,353,740,800 | +106,496 |

Step 4 printed `fstrim: /project: FITRIM ioctl failed: Operation not
permitted`, exit 1. With the capability it would still have had nothing
to send: the device's discard limit is 0.

`pinfold disk list --json` reported `"allocated":2247397376` after
step 7, the same as `du`.

## Conclusion

- Deletion does not return space. Each delete followed by `sync` added
  about 100 KiB of host allocation.
- Trim does not work. The box user lacks the capability. The virtio
  drives offer no discard, and pinfold's drive config asks for none.
- Stop and start return nothing.
- Host allocation grows unevenly. Of four 2 GiB rewrites after the
  first write, three added 0.16 to 134 MB of host allocation and one
  (cycle 3) added 1.97 GB. This is one run. After five cycles the image
  had 4.35 GB allocated while the guest filesystem reported 2 MB used.
  Nothing bounds the growth below the disk's logical size: 64 GiB by
  default, as here, or the spec's `size`.
- A user whose host disk is full can only remove the disk and start a
  new one. This sequence follows the spec and was not exercised here:
  1. `pinfold disk list` shows each disk's allocation and name.
  2. With the box running, `git fetch box` on the host. The box's
     branches arrive as `box/*`.
  3. Keep each wanted branch locally: `git merge box/BRANCH` on a
     local branch, or `git branch BRANCH box/BRANCH`.
  4. `pinfold stop`. For files outside Git, `pinfold disk export NAME
     DEST` now, since export needs a stopped disk.
  5. `pinfold disk rm NAME`. An attached disk is refused, hence the stop.
  6. `pinfold start` creates a new, empty disk.
  7. `git push box BRANCH` on the host. The first push creates the box
     repository; the branch lands as `host/BRANCH`.
  8. In the box: `git switch -c BRANCH host/BRANCH`.

  Removing the home disk drops the box's home directory, sessions,
  logins and caches. `clean --unused AGE` also removes disks of
  projects not run for that long.

## Not built

No trim or compaction. A host-side fix would parse a guest-written
ext4 image on the host; pinfold does not do that today (`disk export`
reads a disk through a box).

Firecracker 1.17.0 added an opt-in `discard` drive field (upstream
#6142, `docs/api_requests/block-discard.md`): for writable drives on the
`Sync` IO engine, the default, a guest discard becomes a hole punch in
the backing file. pinfold does not set it. The smallest guest-side fix
would set `discard: true` on writable disk drives and have the guest
init trim each writable disk (`FITRIM`, or the `discard` mount option),
since the box user cannot. That was not tried here.

`fstrim` was tried only as the box user. Recreating a disk after
`disk rm` and `git push box` into it were not exercised.

## Cleanup

`pinfold stop`, `pinfold disk rm --all` under the scratch roots, and the
scratch directory deleted. No Firecracker process remained. The VM's
existing checkout and state were not touched; free space returned to
132 GiB.
