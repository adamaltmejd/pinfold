# Proposal: keep the core, reshape the layer above it

Date: 2026-11-12, against main 42c140e. A proposal, not a decision.
It would supersede parts of `2026-11-11-composable-plan.md`, which stays
as written. ARCHITECTURE.md remains the contract until a ticket changes
it. Nothing here is implemented.

Evidence: `2026-11-12-wrap-microsandbox-notes.md`,
`2026-11-12-linux-harness-measurement.md`,
`2026-11-12-agent-verbs-verification.md` (on PR #286).

## Why

Three things came up while building stage 1.

- Carrying three agents costs 7.5 s on every Linux box start, because
  each mounted harness is re-imaged (11.16 s against 3.65 s).
- Adam asked whether the surface could be `pinfold -- CMD`, as in
  tobi/wrap, and whether agents could be installed in the image.
- Reading wrap showed it already has the shape Adam's goal describes: a
  live project share, one entry point, agents installed at build, root
  in the guest.

wrap and its engine, microsandbox, are not a replacement. They have no
host-side subscription login, their egress control intercepts TLS and
has a record of bypasses, and wrap does not build on macOS today.

## What stays

- The runtimes: Apple `container` and Firecracker.
- No network interface in a box. One proxy socket. The allowlist and the
  proxy rules, with no TLS interception.
- Login routes. Credentials stay on the host.
- Trust: `pinfold allow` and the hash check at every box start.
- The Linux VMM jail.
- No secret value in argv, logs or any file.
- From the 2026-11-11 plan: decisions 1 to 9, 11, 13, 14 and 16, and
  stage 1's detached owner, sessions and join (#276, #277).

## Proposed changes

Each is marked **Adam said** or **proposed**. A proposed item is the
operator's recommendation and needs Adam's answer.

1. **Rely on the VM boundary for privilege.** Adam said, 2026-11-12:
   relying on the boundary makes work in the box much easier. Replaces
   decision 12 and the "No privilege" control. The variant is open; see
   Q1 to Q3 and "Guest privilege" below.
2. **`pinfold -- CMD` is the one entry point.** Proposed. A session is
   one command in the project's box. `pinfold -- claude`, `pinfold --
   make test`, `pinfold -- bash`. Replaces decision 10. Deletes
   `pinfold pi`, `pinfold claude`, `pinfold codex` and `attach`. The
   host `pi` symlink maps to `pinfold -- pi`.
3. **Agents are installed in the image at `pinfold build`.** Proposed.
   Replaces decision 15 and its Linux condition. Deletes harness mounts,
   the box spec's `harnesses` key, `harnesses.json` and
   `box exec --harness` (#284, merged), and the per-start imaging cost.
4. **Latest at build, or pinned.** Open; see Q4.
5. **Agent environment.** Proposed: set every agent's placeholders for
   the whole box, if the probe below passes. Otherwise a wrapper per
   agent in the image sets them. Either way pinfold stops deciding env
   per exec.

Not proposed: any change to disk projects (decision 14), nested
containers, ports, or the share on Linux.

## Guest privilege: what the spec enforces in the guest today

Read from ARCHITECTURE.md at 42c140e, "Always applied" and guarantee 5.
The right-hand column is the operator's reading, not tested.

| Control today | If box work can become root |
|---|---|
| Box work runs as the host uid, capability bounding set empty, `no_new_privs` | Goes. This is the change. |
| Read-only root, no setuid files | A read-only root makes `apt install` fail. A writable root needs a decision: reset each start, or kept. Q2. |
| `user.max_user_namespaces` is 0 (#270, #271) | Goes if nested namespaces are wanted (Chrome's sandbox, bubblewrap for Codex). Q3. |
| No network interface but loopback | Unchanged. Root in the guest cannot create a device the VM does not have. Firecracker has no network device; Apple runs `--network none`. |
| Linux: PID 1 is root with only `CAP_SYS_ADMIN`; the exec service is the host uid; box work cannot leave its cgroup or read the service's descriptors | Root box work can signal or trace PID 1 and the exec service. They answer the unmount at box end and carry exec streams. What a hostile PID 1 could make the host do needs a code read. |
| Linux: vsock is the transport | Root can open vsock connections to the host. Which host ports listen, beyond the proxy, needs a code read. |
| Apple: PID 1 is the host uid; one root exec chmods the proxy socket | Root box work can replace or remove the socket. It loses its own egress and gains none. |
| Apple share: every process looking at a share runs as the host uid (stage 2 constraint) | Breaks. The probe saw the owner flip and Git refuse after an exec as another uid. Needs the probe below. |
| The VMM jail (Linux, guarantee 43) | Unchanged, and it matters more: it is what an escape meets. The Mac has no equivalent. |

What is given up in every variant: a second layer. Guest root can load
kernel modules and drive the virtual devices directly, so the VMM's
device emulation is the whole boundary.

## What happens to stage 1

- **Keep:** the detached owner, sessions on the control socket, join,
  the launch lock, `config-changed`, `pinfold start` and `pinfold stop`
  (#276, #277).
- **Remove, with a ticket:** #284's harness list, mounts and per-exec
  env, if change 3 is accepted.
- **#286 (unmerged):** its verbs go under change 2. Its start sync for
  any command, its scratch-disk fix and its handling of other agents'
  routes are still wanted. Q8.
- **#278:** closes as superseded if changes 2 and 3 are accepted.

## Proposed order

1. Probes below. They decide Q1, Q2 and change 5.
2. Agents in the image; remove the harness mounts.
3. The detached owner and sessions, with `pinfold -- CMD` as the client.
4. Join.
5. Guest privilege, in the variant chosen.
6. Stage 2 as planned: the share on the Mac.

The suite stays removed; AGENTS.md's suspension holds.

## Probes before tickets

- **Mixed uids on an Apple share.** Run `sudo` in a share box and check
  whether Git still works as the host uid afterwards. Also try all work
  as uid 0. Decides Q1 on the Mac.
- **Box-wide env.** With `x-api-key` stripped on claude's route (#287),
  set pi's `ANTHROPIC_API_KEY` placeholder and claude's token placeholder
  together and see what claude sends. A fixture shows the headers.
  Whether a subscription accepts the result needs Adam's own login.
- **pi's login extension** when pi is installed in the image and not
  mounted.
- **Chrome.** Its sandbox needs nested user namespaces on and
  `no_new_privs` off, and it refuses to run sandboxed as root. wrap
  ships `--no-sandbox`. Stage 3's first spike stays, reworded: which
  variant of Q1 to Q3 lets headless Chrome run, sandboxed or not.
- **Code read:** what the Linux host trusts guest PID 1 and the exec
  service for, and which vsock ports the host listens on.

## Questions for Adam

- **Q1. Who is root?** (a) Work runs as the host uid with passwordless
  `sudo`. (b) All work runs as root. Recommended: (a), if the share
  probe allows it; it keeps file ownership on a share simple.
- **Q2. The root filesystem.** (a) Read-only, as today: `sudo` exists but
  `apt install` fails. (b) Writable, reset at each box start. (c)
  Writable and kept per project, as in wrap; this conflicts with
  decision 7, where only the home disk survives. Recommended: (b).
- **Q3. Nested user namespaces on?** Needed for Chrome's own sandbox and
  for Codex's bubblewrap. Recommended: yes, once privilege goes.
- **Q4. Agent versions.** (a) Latest at build, recorded in the image. (b)
  Pinned in the binary and installed at build. Recommended: (a). It
  deletes the harness pins, their download code, the nightly harness
  bump and guarantee 19. It loses the hash check and the tested
  combination; a vendor change can break a login route at the next
  build.
- **Q5. Agent self-updaters.** Recommended: off. An update is a rebuild.
- **Q6. Linux too?** Recommended: yes for changes 2 to 5. For privilege,
  yes only after the code read above.
- **Q7. Codex's sandbox.** If bubblewrap can run (Q3), nothing is needed.
  If not, `pinfold -- codex` passes `--sandbox danger-full-access`
  unless the user gives a sandbox flag. Recommended: wait for Q3.
- **Q8. #286.** (a) Close it and carry its three kept parts into the new
  work. (b) Merge it and delete the verbs later. Recommended: (a).
- **Q9. Confirm deleting** the three verbs, `attach`, and core's
  `harnesses` key and `box exec --harness`.

## Not established

- Every entry in the right-hand column of the privilege table.
- Whether box-wide env works for claude on a subscription.
- Build time and image size with three agents installed.
- How tightly Codex's pinned host helper is coupled to the Codex version
  in the image, which matters under Q4 (a).
