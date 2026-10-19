# Firecracker Linux prototype

2026-10-19. A bounded prototype of one new Linux guarantee, "each box has
its own guest kernel", with Firecracker in place of rootless podman. macOS
stays on Apple `container`. No spec, no shipped code and no dependency
changed. The scripts and raw output are in
[2026-10-19-firecracker-linux-prototype/](2026-10-19-firecracker-linux-prototype/).

## Verdict

**Go on the mechanism, conditional on one spec decision.** Rootless
Firecracker kept every existing control that applies, with pinfold's own
proxy and egress log unchanged, and reached first exec as fast as podman
`box up` on the same host. What it costs is not speed or isolation. It is
live mounts: a box sees a copy, and results come back only through git.
That is a product change on Linux, and either a divergence from macOS or a
change there too. Adam decides that before any implementation ticket.

If live mounts must stay, this is a no-go: Firecracker has no shared
filesystem, and adding virtio-fs would mean a different VMM.

## KVM hosts

| Host | `/dev/kvm` | Notes |
|---|---|---|
| labvm (Debian 13, KVM guest on the homelab node) | yes, nested | i5-9500T, VT-x exposed; `agent` added to `kvm`. All runs below. |
| GitHub x86_64 hosted runner | per the brief, not verified here | Needs a throwaway workflow under `.github` (protected). Third-party runner docs note `/dev/kvm` may need a udev rule for the runner user. |
| GitHub arm64 hosted runner | unverified, likely no | Third-party providers list nested virtualization as unsupported on arm64. |
| yard-sthlm (unprivileged LXC) | no, proposed below | Not applied. Needs Adam's approval. |

### yard-sthlm: proposed, not applied

An LXC shares the node's kernel, so it uses the node's KVM directly; no
nested virtualization is needed unless the Proxmox node is itself a VM.
Node prerequisite: VT-x or AMD-V enabled in firmware and `kvm_intel` or
`kvm_amd` loaded (`ls -l /dev/kvm` on the node).

Preferred, Proxmox VE 8.1+ container device passthrough (the node's PVE
version was not checked), in
`/etc/pve/lxc/<vmid>.conf`:

```
dev0: /dev/kvm,gid=<kvm gid inside the CT>,mode=0660
```

Fallback for older PVE, raw LXC keys plus a gid mapping (host kvm gid
`H`, container kvm gid `C`; `/etc/subgid` on the node gains `root:H:1`):

```
lxc.cgroup2.devices.allow: c 10:232 rwm
lxc.mount.entry: /dev/kvm dev/kvm none bind,optional,create=file
lxc.idmap: u 0 100000 65536
lxc.idmap: g 0 100000 C
lxc.idmap: g C H 1
lxc.idmap: g C+1 100000+C+1 65535-C
```

Then add the Yard user to `kvm` in the container. Only the rootless mode
below can work there: the jailer calls `mknod`, which an unprivileged
container refuses. Unverified.

## What was built

Pins (sha256 in `out/fetch.txt`):
- Firecracker and jailer v1.17.0, x86_64 release tarball, checksum
  verified against the release's `.sha256.txt`.
- Guest kernel: Firecracker CI's `vmlinux-6.1.188`
  (`firecracker-ci/20261016-5732b8331e1e-0/x86_64/`). It has virtio-blk,
  vsock and ext4 built in, and no `vsock_diag`.
- Host: Debian 13, kernel 6.12.107, podman 6.1.4 built as
  `scripts/setup-linux-runner.sh` builds it, pinfold at `7464468` (musl
  release).

### Root disk from the profile image

`build-rootfs.sh`:
1. `pinfold build --profile default` (unchanged pipeline).
2. One layer over it (`guest.Containerfile`): `socat`, `procps`,
   `iproute2`, the exec agent, an init, an attach shell, then the harden
   step again (no setuid files).
3. `podman create` and `podman export`, extracted under `podman unshare`.
4. `mkfs.ext4 -d` under `podman unshare`, so files keep their in-image
   owners. 210 MiB of files, a 316 MiB image.

Steps 2–4 took about 6 s (2 s layer, 4 s export and mkfs). The apt step
may have been served from build cache; the export and mkfs are the
per-profile-change cost either way.

### The backend

`fcbox.sh` (lifecycle), `pf-init` (guest PID 1), `pf-agent.rs` (std-only
Rust, both ends of exec), `pf-shell` (attach shell). Two modes:
- **Jailer** (`FC_JAIL=1`): `sudo jailer` with chroot, a new PID
  namespace and a cgroup; Firecracker runs as uid 1000 in the chroot. Needs
  root at three points: the jailer, a bind mount of the proxy socket into
  the chroot, and teardown.
