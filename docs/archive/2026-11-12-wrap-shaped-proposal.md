# Proposal: keep the core, reshape the layer above it

Date: 2026-11-12, against main 42c140e. A proposal, not a decision.
It would supersede parts of `2026-11-11-composable-plan.md`, which stays
as written. ARCHITECTURE.md remains the contract until a ticket changes
it. Nothing here is implemented. An independent review's findings are
folded in.

Evidence: `2026-11-12-wrap-microsandbox-notes.md`,
`2026-11-12-linux-harness-measurement.md`, and
`2026-11-12-agent-verbs-verification.md`, which is on PR #286, unmerged.

Marks: [code] read from code or spec, [susp] suspected, not tested.

## Why

- Adam asked whether the surface could be `pinfold -- CMD`, as in
  tobi/wrap, and whether agents could be installed in the image.
- wrap already has the shape his goal describes: a live project share,
  one entry point, agents installed at build. Its root in the guest is
  not taken up; see change 1.
- Carrying three agents costs 7.5 s on every Linux box start (11.16 s
  against 3.65 s). This alone does not justify change 3; see the
  alternative there.

wrap and its engine, microsandbox, are not a replacement. They have no
host-side subscription login, their egress control intercepts TLS, and
several allowlist bypasses are reported against it (not checked). wrap
does not build on macOS today.

## Threat model, as Adam stated it on 2026-11-12

1. **Primary: an agent with unfettered access to what the host can
   reach.** Personal data on research servers and elsewhere, through the
   user's files, credentials and network position.
2. **Prompt injection**, as the way an agent is turned.
3. **Secondary: local, unsafe or malicious models** under test.

ARCHITECTURE.md's threat model opens with prompt injection only. If this
proposal goes ahead, that section is reworded to lead with host data.

One tension to settle, Q12: decision 2 of the 2026-11-11 plan accepts
that files an agent writes into a share (hooks, Makefiles, editor
config) later run on the host as the user. That path leads from a
turned agent to exactly the data in item 1.

## What stays

- The runtimes: Apple `container` and Firecracker.
- No network interface in a box. One proxy socket. The allowlist and the
  proxy rules, with no TLS interception.
- Login routes. Credentials stay on the host.
- Host files and profiles: a box writes no host file outside its share.
- Trust: `pinfold allow` and the hash check at every box start.
- The Linux VMM jail.
- No privilege in a box: the host uid, no capabilities, a read-only
  root, nested user namespaces off.
- No secret value in argv, logs or any file.
- From the 2026-11-11 plan: decisions 1 to 9, 11, 13, 14 and 16, and
  stage 1's detached owner, sessions and join as planned (#276, #277;
  not built yet).

## Proposed changes

Each is marked **Adam said** or **proposed**. A proposed item is the
operator's recommendation and needs Adam's answer. Each names the
guarantee it changes.

1. **Privilege stays as it is. The default image is well stocked.**
   Adam said, after first choosing passwordless `sudo`. Box work keeps
   the host uid, no capabilities and a read-only root; decision 12 and
   guarantee 5 stand. System packages are a line in a profile or
   project recipe and a rebuild, where `apt` runs as root. Tools that
   install under the home directory (mise, uv, npm, cargo, pixi) work
   in a box as they are. The built-in default profile gains compilers,
   common `-dev` libraries, mise and pixi; the list is a ticket.
   The reasons are under "Why sudo was withdrawn".
2. **`pinfold -- CMD` is the only way to run something in a box.** Adam
   said. Every other `pinfold VERB` acts on pinfold from outside. Bare
   `pinfold` is `pinfold --help`. A session is one command in the
   project's box; a shell is `pinfold -- bash`. Replaces decision 10;
   touches guarantee 39's `harness-exited` record. Deletes
   `pinfold pi`, `pinfold claude`, `pinfold codex` and `attach`. It is
   not fully command-agnostic: Q9.
