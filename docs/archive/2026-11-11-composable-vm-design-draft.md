# Composable Pinfold VM design

Draft for Adam and Claude to revise. Recorded 2026-11-11 against main
1de856099daf98cacb8b0bac916f6952aaca8d35, tagged v0.5.5.

This document proposes a replacement design. ARCHITECTURE.md remains the
implemented contract until an approved implementation updates it. No runtime
behavior has been changed by this draft. Sections marked open require a decision
or feasibility evidence.

## Purpose

Pinfold provides a convenient VM environment for Pi, Claude Code and Codex,
usable directly or from Herdr. Prefer existing runtime and image capabilities.
Keep Pinfold's responsibilities small and explicit. Moving custom machinery into
another repository alone does not reduce maintenance.

The default is one VM per project folder with writable filesystem sharing.
Agents can edit files and commit directly. Adam accepts the associated Git
metadata risk. An isolated workspace is an independent option.

## Composition

| Component | Owns |
| --- | --- |
| Platform backend | VM creation, startup, stop, storage attachment, filesystem sharing, guest communication and resource enforcement |
| Guest base | Linux userspace and the minimum guest integration needed to run on the backend |
| Small environment | Guest base plus CLI development tools |
| Browser environment | Small plus Chrome or a compatible Chromium distribution, fonts and agent-browser |
| Desktop environment | Browser plus graphical session services, agent-desktop and optional human viewing |
| Agent integration | Launch and configuration conventions for Pi, Claude Code and Codex |
| Pinfold workflow | Configuration resolution, project identity, VM and disk ownership, access policy and user-facing lifecycle |
| Herdr integration | Launching and presenting agent sessions through Pinfold's ordinary interface |

Image composition follows guest base -> small -> browser -> desktop. A child
image reuses the parent's software. VM configuration is separate from image
inheritance. Agent choice is independent of environment tier and workspace mode.
Avoid a separate image for each agent/environment combination.

The three environments are the intended composition. Whether desktop is needed
in the first implementation remains open. Browser automation and screenshots can
run headlessly. Desktop automation without a physical monitor still needs a
graphical session. A remote desktop viewer is a separate feature.

Apple container is the Mac backend candidate. Cloud Hypervisor is the leading
Linux candidate, subject to feasibility checks. Backend identity must not leak
into agent configuration or application image recipes. Backend-specific boot
artifacts may still be necessary. Sharing a recipe does not imply one disk image
or binary works across architectures and runtimes.

Repository boundaries remain open. Prefer establishing working module boundaries
before extracting independently released platform repositories.

## Project and VM identity

Within a Git repository, select the Git root. Warn when invoked from a subfolder.
Outside a repository, select the current directory. Git is optional.

Persistent launches for the same selected project reuse its VM. Multiple agents
share that VM and its workspace. They can interfere with each other's files,
Git index and processes. Separate worktrees or folders provide separate default
VM identities.

Open details include canonical-path handling, moved projects, explicit project
overrides and linked worktrees whose Git metadata lies outside the folder.
A linked worktree must remain usable without silently granting access to an
unrelated parent directory.

## Build, start, stop and rebuild

An image build produces software. A VM build fixes the configuration used by a
VM, and can reuse a suitable existing image.

The VM configuration includes the selected profile, environment, resources,
workspace mode, mounts, egress policy, exposed guest ports and configuration
overrides. A configuration change requires a new VM build. Starting an existing
VM reuses its recorded configuration. It must not silently restart active work
to apply different settings.

Persistent VMs keep running until explicitly stopped. Exiting an agent does not
stop other agents or background development servers. Automatic idle stopping is
outside the initial design.

Rebuilding the same VM replaces its root environment and home. It automatically
reattaches its existing work disk or shares the same host folder when retaining
the workspace mode. A mode change requires an explicit source/destination choice
and preserves the old work until deliberately removed. It does not
recopy the host folder over existing isolated work. A separately created VM can
explicitly attach an existing disk.

