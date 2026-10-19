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

## Revision after the Codex reports

Same day. Inputs: Codex's Firecracker prototype of 2026-10-16 (jailer,
VMM as uid 60001, 42 observations), its worktree isolation investigation
and the sandbox comparison, all dated 2026-10-19 and kept outside this
repository (`~/.codex/worktrees/2007/.../2026-10-19-firecracker-runtime-investigation.md`,
`~/.codex/worktrees/7459/.../2026-10-19-worktree-isolation-investigation.md`,
`/Users/adam/Code/pinfold/docs/archive/2026-10-19-sandbox-comparison-and-recommendations.md`).

### VMM confinement

Firecracker's production guidance requires the jailer or equal
confinement. The first draft ran the VMM unconfined as the user. Two
options:

| | (a) pinfold's own rootless jail (specified) | (b) a privileged fixed-function launcher (Codex) |
|---|---|---|
| Mechanism | user, mount, PID, net, IPC, UTS namespaces; `pivot_root` to an empty tmpfs; bounding set 0; `no_new_privs`; groups reduced to the `kvm` group (the bind-mounted `/dev/kvm` is still root:kvm 0660, and the jail cannot `mknod` and `chown` it as the stock jailer does); Landlock; Firecracker's seccomp | root-installed launcher takes resource IDs, no caller paths; runs the stock jailer; VMM as a dedicated uid (60001) |
| A compromised VMM reaches | the jail: kernel, root and init disks (ro), this box's disks (rw, already guest-writable), `/dev/kvm`, the vsock dir; as the user's uid | the same jail, as a uid that owns nothing |
| A further escape lands as | the user (needs a host kernel bug in userns, Landlock or KVM) | uid 60001, unless the launcher itself has a bug, which is root |
| Install | none beyond today's `kvm` group | a setuid or root-daemon launcher on every host; a Caller change |
| Disk layout | disks stay `~/.local/state/pinfold/disks`, user-owned 0600 | disks must move to root-protected state or be chowned per run; the user loses direct access while a box runs; the vsock dir must be shared between the VMM uid and the user's proxy |
| Owner death | `PR_SET_PDEATHSIG` survives (no setuid) | cleared across setuid; needs Codex's cgroup record and guardian |
| Host requirement | unprivileged user namespaces and Landlock; Ubuntu 23.10+ restricts userns through AppArmor by default | root at install |

Recommendation: (a). It keeps pinfold rootless, which is the reason the
Linux backend exists in this shape; a root launcher reachable by every
local user is new attack surface of its own, and (b) reshapes disks and
ownership. What (a) gives up is a distinct uid: a VMM escape that also
beats namespaces and Landlock is the user. Adam decides.

The stock jailer left `CapBnd=000001ffffffffff` (Codex). Under (a) the
VMM drops its bounding set inside its user namespace, which holds
`CAP_SETPCAP` there; the rule is bounding set 0, `no_new_privs` and only
the `kvm` supplementary group before exec, asserted in row 37. Under (b) the
launcher would have to drop it before setuid.

### Control channel

Codex's guest-origin connection to the control channel timed out
(errno 110), which is not a refusal. A timeout means the peer was
unreachable, so the refusal branch never ran. Two cases differ:
- Guest to CID 2 on a port other than 3128: no `<uds>_PORT` listener
  exists, so Firecracker resets the connection at once. Observable;
  row 1 asserts it with port 3128 as the control.
- Guest to its own exec listener: reachable only through vsock
  loopback in the guest kernel. The listener refuses any peer but CID 2
  with a `peer` frame. Whether the pinned kernel has loopback is an open
  question; if it does, row 1 gains that case.

### Guest PID 1

No verb hot-attaches a disk: disks attach at `up`, at boot. So mounting
needs no lasting privilege. Unmounting at `down` does: `umount`,
remount-ro and `FIFREEZE` all need `CAP_SYS_ADMIN`, and only an unmount
marks an ext4 journal clean. Codex dropped PID 1 to uid 1000 after boot
and relied on host-side fsync; from ext4's view every teardown was then
unclean.

- A (specified): PID 1 stays root but keeps only `CAP_SYS_ADMIN` and
  answers only the unmount request. The exec service, which handles all
  workload input, is its child at the host uid with every set empty.
- B: PID 1 drops to the host uid after mounting. `down` syncs and the
  host kills the VM. Every disk then needs journal recovery, so a
  read-only attachment is refused after every writer, and row 11's
  shared readers go.
- C (unverified): hold the mounts in a child mount namespace that dies
  at `down`. Whether ext4 then marks itself clean without a privileged
  unmount is untested.

A keeps Yard's read-only gate attachments. B buys a guest with no root
process at the cost of those. Adam decides.

### Secrets

Codex's rule is adopted: no secret value in a kernel command line, VM
configuration, image layer, disk or persisted request. On Linux the
spec's env travels over the exec connection after boot. Row 6 extends.
The rule binds pinfold's machinery; a guest process that receives a
secret can still write it down.

### Acceptance trial and apply

Codex's worktree investigation reproduced two apply failures with
generic tools: an atomic host save made after git read the target was
overwritten, and killing a 10,000-file apply at its first mutation left
19 files missing, with retry failing. #112 leaves apply to the caller,
so pi needs its own: `fetch` plus the user's git merge is safe; a
file-level apply is not specified. The caller-workflow trial (Open
questions) gates replacing podman.

### Follow-ups, not in this change

- `exec` and `down` targeting an expected `run_id`, so a stale handle
  cannot act on a new box with the same name.
- Exec metadata apart from guest output (`2026-10-19-run-records-spec.md`
  stage 3: the `--diagnostic-fd`), so exit 3 from a guest is not "no box".

### Decided

Adam, 2026-10-19: VMM confinement (a), the rootless jail; guest PID 1
option A. Preflight names Ubuntu 24.04+'s AppArmor userns restriction
(`kernel.apparmor_restrict_unprivileged_userns=1`) with its own token,
`userns-apparmor`. gominimal/minimal ships an AppArmor profile for the
same case; pinfold only names the cause for now.
