# Worktree support security and startup

Research as of 2026-10-09, against Pinfold `dc1dfc1` and installed
Pinfold 0.2.2. This is a design proposal and evidence record. It does not
change the binding contract in ARCHITECTURE.md.

## Recommendation

Keep one minimum security contract. Choose how changes reach the host
separately from whether the agent can use Git. Do not make a fresh clone
the default cost of launching an agent. Do not introduce a security-level
ladder that silently permits modification of host Git metadata.

The desired default remains direct editing, with host Git metadata
read-only and commits on the host. Existing ordinary checkouts already
support this without copying the repository. General linked-worktree
editing on the tested default Mac filesystem is still blocked by a real
protection problem, not by Git discovery or cloning performance.

Use scoped direct editing as the immediate option for short tasks in
linked worktrees: expose the checkout read-only and make selected existing
subdirectories writable. The core box interface already expresses this.
It preserves live host edits in those directories and requires no clone.
It does not permit editing root files or creating new root entries.

For arbitrary root-level edits, investigate a narrowly supported
case-sensitive worktree layout before building a new workspace system.
If full support on default APFS is required, the remaining choices are
host-enforced protection of file identities or an isolated working copy
with controlled application of changes. A copy-on-write overlay is a
candidate for the latter, but its stable-source requirement prevents
treating it as an immediate transparent solution.

## Why worktrees were blocked