Open details include active-session refusal during rebuild, transactional failure
handling and recovery if a replacement environment cannot start. A failed rebuild
must not destroy the retained work disk. The exact CLI verbs remain undecided.

## Storage

| Storage | Stop/start | Rebuild |
| --- | --- | --- |
| Root environment | Uses the built environment | Replaced from the selected image |
| Shared workspace | Host files persist | Same host folder shared again |
| Isolated workspace | Persistent ext4 disk | Existing disk reattached automatically |
| Agent home | Settings, sessions and native login persist | Recreated from profile defaults |

Workspace choices are writable sharing, an initial folder copy to an ext4 disk,
an explicitly attached existing disk, a new empty disk, or no workspace disk.

Shared is the default. The complete selected folder, including Git metadata,
is intended to be writable. Isolated mode defaults to copying that folder into
its work disk. Non-repository folders work with either mode. No remote is
required. Git remotes are optional ordinary tools for transferring results.
Explicit file retrieval must also work for non-repository isolated work.

Existing disks can move between VMs. Ordinary ext4 attachment permits one writer
with no other attachments, or multiple read-only attachments with no writer.
Readers that must coexist with a writer need a separate copy or snapshot.
Enforcement must cover all Pinfold-managed attachments, including concurrent
starts and recovery after crashes.

Open: whether guest root-filesystem writes persist across stop/start. This matters
for tools installed interactively. Rebuild replaces the root in either case.

Open details include copy exclusions, symlinks, file metadata, sparse files,
concurrent host changes during copying, disk sizing, export behavior and
read-only mount cleanliness requirements. A live copy is not automatically a
consistent filesystem snapshot.

## Ephemeral commands

Ephemeral lifetime is independent of workspace mode and environment tier. It
uses writable host sharing by default, exactly like a persistent launch.
Isolation must be selected explicitly.

An ephemeral invocation runs its command, streams output and returns its exit
status. On completion it removes the VM and temporary storage it created.
Explicitly attached existing disks survive. Host writes through a shared folder
also survive. Headless agent commands such as claude -p are an intended use case.
API authentication supplied for the invocation is a straightforward initial path.

Open: an ephemeral invocation in a folder that already has a persistent VM needs
an explicit ownership rule. It must never remove the persistent VM or its disks.
A separate temporary VM is a candidate, and would share the host folder when
using the default workspace mode. Cleanup on cancellation and crash recovery
also needs specification. Cleanup failure must be visible.

## Profiles, configuration and authentication

Pinfold has its own configuration directory. Profiles supply Adam's chosen agent
defaults and select an environment and policy. Do not automatically import host
Pi, Claude or Codex settings.

Environment variables override profile configuration when building the VM.
The full precedence order and treatment of project configuration remain open.
Record resolved nonsecret configuration so reuse and rebuild behavior is clear.

Native login inside each VM is an acceptable baseline. Its credential store is
in the persistent home, so login survives stop/start and resets on rebuild.
This means processes in that VM can access guest-held credentials. Profile reuse
does not by itself promise a shared OAuth session across VMs.

Profiles may reference host-held credentials. Optional host-mediated routes can
keep reusable tokens outside guests where the agent supports that interface.
A shared OAuth integration requires a single refresh owner and verified support
from the provider and agent. Avoid inventing cross-VM auth-file synchronization.

Open: login compatibility for each CLI, device/manual login versus localhost
callbacks, ephemeral API credentials, and how host-held secret references are
resolved at restart. The build record must not serialize secret values. Agent
configuration defaults and guest-created credential persistence are distinct.

## Networking and ports

Egress remains restricted by an allowlist, including browser traffic and traffic
from any nested containers. Profile contents do not enforce security. The host
runtime/policy arrangement must prevent a guest, including guest root when
allowed, from bypassing the allowlist.

