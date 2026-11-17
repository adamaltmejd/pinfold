# Share mount: image-path admission finding

Recorded 2026-11-17 against `a9e1ec68a76a2e2f854dd8cb82ad296f57cd0701`.
Implementation stopped at the brief's instruction to stop when the plan
cannot provide the specified control without a design decision.
No implementation or living-spec change is retained.

## What needs a decision

PLAN item 31 requires destination ancestors not to redirect attachments
through symlinks. The brief requires overlapping attachments to refuse at
validation regardless of their order. Host source checks and lexical guest
normalization cannot establish this for symlinks in the selected image.

The existing default image has `/var/run -> /run`. Two lexically distinct
guest destinations can therefore name the same effective attachment point.
A caller attachment at `/var/run/probe-share279/.git` hides read-only Git
protection at `/run/probe-share279/.git`. Different host sources need not
overlap. A source-inode check does not detect this destination alias.

This does not change the protected host Git bytes in this reproduction.
It replaces what the box sees at the protected Git path with writable
metadata. That violates the specified refusal of obscuring attachments.

Inspection: `Plan::validate_guests` compares lexically normalized paths.
Apple `image_info` reads image identity and labels, not its filesystem tree.
Apple `up` passes bind destinations to the native runtime. Neither supplies
image-ancestor metadata to admission. A host no-follow share walk covers
symlinks in mounted host trees; it does not cover this image symlink.

Proposed revision, not implemented: specify an image-path inspection phase.
A trusted supplied-init helper would run as the host uid, without shares,
disks, egress or image executables, against the frozen selected image.
It would inspect every destination component without following symlinks,
including init, login, profile, disk, share and generated Git paths.
A symlink component would refuse as `spec`; no path would be rewritten.
Its temporary runtime would need ordinary ownership, cancellation and
checked teardown. Specify where this sits relative to admission's current
no-resource-creation contract before building it. Linux share refusal
would still precede all resource creation. A host-side image-tree reader
is an alternative, but its layer/whiteout semantics and cost are unmeasured.

No helper, image parser or weaker path-only rule was added without that
decision. An incomplete candidate was saved outside the repository at
`/tmp/pf279-candidate.patch`, then its tracked edits were restored.

## Native observations

Commands, with no image build or real credential:

```sh
python3 docs/archive/2026-11-17-share-mount/layout-probe.py
python3 docs/archive/2026-11-17-share-mount/image-path-probe.py
```

These are native Apple feasibility probes. They are not acceptance of an
implemented public `pinfold box up` boundary.

[Image-path output](2026-11-17-share-mount/image-path-native.txt):
- Lines 1-2 name the distinct declared destinations.
- Lines 11-15 show the image symlink, replacement metadata, and read-only
  and writable mounts at the same effective guest path.
- Lines 17-19 show original protected bytes retained and replacement bytes
  changed by the box.
- Lines 29-32 show the corrected layout reading the original Git data,
  refusing a Git write, and successfully editing an ordinary host file.
- Lines 33-35 retain protected bytes and fixture removal.

[Alternate-case output](2026-11-17-share-mount/layout-native.txt):
`host alternate-case resolves: True` establishes a case-insensitive source.
Both `.git/config` and `.GIT/config` writes fail read-only; both reads
return `protected`. The host retains those bytes. This particular case
alias does not bypass Apple 1.7.1's nested mount in this observation.
It is not proof about other filesystems, runtimes or aliases.

## Scope and cleanup

[Environment output](2026-11-17-share-mount/environment.txt) records the
runtime, selected existing image, inventories, cleanup and final free disk.
Only temporary fixture directories and the named `probe-share279-*`
containers were created. Scripts retain each fixture's removal result.
No volume or image was created. The pre-existing buildkit was left running.
Final free space was 53 GiB at the retained check. No build ran.

No public CLI acceptance, Linux execution, fmt or clippy was run after
this design stop. No test was added. Caller changes: none.
BRIEF.md, PLAN.md and REFERENCE remain uncommitted. Nothing was pushed.
