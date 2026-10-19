# VMM jail and guest PID 1 prototype

2026-10-19. Checks the two Linux designs in draft PR #112 (spec at
`63bdc57` on `claude/vm-disk-spec`) before it leaves draft: the rootless
VMM jail `box up` builds, and guest PID 1 option A. Option C was tried
too. Builds on PR #111's prototype (Firecracker 1.17.0, the Firecracker
CI kernel 6.1.188). Scripts, launcher, guest init and raw outputs are in
`2026-10-19-vmm-jail-prototype/`. None of it is pinfold code.

Host: a fresh labvm lease, Debian 13 amd64, kernel
6.12.107+deb13-cloud-amd64, 2 vCPU, 4 GiB, nested KVM. User `agent`
(uid 1000) with ten supplementary groups (adm, sudo, video, ...) plus
`kvm` (added by `setup.sh`). Artifacts: firecracker sha256 `99ad0f5c...`,
vmlinux `c1628885...` (same pins as #111).

## Verdict

Both designs work as specified, with four spec lines to change. The jail
runs Firecracker rootless with `CapBnd` 0, `NoNewPrivs` 1, seccomp on,
six new namespaces, only the jail's mount points and a Landlock domain
that refuses what the mounts allow. Option A leaves the disk clean every
time and a read-only VM then mounts it without recovery. The jail costs
about 3 ms of a roughly 950 ms boot.

What fails as written:
1. Supplementary groups cannot be reduced without privilege. The VMM
   keeps all of the user's groups.
2. The VMM's mountinfo names paths under the user's home: field 4 of
   each bind is its source path. No mount point is under home.
3. Option A has to end box work before it unmounts. A live process with
   its cwd on the disk makes `umount` fail with EBUSY. So PID 1 kills
   the work cgroup at `down`, which goes against "pinfold never signals a
   process in it".
4. Nothing in the spec covers this: Firecracker ends a host-initiated
   vsock stream as soon as the host half-closes it. The exec protocol
   needs an in-band stdin EOF.

## Method

- `fcjail.rs` (std only, about 300 lines) is the `box up` side. It
  `clone`s the VMM with new user, mount, PID, network, IPC and UTS
  namespaces, sets `PR_SET_PDEATHSIG`, maps only uid and gid 1000 to
  themselves (`setgroups` deny), and mounts a 64 KiB tmpfs. Into it go
  read-only binds of firecracker, vmlinux, the root and init disks and
  config.json, plus rw binds of the data disk, `/dev/kvm` and the vsock
  dir. Read-only remounts carry the source's locked flags. It then
  remounts the tmpfs read-only, `pivot_root`s and detaches the old root.
  Last come `no_new_privs`, Landlock (ABI 6: per-path fs rules, TCP bind
  and connect handled with no rules, abstract-socket and signal scoping),
  bounding set dropped to 0, every capability set emptied, and `execve
  /firecracker --config-file /config.json --no-api`. `--no-jail` runs the
  same launcher with plain fork and exec, for timing.
  `FCJAIL_PROBE=1` runs escape probes in place of the `execve`, with the
  same credentials and domain. `FCJAIL_NO_LANDLOCK=1` is the probes'
  control.
- `pf-guest.rs` is guest PID 1, the exec service and a vsock probe.
  `pf.mode=` on the kernel command line picks A, C or `ro`. Every vsock
  listener checks the peer CID from `accept` and answers anything but
  CID 2 with `peer`.
- The guest root is a debootstrap trixie minbase (curl, socat, procps)
  with no setuid files. Drives: rootfs (ro, `/dev/vda`), init disk (ro,
  `vdb`; it holds the init but the guest boots from the rootfs copy), data
  disk (`vdc`, made by the user with `mkfs.ext4 -E root_owner=1000:1000`).
- Egress goes through `proxy.py`, a stand-in for pinfold's proxy: CONNECT
  on `vsock/v.sock_3128`, api.github.com allowed, everything else 403,
  every decision logged. #111 already ran pinfold's own proxy over this
  path. The jail changes only where the socket's directory is seen from.
- Readiness is one guest connection to host port 1000. The host's
  one-shot listener unlinks itself, so once ready the vsock dir holds only
  `v.sock` and `v.sock_3128`.

## 1. Jail (`out/check-jail.txt`)

From the host, with the box up and egress working:

| Observation | Result |
|---|---|
| `PPid` of the VMM | the launcher |
| Uid/Gid | 1000 everywhere |
| `CapInh/Prm/Eff/Bnd/Amb` | all 0 |
| `NoNewPrivs`, `Seccomp` | 1, 2 (Firecracker's own filter) |
| `NSpid` | `<host pid> 1`: Firecracker is PID 1 of its namespace |
| user, mnt, pid, net, ipc, uts ns | all differ from the launcher's; cgroup ns is shared (not in the spec) |
| mount points (mountinfo field 5) | only `/` (tmpfs, ro) and the 8 binds; none under `/home` |
| mountinfo field 4 | 7 of 9 lines name a source under `/home/agent/...` |
| `Groups` | `4 20 24 25 27 29 30 44 46 992 1000`, the same as the launcher's |
| guest `uname -r` | 6.1.188 (host 6.12.107); only `lo` |
| egress | api.github.com 200; example.com 403, logged `deny` |
| SIGKILL of the launcher | VMM gone 50 ms later (`PR_SET_PDEATHSIG`) |

Supplementary groups: `setgroups([])` returns EPERM before `unshare` (no
CAP_SETGID) and again inside the user namespace. There an unprivileged
`gid_map` needs `setgroups` set to `deny`, and that is permanent. Only a
privileged helper (`newgidmap`) could allow it, and even then the `kvm`
gid would have to be mapped. So the VMM keeps every group. In the jail
they reach nothing: the tree holds only user-owned files and `/dev/kvm`,
the network namespace is empty, and abstract sockets are per network
namespace.

Escape probes. Each was run after every restriction; the host-side
controls succeed (TCP 200, abstract connect ok, `kill -0` ok):

| Probe | Landlock on | Landlock off |
|---|---|---|
| open `/dev/kvm` rw, `KVM_GET_API_VERSION` | ok, 12 | ok, 12 |
| open `/data.ext4` rw, `/vmlinux` ro | ok | ok |
| connect `/vsock/v.sock_3128` | ok | ok |
| `execve /firecracker --version` | runs | runs |
| open `/vmlinux` rw; create `/x` | EROFS | EROFS |
| `/proc`, `/home`, `/etc/passwd` | ENOENT | ENOENT |
| `kill(host pid, 0)` | ESRCH | ESRCH |
| create a regular file in `/vsock` | **EACCES** | ok |
| `mknod` a fifo in `/vsock` | **EACCES** | ok |
| `ftruncate /data.ext4` | **EACCES** | ok |
| TCP connect 127.0.0.1:8080 | **EACCES** | ENETUNREACH |
| connect abstract `@fcjail-probe-abstract` | ECONNREFUSED | ECONNREFUSED |

The mounts and namespaces do most of the work. Landlock adds the refusals
inside the writable vsock dir and on the disk, plus a second refusal for
TCP. The abstract socket fails in the empty network namespace before
Landlock's scope check is reached. Landlock does not cover connecting to
a pathname unix socket, so only the mount tree keeps the VMM from the
host's other sockets.

Unprivileged user namespaces:
- Debian 13 allows them by default (`kernel.unprivileged_userns_clone=1`).
  With it set to 0, `clone` fails with EPERM, which preflight can name.
- `kernel.apparmor_restrict_unprivileged_userns`, the Ubuntu knob, does
  not exist on this kernel. The Ubuntu case could not be reproduced.
- This kernel has two other AppArmor knobs,
  `apparmor_restrict_unprivileged_unconfined` and
  `unprivileged_userns_apparmor_policy`. With either at 1, the jail still
  works.
- A first run left both knobs at 1, and the recorded run started that
  way. Their defaults were not captured (they are readable only by root).

## 2. Control channel (`out/check-channel.txt`)

Probes from inside, as uid 1000, after ready:

| Connect | Result |
|---|---|
| CID 2 port 1024 (exec), 1026 (PID 1 control), 1000 (ready, now gone), 4444 | ECONNRESET after 0.2–0.3 ms |
| CID 2 port 3128 | connected after 0.3 ms (control) |
| own CID 3, ports 1024 and 1026 | ENODEV at once |
| CID 1 (`VMADDR_CID_LOCAL`), ports 1024, 1026, 4444 | ETIMEDOUT after 2.0 s |

The pinned kernel has `# CONFIG_VSOCKETS_LOOPBACK is not set`
(`/proc/config.gz`, `out/kconfig.txt`). Without a loopback transport, a
CID 1 connect goes out the virtio transport, Firecracker drops it, and the
guest's 2 s vsock connect timeout fires. No guest connection reaches a
guest listener, so the `peer` branch cannot be triggered from inside.
Host-initiated connections arrive as peer CID 2: the exec service and
PID 1 log `peer_cid=2`, which is the branch's positive control.

The kernel also has `CONFIG_USER_NS=y`. That bears on the open question
about user namespaces in the guest; turning them off at run time
(`user.max_user_namespaces=0` from PID 1) was not tried.

Half-close (`out/check-halfclose.txt`): a host client that sends
`CONNECT 1024` and a command, then `shutdown(SHUT_WR)`, gets only `OK`.
The guest's output is lost and the stream ends after about 1 s. With the
write side left open, the same request returns its output. So stdin EOF
cannot be a host half-close; the exec protocol needs a frame for it.

## 3. Guest PID 1 (`out/check-disk.txt`)

Each trial uses a fresh 256 MiB disk. It writes 64 MiB of random data and
checksums without `sync`, then leaves `sleep 1000` running with its cwd
on `/data`. That is the hardest case: dirty pages plus a busy mount. After
`down`, the host reads `dumpe2fs -h`, then a second VM attaches the same
image read-only (`is_read_only`, mounted `ro` without `noload`) and checks
the checksums.

`Filesystem state: clean` comes out on every image, including the killed
ones. The observable is the `needs_recovery` feature.

| Trial | `down` | Host: `needs_recovery` | Read-only VM |
|---|---|---|---|
| negative control | VM killed | set | mount EROFS |
| A, no kill | `umount` → EBUSY, then VM killed | set | mount EROFS; the console shows "write access unavailable, cannot proceed (try mounting with noload)" |
| A ×3 | cgroup.kill, wait `populated 0`, sync, umount: ok in 41–43 ms | clear | mounts; `f: OK` |
| C ×3 | `kill(-1)` from PID 1 at uid 1000, reap all: 24–36 ms | clear | mounts; `f: OK` |

Option A, from `/proc/1/status` and the exec service's status inside the
box:
- PID 1 is uid 0 with `CapBnd`, `CapPrm` and `CapEff` all `0x200000`
  (`CAP_SYS_ADMIN`), and `CapInh` and `CapAmb` 0.
- The exec service is uid and gid 1000, no groups, every set 0,
  `NoNewPrivs` 1.

PID 1 ends the work without `CAP_KILL`: it writes `1` to
`/sys/fs/cgroup/work/cgroup.kill`, a root-owned file, using uid 0's owner
access. The same cgroup carries the pids cap. A uid-1000 process cannot
leave the cgroup, because the `cgroup.procs` files are root-owned.

Option C: the exec service `unshare`s a mount namespace and mounts the
disk there while still root, then drops to uid 1000 with nothing. PID 1
drops the same way and keeps only the control listener, bound earlier. At
`down` it runs `kill(-1, SIGKILL)` and reaps until ECHILD. By the time
the last child is reaped, `/sys/fs/ext4/vdc` is already gone, so the
superblock is shut down by the end of the last process's exit. ext4 comes
out clean in all three trials. C's limits are untested: a process outside
the mount namespace holding a reference (a passed fd) would keep the
filesystem mounted. With every process killed, none remains.

## 4. Startup (`out/measure.txt`)

Ten alternating runs each, from launcher start to the guest's ready line,
with a fresh disk before each run:

| | ready median | min–max | launcher setup |
|---|---|---|---|
| jailed | 963 ms | 845–987 | mean 2.8 ms (1.0–6.0) |
| unjailed | 966 ms | 895–1039 | mean 0.24 ms |

The difference is within run-to-run noise. Single boots outside the loop
reached ready in 683–716 ms. Why the loop is slower was not
investigated; both modes move together, so the comparison holds. VMM RSS at ready is 60–66 MiB in both modes.

## Spec lines

Held:
- Threat model "VMM jail" and "VMM jail" section: namespaces,
  `pivot_root` into an empty tmpfs holding exactly the listed files,
  detached old root, bounding set 0, `no_new_privs`, Landlock limited to
  those paths, Firecracker's seccomp after. No privileged helper.
- Always applied: the VMM runs as the user with a parent-death signal,
  and the VM ends with its owner.
- Runtimes: Firecracker refuses nothing here as rootless. Debian 13
  allows unprivileged user namespaces, and preflight can detect
  `unprivileged_userns_clone=0` (`clone` EPERM).
- Transport: CID 2 on a port with no listener resets at once; 3128
  connects; the only host listeners after ready are `v.sock` and
  `v.sock_3128`; guest listeners see the host as CID 2.
- Lifecycle 5: PID 1 mounts, forks the exec service at the host uid with
  every set empty, and keeps only `CAP_SYS_ADMIN`.
- Lifecycle 8 and Disks "Clean": after A's unmount, a read-only
  attachment mounts with no recovery. After a kill, the guest refuses
  it on its own.
- Row 1 (Linux part), row 5 (Linux part), row 11's read-only sharing.
- Row 37, apart from the groups clause and the home clause.

Failed or incomplete:
- VMM jail, "supplementary groups reduced to the one that grants
  `/dev/kvm`", and row 37's "exactly one supplementary group": not
  possible rootless.
- Row 37's "mountinfo ... not the user's home": the bind sources appear
  in field 4.
- Lifecycle 8 and Process supervision: unmounting needs the work ended
  first. "Pinfold never signals a process in it" conflicts with A's
  cgroup.kill at `down`.
- Open question on vsock loopback: settled; the pinned kernel has none.
- Not covered by the spec: host half-close ends the stream.

## Edits #112 needs

1. VMM jail: replace "supplementary groups reduced to the one that grants
   `/dev/kvm`" with "the user's supplementary groups, which reach nothing
   in the jail". Row 37: drop "exactly one supplementary group, the one
   owning `/dev/kvm`".
2. Row 37: "its mountinfo holds only the jail's entries and not the
   user's home" becomes "its mount points are only the jail's, and none
   is under the user's home".
3. Lifecycle 8 (Linux): "init ends box work (its cgroup's `cgroup.kill`),
   then syncs and unmounts the disks". Process supervision: say that at
   `down` init kills the box's work cgroup, so the "never signals" rule
   covers only the host side and `exec`.
4. Lifecycle 5: PID 1 places the exec service in a work cgroup that
   holds the pids cap and is root-owned, so box work cannot leave it.
5. Open questions: delete the vsock-loopback item. Row 1 stays as
   written. A guest connect to CID 1 times out after 2 s, so no row
   should assert on CID 1.
6. Transport, or the exec protocol: stdin EOF travels as a frame. The
   host never half-closes an exec connection.
7. VMM jail: name a Landlock rule set that is sufficient. It is not
   shown to be minimal; no right was removed one at a time. Read and execute
   on the binary; read on the kernel, disks and config; read and write on
   the box's disks; read, write and `IOCTL_DEV` on `/dev/kvm`; and
   `MAKE_SOCK`, `REMOVE_FILE` and `READ_DIR` on the vsock dir. Handle
   every access right of the running ABI, including TCP. This is
   optional. ABI 5 or later is what makes `/dev/kvm`'s ioctls an
   explicit right.

Option C also works. It would let PID 1 hold nothing after boot, but it
relies on every process in the box being killed. A needs only the cgroup
it already has for the pids cap. A stays the recommendation.

## Not verified

- The Ubuntu AppArmor restriction
  (`kernel.apparmor_restrict_unprivileged_userns=1`): the knob is absent
  on Debian's kernel. On Ubuntu it reportedly lets `unshare` succeed and
  strips capabilities in the new namespace. If so, the first failure is
  a `mount` EPERM, not `clone`, and the `userns-apparmor` token must be
  chosen from the sysctl, not from the errno. This needs an Ubuntu host.
- GitHub's runners, yard-sthlm's LXC and arm64.
- pinfold's own proxy inside the jail (#111 ran it over the same vsock
  path, outside a jail).
- Runtime `user.max_user_namespaces=0` in the guest.
- Row 6: no secret was sent over an exec connection, so the env path
  (absent from cmdline, VM config and disks) was not exercised.

## Reproduce

On a fresh lease: run `setup.sh`, then `build.sh`, then
`check-jail.sh`, `check-channel.sh`, `kconfig.sh`, `check-disk.sh`,
`check-halfclose.sh` and `measure.sh`. All scripts source `lib.sh`, and
`kill-all.sh` stops leftovers. No secret is used anywhere.