Build-time dependency installation reduces runtime downloads but cannot cover
arbitrary future dependency changes. Model access, package registries, Git
remotes and Internet browsing require the relevant explicit policy. Local UI
development should work with the browser and application server in the same VM.

Expose guest ports only when declared. Bind host listeners to loopback by default.
Use an automatically allocated host port unless a fixed port is requested.
Report the actual URL. Separate VMs can each use guest port 3000 with distinct
host ports. A requested fixed host port that is occupied fails clearly.

The guest-port declaration is fixed at VM build time. Automatic host-port
allocation is runtime state, stable while running. Stability across stop/start
is not yet promised. Browser-control and desktop-viewing endpoints need explicit
exposure just like application ports. CLI environments may expose development
servers too.

Open: transport choice, enforcement for all browser protocols, local DNS behavior,
port-forward lifecycle, access from the guest to host services, and build-time
network policy. Do not infer unrestricted build networking from this draft.

## Security contracts

The VM is the principal isolation boundary. Normal Linux guest functionality is
intended to remain available. Application protections such as Chrome's own
sandbox should operate normally. Guest root, namespace capabilities and nested
container support require a concrete supported policy, rather than inheriting
Pinfold's current restrictions without review.

Containers inside the VM and nested virtualization are different requirements.
Nested containers are relevant to optional subagent separation. Support for
running another VM inside the guest remains open and platform-dependent.

The security description must cover the resolved environment, workspace mode,
authentication method and access policy together:

- Shared mode authorizes modification of the shared host folder, including Git
  metadata and hooks. Host programs may later act on agent-modified content.
- Isolated mode confines workspace edits to its disk until an explicit transfer.
  The initial copy exposes all included content to the VM.
- Neither mode prevents disclosure through authorized model or other services.
- Guest-held credentials are accessible within that VM.
- Multiple agents in one VM share its authority and are not isolated from one
  another by default.
- Browser and desktop tools operate inside the guest. Host desktop control is
  not implied by selecting those environments.

The remaining host filesystem, Pinfold configuration and host-held credentials
must stay outside writable guest access unless deliberately granted. Accepting
Git risk does not authorize unrelated host access or an egress bypass.

## Feasibility and implementation sequence

1. Review this draft and resolve lifecycle, copy and configuration ambiguities.
2. Check backend capabilities against the same requirements on Mac and Linux.
3. Trial small and browser environments with the actual agent CLIs, including
   native login, persistence, headless rendering and explicit development ports.
4. Define the minimal backend interface from observed requirements. Evaluate
   guest communication, image conversion, filesystem sharing and cleanup costs.
5. Write the replacement architecture and map each existing guarantee to retained,
   deliberately changed or removed behavior. Review that change explicitly.
6. Implement in reviewable pieces and validate through real user entry points.

Cloud Hypervisor feasibility must include image boot, guest execution and PTYs,
virtiofs integration, ext4 attachment, host enforcement, x64/arm64 support and
resource accounting. Apple container must meet the same user-visible requirements.
Chrome/Chromium and automation tooling availability must be verified on both guest
architectures. A VMM alone does not supply a complete container/image runtime.

Any trial must report limitations, and must distinguish successful guest software
operation from enforced host protection. Existing CI/performance issues remain
separate from adopting the new contract. Audit the final suite once the intended
behavior is settled.

## Sources consulted during discussion

- Current repository ARCHITECTURE.md at the commit named above.
- https://github.com/apple/containerization
- https://github.com/cloud-hypervisor/cloud-hypervisor
- https://github.com/tobi/wrap
- https://github.com/tobi/wrapvm
- https://developer.chrome.com/docs/automation-and-testing/headless
- https://github.com/vercel-labs/agent-browser
- https://herdr.dev/core/

Upstream capabilities require rechecking during feasibility work. This draft
records the agreed design and proposed boundaries, not a completed backend audit.
