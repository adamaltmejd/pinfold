# Linux guest init boot check (2026-10-20, #140)

`pinfold init boot` as PID 1 of a Firecracker VM, checked on a labvm
lease before the adapter exists (#141). The host side here is
`fcinit.py`, a throwaway client for the init's framing; it is not pinfold
code and runs no jail (#139). Scripts and raw output are in
[2026-10-20-linux-init-boot/](2026-10-20-linux-init-boot/).

Host: Debian 13, kernel 6.12.107, nested KVM, Firecracker v1.17.0 release
binary. Guest kernel: the `guest-kernel-6.18.57-firecracker-v1.17.0`
prerelease asset, sha256 `1a1b28b3…afc99` (the published asset; the
digest in `2026-10-20-guest-kernel-boot.md` names its CI build). Root
disk: a Debian trixie minbase with python3-minimal, no setuid files. Init
disk: `/pinfold` (the x86_64 musl CLI) and empty `/dev` and `/mnt`.

## Boot contract

The init disk is the kernel's root (vda, read-only), booted with
`init=/pinfold -- init boot`. PID 1 dials host port 1000; the BOOT frame
names the uid, gid, the image root device and the attachments; each new
disk's `from` follows on that connection in the `init receive` stream
format, written by the exec service as the box user. `up` then sends the
env as the first request on the exec port (1024); an exec that arrives
earlier waits for it. PID 1 answers UNMOUNT on port 1025. The framing is
documented at the top of `crates/pinfold/src/init/firecracker.rs`.

The image root is mounted read-only under an overlay on tmpfs, which
makes the mount points and the empty `/etc/resolv.conf`, and is then
bind-remounted read-only. PID 1 then `pivot_root`s and detaches the init
disk.

## Results (`out/checks.txt`)

| Row | Observation |
|---|---|
| 1 | Only `lo`. From inside, CID 2 on 1024, 1025 and 4444 reset in 0.2–0.3 ms; 3128 connects to the host's socket, and so does the TCP relay on `127.0.0.1:3128`. `/proc/config.gz`: `# CONFIG_VSOCKETS_LOOPBACK is not set`. The CID-2-only check on guest listeners cannot be triggered from inside without loopback; host connections arrive as CID 2 and are served. |
| 5 | PID 1: uid 0, `CapPrm`, `CapEff` and `CapBnd` `0x200000`, `CapInh` and `CapAmb` 0. Exec service: uid and gid 1000, no groups, every set 0, `NoNewPrivs` 1, cgroup `/work`. Exec'd work the same. `unshare -U` fails with ENOSPC (`max_user_namespaces` 0); `touch /x` EROFS; no setuid or setgid file. |
| 6 | The env value arrives in the box. It is in none of `/proc/cmdline`, `config.json`, the console log, the init disk, the root disk or either data disk. Control: the same grep finds it in the work disk after the box writes it there. |
| 11 | `from`: a 644 file, a 755 script, a symlink as a link and a nested dir; the FIFO is skipped; the disk root and files are the box user's. A read-only disk refuses writes; its root shows `0:0` because the fixture was made by `mkfs -d` on the host and never attached writable, the attachment that gives a disk's root to the box user. Hardest case: 64 MiB unsynced plus a `sleep` with its cwd on the disk; `down` answers clean, `dumpe2fs` shows no `needs_recovery`, and a second VM attaches it read-only and verifies the checksum. |
| 20 | Exec'd work has `oom_score_adj` 1000. A memory hog exits 137; stat's `oom_kills` goes 0 → 1, every field present (`pids.max` 2048); the box still answers. |

Exec protocol: exit codes 7 and 137 (signal) pass through; stdin reaches
`cat` and ends by the STDIN_END frame; stdout and stderr stay apart; the
workdir applies; a missing program exits 127; `--tty` runs on
`/dev/pts/0` and a RESIZE to 40×100 shows in `stty size`; an orphan
leaves no zombie.

Review fixes, rechecked on a second lease:

- **Pids cap.** In a 1 GiB box, a process spawns until the work cgroup
  refuses spawn 2042 (EAGAIN; the service's threads hold the rest of
  `pids.max` 2048), then holds it there. An exec and a stat at the cap
  get their connection dropped, the VMM stays up, and once the processes
  end the box answers. At 512 MiB the box
  user's `RLIMIT_NPROC` (half the kernel's `threads-max`) binds first, near
  1900.
- **Kernel paths.** A disk at `/proc/sys/user` fails `up` in the guest,
  naming the path. The host refuses it earlier, as `spec` (row 17).
- **PID 1's exit ends the VM.** `kill -9` of the exec service: the VMM
  exits within 0.1 s and the console shows "Kernel panic - not syncing:
  Attempted to kill init!" (`panic=1 reboot=k`).

## Sabotages (`out/sab-*.txt`)

The first five ran against the first version of `checks.sh`, the last two against the current one. Each is a one-line patch of `firecracker.rs`, built and run through the
whole check (`sabotage.sh`):

| Sabotage | What failed |
|---|---|
| no `cgroup.kill` at UNMOUNT | `down` fails `box work did not end`; the disk keeps `needs_recovery`; the read-only reattach fails EROFS |
| PID 1 keeps every capability | PID 1's `CapBnd` `000001ffffffffff` |
| service skips the bounding-set drop | exec'd `CapBnd` `000001ffffffffff` |
| no `max_user_namespaces` 0 | `unshare -U` succeeds; the sysctl reads 1904 |
| no `oom_score_adj` raise | exec'd work at -1000 (the service's); the hog panics the guest, "Out of memory and no killable processes" (`out/sab-no-oom-raise-console.txt`) |
| `thread::spawn` in place of `Builder::spawn` | at the cap, the exec and stat connections end and the VMM exits: the service panicked, PID 1 exited |
| no guest refusal of kernel paths | the disk at `/proc/sys/user` comes up `ready` |

The first version of the pids check (now replaced) hit the box user's
`RLIMIT_NPROC` instead: `fcinit.py` ignored `--mem`, so every box had
512 MiB.

## Found on the way

- After `setresuid` the service is not dumpable, so its `/proc/self`
  files belong to root and the child's `oom_score_adj` write failed
  EACCES. The child sets itself dumpable just before `execve`, which
  would make it dumpable anyway.
- devpts mounted `nodev` refuses its own pty slaves (EACCES).

## Not covered here

The jail, the real host client, `stat`'s memory limit (the host fills it
from the spec's `memory`), and the e2e rows: they run on Linux CI once
#141 switches the runtime.
