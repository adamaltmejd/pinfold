# Root and init disk boot check (2026-10-20, #138)

`image::root_disk`, `artifacts::init_disk` and `artifacts::vmm` from PR
#149 (head 8f862d8, with #150 and #151 merged), run on a labvm lease and booted under Firecracker. Nothing calls them
before the Firecracker runtime (#141), so `scratch-disks.patch` adds a
throwaway `pinfold __disks ID INIT` that calls the three and prints their
paths. It was never committed. `boot.sh` is the whole run; `out.txt` its
output from the build on. Two boots: the image's
`/bin/sh` as init with the init disk at `/mnt`, then the merged Linux init
(#151) as PID 1 from the init disk, driven by #151's throwaway host side
`fcinit.py`.

Host: Debian 13, kernel 6.12, nested KVM, podman from Debian, the CLI built
for `x86_64-unknown-linux-musl` (static). The image is
`debian:trixie-slim` plus a file owned by 1234:2345.

| Check | Result |
|---|---|
| Linux clippy, `-D warnings`, before the patch | clean |
| `vmm` installs Firecracker v1.17.0 and the pinned kernel into `artifacts/vmm/1.17.0+6.18.57/linux-x64/` | yes; `artifacts` lists it, `cached: true` |
| root disk from the image, inside podman's user namespace | 3.4 s for all three; 109 MiB, 76 MiB allocated |
| boots read-only as `/dev/vda` | `/` refuses a write: read-only file system |
| image owners kept | `/etc/boot-marker` is `1234:2345` in the guest |
| init disk as `/dev/vdb`, read-only | `/mnt/pinfold --version` prints `pinfold 0.2.7`; a write is refused |
| guest kernel | `uname -r` is `6.18.57` |
| #151's init boots from the init disk (kernel root, `/dev` and `/mnt` present) | `up` prints `ready` |
| its exec runs in the root disk | `uname=6.18.57`, `marker-owner=1234:2345`, `id=1000:1000`, `/etc` refuses a write |
| `down` | `clean` |
| `image rm` removes the root disk | `root/` is empty after it, staging included |

The first run failed: Debian keeps `mkfs.ext4` in `/usr/sbin`, off a user's
PATH. The disk writers now use the preflight's lookup (PATH, `/usr/sbin`,
`/sbin`). The review of #149 then found the init disk lacked the `/dev` and
`/mnt` #151's init needs; the second boot covers that path.

The init disk holds the debug build (75 MiB with debug info); a release
CLI is smaller.