The September investigation demonstrated host execution through hooks,
`core.fsmonitor`, replacement of partially protected `.git` directories,
and a planted `commondir`. The project selected whole-directory read-only
protection and host-owned commits to avoid a larger Git handoff system.
Linked worktrees were refused because `.git` is a writable pointer file
whose target the host subsequently trusts. See
[the original experiments](2026-09-22-spikes.md#2026-09-22--protecting-git-with-mounts)
and [the design decision](2026-09-23-grilling.md#git-read-only-git-replaces-the-handoff).

The October investigation in this conversation confirmed:

- Installed Pinfold refuses a linked worktree as `protected-path-invalid`.
- In direct runtime experiments, protecting the literal `.git` with
  `--read-only-path` blocked ordinary writes, unlink and replacement.
  Nevertheless, writing `.GIT` on the case-insensitive host filesystem
  changed the same pointer. The pointer had one hardlink. A subsequent
  host `git status` executed a harmless fsmonitor marker from the redirected
  repository. This is a proposed-layout failure; current Pinfold refuses
  that layout.
- A preexisting writable hardlink to the protected pointer also bypassed
  pathname protection. A corresponding preexisting hardlink to an
  ordinary repository's `.git/config` bypassed the existing directory
  mount arrangement. The latter is a separate existing gap, not a reason
  to accept the new case-alias bypass.

The historical statement that Apple cannot mount individual files needs
qualification. File support was introduced in
[containerization PR 487](https://github.com/apple/containerization/pull/487).
However, [Container 1.5.0](https://github.com/apple/container/blob/1.5.0/Package.resolved)
pins containerization 0.47.0. Its
[file mount implementation](https://github.com/apple/containerization/blob/bc994b88df46207fad7775b0eabc51947e315881/Sources/Containerization/FileMount.swift)
exports the source parent and binds the selected filename in the guest.
This differs from the PR's original temporary-hardlink design.
The installed CLI's `--mount` rejected a file source; `--volume` accepted
it but made the parent checkout read-only in the tested arrangement.
Neither an upstream feature announcement nor a literal file bind proves
protection against aliases through another writable export.

Removing the Git executable, omitting external Git directories, setting
`GIT_DIR`, or creating private guest metadata does not stop shell code
from rewriting an exposed host pointer. A no-Git capability is useful,
but it is not a weaker-security workaround for a writable checkout root.

## Relevant tooling

| Tool or mechanism | What fits | What does not transfer directly |
|---|---|---|
| [Docker Sandboxes](https://docs.docker.com/ai/sandboxes/workflows/git/) | Explicit distinction between direct edits, private clones and host worktrees. | Its host-worktree mode omits Git access; that alone does not establish pointer integrity. Clone mode requires fetching or pushing changes back. |
| [agent-sandbox.nix](https://github.com/archie-judd/agent-sandbox.nix/blob/ee9d60f888db958e3831220c5b1e869b08d4d3b0/launcher/lib/git_state.py) | Git topology discovery, including common metadata and worktree pointers. No history copy. | It permits Git writes while protecting selected metadata. Its native sandbox enforcement differs from Pinfold's VM shares. Reviewed tests cover direct pointer attacks, not the confirmed case-alias attack. |
| [Anthropic sandbox-runtime](https://github.com/anthropics/sandbox-runtime/blob/3f0bad7345238f47736435e3f2b064399c1cad74/README.md) | Native filesystem policy, explicit treatment of dangerous Git paths and documented limitations. | Its standalone library does not establish Pinfold's complete linked-worktree policy. It explicitly warns about preexisting hardlinks. This is not a claim about a vulnerability in Claude Code. |
| [Agent Safehouse](https://github.com/eugene1g/agent-safehouse/blob/398d67ff25c3d85daf2f7bc57ed34f59dd1dcacd/bin/lib/support/git-discovery.sh) | Worktree-aware discovery and inexpensive direct access. | It grants common Git metadata writable. Its intended capability differs from Pinfold's host Git protection. |
| [devsandbox](https://github.com/zekker6/devsandbox/blob/main/internal/sandbox/tools/git.go) | Mounts external common Git data at the needed path; distinguishes read-only and writable Git. | Reviewed code and tests did not establish protection of the original gitfile through host aliases. |
| [NVX](https://github.com/microsoft/nvx/blob/dev/doc/run.md#access-policy) | Useful architectural example of enforcing share policy on each host-side filesystem request. | It is a different Linux/Windows runtime, not a Mac dependency to adopt. Its documentation still excludes preexisting host aliases from its pathname policy. |
| [git-lazy-mount](https://github.com/mohsen1/git-lazy-mount/blob/main/docs/worktree-model.md) | Lazy materialization from a fixed Git tree with private changes and a synthetic protected gitfile. | Introduces a virtual filesystem, index interactions, durable journal and different dirty-file semantics. Too much machinery for this Pinfold change without evidence of need. |

These are source and documentation comparisons, not comparative runtime
security certifications. The useful borrowing is topology discovery and
explicit workspace semantics. Replacing Pinfold's runtime with one of
these tools is not justified by this investigation.

## Startup measurements

Local synthetic experiment on arm64 macOS 27.0.1, Apple Container 1.5.0,
Pinfold 0.2.2, Git 2.56.0, cached `pinfold/profile-full:latest` and Pi 1.1.0.
The fixture had 20,001 tracked files, about 78 MiB of checkout data, and a
128 MiB incompressible blob retained only in history. Packed Git data was
about 129.5 MiB. The filesystem resolved `.git` and `.GIT` to the same
file; the gitfile's link count was one.

Each workspace variant started three fresh real Pinfold boxes sequentially.
Each box had 2 CPUs, 1 GiB memory, a private fixture home, the pinned Pi
harness and an empty egress allowlist. Timing waited for Pinfold's `ready`
event. The empty-directory group ran first, so cache warming and machine
variation prevent interpreting the group difference as an improvement.

| Operation | Median | Observed range |
|---|---:|---:|
| Empty-directory box to ready | 2.890 s | 1.612–5.429 s |
| Linked worktree root read-only, `src/` writable, to ready | 2.447 s | 1.469–2.510 s |
| Pi `--version` in scoped box | 0.736 s | 0.675–0.827 s |
| `true` through `box exec`, scoped box, nine samples | 0.130 s | 0.104–0.497 s |
| Scoped-box teardown | 0.503 s | 0.393–0.618 s |
| Local `clone --no-hardlinks`, including checkout | 2.962 s | 2.958–4.969 s |
| Local `clone --shared`, including checkout | 2.865 s | 2.379–3.331 s |
| Local `clone --shared --no-checkout` | 0.064 s | 0.062–0.075 s |
| APFS `cp -cR` of linked checkout | 3.296 s | 3.106–3.334 s |

Clone/copy measurements used three runs each, after the box measurements.
They measured host preparation only, not execution in Docker Sandboxes.
No network cloning, cold image download, model request, million-file
monorepo or Linux runtime was measured. Pi version output is a harness
startup probe, not time to a useful model response. This small sample does
not establish percentiles or a performance SLA.

The result supports separating history transfer, checkout materialization,
VM startup and harness startup. Avoiding object copies did not remove the
checkout cost. APFS data-block sharing did not remove the per-file work in
this directory-copy experiment. Mounting the existing large checkout did
not require an additional checkout step.

The scoped-box functional probe also observed a host source file change,
failed writes to `.git`, `.GIT` and root `README.md`, failed unlink of
`.git`, and unavailable Git status because the pointer target was not
mounted. The host verified unchanged pointer bytes and README contents.
The fixture had no malicious hardlinks or nested protected configuration;
this probe is not a complete security certification of arbitrary scopes.
All six boxes were removed. Only the preexisting BuildKit container
remained running.

Raw timing samples, in seconds:

```text
empty ready:       5.4285 2.8897 1.6119
scoped ready:      1.4687 2.5098 2.4468
scoped Pi version: 0.7361 0.8267 0.6754
scoped exec true:  0.1299 0.1231 0.3269 0.1054 0.1042 0.1834 0.4965 0.1224 0.1632
scoped down:       0.5031 0.3934 0.6177
clone independent: 2.9622 4.9691 2.9581
clone shared:      2.8654 3.3309 2.3791
shared no checkout:0.0753 0.0621 0.0638
APFS copy:         3.3335 3.1058 3.2956
```

## Proposed user experience

For ordinary interactive editing, retain the existing launch and host
commit workflow. For an orchestrated job, create one box and use repeated
`box exec` calls for that job. Both capabilities exist today. Do not keep
a global pool of previously used boxes: processes, writable homes and
temporary state can carry between tasks. Pinfold's existing cached images
and pinned harnesses already avoid rebuilding and downloading per launch.

For linked-worktree tasks limited to existing source directories, a caller
can declare the writable scope and mount the rest read-only. A future
high-level wrapper should display the scope, protect configured paths
inside it, and refuse any widening that exposes protected metadata.
Build output directories must be included deliberately. Do not silently
fall back to a writable root when a command encounters a read-only file.

For unrestricted worktree editing, keep the refusal until a supported
arrangement passes the real boundary tests. Case-sensitive worktree
storage is a candidate requiring one-time placement rather than per-run
cloning. It still needs safe gitfile mounting, metadata protection,
alias checks and lifecycle validation. A filesystem label and a single
`nlink` check are not sufficient evidence. Do not migrate the user's
worktree storage automatically.

An independent ordinary checkout prepared once per task or branch is an
available fallback. It does not need to be cloned again for each agent
invocation. It does change where the user reviews edits and does not
automatically reproduce dirty, staged, untracked or ignored source state.
Those semantics must be explicit rather than silently copying only HEAD.

Git history access should remain independent of write scope. Where safely
available, read-only status, diff and history help agents. Omitting history
can simplify a scoped task; it should not weaken host metadata protection.

## Alternatives that need more than a flag

[Git object borrowing](https://git-scm.com/docs/git-clone) can avoid copying
history with `--shared`, while `--no-checkout` avoids materializing files.
It does not supply an editable checkout, protect its original gitfile or
reproduce dirty state. Borrowers depend on source object retention; source
maintenance can remove objects they still need. Ordinary local clones use
hardlinks by default, so they are not automatically independent storage.
Do not build a new object cache before the workload demonstrates a need.

[OverlayFS](https://docs.kernel.org/filesystems/overlayfs.html)
can defer copying file contents until a write. However, its documented
contract forbids changing the underlying filesystems while the overlay
is mounted. A host editor modifying the source tree violates that
assumption. A stable snapshot, a dedicated immutable source, or a different
filesystem design is required. Snapshot preparation and guest mount
privileges must be included in the startup and security evaluation.
Pinfold's current capability-dropping policy must not be weakened merely
to make an overlay mount work.

An isolated editing workflow would also need a small trusted importer.
It should accept explicit file changes, check whether host originals have
changed, reject protected paths and aliases, reject traversal and symlink
redirection, and apply regular-file replacements without truncating an
existing hardlinked inode. Deletions need explicit treatment. Do not unpack
an agent-controlled archive directly over the host checkout or run host
Git against agent-controlled metadata. A bounded set of staged files may
serve tiny root-file edits before a general virtual filesystem is justified.

Host immutable flags such as `uchg` are not a suitable default workaround.
They mutate user-owned repository state, interfere with host operations,
require crash and overlapping-run ownership rules, and need proof that
the guest cannot clear them through the sharing stack. A host Git RPC
bridge would likewise revive the rejected execution and argument-filtering
boundary. Neither is a small usability change.

## Next decision gates

1. Treat scoped editing as a caller workflow with known limits. Before a
   first-class launch option, test nested protected paths, case aliases,
   symlinks, preexisting hardlinks and overlapping writable exports. Make
   the existing hardlink gap an explicit security decision or fix; do not
   silently declare it outside the threat model.
2. Run one bounded experiment for full direct editing on case-sensitive
   storage. Require normal edits and read-only Git operations to succeed,
   every pointer/metadata write route to fail, and host marker observers
   to show no execution. Verify actual Apple and Linux behavior separately.
3. Only if default-volume full editing is required, compare a host-enforced
   export with isolated staging or snapshots. Count implementation and
   recovery costs as well as startup time. Do not build both speculatively.
4. Keep repository-sized work off the normal launch path. Measure readiness,
   harness startup, first useful operation and teardown separately, with
   cold and warm cases. Reuse an owned box within one job before adding
   a pool or persistent daemon.

No production code, guarantees or runtime settings were changed by this
investigation. Astra reviewed the security tradeoffs independently;
separate reviews covered other tools and Pinfold's existing capabilities.
