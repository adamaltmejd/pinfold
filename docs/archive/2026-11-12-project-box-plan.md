# Project box plan

Date: 2026-11-12, against main 42c140e. Worked out by Adam and Claude
after Astra's review of an earlier proposal; an independent review
against the code is folded in. It would supersede parts of
`2026-11-11-composable-plan.md`, which stays as written. ARCHITECTURE.md
remains the contract until a ticket changes it. Nothing here is
implemented. Adam answered most of the open questions on 2026-11-12;
the answers are decisions 13 to 22, and three questions remain.

Evidence, all dated 2026-11-12: `linux-harness-measurement.md` (merged),
`apple-share-concurrency-probe.md`, `readonly-git-mount-probe.md` and
`wrap-microsandbox-notes.md` (all merged), and
`worktree-readonly-git-probe.md` (PR #291, under review).

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
2. **`pinfold -- CMD` is the only way a user runs something in a box.**
   Every other `pinfold VERB` acts on pinfold from outside. Bare
   `pinfold` is `pinfold --help`. The process interface for programs
   (`pinfold box …`) is unchanged.
3. **Privilege stays as it is.** The host uid, no capabilities, a
   read-only root. System packages are a recipe line and a rebuild. The
   default image is well stocked. Guest `sudo` was considered and
   withdrawn.
4. **Agents are installed in the image at `pinfold build`,** latest at
   that build, with the runtimes. No separate update flag. An agent may
   update itself inside a box. Q10 and Q11 ask how far each half can be
   honoured.
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
12. **PR #286 is closed.** Its scratch-disk fix and route handling are
    carried where they still apply.
13. **A project is the Git root** when there is one, else the current
    folder. Outside Git on Linux, where there is no share, the start is
    refused.
14. **`pinfold config edit`** opens the project's file under pinfold's
    host configuration directory. A per-project recipe lives there too.
15. **A share that holds pinfold's own state, configuration or cache is
    refused,** the project or an extra mount. That also refuses `$HOME`.
16. **A disk mount gets one automatic push** of the host's branches at
    its first start. The exit summary and scratch disks go.
17. **One copy-out command** takes files out of a disk without Git. It
    runs in a throwaway box, as disk export does today. The host never
    mounts a box's disk.
18. **The host `pi` symlink is dropped.** It may return later.
19. **The profile's recipe alone decides which agents are installed.**
    Login routes are configured separately; an unused route is harmless.
20. **A Linux project is always a disk mount.** The share on Linux stays
    a later design.
21. **A project with no configuration is refused,** with a one-line
    command to create it. A moved or renamed project is such a project.
22. **`pinfold -- CMD`** runs the command in the box at the folder
    matching the host's current one. It gets a terminal when pinfold
    has one. Its exit code is passed through, and pinfold's own failures
    use codes a command is unlikely to return. Herdr's agent label is
    the command's first word.

## Operator's calls

These follow from the decisions but were not put to Adam one by one.

- **Login routes, not a harness list, drive a box's login files.**
  Today the spec's `harnesses` key is what writes Codex's config, pi's
  login extension and each agent's placeholder variables. With the key
  gone, the configured routes write them. The files are mounted
  read-only so that an agent's self-update or `/login` cannot overwrite
  them; this is robustness, not a control.
- **Placeholders must not collide.** claude's login is two environment
  variables, and claude prefers pi's `ANTHROPIC_API_KEY` placeholder
  when it sees it. Each agent's variables are scoped to that agent: by
  its own settings file where it has one, else by a wrapper on `PATH`.
- **The check on shared folders moves, it is not deleted.** The refusal
  of a folder holding pinfold's state, configuration or cache lives in
  `trust.rs` today. It moves to mount validation and covers every share
  mount, before any state is created.
- **The Git remote keeps the fetch rules it can.** A remote helper keeps
  bundle verification, `index-pack --fsck-objects` in a quarantine, the
  size and branch limits and the ref-name checks. Whether it can keep
  the single ref transaction and "no object lands from a rejected
  bundle" is unproven, because Git itself updates the refs after a
  helper returns. The remote's guarantee ends at refs: `git pull box`
  checks out box content, which is already under "not protected".
- **Agent pins go; other pins stay.** Firecracker, the guest kernel and
  the builder stay pinned. Codex's host login helper is a row of the
  harness pin file, so that file shrinks to it and the download code
  stays.
- **Stage 1's lifecycle is kept as planned:** the detached owner,
  sessions, join, `config-changed`, `pinfold start` and `pinfold stop`
  (#276, #277). `pinfold -- CMD` is the session client.

## What changes against 2026-11-11

| Decision then | Now |
|---|---|
| 1. The share is the Mac default, Git metadata included | Share by default on the Mac; `.git` read-only in the box (7) |
| 2. Host code execution accepted for shares | Kept (8). Files under the top-level `.git` are no longer box-writable |
| 3. The one-second window accepted | Replaced. Shared Git state lost history in 40 of 40 trials inside 0.75 s. With one writer nothing is damaged; box Git reads can be wrong for about a second |
| 8. Trust unchanged | Deleted with the project-held configuration (10) |
| 10. `pinfold pi`, `claude`, `codex` | `pinfold -- CMD` (2) |
| 11. Worktrees inside the root join its box | Open: Q12 |
| 12. Small image, privilege unchanged | Privilege unchanged; the default image grows (3) |
| 14. Disk projects frozen as they are | Disk mounts stay; their crossing is rebuilt as a Git remote (9) |
| 15. A box carries all three agents, mounted | The image carries whichever agents its recipe installs (4) |
| 16. Outside Git the folder is a share | Kept on the Mac; refused on Linux (13) |

Guarantees: 19 (pinned harness) and 46 (exit summary) are deleted. 12
(trust) is deleted, and its shared-folder and identity assertions move
to 13. 10 (crossing), 13, 18 (configuration as data), 26 and 39 are
rewritten. 44 changes if the Linux build host list grows. 11 and 45
are rewritten for decisions 16 and 17. 5, 8, 43 and the proxy guarantees stand. A
writable share is a new kind in the box spec, where every mount is
read-only today, and the spec's opening and its "Host files" row
change. The profile's existing `share/` mount needs another name.

Code that goes: `trust.rs`'s hash check and `pinfold allow`; the agent
pins and mounts, the spec's `harnesses` key, `harnesses.json` and
`box exec --harness` (#284); `pinfold pi`, `attach`, `pinfold push`,
`pinfold fetch`, the exit summary and scratch handling; the host `pi`
symlink and its install step. The start sync shrinks to the first push
of decision 16.

Caller changes, all hard breaks: the spec's `harnesses` key and
`box exec --harness`; the launch record's `harnesses` field; `pinfold
pi`, `attach`, `push`, `fetch` and `allow`; `pinfold artifacts` output;
the `config` output's mode, source and trust fields.

## Threat model rows that change

- **Host files.** A box writes no host file outside its share mounts.
  On Apple a read-only mount is a mount flag set in the guest: the probe
  showed `ro` on a filesystem whose own option is `rw`. So "No
  privilege" is what keeps every read-only mount read-only on the Mac,
  the project's `.git` included.
- **Not protected, for a share:**
  - Files the box writes run later on the host as the user: Makefiles,
    package scripts, editor configuration. Decision 8.
  - Git itself running box-written files: hook managers whose scripts
    live in the working tree (husky, lefthook, pre-commit),
    `.gitattributes` naming a filter the host already defines, a nested
    `.git` the box creates, and a worktree's or submodule's `.git` file
    it rewrites to point at a Git directory it wrote. Only files under
    the top-level `.git` are read-only.
  - Two writers to one file within about a second. A box edit made
    after a host save-by-rename replaced the host's content in 15 of 15
    trials, and a failed in-place write left the host file empty in 15
    of 15 (#289).
- **Agents are no longer checksum-pinned.** A recipe's install step
  trusts what the vendor serves at build time. The agents run in the
  box, which is untrusted either way.
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

1. Login per agent without a harness list: pi using its route without
   the `-e` flag; claude's two variables scoped to claude by a settings
   file; what collides when all three agents' settings are present.
2. A Git remote helper: a hostile bundle leaves no ref and no object; a
   remote configured to write `refs/heads/*` is refused; a partial
   failure changes no ref; a stopped box is reached through a transient
   one.
3. Worktrees under a read-only `.git`, at the host's path, including a
   box rewriting a worktree's `.git` file.
4. Headless Chrome with its sandbox off, through the proxy.
5. Each agent's updater: where it writes, which hosts it needs, and
   whether today's default env turns it off.
6. The default image: contents, size and build time on both platforms,
   and Linux build egress for the agents' download hosts.
7. Agents under a read-only `.git`: what each does when a Git write
   fails, the profile text that tells it only the host commits, a stale
   read after a host commit, and `git status` time on a large
   repository.

## Order

1. Probes 1 to 3. Probe 3 is run (PR #291).
2. Configuration: mounts as configuration, per-project configuration on
   the host, the hash check deleted, the shared-folder check moved.
3. Agents in the image, with login files driven by routes. Probes 4 to
   6 gate this step. Per-exec agent env is not removed before its
   replacement exists.
4. The detached owner, sessions and join, with `pinfold -- CMD`.
5. The share on the Mac with a read-only `.git`. Probe 7 and the review
   of #289 gate this step.
6. The Git remote; then `push` and `fetch` are removed. Until this
   lands they remain the only crossing on Linux.
7. The suite, rewritten from ARCHITECTURE.md. No release before it.

Issues #276 to #281 are re-scoped after Adam confirms.

## Open questions for Adam

Q1 to Q9 and Q13 are answered; see decisions 13 to 22.

- **Q10. Agents' own updaters.** An updater needs its download host on
  the allowlist, and these are likely shared hosts such as GitHub
  release assets or npm. Today's default allowlist already includes
  `registry.npmjs.org`. Options: (a) On, with their hosts allowlisted.
  (b) Off; a rebuild updates. Recommended: (a) for hosts already
  allowlisted by default, (b) for the rest, decided per agent after
  probe 5.
- **Q11. How a build gets the latest agents.** Adam, 2026-11-12: a
  build should not download an agent it already has; use ordinary
  package caching. Options: (a) The agent step runs on every build with
  the package manager's download cache kept between builds. The package
  manager asks its index for the latest version and downloads only a
  new one. pinfold knows nothing about agents. The layer is rewritten
  each build. (b) pinfold asks each agent's release index for the
  latest version and passes it to the build as an argument, so an
  unchanged version reuses the cached layer exactly. pinfold must then
  know each agent's index. Recommended: (a), if probe 6 shows each
  agent's installer uses a cache and the rewritten layer is cheap.
- **Q12. Worktrees.** Probe 3 (PR #291, under review): in a linked
  worktree under a read-only main `.git`, box Git reads work and box
  Git writes fail. The box can point the worktree's `.git` file at a
  Git directory it wrote, and host Git then ran the box's fsmonitor and
  hooks. Mounting that file read-only did not prevent it, because the
  box can rename the worktree's folder. This is the host execution
  decision 8 accepts, already listed under "Not protected". Options:
  (a) Refuse to share a linked worktree; share only a main checkout.
  (b) Share the main root and let the box work in worktrees beneath it,
  read-only for Git like the rest. Recommended: (b). Whether a nested
  repository the box creates in a main checkout gives the same path was
  not tried; if it does, (a) buys nothing.

## Not established

- Every item under "Probes before tickets".
- Whether a subscription login accepts each agent configured this way.
- How tightly Codex's pinned host helper is coupled to the Codex
  version in the image.
- #291's results, until that record is reviewed.
