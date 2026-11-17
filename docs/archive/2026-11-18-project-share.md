# Mac project shares (#280)

Implemented and exercised on 2026-11-17; this record uses the date requested
by the brief. Base `bbf396e`; implementation `aa01770`. No test, dependency,
image inspection path, cache, Git wrapper or release was added.

## Implementation and inspected boundaries

Mac configuration creation selects `share`; Linux selects `disk`. Existing
explicit disks remain disks. Shares use the existing core mount admission,
alias walk, Git attachment and attachment order. Project validation calls
that same implementation. Repeated common/profile protected-path checks are
removed from share project admission; its resolved project configuration,
recipe and directory remain additional exclusions, checked in both directions.
Doctor/build retain their complete checks. Disk-only project validation does
not perform share preparation or its init-path lookup.

A share owner allocates only its home disk and skips Git synchronization.
No code deletes or copies back an old project disk. Discovery and maintenance
ownership still use the project identity and labels, independently of project
disk presence. Snapshots add mount kind, canonical paths and generated Git
protection definitions, never contents, HEAD, refs, index or mtimes.
The owner compares Git layout before resolving a new admission. A non-Git
share gaining a Git marker stays unprotected and refuses another admission.
Recorded-root stop bypasses current Git discovery and configuration.

Worktree discovery selects a plain ancestor main root independently of the
pointer. Bounded descriptor-relative no-follow reads parse the marker,
commondir and protected backlink. Inspection: no pointer-selected host Git
is run, and these metadata reads stay beneath that selected root. Runtime
hostile-pointer coverage below observes absence of outside execution; it is
not a syscall trace proving absence of every outside read.

Crossing probes the owner's roots and share kind before reading changed
configuration or taking a crossing lease. The owner also rejects Borrow.
Read-only lock observation waits for launch ownership and leaves dead-owner
recovery to the existing discovery path. Project push/fetch maintenance now
runs after roots/mount admission, before a crossing lease. An initial Linux
candidate reproduced maintenance writes before refusal; that ordering is fixed.

All three native instruction paths share one embedded guidance source.
Missing-only seeding is unchanged. Pi's `/work` advice is removed. README and
ARCHITECTURE describe both kinds, reports, stop, ownership and the accepted
large-tree Git status cost. Root-only Git protection, writable nested Git,
image-alias limits and no cross-box coordination remain as in #328.

## Public CLI observations

Run from this checkout with the existing default image; every script uses
private configuration/state and disposable projects. Scripts and retained
outputs are in [the evidence directory](2026-11-18-project-share/).

- `python3 docs/archive/2026-11-18-project-share/apple.py`:
  [apple.txt](2026-11-18-project-share/apple.txt) records simultaneous first
  sessions sharing one generation, edits from both, cancellation of one while
  the other continues, keep/join/stop and final-departure races, escaped child
  survival until checked box removal, and ordinary dead-owner recovery.
  Protected path names, bytes, mode, device/inode, links, size, uid/gid and
  mtime/ctime match across guest work. Read-induced atime is excluded.
  A host commit permits joins and guest Git reads its new HEAD. A nested
  linked worktree uses the invoking cwd, main owner and ordinary Git reads.
  External/separate Git and a guest-written hostile pointer refuse.
  Changed mount and roots refuse; restored configurations join.
  Shares report null project disks, allocate none and create no host remote.
  Non-Git shares expose files, retain home, refuse gained Git and stop without
  config after a hostile marker appears. Explicit Mac disks isolate edits,
  synchronize, push/fetch, and keep their old project volume after a share
  launch. Command statuses 0, 23, 125, 126 and 127 pass through.
- `python3 docs/archive/2026-11-18-project-share/lifecycle.py`:
  [lifecycle.txt](2026-11-18-project-share/lifecycle.txt) records owner SIGKILL
  during root socket startup, before CMD, and during teardown with the owner
  held at an observed native kill/stop helper before rm. These are the exact
  windows, not every instruction or a post-retiring-receipt crash. Replacement
  is held before native rm: only the old box and claim exist, no replacement
  command executes. After checked removal and retirement a distinct generation
  runs. The completed command retains 0 across teardown-owner death.
  Keep racing final departure retains one generation; stop removes it.
- `python3 docs/archive/2026-11-18-project-share/edges.py`:
  [edges.txt](2026-11-18-project-share/edges.txt) adds non-Git running-share
  crossing after disk-config edits, hostile topology and restored controls,
  untracked Git-subdirectory cwd, linked project config and core protected
  roots with corrected successful controls, and no-config crossing. External
  worktrees/separate Git refuse before configuration or project state exists.
  A direct protected Git write fails read-only while ordinary writes and Git
  reads succeed; the protected snapshot remains unchanged.
- `python3 docs/archive/2026-11-18-project-share/guidance.py`:
  [guidance.txt](2026-11-18-project-share/guidance.txt) observes both guidance
  sections in actual Pi, Claude and Codex fixture HTTP requests. A subsequent
  share-to-disk change preserves existing user instruction content; requests
  carry the user marker and lack the seed guidance. Every response is fixture
  HTTP 400 and agents exit 1: delivery, not compliance or model success.
  No Mac profile/image rebuild was needed or performed.

## Linux, corrections and cleanup

The specified dev hostname failed DNS resolution. The approved disposable
Debian 13 fallback used the homelab-trial-vm skill. Only public code changes
and fixture scripts were transferred. Automatic review initially rejected
transfer as potentially private; GitHub verified public visibility and the
reviewed code-only patch was then approved. No connection bypass was used.

`linux.py` ran in that VM with the real musl binary. Its first run built the
tiny Git fixture image named in [linux.txt](2026-11-18-project-share/linux.txt);
the final-source run reused it. Linux config creation reports disk. With
maintenance due, runtime PATH unavailable and absent state/cache roots, all
four project entrypoints refuse share as spec; configuration, state, cache
and ownership file-name/content inventories remain unchanged. Disk projects
then run, preserve private project/home data, synchronize and push/fetch.
All project boxes/disks and fixtures are removed before VM release.

Driver corrections: project setup refusals are 125, not 1; a non-Git
subfolder is its own project, so the untracked-cwd case uses Git. Local model
fixtures initially hit proxy 403; explicit loopback proxy exclusions fixed
the driver. A short-lived rm helper escaped the first signal gate; holding
the parent at native kill/stop fixed that observation. These were driver
failures, not production failures. The initial compile lacked Deserialize;
the first Linux target installation selected the wrong toolchain. Both were
corrected. The sandbox's sccache denial was handled with RUSTC_WRAPPER empty.

[checks.txt](2026-11-18-project-share/checks.txt) records every Mac build's
free-disk observation, final fmt and both platform clippy results, Linux
builds, final inventory, VM release and free disk. All Mac builds had at least
34 GiB available. No image was created on the Mac. Linux's fixture image goes
with release of the erased disposable VM. No unrelated runtime resource was
removed. Native TTY/transport behavior and durable retirement internals were
inspected as retained implementation; the broader suspended suite was not run.

Caller changes: Mac config creation defaults to share; explicit share config
is accepted only on Mac. Nested linked worktrees beneath a shared main root
join its owner. Config JSON replaces `project.guest` with `project.mount`
(kind/host/guest), and shares report null `project.disks.project`.
Shares have live host edits, no project disk/sync/crossing, share-no-crossing
refusals, topology-sensitive config-changed admission and recorded-root stop
with damaged markers. Built-in native instructions cover both kinds, with
the missing-only limit. Project crossing maintenance is deferred until roots
and mount admission. No CLI flag or exit code changes. No design departure;
no migration, copy-back, performance workaround, push or PR.
