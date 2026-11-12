# Project box plan

Date: 2026-11-12, against main 42c140e. Worked out by Adam and Claude
after Astra's review of an earlier proposal. It would supersede parts of
`2026-11-11-composable-plan.md`, which stays as written. ARCHITECTURE.md
remains the contract until a ticket changes it. Nothing here is
implemented, and the open questions at the end are not answered.

Evidence, all dated 2026-11-12: `linux-harness-measurement.md` (merged),
`apple-share-concurrency-probe.md` (PR #289, not yet reviewed),
`readonly-git-mount-probe.md` and `wrap-microsandbox-notes.md` (PR #290).

## Goal and threat model

Goal, unchanged: agents in boxes, agents on the host and the user work
in one project folder at once. An untrusted agent makes a quick edit
with no step to move files.

Threat model, in Adam's order:

1. An agent with unfettered access to what the host can reach: personal
   data on research servers and elsewhere, through the user's files,
   credentials and network position.
2. Prompt injection, as the way an agent is turned.
3. Secondary: local, unsafe or malicious models under test.

## Decided by Adam

1. **Keep the core.** No network interface in a box, the allowlisting
   proxy without TLS interception, host-side login routes, the Linux VMM
   jail. tobi/wrap and microsandbox are not adopted.
2. **`pinfold -- CMD` is the only way to run something in a box.** Every
   other `pinfold VERB` acts on pinfold from outside. Bare `pinfold` is
   `pinfold --help`.
3. **Privilege stays as it is.** The host uid, no capabilities, a
   read-only root. System packages are a recipe line and a rebuild. The
   default image is well stocked. Guest `sudo` was considered and
   withdrawn.
4. **Agents are installed in the image at `pinfold build`,** latest at
   that build, with the runtimes. A build always installs the latest;
   there is no separate update flag. An agent may update itself inside a
   box.
5. **Agent settings are configuration,** carried by the profile. The
   default profile runs each agent with its own sandbox and permission
   prompts off. Chrome runs with its sandbox off.
6. **The project folder is a mount, and a mount is a share or a disk.**
   Configuration names mounts directly. "Isolated" is a disk mount, not
   a mode.
7. **In a share, only the host commits.** The box sees the project's
   `.git` read-only.
8. **A share accepts host execution of what the box writes.** pinfold is
   not the only precaution there: a more trusted agent on the host
   operates the box and the repository. Where pinfold is the only
   precaution, Adam uses a disk mount.
9. **Disk crossing is cleaned out.** Commits cross only through a Git
   remote. `pinfold push`, `pinfold fetch` and the rest go.
10. **Per-project configuration moves out of the project folder** to
    pinfold's host configuration. `pinfold allow` and the hash check go.
11. **An egress report for the operating agent** is wanted, later.
12. **PR #286 is closed.** Its start sync for any command, scratch-disk
    fix and route handling are carried into the new work where they
    still apply.

## Operator's calls

These follow from the decisions but were not put to Adam one by one.

- **Agents' own updaters write to the home disk,** since the image is
  read-only; that copy takes precedence on `PATH`. An updater is left on
  only if its download host can be allowlisted without opening a
  shared-tenant domain. Otherwise it is off and a rebuild updates.
- **Each agent learns its login route from a file pinfold writes at box
  start and mounts read-only,** as Codex's `/etc/codex` is today. It is
  not placed in the home directory, which the box can write. Unproven
  for claude and pi; a wrapper on `PATH` is the fallback.
- **The Git remote wraps today's validated fetch.** `git fetch box` runs
  a pinfold remote helper that keeps the spec's rules: one ref namespace
  under `refs/remotes/`, no hook or config crossing, `index-pack
  --fsck-objects` in a quarantine, one ref transaction. `git push box`
  sends branches in the same way `push` does today. Unprobed.
- **Agent pins go; other pins stay.** Firecracker, the guest kernel, the
  builder and Codex's host login helper remain pinned in the binary.
- **Stage 1's lifecycle is kept as planned:** the detached owner,
  sessions, join, `config-changed`, `pinfold start` and `pinfold stop`
  (#276, #277). `pinfold -- CMD` is the session client.

## What changes against 2026-11-11

| Decision then | Now |
|---|---|
| 1. The share is the Mac default, Git metadata included | Share by default on the Mac; `.git` read-only in the box (7) |
| 2. Host code execution accepted for shares | Kept (8). The project's own hooks and Git config are no longer writable by the box |
| 3. The one-second window accepted | Replaced. Shared Git state lost history in 40 of 40 trials inside 0.75 s. With one writer nothing is damaged; box Git reads can be wrong for about a second |
| 8. Trust unchanged | Deleted with the project-held configuration (10) |
| 10. `pinfold pi`, `claude`, `codex` | `pinfold -- CMD` (2) |
| 12. Small image, privilege unchanged | Privilege unchanged; the default image grows (3) |
| 14. Disk projects frozen as they are | Disk mounts stay; their crossing is rebuilt as a Git remote (9) |
| 15. A box carries all three agents, mounted | The image carries whichever agents its recipe installs (4) |
| 16. Outside Git the folder is a share | Open: Q1 |

Guarantees affected: 12 (trust) and 19 (pinned harness) are deleted. 13,
26, 39 and 46 are rewritten. 5, 8, 43 and the proxy guarantees stand.
The count goes down.

Code that goes: `trust.rs` and `pinfold allow`; the harness pins,
download and mounts, the spec's `harnesses` key, `harnesses.json` and
`box exec --harness` (#284); `pinfold pi`, `attach`, `pinfold push`,
`pinfold fetch`, the start sync, the exit summary and scratch handling.

## Threat model rows that change

- **Host files.** A box writes no host file outside its share mounts.
  On Apple a read-only mount is a mount flag set in the guest: the probe
  showed `ro` on a filesystem whose own option is `rw`. So "No
  privilege" is what keeps every read-only mount read-only on the Mac,
  the project's `.git` included.
- **Not protected, for a share:**
  - Files the box writes run later on the host as the user: Makefiles,
    package scripts, editor configuration. Decision 8.
  - A nested `.git` the box creates, or a worktree's or submodule's
    `.git` file it replaces. Only the project's top-level `.git` is
    read-only, and mounts are fixed at box start.
  - Two writers to one file within about a second. A box edit made
    after a host save-by-rename replaced the host's content in 15 of 15
    trials, and a failed in-place write left the host file empty in 15
    of 15 (#289, unreviewed).
- **Egress.** Unchanged, except that the Linux build's fixed host list
  must admit the agents' download hosts. That list is a control, so the
  additions are a spec change with named hosts.

## Considered and rejected

- **Adopting wrap or microsandbox.** No host-side subscription login;
  TLS interception; a 5 s stale window on their share; wrap does not
  build on macOS at the revision read.
- **Guest `sudo`.** An `apt install` in a box lasts one session with a
  reset root, and root in the guest would defeat Apple's guest-side
  read-only flag.
- **A shared, writable `.git`.** History loss and deleted files whenever
  both sides write Git state within a second. No mitigation tried was
  general.
- **The box keeping its own Git state over a shared folder.** Box
  commits look like uncommitted changes on the host and can be committed
  twice.
- **TLS interception.** Not needed for the allowlist or for login
  routes. Revisit if allowlisted services that accept writes become the
  exfiltration path that matters.
- **A per-version disk image cache for mounted agents.** It would fix
  the Linux start cost, but agents in the image fix it at the source.

## Probes before tickets

1. Each agent reading its login route from a mounted settings file:
   claude, and pi without its `-e` extension flag.
2. A Git remote helper over the validated fetch, both directions.
3. Worktrees inside the root under a read-only `.git`, at the host's
   path.
4. Headless Chrome with its sandbox off, through the proxy.
5. Each agent's updater: where it writes, which host it needs.
6. The default image: contents, size and build time on both platforms,
   and Linux build egress for the agents' download hosts.
7. A box Git read retried after a host commit: what an agent sees, and
   what the profile should tell it.

## Order

1. Probes 1 to 3, and Adam's answers below.
2. Configuration: mounts as configuration, per-project configuration on
   the host, trust deleted.
3. Agents in the image; the harness machinery removed.
4. The detached owner, sessions and join, with `pinfold -- CMD`.
5. The share on the Mac with a read-only `.git`.
6. The Git remote; the old crossing removed.
7. The suite, rewritten from ARCHITECTURE.md. No release before it.

Issues #276 to #281 are re-scoped after Adam confirms.

## Open questions for Adam

- **Q1. What is a project?** With configuration off the project and
  scratch gone, a project could be the folder `pinfold` is run in, or
  the Git root above it. Outside Git on Linux, where there is no share,
  the choices are to refuse or to give an empty disk. Recommended: the
  Git root when there is one, else the current folder; outside Git on
  Linux, refuse.
- **Q2. How is per-project configuration created and edited?**
  Recommended: `pinfold config edit` opens the project's file under
  pinfold's configuration directory; a project with none uses its
  profile alone. A per-project recipe moves there too.
- **Q3. Which folders may be shared?** Today a project holding pinfold's
  own state, configuration or cache, which includes `$HOME`, is refused.
  Recommended: the same refusal for every share mount, extra ones
  included.
- **Q4. A disk mount starts empty.** With the start sync gone, the first
  step is `git push box`. The exit summary and scratch disks go too.
  Confirm, or keep one automatic first push.
- **Q5. Getting files out of a disk without Git.** The host cannot mount
  the disk: macOS has no ext4, and the design keeps the host kernel from
  parsing a box's filesystem. Recommended: keep one copy-out command
  that runs in a throwaway box, as disk export does today.
- **Q6. The host `pi` symlink.** It makes plain `pi` on the host run in
  a box. Recommended: keep it as a shorthand for `pinfold -- pi`.
- **Q7. Which agents does a profile install?** Recommended: the
  profile's recipe alone decides; no separate key. Login routes for an
  agent that is not installed are left out.
- **Q8. Linux.** Everything here except the share applies on Linux, so a
  Linux project is always a disk mount. Confirm that the share on Linux
  stays a later design.

## Not established

- Every item under "Probes before tickets".
- Whether a subscription login accepts each agent configured this way.
- How tightly Codex's pinned host helper is coupled to the Codex
  version in the image.
- #289's results, until that record is reviewed.
