# Alternatives research, 2026-10-15

Read-only primary-source investigation for Pinfold. No runtimes, boxes, builds, mounts or user repositories were changed. Read current `<checkout>/AGENTS.md` and ARCHITECTURE.md threat model, runtimes, Always applied, protect and Git sections. The current contract requires VM-per-box on macOS, rootless Podman on Linux, no guest writes to host Git metadata, protected host-executed configuration, and host-owned commits. This report is source evidence, not an empirical security claim.

## Decision relevance

None of these six tools supplies a demonstrated drop-in solution satisfying the complete case-insensitive-host-worktree requirement. Native Seatbelt tools change the macOS isolation boundary. Docker and devsandbox worktree support do not establish protection of the writable host gitfile. NVX provides a materially different place for enforcement: the host filesystem server. It has no macOS backend and its policies require careful selection; it is architectural evidence, not an adoption recommendation. Docker clone mode illustrates a useful compromise, but its documented commit/fetch workflow does not preserve an arbitrary dirty working tree and staged partition by itself.

## Exact baselines

Shallow public source checkouts fetched 2026-10-15:

| Repository | Commit |
|---|---|
| anthropics/sandbox-runtime | `9a277e5e04af1ab1b04feea4400a6ed8a5b72039` |
| archie-judd/agent-sandbox.nix | `a631434d961856e1ebe45f0904b0acbc17080845` (5.6.1 release commit) |
| eugene1g/agent-safehouse | `3806b34ad38e65d30379012900d6094fcfbc9190` |
| zekker6/devsandbox | `14a9a2061979a9e0ba327fce88e91fbe6e9d1e63` |
| microsoft/nvx, dev branch | `2735f2aceb6877eab7c00a924281b4097cf0b91e` |
| nanvix/openvmm, NVX submodule pin | `7f225e2cd635bbd1e1839e587e659996433396c8` |

## Docker Sandboxes

