# VMM jail check (#139)

2026-10-20. Runs pinfold's jail module,
`crates/pinfold/src/core/runtime/firecracker/jail.rs`, before the
Firecracker adapter (#141) exists. A scratch driver, `pjail`, plays
`box up`'s part: it `#[path]`-includes the module, calls `jail::spawn`
and waits. The guest, proxy stand-in and pins are the 2026-10-19
prototype's (`2026-10-19-vmm-jail-prototype/`). Scripts and the raw
output are in `2026-10-20-vmm-jail-check/`. None of it is pinfold code.

Host: a fresh labvm lease, Debian 13, kernel 6.12.107, Landlock ABI 6,
user `agent` in `kvm` and nine other groups.

## Row 43, observed from the host (`out.txt`)

Two boxes up, each with its own data disk:

| Observation | Both boxes |
|---|---|
| VMM's `PPid` | its `pjail` |
| Uid, Gid | 1000 |
| `CapInh/Prm/Eff/Bnd/Amb` | 0 |
| `NoNewPrivs`, `Seccomp` | 1, 2 |
| user, mnt, pid, net, ipc, uts namespaces | differ from `pjail`'s; cgroup shared |
| `/proc/PID/root` | `config.json dev/kvm disk0.ext4 firecracker init.ext4 root.ext4 vmlinux vsock/` |
| `/disk0.ext4` inode | its own disk's, not the other box's |
| mount points (mountinfo field 5) | `/` and the 8 binds; none under `/home/agent` |
| egress | api.github.com 200; example.com 403 |

SIGKILL of box a's `pjail` ended a's VMM in 48 ms; b's stayed up.

Escape probes run as `/firecracker` in the full jail (`pjprobe.rs`), with
host controls succeeding: `/dev/kvm` and the disk open read-write,
`KVM_GET_API_VERSION` 12, the proxy socket connects. Refused: writing
`/vmlinux` or creating `/x` (EROFS); `/proc`, `/home`, `/etc/passwd`
(ENOENT); a file, a fifo or `ftruncate` in the vsock dir or on the disk,
TCP to 127.0.0.1:8080, and opening `/root.ext4` read-write (EACCES, from
Landlock); the host's abstract socket (ECONNREFUSED); `setgroups([])`
(EPERM). The probe's `kill` check reached only itself: the jail's empty
environment dropped the host pid it was given.

A failing step is named, for example `VMM jail: bind
.../jail/disk0.ext4: No such file or directory`.

## Preflight's user-namespace probe

- Debian, `kernel.unprivileged_userns_clone=0`: `clone` fails with EPERM.
  Doctor reports `userns`.
- Ubuntu 26.04 (GitHub's runner), AppArmor's restriction on, an
  executable outside the profile's path: `clone` succeeds and writing
  `/proc/self/setgroups` fails with EACCES. The prototype could not test
  this. It is why the token comes from
  `kernel.apparmor_restrict_unprivileged_userns`, not from the error.
- The profiled executable passes on the same runner.

## Other findings

- `pinfold` refuses a replaced executable before any command runs:
  `update::working()` canonicalizes `current_exe()`, and a path ending
  in " (deleted)" fails with ENOENT. `apparmor-profile` needs no check of
  its own.
- Debian puts `mkfs.ext4` in `/usr/sbin`, off a user's PATH. Preflight
  also looks there and in `/sbin`.

## Sabotages run (Linux CI, the doctor test alone)

| Change | Failing assertion |
|---|---|
| `kvm` checks group membership | the profiled binary reports `kvm` |
| `kvm` opens without the ioctl | the Landlock scenario has no `kvm` line |
| token chosen from the probe's error | the unprofiled copy reports `userns` |
| probe skipped | the unprofiled copy reports no token |
