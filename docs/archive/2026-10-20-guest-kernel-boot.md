# Guest kernel boot check (2026-10-20, #122)

The first `guest-kernel/build.sh` output, from the PR #146 CI run
39417072090, boots under Firecracker v1.17.0. Prototypes before this ran
Firecracker CI's 6.1.188; Firecracker's kernel policy lists 6.1 guest
support only to 2026-09-02 and 6.18 to 2028-06-01, so Adam chose 6.18.

## Build

- Linux 6.18.57, sha256 and kernel.org signature (Greg Kroah-Hartman's
  stable key) checked; Firecracker v1.17.0's
  `microvm-kernel-ci-x86_64-6.18.config` plus `IKCONFIG`,
  `IKCONFIG_PROC`, and `VSOCKETS_LOOPBACK` off.
- ubuntu-26.04 runner, gcc 15.2.0, binutils 2.46: 10 min 42 s for the job.
- `vmlinux-6.18.57`, 29.9 MB, sha256
  `1b0e6b898ff75ff39a26aff5fb0ff4097565dce17b7dd21372384c8c0b89d83f`.
  The build is not reproducible (timestamp and runner host are embedded),
  so this digest names this artifact only.

## Boot

Host: a labvm lease, Debian 13, kernel 6.12.107, nested KVM, Firecracker
v1.17.0 release binary. `run.sh` boots a read-only ext4 root holding
busybox, socat and `init`; 1 vCPU, 256 MiB, vsock CID 3. `boot.txt` is the
output with kernel log lines removed.

| Check | Result |
|---|---|
| boots, runs `/init` | yes, about 2 s from Firecracker start to exit |
| `uname -r` | `6.18.57` |
| `/proc/config.gz` | `CONFIG_IKCONFIG_PROC=y`, `# CONFIG_VSOCKETS_LOOPBACK is not set` |
| also on | `CONFIG_VIRTIO_VSOCKETS=y`, `CONFIG_USER_NS=y`, `CONFIG_SECURITY_LANDLOCK=y` |
| guest to CID 2 port 3128 | host's `v.sock_3128` listener received the line |
| `reboot -f` | Firecracker exits 0 |

Devtmpfs is mounted on `/dev` by the kernel (`DEVTMPFS_MOUNT`).

Not checked here: row 1's resets on other CID 2 ports, the exec service,
OOM, and the jail. They land with #138–#140 on Linux CI.
