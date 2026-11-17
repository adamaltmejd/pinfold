# Step 5: corrected share plan

Recorded 2026-11-17 against `a9e1ec68a76a2e2f854dd8cb82ad296f57cd0701`.
This records the operator's decisions for #279 and #280.
ARCHITECTURE.md remains binding until the implementation changes it.
The November 11 and November 12 plans and their probe records remain unchanged.

Configuration, image-installed agents and detached project ownership have landed.
Step 5 adds Apple shares to that owner and session lifecycle.
It does not replace the lifecycle or change disk synchronization.
Disk crossing remains until step 6.

## Corrections to the earlier account

#279 builds on configuration, owner processes, private runtime sockets,
durable retirement and unprivileged execution helpers already present at HEAD.
Its protection covers the admitted main repository's top-level `.git` directory.
Linked-worktree pointer files and nested repositories remain writable.
The box can create and commit to another repository.
"Only the host commits" refers to the protected main repository's metadata.

#280 makes a share the default when creating Mac project configuration.
An explicit disk selection stays supported.
The existing detached owner receives the share in its plan and comparison snapshot.
Linked worktrees beneath the main root join that project's owner.
A share has one home disk, no project disk, no Git synchronization and no crossing.
HEAD instead accepts disk configuration, allocates two disks,
rejects linked-worktree markers and synchronizes each new Git-project owner.

#298's original unchecked list no longer states the whole gate.
Its November 15 comment supersedes that list.
Probe 7 established bounded agent startup and delivery of failed Git writes
to fixture models through native Apple mounts.
It did not validate an implemented pinfold share.
Production instruction delivery and comparable status measurements were still open.
Subscription login and Linux Chrome work remain separate.
Linux shares need refusal verification, not successful share execution.

Read-only Git prevents the competing metadata writes measured previously.
It does not prevent working-file loss, corruption or host execution of guest edits.
Earlier guest reads recovered after roughly a second in the observed cases.
No maximum stale interval was established.
The earlier 100,001-file status samples were about 13–21 seconds in the guest
and 0.18–0.33 seconds on the host.
They were first and repeated samples without cache flushing.
They were not cold and warm benchmarks.
The proposed first-start-only disk push is not HEAD's behavior.
Leave existing disk synchronization unchanged in this step.

## Resolved operator decisions

Do not refuse every multiply linked file in a share.
Walk without following symlinks, as the host uid.
Refuse every symlink beneath the protected `.git`.
Collect device and inode identities of protected files with more than one link.
Refuse a file outside `.git` in the share only when its identity matches that set.
A working-tree hardlink to a file outside the share is allowed.
It shares the user's selected content and is not protected Git metadata.
Admission does not cover aliases the host creates after admission.
Writable attachments must not expose protected sources through another path
or obscure the generated read-only Git attachment.
The new native probes show why both alias and attachment checks are required.

All three bundled agents receive share and disk guidance through native home files.
Seeds are missing-only and preserve existing user instructions.
Pi's existing extension also needs its incorrect project-path advice corrected.
The observed native paths are `.pi/agent/APPEND_SYSTEM.md`,
`.claude/CLAUDE.md` and `.codex/AGENTS.md`.
Pi also retained its existing `.pi/agent/AGENTS.md` instruction.
A pre-existing native instruction file prevents a missing-only seed from landing.
Existing seed paths stay untouched under the approved missing-only policy.
The observed guidance gap remains part of the implementation review.
Instructions provide guidance and never enforce containment.
The native Git probe refused ordinary reads for dubious ownership.
A fixture-specific `safe.directory` entry allowed them; no wildcard was needed.
Do not assume matching process uid alone establishes ordinary Git reads.

Measure the Git status cost, record it and accept it for this step.
Do not add a cache, wrapper, index copy or synchronization delay.
Keep the profile directory named `share/`.
It continues to hold read-only profile files.

## Delivery split

First, commit this corrected plan and the small probe evidence only.
The companion [probe record](2026-11-17-share-probes.md) distinguishes native
observations, public pinfold behavior, code readings and unestablished cases.
The companion record retains Linux first-stage ordering through the real CLI.

Second, implement #279: mount kinds and admission, Apple enforcement,
trusted socket setup, early Linux refusal, launch records and matching spec rows.
Use a fixed operation in supplied init for socket setup, not the image's `chmod`.
Every helper inspecting share data runs as the host uid.

Third, implement #280: discovery, configuration defaults and reporting,
owner integration, share crossing refusal, agent guidance and matching spec rows.
Preserve generation ownership, retirement, stop behavior and explicit disk projects.
Host edits and commits do not change the startup configuration snapshot.
Share launch never silently deletes old project volumes.
Any selected disposable-volume cleanup is a separate explicit operation.

Each implementation builder exercises the real CLI and records its observations.
Native Apple probes do not validate pinfold's future enforcement boundary.
Add no tests while the suite is suspended and cut no release.