- **Rootless** (`FC_JAIL=0`): Firecracker runs directly as the user, who
  is in `kvm`. No sudo anywhere. Firecracker still installs its own seccomp
  filters; the chroot, PID namespace and cgroup are lost.

(a) **Copy-in.** `mkfs.ext4 -d PROJECT` makes the box's second disk,
`.git` included, sized at the tree plus 50% plus 256 MiB. Nothing of the
host is mounted in the guest. Results leave through `git bundle create -
--all` over exec's stdout; the host runs `git fetch BUNDLE
'refs/heads/*:refs/remotes/box/*'`. A bundle carries refs and objects, no
hooks and no config.

(b) **Proxy-only egress.** No network interface is configured, so the
guest has only `lo`. A guest connection to CID 2 port 3128 makes
Firecracker connect to `<uds>_3128`, which is pinfold's proxy socket of
a running podman `box up` (the "proxy donor"): a bind mount in jailer
mode, a symlink rootless. In the guest, `socat` relays `127.0.0.1:3128`
to that vsock port, as `pinfold init` relays to the unix socket today.
The proxy, its allowlist and its egress log are pinfold's, unmodified.

(c) **Exec.** The host connects to Firecracker's `uds`, sends `CONNECT
1024`, and speaks a small framed protocol: request (workdir, argv), stdin
frames and EOF one way, stdout, stderr and the exit code the other. The
guest agent runs each command under `setpriv` as uid 1000 with an empty
bounding set and `no_new_privs`, in its own process group. If the
connection drops before the exit frame, the agent kills the group.
Attach is `CONNECT 1025` to a `socat` pty running `pf-shell`. Readiness
is the guest connecting once to host port 1000 after its listeners
report up.

## Controls checked

`checks.sh`, run in both modes; all 16 passed in both
(`out/checks-jail1.txt`, `out/checks-jail0.txt`).

| Check | Observation |
|---|---|
| Own kernel | host `6.12.107+deb13-cloud-amd64`, guest `6.1.188` |
| No NIC | `ip -o link`: `lo` only |
| No direct route | `curl --noproxy '*'`: exit 6, cannot resolve |
| Egress allowed | `api.github.com` 200, log line `allowed`, `allowlisted` |
| Egress refused | `example.com`: proxy 403 (curl exit 56), log line `refused`, `not allowlisted` |
| Exec streams | stdout and stderr separate, exit code 7 returned |
| Exec identity | uid 1000, `CapBnd` 0, rootfs read-only, stdin piped through |
| Cancel | killing the host client leaves no guest `sleep` |
| TTY attach | `/dev/pts/0`, command output returned |
| Host checkout | guest writes and edits do not appear in the host tree; guest mounts are `/dev/root` (ro) and `/dev/vdb` |
| Push-back hooks | guest planted five `.git/hooks`, `core.hooksPath`, `core.fsmonitor`, and tracked `.githooks/` and `.husky/` files, each touching a host marker; after host fetch, fast-forward and two checkouts, no marker |
| Hook control | the same hook copied into the host's own `.git/hooks` does create the marker |
| Teardown | no firecracker or agent process, no state dir, no mount, no cgroup |

Sabotage: pointing the proxy symlink at a missing socket fails both
egress checks (`out/extra.txt`). The other refusal checks each carry the
positive control named in the table.

## Measurements

labvm: 2 vCPU, 4 GiB, nested KVM. Absolute numbers are pessimistic; the
ratio is what carries. Guest: 2 vCPUs, 1 GiB. Serial console logged to a
file in every run. Cold means page cache dropped first. Firecracker's
time excludes proxy start (the donor's proxy was already up); podman's
includes it. The idle donor box ran throughout. Raw: `out/measure.txt`.

Startup to the first `exec true` returning, small fixture repo:

| | cold | warm (5 runs) |
|---|---|---|
| podman `box up` + `box exec` | 1669 ms | 904–946 ms |
| Firecracker, jailer | 1218 ms | 814–859 ms |
| Firecracker, rootless | 1157 ms | 763–813 ms |

Firecracker's warm time is about 50 ms of copy-in and 720–790 ms from
launch to the guest's ready line.

Copy-in (`mkfs.ext4 -d`, cold cache):

| Tree | Size | Files | Time |
|---|---|---|---|
| fixture repo | 272 KiB | 69 | 72 ms |
| pinfold checkout with history (539 commits) | 5.3 MiB | 354 | 137 ms |
| pinfold `git archive` tree with `target/` | 178 MiB | 1084 | 1171 ms |

Memory per box, different observers, not comparable as one number:
- podman: `box stat` memory (cgroup `memory.current`) 2.4 MiB warm.
- Firecracker: VMM RSS 79–83 MiB after boot and one exec; guest `free`
  used about 100 MiB. With the 178 MiB tree on a 522 MiB work disk, the
  guest read the tree, wrote until the disk was full (about 340 MiB),
  read that file back and deleted it; VMM RSS was then 563 MiB and stays: without a balloon device, guest page cache
  is never returned, so a long-lived box tends toward its full `memory`.

## Gaps

- **Root.** The jailer needs root, and pinfold's Linux contract is
  rootless. Rootless mode works but drops the VMM's chroot, PID namespace
  and cgroup; Firecracker's own seccomp remains. A product would run
  rootless and put the VMM in a systemd user scope for limits.
- **Owner death.** If `box up` is SIGKILLed, the VM keeps running until
  pruned; its egress dies with the proxy socket, so it still fails
  closed. Needs `PR_SET_PDEATHSIG` or a scope that dies with the owner.
- **arm64.** Not tried. Firecracker ships aarch64 builds; GitHub arm64
  runners likely have no `/dev/kvm`, so Linux arm64 CI would need a
  self-hosted host.
- **CI.** Unverified on GitHub x86. The e2e suite assumes live mounts in
  many rows; a backend means a new runtime adapter and a rewrite of those
  rows.
- **Image pipeline.** Every profile change adds export and mkfs (about 4 s
  here for 210 MiB) and a disk image of the profile's size per version.
  Harness installs (`/opt/pinfold/<name>`) and profile `share/` would need
  read-only disk images too; `share/` is "live, never copied" today.
- **Disk.** The work disk has fixed headroom: the 178 MiB tree got a
  522 MiB disk, which filled after about 340 MiB of writes (`No space
  left on device`). `mkfs.ext4 -d` copies host
  uids, which matched here only because the user is uid 1000; a backend
  needs a keep-id equivalent. `lost+found` is root-owned in the workspace.
- **Guest root.** PID 1 and the three `socat` relays run as root for the
  box's life. Only PID 1 needs to (for mounts); the relays and agent can
  run as the box user, since vsock ports from 1024 need no capability.
- **Exec protocol.** No window-size propagation on attach, no signal
  forwarding beyond close-to-kill, no env passing. `box stat` (pids, OOM
  kills) has no source; the host sees one process.
- **vsock reach.** The guest can ask for any `<uds>_N`; only names the
  host created resolve (3128, and 1000 until ready).
- **Untrusted bundle.** The host parses a guest-made pack with `git
  fetch`, the same exposure as fetching from any remote.
- **Kernel.** The guest kernel is pinned here by URL and sha256, but it is
  Firecracker CI's build, not one pinfold builds.

## Guarantees a spec change would add or alter

- **Kernel boundary:** "A VM per box" on both OSes. New row: a box's
  `uname -r` differs from the host's.
- **No network** (row 1): same contract; mechanism is no NIC plus vsock.
- **Nothing can gain privileges** (row 5): "PID 1 runs as the host uid"
  no longer holds; PID 1 is root inside the guest.
- **Host and box share files seamlessly** (row 10): removed on Linux.
- **The box cannot write `.git`** (rows 11, 22): replaced by "results
  reach the host only as fetched refs, and no guest hook or config runs
  on the host". Protected editor config becomes moot.
- **A changed project file stops the run** (row 12): the box can no
  longer change a host file.
- **Project state persists** (row 13): the project home would be a
  persistent per-project disk image, not a mount.
- **OOM** (row 20) and **seccomp** (row 37): replaced by the VM's memory
  limit and the VMM's seccomp; row 37 is podman's.
- **Losing the owner fails closed** (row 8): needs the owner-death fix.

## Caller changes this would imply

- No live mounts on Linux: `mounts` become copy-in disks; writes stay in
  the box. Results come back only by git (refs under a remote namespace),
  so untracked and ignored files never return.
- `exec`, `exec --tty` and `attach` run through a guest agent, not the
  runtime's exec: same flags, but window resizes and signals need agent
  support.
- `box stat` fields would change source or meaning.
- Either macOS moves to copy-in too, or the box spec's `mounts` mean
  different things per OS.

## Open questions for Adam

1. Is copy-in plus git push-back acceptable for interactive `pinfold pi`
   on Linux, or only for programmatic boxes?
2. Same semantics on macOS (copy-in on Apple `container`), or a per-OS
   divergence?
3. May yard-sthlm get `/dev/kvm` as proposed above?
4. Is a non-pinned-by-pinfold guest kernel (Firecracker CI's) acceptable,
   or does pinfold build its own?
