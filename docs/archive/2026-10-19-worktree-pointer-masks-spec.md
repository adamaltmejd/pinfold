# Linked worktree spec: native pointer masks

2026-10-19. Spec-only change. Nothing is implemented. This file records why
ARCHITECTURE.md admits one linked-worktree layout, and what is still open.

## Source research

The research ran 2026-10-09..15. It sits on the local-only branch
`wip/exploration-2026-10-19` and is being archived to main separately.
Conclusions came in this order:

1. A case-sensitive APFS volume:
   `2026-10-15-worktree-recommendation.md`.
2. An SSHFS broker: `2026-10-15-worktree-filesystem-enforcement.md`.
3. Native pointer masks: `2026-10-15-native-worktree-protection.md`, with
   `independent-review.md` and `identity-source.md` in its bundle. This is
   the latest conclusion and the one adopted.

On case-insensitive APFS, Apple container 1.5.0 masked the pointer
read-only in all eight ASCII case spellings, with no host ACL. Every
spelling denied:

- write and chmod (EROFS)
- a new hardlink (EXDEV)
- unlink, rename and replacement (EBUSY)

Ordinary edits, hardlinks, mkdir and atomic saves at the root succeeded.

Two results set the limits of the design:

- With a nested `deep/middle/hooks` mask, renaming `deep` succeeded.
- A deliberately pre-existing hook hardlink let chmod enable the hook, and
  the next host commit ran it.

Nested protected paths are therefore refused, and alias admission stays.

The spelling sample was 3 Unicode lookalikes and 27 names with ignorable
characters inserted. None resolved to `.git`. The spec states this as an
assumption about APFS, not a proof. HFS+ and network filesystems were not
tested, so they are refused.

## Verification on main (59c1612)

**cfd2eb4, effective hook paths through aliases.** Verified by reading
`pi/git.rs` and the G11 test. The test was not rerun.

- `hooks_path` walks a missing leaf up to its existing ancestor and follows
  a symlinked ancestor's target.
- It refuses a `..` component below a missing leaf.
- The G11 test covers:
  - a symlink to the hooks directory
  - a missing leaf below that symlink
  - a dangling symlink into the project
  - a `..` traversal through a missing parent
- macOS `realpath` returns the on-disk case, so an uppercase spelling of
  the project root also canonicalizes into the root and is refused.
- The superseded research's claim that `hooks_path` returns `None` for an
  absolute outside-root path predates cfd2eb4. It is closed.

**fe4ef2f, observed alias refusal.** It indexes regular files only.

## Admission gaps reproduced through the CLI

Both runs used a debug build of 59c1612, Apple container 1.5.0 on macOS
27.0.1, and isolated XDG roots.

- **adamaltmejd/pinfold#106.** Project with `core.hooksPath=.husky/_`, run
  with `pinfold pi --mode rpc`.
  - Control: writing `.husky/_/pre-commit` fails with EROFS.
  - `mv .husky .husky.moved`, then recreating `.husky/_/pre-commit` as an
    executable, succeeds.
  - The next host `git commit` ran the hook.
  - This breaks guarantee 11 on main today, outside worktrees. Husky's
    default layout is affected.
- **adamaltmejd/pinfold#107.** `box up` with three mounts: `P` writable,
  an empty `P/.vscode` read-only, and `P` writable again at `/alias`.
  - It reached `ready`.
  - Control: writing directly into `P/.vscode` fails with EROFS.
  - Writing `/alias/.vscode/tasks.json` succeeded and landed on the host.

## Guarantee rows: hardest case and sabotage

No new test. Each row extends an existing test.

