# /tmp scratch disk design (#506)

Always applied gave every box a tmpfs `/tmp`, held in guest memory. A
Firecracker box has 1 GiB by default, so large builds there fail or
pressure the VM. RRT worked around it with `TMPDIR` on the home disk
(5–11 GiB per gate run), where nothing removes the data. The property
kept is a fresh, empty, private `/tmp` per box. It now costs host disk,
not guest RAM.

## Decisions

- **Not a Disk.** Scratch is per-start box state, like Linux mount
  images. No claim, clean mark, `disk list`, `rm --all`, `export` or run
  record entry. Crash recovery comes from existing box cleanup: the Linux
  image lives in the generation's state directory, and the Apple volume
  is removed with the box and before each start of the same name.
- **Create at each start, never wipe.** Creation is cheaper than a wipe
  and is empty by construction.
- **Fixed 64 GiB, sparse, no key.** This matches the disk default. Adam
  chose no key on 2026-11-27. Add a key when a box needs another size.
- **Caller changes.** On Linux `/tmp` takes one of the 14 attachments,
  so a spec with 14 is now refused `spec`. That limit is the virtio
  device count, so it cannot grow. Guest paths below `/tmp` are now
  refused `spec`, because a nested attachment would make `/tmp` non-empty
  and its mount point root-owned.
- **Apple readying uses step 6's root invocation.** That invocation now
  runs on every Apple start, not only egress boxes, and it also readies
  `/tmp`: it removes the empty `lost+found` and sets mode 1777. The
  rejected alternative was step 2's preparation box at each start. It adds
  no root operation, but it boots a second VM per start (about 0.7 s
  against about 70 ms). Adam: build the best long-term design, and spec
  changes are fine.
- **Apple root operation checks before it acts.** An image symlink can
  alias a declared share or disk to `/tmp`. Validating only that the path
  is a mount root would then chmod a share's top on the host. The
  operation requires ext4, a mount root owned 0:0 with mode 0755, and
  only an empty root-owned `lost+found`. Shares are virtiofs, and disk
  roots are owned by the box user.
- **Apple volume named by box only.** `pinfold-tmp.<box name>`, with no
  state root key. Box names are claimed per user across the host, and
  prune can run from any state root. The volume's removal is part of
  confirmed runtime removal, so a failed `volume rm` keeps the record and
  prune retries.
- **No online discard.** ext4 reuses freed blocks, so host allocation
  tracks peak use, bounded by 64 GiB, until the box is removed. Measure
  before adding `discard`.
- **Builders keep tmpfs.** Image preparation VMs have their own work
  disk and are not agent boxes.
- **Linux journal off.** A journal does nothing for data that is
  discarded at stop, and it allocates about 0.5 GiB per start.
- **Mount flags unchanged.** Apple's tmpfs `/tmp` and an Apple volume
  both mount `rw,relatime`. Linux keeps `nodev`.

## Measurements (2026-11-27)

| Step | Host | Time | Allocated |
|---|---|---|---|
| `container volume create -s 64G` | Mac, container 1.7.1 | 38 ms | 6.5 MiB |
| root `container exec` running `chmod 1777 /tmp`, `rmdir lost+found` | Mac | 67 ms | — |
| `container run --rm` (preparation-box cost) | Mac | 0.7 s | — |
| `mkfs.ext4 -m 0` on a sparse 64G file, with journal | dev VM, e2fsprogs 1.47.2 | 35–39 ms | 519 MiB |
| the same, `-O ^has_journal` | dev VM | 29 ms | 4.8 MiB |

A fresh Apple volume's root is `0:0 755`, so the box user cannot write to
it. Under `--cap-drop ALL`, a root exec can still chmod it because root
owns it. An anonymous volume (`-v /tmp`) was rejected for two reasons:
`--rm` leaves it behind, and its size is fixed at 504G.

## Cost

On Linux the scratch image takes one of the 14 attachments. `/tmp`
contents now sit on the host disk until the box is removed, and until
`box prune` after a crash. Before, they were lost with the VM.
