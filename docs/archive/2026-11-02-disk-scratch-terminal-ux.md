# Disk, scratch and terminal changes

Adam requested readable disk management, bulk removal, origin paths and
reusable host login. He approved removing both scratch disks after a clean
exit with confirmed empty work, including home-only sessions and logins.

## Contract

- Disk commands are human-readable by default. A trailing `--json` keeps
  the previous structured records and exact byte counts.
- `disk rm --all` attempts every disk in the current state root's snapshot.
  It preserves attached disks, continues failures and exits nonzero if any
  removal fails. It does not detach boxes or act atomically across disks.
- Repository origins use existing checkout records. New scratch records
  identify the starting folder without making it a project or a mount.
  Older scratch and caller disks may have unknown origins.
- Empty scratch cleanup reuses the existing box and disk capabilities.
  After the writable box stops, a no-egress inspector attaches only work
  read-only. Its resolved image must match the original run. Both disk
  claims remain held through observation, teardown and removal checks.
  This can add one transient VM after an apparently empty successful run.
- Unreadable or incomplete observations retain the disks. New-volume
  initialization removes only an empty formatter recovery directory, before
  population. Nonempty recovery content is never removed by that operation.
- Host login routes stay separate from scratch storage. No shared writable
  home, guest auth-store copying, callback forwarding or new auth framework.

## Terminal defect

Installed Apple container 1.6.1 left the inherited terminal input descriptor
nonblocking after successful `pinfold pi --version`. A real PTY observer
measured F_GETFL 2 before and 65542 after, with termios restored.

The command owner now restores the saved mutable file flags before return.
Darwin also exposes its sticky kernel FWASWRITTEN bit through F_GETFL;
the regression test writes and drains one byte before its baseline, then
retains exact equality checks for blocking and nonblocking callers.
This proves the descriptor fix, not every Flyline interaction or fatal-signal
case. No blanket terminal reset or keyboard-protocol pop was introduced.

## Test audit and source review

- G9: real `box exec --tty` on a host PTY preserves its prelaunch flags.
  Omitting restoration reintroduces the observed nonblocking leak.
- G11: bulk removal refuses the live writer's disk; after writers stop,
  the same bulk path removes all remaining fixture disks.
- G13: concurrent scratch A retains and exports known work; B has only
  home data and loses both disks on clean exit; C has unreadable work and
  retains both. Expectations come from fixture bytes and host inventory.
- Existing structured-output consumers explicitly request `--json`.
  No wording, spacing or size-format assertions were added.
- Independent correctness/security and lean/spec/test-audit reviews passed.
  Review caught and resolved unsafe advisory recovery-directory omission
  and unrelated origin-record garbage collection during concurrent startup.

## Validation snapshot

- Formatting, whitespace checks and all-target locked Clippy passed.
- Final ordinary Mac suite: 26 passed, 0 failed, 1 ignored, 95.45 seconds.
  The unchanged real five-minute response gate remains separately ignored.
- Initial failures and the focused scratch fix are retained alongside the
  final run. Apple's runtime rejects bare image IDs; inspection now uses
  the original local reference and checks its resolved ID before observing.
- Linux CI is the next exact-commit gate. No merge, release or installation
  of the candidate is authorized by this validation record.

Private local receipts: the `pinfold-disk-scratch-terminal-2026-11-02`
work archive. Raw logs are not checked in.
