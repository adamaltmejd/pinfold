# Apple disk checks for #134 (2026-10-20)

Apple `container` 1.5.0 on the M5, rerun before building disks. This
settles the review note's two readings of read-only sharing
(`2026-10-20-vm-disk-spec-review.md`, Apple checks).

- Two boxes attach one volume read-only at once
  (`--mount type=volume,source=V,target=/d,readonly`). Both show
  `/dev/vdc /d ext4 ro`, both read the file a writer left, and `touch`
  fails with `Read-only file system` in each.
- While those two hold it, a writable attachment fails at VM start: "The
  storage device attachment is invalid." pinfold's `disk-in-use` check
  must come first.
- A stopped container that names a volume blocks `volume rm` ("currently
  in use"); pinfold removes its boxes at `down`, so this only bites a
  leftover.
- A writer killed mid-append (`container kill --signal KILL`, then `rm`)
  leaves a volume the next box mounts read-only with every line written.
  So a successful runtime stop is a clean detach on Apple.
- A fresh `volume create -s 64G` allocates 10 MB on the host.
- `--cap-drop ALL` applies to `container exec --user 0:0` too, and exec
  takes no capability flags: the root exec cannot `chown` a new volume's
  root (EPERM). A transient `container run --rm --user 0:0 --cap-drop ALL
  --cap-add CAP_CHOWN --network none` of the box's image gives it to the
  box user instead, at creation, outside the box (spec step 2).
