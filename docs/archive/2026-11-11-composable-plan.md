# Composable pinfold: agreed plan

Recorded 2026-11-11 against main 1de856099daf98cacb8b0bac916f6952aaca8d35.

This follows Astra's draft, `2026-11-11-composable-vm-design-draft.md`,
kept as written. Adam and Claude worked through it on 2026-11-11, and
Adam confirmed it that day. ARCHITECTURE.md remains the contract. Each
ticket below changes it in its own commit. Nothing here is implemented.

Evidence: `2026-11-11-apple-share-probe.md`.

## Goal

Agents in boxes, agents on the host and the user work in one project
folder at the same time. An untrusted agent can make a quick edit with
no step to move files in or out.

Pi, Claude Code and Codex run the same way, directly or from Herdr.

## Terms

The existing terms stay: box, project, disk, profile. The draft's "VM"
and "workspace" are not used. One term is added, in stage 2:

**Share**: a host directory a box reads and writes live.

## Decisions

1. **The share is the default on the Mac.** The agent writes the project
   folder directly, Git metadata included.
2. **Host code execution is accepted for share projects.** Hooks, package
   scripts, Makefiles and editor config the agent writes run later as the
   user, outside the allowlist. The threat model lists this as not
   protected. For a share project the allowlist bounds only what runs in
   the box.
3. **The one-second window is accepted.** On Apple `container` a host
   delete, rename, replace or chmod is stale in the box for about one
   second after the box last looked at that path. A box that commits
   while its view of a branch is stale can replace the branch with a
   parentless commit, leaving the history in the reflog. The probe
   produced this once. It is listed as not protected.
4. **One box per project.** A detached per-box owner process holds the
   box, its proxy and its credentials. There is no global daemon. Owner
   death still fails closed.
5. **Sessions.** A later launch in the same project joins the running box
   as a new session. It never starts a second box and never changes the
   running box's policy. If the configuration differs from what the box
   started with, the launch is refused and names the difference.
6. **A box ends when its last session exits.** `pinfold start` keeps it
   past its last session, and `pinfold stop` ends it. A flag cannot do
   this, because agent arguments pass through. One agent exiting never
   stops another.
7. **No "VM build".** Only images are built. Policy is read at box start,
   so a policy change means stop and start. The home disk survives
   everything but explicit removal.
8. **Trust is unchanged.** `.pinfold.toml` and the project recipe are
   hash-checked at every box start. A change refuses the start until
   `pinfold allow`.
9. **Login routes are the baseline.** Native login in the box stays
   available as today, with no new work.
10. **Three agents.** `pinfold pi`, `pinfold claude` and `pinfold codex`
    use one launch path. Pi-specific parts stay pi-only.
11. **Worktrees.** The project is the main repository root. A linked
    worktree inside that root uses the same project, box and home; its
    session starts in the worktree folder. A worktree outside the root
    is refused as `worktree`.
12. **Small image first.** Box privilege stays as it is: one uid, no
    capabilities, no nested user namespaces. The browser image follows
    once the small one is verified, as a profile recipe.
13. **Mac first for the share.** Linux keeps Firecracker and disk
    projects.
14. **Disk projects stay as they are.** The project disk, `push`,
    `fetch`, scratch and disk export get no new work. They are today's
    isolated mode on both platforms. A later isolation design, likely
    using Git remotes, replaces them.
15. **A box carries all three agents.** Today a box spec names one
    harness, and a login route must name it. A project's box instead
    starts with pi, claude and codex and every configured login route,
    so any of them can join later. No configuration key names agents.
    This holds if the Linux start cost measures acceptable; otherwise a
    key names them.
16. **Outside Git, the folder is a share**, on the Mac, built with
    stage 2. A folder holding pinfold's state, configuration or cache is
    refused as `host-path-in-project`, which also refuses `$HOME`.
    Scratch stays where there is no share.

## Not in this plan

- Port exposure to the host.
- Desktop image, guest root, nested containers. The browser image is
  stage 3.
