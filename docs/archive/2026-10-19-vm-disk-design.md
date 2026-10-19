# VM disks, push, fetch and export: design

2026-10-19. The rationale for the spec change that replaces writable
mounts with disks. The evidence is the Firecracker prototype
(`2026-10-19-firecracker-linux-prototype.md`, PR #111).

## Decisions

Adam, 2026-10-19:
1. Copy-in with git push-back also covers interactive `pinfold pi`.
2. The same model on macOS and Linux.
3. yard-sthlm gets `/dev/kvm`.
4. The middle path: users can move changes in and out through a defined
   protocol, and Yard runs gates on a lane's files while its implementer
   VM is down.

## Why one-way

A host write into a box is safe: the guest reads only what the host chose
to send. A guest write onto the host is what the threat model fears. The
read-only `.git` mount, `protect`, the `mount-alias` scan and rows 11 and
22 exist only to contain it. Removing every writable host mount removes
that whole class, and the controls with it:
- `push` is host to box and may run as often as wanted.
- `fetch` is box to host, as git refs under a remote namespace. A bundle
  carries objects and refs, no hooks and no config. The prototype showed
  planted hooks, `core.hooksPath`, `core.fsmonitor` and tracked hook files
  staying inert through fetch, merge and checkout.
- `export` covers non-git output, into a new directory, regular files
  only.

## Why disks outlive boxes

Yard needs an implementer's files to stay after its VM stops, so a gate
can run on them. A disk is the unit that persists. The host must never
read one: a loop mount parses a guest-written ext4 in the host kernel and
needs root, and any reader would hand planted hooks and symlinks to the
host as files. So a disk is only ever read by a box:
- in-box gates and review: a fresh box attaches the lane's disk read-only;
- host gates (the Mac suite, anything needing KVM or the real runtime):
  `fetch` into the lane's clone, as Yard already works from clones.

ext4 cannot be shared writable between kernels, and a read-only mount of
a disk that needs journal recovery fails on a read-only drive. Hence one
writer, many readers, and `disk-unclean` until a writable attachment
recovers a disk whose box was killed.

## Alternatives not taken

- Live mounts with a guest kernel: cloud-hypervisor or libkrun with
  virtio-fs. They keep today's workflow but bring back the host daemon
  surface and every mount control.
- Continuous two-way sync over vsock: edits look live, but the guest
  writes the host checkout again.
- Host reads of a stopped VM's disk: see above.

## Gaps

- One pi box per project; a second needs its own project disk.
- `push` never deletes. A deletion on the host reaches the box only by
  git (`push --repo`, then a merge in the box).
- Read-only host directories (harness installs, profile `share/`) become
  per-start snapshots. On Linux each is a disk image per start, at a cost
  not yet measured; Apple may keep them live.
- `push`, `fetch` and `export` with no running pi box start a transient
  box without egress, about a second each in the prototype.
- `pinfold profile new --from-project` now exports through a box.
- Project home caches are inside the home disk; `clean` cannot trim them.