The current [architecture](https://docs.docker.com/ai/sandboxes/architecture/) describes direct workspace passthrough through virtiofs, without synchronization; host/guest edits appear directly. Private files and VM state persist across restarts. Therefore no conflict reconciliation protects simultaneous edits. The actual writer is the host filesystem passthrough implementation, not the agent or guest mount flags. The documentation does not expose enough implementation detail to establish inode-identity protection or discovery-to-use behavior.

The [Git workflow](https://docs.docker.com/ai/sandboxes/workflows/git/) says linked-worktree mode exposes the worktree while omitting parent metadata. This prevents ordinary Git use because the pointer cannot resolve; it does not claim the host `.git` pointer is immutable. Direct mode is thus not evidence for Pinfold's guarantee. Clone mode exposes source read-only and uses a persistent private Git clone. Results move through fetch/push; its CLI also adds/updates/removes a host remote. Agent-owned commits and import-by-branch differ from the requested workflow. Documentation does not establish preservation/import of staged, unstaged, ignored and untracked state. Docker's runtime and networking model would change Pinfold's binding runtime contract.

Decisive prerequisite before trying Docker direct mode: obtain a host-enforced filesystem policy for the pointer and protected configuration, including aliases. Do not retest simply hiding the parent Git directory; it is the already-disproved mechanism.

## Anthropic sandbox-runtime

[Source](https://github.com/anthropics/sandbox-runtime/tree/9a277e5e04af1ab1b04feea4400a6ed8a5b72039). Native child processes write host files through the host kernel; macOS uses `sandbox-exec`, Linux uses bubblewrap, socat and ripgrep. No VM/exporter or synchronization layer exists. Thus native-process policy success cannot establish protection for Virtualization.framework sharing.

[`sandbox-utils.ts`](https://github.com/anthropics/sandbox-runtime/blob/9a277e5e04af1ab1b04feea4400a6ed8a5b72039/src/sandbox/sandbox-utils.ts) explicitly excludes `.git` from mandatory denied directories to allow Git operations. [`macos-sandbox-utils.ts`](https://github.com/anthropics/sandbox-runtime/blob/9a277e5e04af1ab1b04feea4400a6ed8a5b72039/src/sandbox/macos-sandbox-utils.ts) adds mandatory denies for hooks/config and caller-specified deny paths. Recent code pins ancestors against rename and handles literal/glob distinctions; these are worth reading if native confinement is deliberately chosen, but do not change who writes a VM share. The default policy is not host-owned Git metadata. A custom deny policy plus native alias/race tests would be a separate candidate requiring an explicit macOS kernel-boundary change. Source inspection here does not establish hardlink/case-alias safety.

## agent-sandbox.nix

[5.6.1 source](https://github.com/archie-judd/agent-sandbox.nix/tree/a631434d961856e1ebe45f0904b0acbc17080845). Nix prepares software; native Seatbelt/bubblewrap confines the actual host-writing child. No copy/sync or metadata substitution. [Git discovery](https://github.com/archie-judd/agent-sandbox.nix/blob/a631434d961856e1ebe45f0904b0acbc17080845/launcher/lib/git_state.py) reads common metadata and protects config, worktree pointers, alternates and existing hooks paths; everything else in Git remains writable. [Seatbelt policy](https://github.com/archie-judd/agent-sandbox.nix/blob/a631434d961856e1ebe45f0904b0acbc17080845/launcher/lib/launch_config/darwin/seatbelt.py) uses literal/subpath denies. The README explicitly permits commits, fetches and branch switches. This violates the requested metadata ownership by design, although it has broader worktree-layout discovery than literal `.git` mounting. Nix is an additional deployment dependency; optional in-sandbox Nix requires a configured multi-user daemon and introduces separate authority checks. No reason to install it for this requirement.

## Agent Safehouse

[Source](https://github.com/eugene1g/agent-safehouse/tree/3806b34ad38e65d30379012900d6094fcfbc9190). macOS native `sandbox-exec`, delivered as a shell script or Homebrew tool. Direct host writes; no sync or VM. [`policy_render_build_git_worktree_common_dir_rule_block`](https://github.com/eugene1g/agent-safehouse/blob/3806b34ad38e65d30379012900d6094fcfbc9190/bin/lib/policy/render.sh#L536) deliberately emits `allow file-read* file-write*` over the common Git directory. Other linked worktrees are readable from a startup discovery snapshot. Thus default worktree support grants precisely the metadata authority Pinfold denies. Useful as native confinement reference only. Native policy adaptation would still change the VM boundary and require independent alias, hardlink, rename and host-change tests.

## devsandbox

[Source](https://github.com/zekker6/devsandbox/tree/14a9a2061979a9e0ba327fce88e91fbe6e9d1e63). Linux bubblewrap, macOS Docker, optional libkrun via Podman/crun. Docker/krun writes arrive through the runtime's host-volume implementation; bubblewrap writes through host syscalls/mounts. Tool configuration can be overlaid/copied, but the project remains directly writable rather than transactionally imported.

[`readOnlyBindings`](https://github.com/zekker6/devsandbox/blob/14a9a2061979a9e0ba327fce88e91fbe6e9d1e63/internal/sandbox/tools/git.go#L260) mounts the main repository `.git` read-only for linked worktrees and overlays a sanitized config. The function does not mount the worktree's pointer file read-only. Its own comment explains the writable project's pointer must resolve into that separately mounted metadata. Hence the reviewed binding construction has no new mechanism protecting that pointer or aliases. This is source evidence of a missing mechanism, not a reproduced live exploit. Do not adopt based on its “readonly Git” label. Default runtime changes and broader tool/config integration add burden without solving the decisive boundary.

## NVX / OpenVMM

[Host-filesystem design](https://github.com/microsoft/nvx/blob/2735f2aceb6877eab7c00a924281b4097cf0b91e/doc/design/3.6-host-filesystem.md) locates enforcement inside the VMM's FUSE server. Guest requests are checked before host filesystem operations. Directory exports may deny paths, permit exceptions and restrict writable subtrees. Each mutation checks namespace entries and known object aliases; protected root identities block some alias paths. Host changes remain live, without a sync/conflict layer; ordinary concurrent overwrites are ordinary filesystem overwrites. Host/guest locks are not forwarded. Zero cache lifetimes/direct I/O are intended to expose host edits promptly.

[Actual NVX export planner](https://github.com/microsoft/nvx/blob/2735f2aceb6877eab7c00a924281b4097cf0b91e/nvx-rs/src/host/filesystem.rs) explicitly warns that a **read-only mapping nested inside a read-write mapping is protected only by a guest bind mount**. Use a genuinely host-denied subtree or host write policy, not that mapping arrangement. The planner rejects duplicate/overlapping roots and records identities for movable denied objects. This is materially different from launch-only alias admission, but not universal inode protection.

At the pinned OpenVMM source:

- [policy.rs](https://github.com/nanvix/openvmm/blob/7f225e2cd635bbd1e1839e587e659996433396c8/vm/devices/virtio/virtiofs/src/microvm/policy.rs): pins declared denied roots and traversed allowed ancestors, not an obvious recursive inventory of every child inode.
- [inode.rs](https://github.com/nanvix/openvmm/blob/7f225e2cd635bbd1e1839e587e659996433396c8/vm/devices/virtio/virtiofs/src/microvm/inode.rs): write policy checks aliases **known for the object**. An unobserved pre-existing hardlink to a protected descendant deserves a decisive test; do not infer safety from alias bookkeeping alone.
- [confine.rs](https://github.com/nanvix/openvmm/blob/7f225e2cd635bbd1e1839e587e659996433396c8/vm/devices/support/fs/lxutil/src/unix/confine.rs): confines Linux walks with `openat2`, or `openat(O_NOFOLLOW)` component traversal, holding parent directory handles. This stops symlink redirection; it does not by itself establish that a concurrent trusted host rename preserves a path's policy classification.

Current host support is Linux KVM/MSHV and Windows WHP. KVM requires `/dev/kvm` access. Python 3.10+ orchestrates downloadable binaries; custom builds include a pinned kernel/OpenVMM. There is no documented macOS backend. Taking only this exporter into Apple Container would be a substantial new integration and macOS filesystem port, not a small Pinfold change. The server also has concrete limits: 4096 nodes/handles and directory listing cap 4096 entries, so representative repositories matter.

## Small decisive experiments if pursuing host service enforcement

1. Deny `.git` and an editor config subtree, allow the complete project. First test root edits and Git reads using separately read-only metadata. Verify host bytes and identities.
2. Pre-create an ordinary-name hardlink to a protected descendant **before any guest lookup of the protected name**. Try the alias first. Repeat with host-created alias after readiness. This targets “known aliases” assumptions.
3. Hold an open writable file, then have the trusted host move/replace it across a protected boundary; exercise write/truncate/chmod through the handle. Define whether host moves are prohibited during sessions or whether identity enforcement follows them.
4. Exercise protected ancestor rename/exchange, unlink/recreate, case aliases, symlink retargeting and overlapping exports. NVX Linux success would still leave APFS/macOS untested.
5. Test two simultaneous boxes and host editor atomic-save rename behavior. Direct sharing needs a declared ordinary last-writer behavior, not claims of merge/conflict safety.

For isolated persistent working files, the decisive contract is instead a base snapshot plus host-side application with conflict refusal. Preserve the host index independently, represent deletions and file types, reject protected path aliases, and apply through no-follow anchored paths with verified base identity/content. A generic sync tool's copy success is insufficient evidence; investigate both conflict detection and crash recovery at the host writer.

## Resources and cleanup

Source checkouts were created only under `/private/tmp/pinfold-research-alt-*` (about 32 MiB total). Raw OpenVMM source/tree downloads use the same prefix. No code from these checkouts was executed. All source checkouts/downloads were removed after recording this report; only this compact report remains for the parent to incorporate. No services, containers, images, mounts or processes were created.
