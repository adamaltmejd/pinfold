# Y-10: initial Mac runtime checks

2026-10-09, Pinfold 0.2.2, Apple Container 1.5.0, macOS 27.0.1 arm64.
This extends the CLI evidence in `2026-10-09-cli-hardlink-reproduction.md`.
No production implementation or protection guarantee changed.

## Fixture

Two fresh real `pinfold box up` runs used an alias-free repository, a
protected `.git/config`, a protected `.vscode/settings.json`, and separate
writable project, home and additional directories. The first spec listed
protected mounts before writable mounts; the second reversed that order.
Both waited for the CLI's `ready` event. Commands ran through
`pinfold box exec`, without model calls or executable payloads.

The cached `python:3.14-slim` image supplied the probe interpreter and
libc's linkat. The preceding default/full profile attempts found no
`python3` executable; they performed no filesystem probe and shut down
cleanly. Pinfold supplied the same runtime isolation and mount controls.

## Observations

Each successful probe ran 57 operations. All expected protected operations
failed and all ordinary-file controls succeeded, in both mount orders:

- Case-alias append attempts through `.GIT/config`, `.VSCode/settings.json`
  and `.git/CONFIG`, performed before the normal-spelling operations.
- Direct appends to both protected files.
- New links from each protected file into each writable directory, using
  `link`, `linkat` with flags 0, and `linkat` with AT_SYMLINK_FOLLOW through
  a guest-created symlink.
- Renaming either protected file inside its directory and renaming either
  protected directory out of the overlay.
- Ordinary-file append, all three hardlink methods, writes through those
  links, chmod, rename and delete in each writable directory.

The host observed unchanged protected bytes, inode identities and modes
after each run. No protected alias was created, so the conditional write
and chmod operations through such an alias were not reached. Both owners
exited 0 after stdin closure.

Raw operation-level errno results, host observations, mount specs and the
probe are under `/private/tmp/pinfold-y10-mac/result.json` and `probe.py`.
They are temporary investigation evidence, not permanent test fixtures.

## Limits and next gate

These observations support only this tested Mac layout. They are not a
proof of the proposed admission scan. The earlier pre-existing-hardlink
breach remains. Duplicate exports, all protected-path replacement forms,
host topology changes, local-clone compatibility and traversal cost remain
unresolved parts of Y-10.

The delegated Linux attempt acquired a temporary Debian VM, but no
security probe ran and no Pinfold binary was downloaded. Prerequisite
installation encountered an existing package-manager lock; the agent's
turn was then stopped by a safety-system flag. It did not retry the probe.
Cleanup-only follow-up successfully released/reset the lease and reported
`status: idle`. There is no Linux security result from this attempt.

Y-10 remains parked for host verification. Do not admit a security
implementation or claim cross-platform protection from these partial
results. Independent, already-scoped test-audit work may proceed through
Yard's existing review and host landing gates.