3. **Agents are installed in the image at `pinfold build`.** Proposed.
   Replaces decision 15; changes guarantees 19 and 26. Deletes harness
   mounts, the spec's `harnesses` key, `harnesses.json` and
   `box exec --harness` (#284, merged).
   **Alternative for the cost alone:** build one disk image per pinned
   harness version and reuse it. That removes the 7.5 s with no change
   of model. Change 3 then rests only on preference: agents as ordinary
   software in the image, on `PATH`.
4. **Latest at build, or pinned.** Open: Q4.
5. **Agent environment.** Proposed: set every agent's placeholders for
   the whole box if the probe passes; otherwise a wrapper per agent in
   the image. Touches guarantee 26. Known and suspected collisions are
   listed below.
6. **Chrome and Codex run with their own sandboxes off.** Adam agreed
   for Chrome, noting that egress is also heavily limited; Codex is
   proposed on the same reasoning.
   Their sandboxes protect the box's contents from a hostile page or
   tool call. The agent in the box is already assumed steerable by
   hostile content, and the box is the boundary. wrap ships Chrome with
   `--no-sandbox`. Stage 3's first spike reduces to whether headless
   Chrome starts and works through the proxy.

Not proposed: any change to disk projects (decision 14), nested
containers, ports, or the share on Linux.

## Why sudo was withdrawn

- With a root that resets at each box start, an `apt install` in a box
  lasts one session. Anything kept has to be in a recipe anyway.
- A recipe change is reviewed: a project recipe needs `pinfold allow`.
- Root in the guest opened risks the review read from code or
  suspected. The most serious: every read-only mount in an Apple box is
  a live host directory with read-only passed as a mount option [code],
  including pinfold's cached init and agent binaries. The share probe
  found `mount -o remount,rw` works with `CAP_SYS_ADMIN`. If read-only
  holds only in the guest, root rewrites files every later box runs
  [susp]. Others: forging PID 1's clean-unmount answer on Linux, writing
  raw disk blocks that other boxes' kernels then parse, leaving the
  work cgroup, and mixed uids breaking Git on an Apple share.
- None of these arise with today's rules.

The cost: an agent that needs a system library no home-directory tool
provides must stop for a recipe change, a rebuild and a box restart.
Revisit if that is frequent in practice.

## Box-wide env: collisions

- claude prefers `ANTHROPIC_API_KEY` over its token (observed, #286's
  record). #287 is the prerequisite fix.
- `ANTHROPIC_BASE_URL` pointing at claude's route would be seen by any
  Anthropic client in the box, pi included when it has no Anthropic
  route of its own [susp].
- pi's `OPENAI_API_KEY` placeholder may switch Codex to API-key mode
  [susp].
- Any command, such as `pinfold -- make test`, inherits every
  placeholder and base URL.

## What happens to stage 1

- **Keep as planned:** the detached owner, sessions, join, the launch
  lock, `config-changed`, `pinfold start` and `pinfold stop`.
- **Remove, with a ticket:** #284's harness list, mounts and per-exec
  env, if change 3 is accepted.
- **#286 (unmerged):** its verbs go under change 2. Its start sync for
  any command, scratch-disk fix and route handling are still wanted. Q8.
- **#278:** closes as superseded if changes 2 and 3 are accepted.

## Proposed order

1. Probes, and Adam's answers to Q4 and Q10, which gate step 2.
2. Agents in the image, or the per-version image cache.
3. The detached owner and sessions, with `pinfold -- CMD` as the client.
4. Join.
5. Stage 2 as planned: the share on the Mac.

The suite stays removed; AGENTS.md's suspension holds.

## Probes before tickets

1. **Box-wide env**, with #287 fixed. A fixture shows what each agent
   sends. Whether a subscription accepts it needs Adam's own login.
2. **pi's login extension** when pi is installed, not mounted.
3. **Headless Chrome** with `--no-sandbox` through the proxy, as the
   host uid.
4. **The default image's contents:** which tools cover Adam's projects
   without `apt` in a box, and the image's size and build time.

## Questions for Adam

- **Q1 to Q3. Closed.** Privilege, the root filesystem and nested user
  namespaces stay as they are.
- **Q4. Agent versions.** (a) Latest at build. (b) Pinned in the binary,
  installed at build. Recommended: (b) first, (a) later if wanted.
  What (a) costs beyond the hash check:
  - Login routes promise each agent's configuration "for the pinned
    version"; under (a) that stops being a guarantee.
  - "Latest" is latest at first build. Layers are cached, so a refresh
    needs a rule (a cache-busting layer or `--no-cache`) and a version
    label on the image; no `image-outdated` notice can fire.
  - Linux builds have a fixed egress list; an agent's download host may
    not be on it [susp].
  - It deletes less than first claimed: the pin and download code stays
    for Firecracker, the kernel, the builder and Codex's host helper.
- **Q5. Agent self-updaters.** Recommended: off, by each agent's own
  setting. claude's updater writes under the home directory, which
  persists, so an update would shadow the image's version.
- **Q6. Linux too?** Recommended: yes for changes 2 to 6.
- **Q7. Codex's sandbox.** Under change 6, `pinfold -- codex` passes
  `--sandbox danger-full-access` unless the user gives a sandbox flag.
- **Q8. #286.** (a) Close it and carry its three kept parts into the new
  work. (b) Merge it and delete the verbs later. Recommended: (a).
- **Q9. What stays agent-specific under `pinfold -- CMD`.** pi uses its
  login routes only when started with its extension; Codex needs its
  sandbox flag; Herdr needs the agent's name; the terminal record names
  the harness; a login route must name a known agent. Options: (a) a
  small table of known agents in the binary, keyed by the command's
  first word. (b) Push each into the image: seeded pi settings, a
  wrapper per agent. Recommended: (a).
- **Q10. Where the install lives.** (a) In the built-in profile recipes:
  images built otherwise have no agents. (b) pinfold adds its own final
  layer to every build: a new build step, with consequences for recipe
  hashing and trust. Recommended: (b), after reading how the image
  identity is computed.
- **Q11. Confirm deleting** the three verbs, `attach`, and core's
  `harnesses` key and `box exec --harness`.

- **Q12. Host execution of agent-written files in a share.** Given the
  threat model above, is decision 2 still accepted as is? Options: (a)
  accept, as decided on 2026-11-11. (b) Accept, and have pinfold warn
  when a session changed files that commonly execute on the host (Git
  hooks and config, editor and tool configuration). (c) Keep `.git`
  out of the share. Recommended: (b); (c) breaks the goal of agents
  committing in place.
- **Q13. TLS interception.** Not built today. The proxy tunnels HTTPS to
  allowlisted names without reading it, and credentials are added on
  routes, where the box speaks plain HTTP to the proxy and the proxy
  speaks TLS upstream. Interception would add: rules by path and method
  (for example read-only GitHub), a check against Host-header fronting,
  which the spec lists as not protected, and credential injection for
  tools that cannot be pointed at a route. It costs: a CA in every trust
  store in the guest, breakage with pinned certificates, a large parser
  on the host reading hostile bytes, and the host seeing all plaintext.
  microsandbox's reported bypasses are in this code. Recommended: not
  now; revisit if "allowlisted services that accept writes" becomes the
  exfiltration path that matters.

## Not established

- Every [susp] above.
- Whether box-wide env works for claude on a subscription.
- Build time and image size with three agents installed.
- How tightly Codex's pinned host helper is coupled to the Codex version
  in the image.
