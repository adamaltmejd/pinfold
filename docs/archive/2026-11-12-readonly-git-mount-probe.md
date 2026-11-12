# Read-only `.git` inside a writable Apple share

Date: 2026-11-12. Evidence for `2026-11-12-project-box-plan.md`.
Diagnostic only. Run by the operator by hand; no script was kept.

## Environment

macOS 27.2.1, Apple `container` 1.7.1, image
`localhost/pinfold/profile-default:latest`, host uid:gid 501:20. A
throwaway repository `R` with one commit, under a temporary directory.

```
container run -d --user 501:20 -v R:/work -v R/.git:/work/.git:ro \
  --entrypoint sleep IMAGE infinity
```

Each observation was a `container exec -w /work`.

## Mounts in the box

```
/work       rw,relatime - virtiofs virtiofs rw
/work/.git  ro,relatime - virtiofs virtiofs rw
```

The second line's mount flag is `ro` while the filesystem's own option
is `rw`. Read-only is a mount flag set in the guest.

## Writes

| Attempt in the box | Result |
|---|---|
| Append to `a`, create `b` | Exit 0; both seen on the host at once |
| `touch .git/x` | "Read-only file system", exit 1 |
| Write `.git/hooks/pre-commit` | "Read-only file system", exit 2 |
| Append to `.git/config` | "Read-only file system", exit 2 |
| `mv .git/HEAD .git/HEAD2` | "Read-only file system", exit 1 |
| `mv .git .git2` | "Device or resource busy", exit 1 |
| `ln .git/config stolen` | "Invalid cross-device link", exit 1 |
| `mount -o remount,rw /work/.git` | "must be superuser", exit 32 |
| `umount /work/.git` | "must be superuser", exit 32 |

Removing a file under `.git` was not tried.

## Git in the box

- `git status --short`, `git diff --stat` and `git log --oneline` exit 0
  and are correct.
- `git add b` and `git commit -am box` fail: "Unable to create
  '/work/.git/index.lock': Read-only file system", exit 128.
- On the host afterwards: one commit, `git fsck` silent, `.git` holds no
  new file and no hook.

## Host commits while the box reads

Twenty times: the host appended to a file, ran `git add -A` and
committed; the box at once ran `git log -1` and `git status`, then
appended to its own file.

- The box's read was correct in 3 of 20. In 17 it said "your current
  branch 'main' does not have any commits yet" and listed every file as
  untracked.
- After 1.2 s the box's `git log -1` and `git status` were correct.
- The host ended with 21 commits, `git fsck` silent, and all 20 of the
  box's appended lines.

## What this shows and does not

- A box without privilege cannot write the host's `.git` through this
  mount, and its Git reads work.
- The read-only flag holds only while box work cannot mount. It rests
  on the box having no capabilities.
- Box Git reads are wrong for up to about a second after a host Git
  write. Nothing is damaged.
- Not tried: file removal under `.git`, worktrees, submodules, a `.git`
  file in place of a directory, Linux.
