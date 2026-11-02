# Scratch Pi, configuration fallback and disk export

Planning snapshot, 2026-11-02. No implementation or contract change has
landed. ARCHITECTURE.md remains binding. Astra independently reviewed the
integration and the global configuration extension. The choices below are
recommendations for implementation, not claims about current behavior.

## Configuration and identity

Use the repository root for configuration inside Git, and canonical cwd
outside Git. Read its `.pinfold.toml` when present. Otherwise read
`$XDG_CONFIG_HOME/pinfold/pinfold.toml`, or
`~/.config/pinfold/pinfold.toml` under the existing XDG rules. Create neither.

Select one file, not two merged layers. An empty local file suppresses the
global file. Invalid or unreadable local files, including dangling symlinks,
fail instead of falling back. Precedence remains environment, selected
file, selected profile, built-in defaults. Lists replace lower lists.
Keep selection in the existing Config loader and report the selected source.

Global settings are host-owned policy, like user profiles, without per-folder
approval. Local config and its declared recipe retain exact-byte approval.
Removal of an approved local file retains the existing approval requirement;
fallback does not conceal that change. Keep local and global bytes distinct
in trust accounting. Global config accepts profile, image, allow, routes,
cpus and memory. Reject containerfile there; a profile supplies a reusable
recipe without inventing global recipe identity or cwd-relative behavior.

Separate configuration approval from protected-path validation of a writable
host project. Scratch cwd supplies config but is never the guest project.
This must permit starting from home without exempting a pathname or skipping
config trust. Preserve repository protected-path and core write controls.
Use one context classification in pi, allow, build, config and doctor.
Distinguish confirmed no-repository from broken Git discovery; retain
worktree and Git-metadata-directory refusals.

Configuration, trust and recipe image identity stay tied to canonical folder.
Scratch storage uses a fresh invocation identity, never cwd or PID alone.
Do not pass that fresh identity to project image resolution.

## Retained disks

Each repo-free invocation gets independent home and work disks, mounted at
`/home/pi` and `/work`. Skip Git sync and import no host folder. Retain both
disks and print their names. Reuse disk list, rm, attachment, ownership and
dirty-state handling. No scratch registry, project-state record, project
cleanup label, resume command or new top-level verb. Existing repository
storage and crossing remain unchanged. Config reports scratch allocation
as occurring on launch, not a fictitious deterministic disk pair.

## Generic export

Proposed interface: `pinfold disk export NAME DEST`, for a stopped disk.
Use an ordinary helper box with no egress, profile, harness, seeds, forwarded
environment or host destination mount. Do not load local/global Pi policy
for it. Use Pinfold's supplied static init helper, not guest tar or scripts.

Require an existing, filled, clean, unattached disk inside the existing claim
lock. A precheck followed by ordinary startup can create a missing disk and
is insufficient. Do not silently repair an unclean disk writable.

The host validates an untrusted stream before publishing output. Bound paths,
targets, entry counts and bytes; reject truncation, special files, duplicate
and colliding paths, and traversal. Preserve symlinks as links, never follow
them. Keep ordinary permissions plus executable bits, not ownership or
special metadata. The existing seed receiver is not safe for host extraction.
Share framing constants only where useful; add no generic archive framework.

DEST must not exist. Recommended contract: export into a private sibling
staging directory and publish without replacing any existing destination;
failure removes task-owned partial output. Verify publication on both host
platforms rather than assuming ordinary rename is sufficient.

Before implementation, specify the helper's local image selection and any
override, exact transfer limits/deadline, and publication semantics. Never
infer an image from cwd or disk metadata or automatically build/pull one.
Explicitly reconcile user-requested export with the spec's no-secret-file
rule and fetch-only wording. No credential forwarding exception is implied.

## Exit summaries

On normal harness completion, collect a short bounded, cancellable advisory
observation before teardown. Background processes mean it is not a frozen
snapshot. Signal, audit or box termination abandons collection immediately.
Failure never replaces the harness result or hides teardown failure.

Scratch reports escaped paths and sizes from work only, plus disk names and
export guidance. Repository summaries distinguish fetchable work on all
branches from tracked/untracked work fetch cannot carry. Describe outstanding
work, including retained work, against explicit host branch baselines.
Do not fetch objects, show file contents or commit messages, or run guest
configuration on the host. Parse all guest observations as untrusted.
Bound bytes, entries and duration, mark truncation or unavailable inspection,
and stay quiet when no outstanding work is observed.

## Delivery and validation

Implement export first, scratch/context/global fallback second, summaries
third. Update ARCHITECTURE and README alongside the corresponding code.
Keep CLI serialization in cli, configuration selection in config, trust in
trust, lifecycle composition in pi, and export/admission in core. Reuse Box,
runtime exec and existing disk claims rather than parallel lifecycle code.

Extend guarantees 12, 13 and 18 for approval, scratch independence and config
selection; reconcile reports and launch records under 27/39. Keep guarantee
11's existing disk ownership/recovery scenarios. Add explicit export and
summary guarantees for their new promises; reconcile guarantee 10's scope.
Use real CLI/guest scenarios, external expected values, positive controls,
and named sabotage. No unit tests, mocks or test-only product paths.

Review each stage independently for correctness and test value. Run fmt,
locked all-target Clippy and full Linux/Mac suites before integration. Keep
the existing suite budget. Release notes must name config/report changes,
repo-free behavior and the export interface under Caller changes. No merge
or release is authorized by this planning snapshot.