- A share on Linux, Cloud Hypervisor, Linux arm64.
- Movable disks, copy-in, new disk modes.
- Idle stop.
- Worktrees outside the repository root.

## Stage 1: sessions and agents on disk projects

Both platforms. No threat-model change.

| Ticket | Change | Guarantee |
|---|---|---|
| 1.1 | A detached owner holds a project's box. The box ends when its last session exits, on `pinfold stop`, or on owner death. `pinfold start` keeps it past the last session. | New: a project's sessions share one box. Guarantee 8 holds for the detached owner. |
| 1.2 | A second launch joins the running box. A launch whose configuration differs from the box's is refused with its own token. | Same new guarantee. Replaces the pi layer's refusal of a second start as `disk-in-use`. |
| 1.3 | A box spec takes a list of harnesses, with a login route for each. `pinfold claude` and `pinfold codex` beside `pinfold pi`, on one launch path. | Extends guarantees 19 and 26: each agent launches pinned, and each login stays on the host, in one box. |

Stage 1 does not relieve the friction in the goal. Agents still need
`fetch` to get work out of a project disk.

## Stage 2: the share, on the Mac

| Ticket | Change | Guarantee |
|---|---|---|
| 2.1 | The box spec names a share. Apple only; Linux refuses it before creating anything. Threat model and "Not protected" gain decisions 2 and 3. | New: a box writes only its share. Guest writes appear on the host; host files outside the share are unchanged. Narrows guarantee 10 to disk projects. |
| 2.2 | A project is a share or a disk. Share is the Mac default. The project is the main repository root; inside worktrees join its box; outside worktrees are refused. Outside Git the invoking folder is the share, refused when it holds pinfold's own roots. No start sync, exit summary, `push` or `fetch` for a share project. | Edits guarantee 13. Guarantee 12 holds with the configuration inside the share, and its `host-path-in-project` refusal covers a folder outside Git. |
| 2.3 | Retest the one-second window on each Apple `container` release. | Standing ticket; closes when the window is gone. |

Constraint for 2.1: every process that looks at a share runs as the host
uid. A lookup under another uid changes the owner the box reports and
breaks Git. Apple's root exec in lifecycle step 6 touches only the proxy
socket.

## Stage 3: the browser image

Adam needs this soon after the small image is verified. It is a profile
recipe, not a core concept. Two questions need a spike before tickets:

- Whether headless Chrome runs with nested user namespaces off and
  `no_new_privs` on (decision 12, #270, #271), and what it needs if not.
- Whether a browser works through the proxy as it is: port 443 and 80
  only, no WebSocket on routes, allowlist by name.

## Later, each its own design

- Port exposure, against the no-interface rule.
- The share on Linux. virtiofsd's cache policy is configurable there;
  its place relative to the VMM jail (guarantee 43) is undecided.
- Isolation redesign, and deleting crossing, export and scratch.
- Outside worktrees.

## Outside worktree layout, for the later ticket

```
~/code/repo/.git/                  objects, refs, hooks, config
~/code/repo/.git/worktrees/x/      this worktree's HEAD and index
~/elsewhere/x/repo/                the worktree's files
~/elsewhere/x/repo/.git            file: "gitdir: ~/code/repo/.git/worktrees/x"
```

A box would need both the worktree and `~/code/repo/.git` writable, at
their host paths. It could not join the main repository's box, because
shares are fixed at start. Two boxes would then write one `.git`. The
Codex app keeps worktrees this way. Claude Code keeps them inside the
repository.

## What changed from the draft

- Shared mode's security statement was wrong: it claimed no egress
  bypass. Decision 2 replaces it.
- The draft selected the alternative that 2026-10-19-vm-disk-design.md
  rejected. Decisions 1 to 3 are the recorded reason: the goal needs live
  shared files, and the cost is stated.
- Persistent VMs had no owner. Decision 4.
- A configuration change reset the home. Decision 7.
- Native login was the baseline. Decision 9.
- Separate worktrees had separate VMs. Decision 11.
- Three image tiers, five workspace choices, movable disks, ephemeral
  lifetime as a mode, port forwarding and Cloud Hypervisor are out or
  deferred.
