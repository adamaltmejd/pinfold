# Apple writable share probe

Date: 2026-11-11. Evidence for the composable plan
(`2026-11-11-composable-plan.md`). Diagnostic only; no guarantee
is published here. Scripts and logs: `2026-11-11-apple-share-probe/`.

## Result

A host directory shared writable into an Apple `container` box is correct
from guest to host. From host to guest it is stale for about one second
after the guest's last lookup of a path, then correct. No setting changes
this, and nothing a box without capabilities can do clears it sooner.

## Environment

- macOS 27.2.1, build 26B131, arm64. Host uid:gid 501:20.
- Apple `container` 1.7.1 (containerization 0.54.0), vminit 0.53.0.
- Guest kernel 6.18.15 aarch64.
- Image `localhost/pinfold/profile-default:latest`.
- `container run -d --user 501:20 -v HOST:/work --entrypoint sleep IMAGE infinity`.
  The guest reports `virtiofs /work virtiofs rw,relatime`.

Each observation was a fresh `container exec`. The first probe used no
sleeps. The second used timed waits to measure the window.

## Guest to host

Correct in 5 of 5 runs for in-place modify, create, delete, rename,
atomic replace, directory rename, `chmod +x` and symlink creation.

## Host to guest

The guest looked each path up, the host changed it, and a fresh exec
observed.

| Host action | Guest within the window | Runs |
|---|---|---|
| Modify in place | New bytes | 5/5 correct |
| Create | New bytes | 5/5 correct |
| Create symlink | Correct | 5/5 correct |
| Delete | `stat` succeeds, `cat` is ENOENT | 5/5 stale |
| Rename | Old name still reads | 5/5 stale |
| Directory rename | Old path still reads | 5/5 stale |
| Atomic replace (temp, then `mv -f`) | `cat` is ENOENT | 5/5 wrong |
| `chmod +x` | Old mode | 5/5 stale |

## The window

- Stale at 0.5 s, correct at 1.0 s, for all four failing operations, in
  3 coarse and 3 fine runs. First correct observation: 0.9 to 1.0 s.
- The window counts from the guest's last lookup of that path. A path
  the guest last looked up 1.2 s or 3 s before the host change was
  correct at once.
- A lookup during the window does not extend it.
- A name first seen as missing appears as soon as the host creates it.
- Nothing was stale from 1 s up to 120 s.

## Git

- Guest `status`, `add` and `commit` on a host-created repository work
  as 501:20 with no `safe.directory`. The host then sees a clean status
  and `git fsck --full` passes.
- After a host commit, guest `git log -1` reported "your current branch
  'main' does not have any commits yet" in 6 of 6 trials at 0 s, 1 of 3
  at 0.5 s and none from 1 s to 30 s.
- First probe, run once: a guest commit made in that state exited 0 and
  left `main` at a parentless commit. The earlier history was reachable
  only through the reflog. Host `git fsck` passed.

## Ownership

As 501:20 only, the mount top and files report 501:20. After an exec as
another uid (1234, or 0), the next exec as 501 saw that uid as the
owner, and git refused with "dubious ownership" in 5 of 5 alternations.
After a run of 501-only execs it reported 501:20 again. Host owners
never changed.

## Mitigations tried

| Attempt | Result | Needs |
|---|---|---|
| `echo 2` or `3` to `drop_caches` | Correct at once | `CAP_SYS_ADMIN` and a writable `/proc/sys` |
| `echo 1` to `drop_caches` | No effect | same |
| `mount -o remount,rw` | Correct at once | `CAP_SYS_ADMIN` |
| `ls` of the parent | Heals replaced and chmodded names, not removed ones | nothing |
| `sync` | No effect | nothing |
| `-v …:ro`, `:cached`, `--mount type=bind` or `virtiofs` | Unchanged | — |
| `cache=none` | Rejected: "unknown directive cache" | — |

A second box on the same directory that had not looked the paths up saw
the truth while the first was stale. The stale state is per box.

## Where it comes from

Sources, read 2026-11-11 by a research subagent. The operator did not
re-read them. Some GitHub searches were rate-limited, so the issue list
may be incomplete.

- containerization 0.54.0 builds one `VZVirtioFileSystemDeviceConfiguration`
  from `VZSharedDirectory(url:readOnly:)` and mounts it in the guest with
  no options. `-v` options reach only a bind mount of a subdirectory.
- Apple's `VZSharedDirectory` and `VZVirtioFileSystemDeviceConfiguration`
  expose a tag, the share and `readOnly`. There is no cache setting.
- Linux `fs/fuse/dir.c` (v6.18) revalidates an entry only after the
  lifetime the server returned. `virtio_fs.c` accepts only `dax` options.
- virtiofsd's `--cache auto` returns one-second lifetimes.
- Apple developer forum thread 828366 (June 2026) reports stale contents
  and deleted paths still visible. Apple acknowledged the feedback
  number and nothing more. The poster also says a host temp-file rename
  is visible to the guest, which this probe contradicts.
- Open stale-content reports on the same framework: Tart #1271 (Git
  repositories stale or corrupted after host edits, macOS guests; fixed
  only by remount) and Docker for Mac #7501.
- Open missing-file-event reports, a different symptom: apple/container
  #141, Lima #5530, Podman #22343, Colima #1647.
- apple/container #1940: only `ro` is applied from `-v` options.

Inferred, not read from the protocol: the stale state is the guest
kernel's FUSE entry and attribute cache, with a one-second lifetime set
by Apple's server, which sends no invalidation on host changes.

## Not tested

- The guest-commit history loss was produced once and not repeated.
- Whether a host tool that only rewrites `.git/index`, such as an
  editor's background `git status`, triggers the git failure.
- Linux. virtiofsd's cache policy is a flag there.