| # | Hardest case | Sabotage |
|---|---|---|
| 10 | A box edit at the worktree root is committed by host git, running hooks from the read-only common dir. | Omit the common-dir export: guest `git log` cannot resolve the gitdir. |
| 11 | An uppercase pointer spelling as the box's first lookup. Rename and replacement of the pointer. A `.husky/_` hooks path whose ancestor `.husky` the box would rename (#106). | Mask only the literal `.git`: an uppercase chmod or write succeeds. Drop the direct-child check: pi admits `.husky/_`. |
| 17 | Each unsupported layout leaves no box and no state dir: symlinked or hardlinked pointer, dangling gitdir, submodule pointer, alternates, caller mount at the common dir. | Skip the pointer link-count check: a hardlinked pointer reaches ready. |
| 22 | An empty protected directory under a second writable export, in either mount order (#107). A worktree root as the caller's only mount. | Index regular files only: the empty directory reaches ready. Derive no masks for callers: the pointer is writable. |

The G10 scenario stays before ready. The known ~1 s native metadata cache
makes a commit made after ready unreliable to read at once, and tests may
not retry.

## Linux and podman parity

- ext4 is case-sensitive, so one read-only file bind of the pointer is
  enough. The rule "every spelling the volume resolves to the pointer"
  covers both platforms.
- A directory with ext4 casefolding (`+F`) is refused.
- A bind-mounted file is a mount point, so rename and unlink fail with
  EBUSY, and a hardlink out of it fails with EXDEV.
- The superseded research saw this with raw rootless podman 6.1.4 on Debian
  13.7 x86-64. It was not run through pinfold, and not on arm64.
- Both suites must run the extended rows 10, 11, 17 and 22.
- #106 was reproduced on macOS only. Linux also allows renaming an ancestor
  of a bind mount point, so it is expected to reproduce there.

## Caller changes

- `box up`: a writable export rooted at a linked worktree now gets
  pinfold-derived read-only pointer masks and a read-only common-dir
  export. Before, the pointer was writable.
- `box up`: a caller mount at the worktree's common dir or pointer is
  refused as `spec`. Switchyard must stop mounting the common dir itself.
- `box up`: an unsupported linked-worktree layout is refused as `spec`.
  Before, it was admitted unprotected.
- `box up`: an empty protected directory reached through a second writable
  export is refused as `mount-alias`.
- pi: linked-worktree projects run. Before, they were refused.
- pi: a `protect` entry or in-project `core.hooksPath` below a direct child
  of the root, including husky's `.husky/_`, now refuses as
  `protected-path-invalid`. This is a hard break.
- No new reason token, JSON field, exit code or flag.

## Named alternative: copy in, push back, attach

This follows gominimal/minimal's sessions
(https://github.com/gominimal/minimal, `docs/concepts/sessions.md`):

- `min session activate` uploads the project into the session's
  workspace. `--sync none` skips the upload.
- Results leave only through `git push min://<session>`.
- `min session attach` opens a shell in the running session.
- Hooks are inert there.

In pinfold terms, the box gets a copy of the worktree on box-local storage
and never a writable view of the host checkout. The host takes results
back as Git objects. Docker Sandboxes' clone mode is the same shape
(`2026-10-15-worktree-research/alternatives.md`). This is an alternative to
pointer masks, not only a fallback. Nothing in ARCHITECTURE.md specs it.

### What it removes

- **Pointer mask planning.** No eight-spelling masks, no filesystem
  classification, no APFS name-resolution assumption.
- **Alias and overlap admission for the worktree.** No writable host
  export of the checkout. G22's admission stays for caller mounts that
  remain.
- **The topology assumption.** The host may move, repair or check out the
  worktree while a box runs, because the box holds no view of it.
- **#106 for that project.** No protected directory sits inside a writable
  export, so neither the direct-child rule nor ancestor protection is
  needed.
- **The ~1 s native cache and the 4–10x small-file I/O cost.** Both apply
  to project files only while they are shared.

### What the push-back channel must guarantee

The guest controls everything it sends: refs, objects and tracked file
contents.

- **The host fetches; the guest does not push into the host.** The host
  runs `git fetch` from the box's repository. For example the transport
  could be `ext::pinfold box exec <box> -- git-upload-pack <dir>`, which
  needs `protocol.ext.allow` for that one command. Then the only host code
  on the path is git's fetch-pack, which already treats a remote as
  untrusted. No `receive-pack` runs on the host, so no host receive hook is
  triggered by the guest. The guest's own hooks and config run only in the
  box.
- **Refs land in a namespace**, such as `refs/pinfold/<box>/*`. A fetch
  never updates HEAD, a checked-out branch, the index or the work tree.
  `receive.denyCurrentBranch=updateInstead` never applies.
- **`transfer.fsckObjects=true` on the fetch**, so malformed trees and
  `.gitmodules` paths are refused.
- **Protected paths move from "the box cannot write" to "the box can
  propose".** A commit can change `.vscode/`, `.husky/`, `.gitattributes`
  or a hook script. Those land on disk only when the host checks out or
  merges the fetched ref. The guarantee becomes: nothing the box sends
  reaches the work tree or runs before an explicit host-side merge.
  pinfold could list a fetched range's changes to protected paths. It
  cannot make them safe to merge unread.
- **Unpushed work dies with the box**, because boxes are disposable.
  `down` would need a final fetch, or a refusal while unfetched commits
  exist. That is new lifecycle, not existing behavior.

### Startup cost for a large repo

There are no measurements of a copy into a box. Measured on the host:

- **100,002-file, 477 MB fixture**
  (`2026-10-15-worktree-research/import-benchmark.json`): local
  `git clone` 8.6 s, hashing the base 12.4 s, diffing the work tree 20.2 s,
  patch check and apply under 0.04 s.
- **Same fixture, copy to another APFS volume:** median 32.96 s.

A copy over Apple container's native share pays the 4–10x small-file
penalty, so it would be slower than any of these.

The cheaper shape avoids the per-file copy:

1. Export the host's common dir read-only, as pointer masks also do.
2. Run `git clone --shared` inside the box onto box-local storage.
3. Check out inside the guest.

The box then writes 100k files to its own disk, not across the share. The
cost is a spec change: the box today has a read-only rootfs and a tmpfs
`/tmp`, so a workspace of that size needs a box-local writable volume,
which is not in the spec. "No disk cap on either runtime" is already an
open question.

Dirty and untracked host state does not travel with a clone. Carrying it
needs the snapshot-and-patch import shape in `alternatives.md`. Its cost is
the hash and diff times above.

### What "attach" means here

There is no new mechanism. minimal's `attach` is an interactive shell in
the session's workspace.

- pinfold has `pinfold box exec BOX --tty --workdir D -- bash` (G9 covers
  workdir and tty) and pi's `pinfold attach`.
- What changes is the role. With pointer masks, the host editor sees the
  box's edits live, and attach is optional. With copy+push, the box's work
  is visible on the host only after a fetch, so attach becomes the only
  live view.
- Host editors, file watchers and language servers lose the workspace.

### Comparison

| | Pointer masks | Copy + push |
|---|---|---|
| Host checkout writable by the box | Yes, minus masked paths | No |
| New admission logic | Masks, layout checks, directory overlap | Fetch namespace, fsck, lifecycle on `down` |
| Topology assumption | Required | None |
| Protected paths | Read-only, direct children only | Proposed in commits, merged by the host |
| Startup | Native share, no copy | Clone or copy; seconds for 100k files, not measured in a box |
| Live host view | Yes | Only through attach or fetch |
| New spec surface | None beyond derived mounts | Box-local volume, fetch transport, `down` semantics |

## Dropped alternatives

- **Case-sensitive APFS volume.** It needs a user-attached volume and moves
  the working files. Relocating a 100,002-file tree took a median 32.96 s.
  It is a fallback only if both masks and copy+push are rejected.
- **SSHFS broker.** It needs a new host service and a startup-privilege
  spec change. Overheads were 5.2x for enumeration, 9.3x for a 64 MiB read
  and 6.5x for atomic edits. The proposal is to drop it.
- **Host ACLs and a separate host identity.** ACLs do not stop the owner's
  chmod, and they do not revoke a descriptor that is already open. A
  separate identity needs a privileged helper. These are not adopted.
