# Generic project commands

2026-11-15. PR 1 of PLAN.md, against 2b7364a. Implements #295's command
frontend and #302/#315. Implementation commit: 7e5eec8.
The foreground owner remains; joining and detached ownership are not
implemented. The initial no-departure claim was incorrect: fresh-disk
`lost+found` cleanup was removed outside the plan. The
[review correction](2026-11-15-project-command-review.md) records its
restoration and the missed status, cwd, path and terminal cases.

## Implementation

`pinfold -- CMD` executes literal UTF-8 argv. Bare invocation prints help
successfully. The Pi/attach dispatch and special Pi symlink interpretation
are removed. Core process-interface exit codes remain unchanged.

One execution context carries cwd, TTY and the optional agent label.
Apple's private environment transport uses fixed stdlib encoding. Linux
extends its existing EXEC frame. Trusted init applies Herdr and terminal
identity after restoring box variables, and initial dimensions before CMD.
Project commands use Runtime::exec directly on Linux.

All projects use their canonical identity, project path and persistent
project/home disks. Mac non-Git folders import nothing. Discovery, config,
protected paths and the local image are checked before project state.
The protected-path check also precedes non-Git configuration creation.
Git initialization, startup push and fast-forward remain unchanged.

Deleted both summary modules, their dispatch, scratch IDs/origin records,
empty-scratch inspection/removal, the now-unused removal guard, disk-lock
helper and signal timeout. Existing crossing and disk export remain.
Host-path diagnostics share control-character escaping, including profile
read/lookup errors. README and ARCHITECTURE describe the intermediate state.
No dependency, test, migration, release or excluded-path change was made.

## Verification

[checks.txt](2026-11-15-project-commands/checks.txt) records builds, lint,
environment, image identity and free disk. Mac used Apple container 1.7.1
and the existing profile-default image. Linux used the stipulated disposable
2-vCPU/4-GB/32-GB VM and a static x64 musl executable. Its small Debian image
was built with [linux-image.sh](2026-11-15-project-commands/linux-image.sh).
All guest commands used two CPUs and 1 GiB memory. Only fixture credentials
were used. No host login or credential store was read.

Run the retained scripts in order, with a short fresh `$W`:

```sh
python3 cases.py "$BIN" "$IMAGE" "$W"
python3 terminal.py "$BIN" "$W"
python3 edges.py "$BIN" "$W"
python3 finish.py "$BIN" "$W"
```

`cases.py` also creates `$W` if omitted. It supplies isolated config/state
roots, the maintenance stamp, disabled update checks and the real cache.
Linux jobs were transferred/executed only through `labvm exec -- bash -lc`.
The final source patch on the VM equals the final local source patch.

[mac.txt](2026-11-15-project-commands/mac.txt) and
[linux.txt](2026-11-15-project-commands/linux.txt) retain commands, statuses
and output excerpts. Paths use `$W`, `$BIN` and `/Users/user` or `/home/user`.
Invalid argument bytes are escaped. Binary output has its byte count and
digest; the driver compares every byte before printing success. Doctor
excerpts retain the affected structured lines. Unrelated fields are omitted.

Both platforms passed literal empty/space/metacharacter/help argv, separate
binary I/O and stderr, exits 0/23/125/126/127 and signal status 143, nested
canonical cwd including a host symlink, exact first-word Herdr precedence,
home/project persistence and non-TTY execution. Concurrent second launch
returned 125/disk-in-use; the first completed and the same second command
then succeeded. No-config, malformed command and missing-image refusals
had successful corrected controls. Protected-path refusals left project
state absent; external roots permitted the same project. Bare help created
neither config nor state. Newline paths could not forge diagnostic lines.
Mac non-Git disks persisted without importing a host fixture, appeared in
config, and received protected-path checks. Linux non-Git launch refused.

[terminal.txt](2026-11-15-project-commands/terminal.txt) shows two distinct
fast starts immediately observing 36x120 and the host TERM/COLORTERM despite
conflicting box values. Native resize then delivered 41x133 to a direct
Perl signal handler. No sleep or retry waits for terminal correctness.
Termios and mutable stdin flags matched their before values. Apple also
reported an additional F_GETFL bit, 0x10000; the raw values are retained.
Development PTY drivers had errors and a Bash trap observer timed out
(checks.txt). The retained observer avoids those driver mechanisms.

Synthetic route requests succeeded on both platforms. Launch/terminal JSON
and egress JSONL contained no fixture credential, argv field or command
output marker. Normal completion used sessions-ended. Aborted development
driver commands also left signal records; these were not ordinary finishes.
Own fixture boxes and disks were absent after cleanup. No Mac image was
created; the Linux probe-commands image and build tag remain. The VM lease
was left active. Final free disk: Mac 56 GiB; Linux 25G.

The suspended suite was not run or replaced. Forced teardown failures and
security-stop status precedence were reviewed in code, not fault-injected.
Real subscription logins, full profile rebuilding and PR 2 are not verified.

## Caller changes

- `pinfold pi`, `attach` and special Pi symlink interpretation are removed.
- Bare invocation is successful help; `--` introduces literal command argv.
- Project infrastructure failures use 125. Command statuses, including
  125/126/127 and signal statuses 128+n, pass through. An observed nonzero
  status survives teardown failure; teardown failure after success uses 125.
- `harness-exited` becomes `sessions-ended`. Exit summaries are removed.
- Mac non-Git disks persist and appear in config; empty work is not deleted.
