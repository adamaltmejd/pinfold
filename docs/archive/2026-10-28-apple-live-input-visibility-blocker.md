# Apple live-input visibility blocker

Date: 2026-10-28. Phase 4 of the approved modular plan.
Base: `d7656d7dbe832635615f3dd69ae3840e68c6f7ab` (phase 3 merged).
This is diagnostic evidence and a proposal, not a new caller guarantee.

## Result

Native Apple read-only directory sharing did not meet the approved live
contract on this host. A fresh guest open after a completed host rename
still read the old path. After atomic file replacement, a fresh guest open
returned ENOENT although the host confirmed the replacement's inode and
bytes. Both a dedicated `/data` mount and a mount nested in a writable
disk at `/workspace/inputs` showed these failures.

The same failures occurred with the clean, reviewed phase 3 binary and its
existing box mount spec, with no `live` field. They do not depend on the
new configuration or resolved-source code. This does not isolate the
underlying defect to a particular upstream component.

Do not publish the phase 4 guarantee, accept eventual visibility as its
implementation, or mask this failure with sleeps or retries. No feature
commit or binding-spec/README change was made. Phase 5 was not started.

## Environment and identity

- macOS 27.0.1, build 26A434; arm64.
- Apple Container CLI 1.6.1, build `release`, commit `unspeci`.
- Guest kernel: `6.18.15 #1 SMP Tue Mar 17 01:36:53 UTC 2026 aarch64`.
- Default profile image id:
  `6d7509f79fb54099f271330942a911d71a82eba394fad33d428c532f25c1ef76`.
- Clean phase 3 source: `00c7fe5d14d2054e6265aba7df345c847c2bb46f`.
  Binary SHA-256:
  `d1d87f763d519ae29a5318510ca2d9c84a5d9abc1411800c4f7cdb27775364a0`.
- Initial phase 4 candidate binary SHA-256:
  `21d5d054be3cf5e51f5387685d6bd0124f48ef68baf29456dc53d8335a631581`.
  Its source was uncommitted on the base above. The candidate patch is
  retained locally under the evidence root below as
  `candidate/incomplete-candidate.patch`.

## Method and observations

Each box used the real binary, no egress, and a native read-only mount.
The source declaration was a symlink to the fixture directory. Neither
the attached root nor that symlink changed. The nested case also attached
a new 1 GiB writable disk at `/workspace`. `/proc/mounts` reported
`virtiofs ro,relatime` for each input.

Every host operation completed synchronously. Host stat/read evidence was
recorded before launching a separate `pinfold box exec ... -- cat ...`.
No guest file descriptor was retained across host operations. Positive
and negative lookups were primed before mutation. No sleeps, polling,
cache flushes, directory scans or retries were inserted.

The clean phase 3 dedicated-mount observations were:

| Completed host action | Host evidence | Fresh guest observation |
|---|---|---|
| Modify `nested/value` in place | `modified and longer\n` | Exit 0, same bytes |
| Create previously absent `created` | `created\n` | Exit 0, same bytes |
| Rename `removed` to `renamed` | Old path absent; new inode 775448877, `remove me\n` | `cat /data/removed`: exit 0, `remove me\n`; new path also reads |
| Atomically replace `nested/value` | New inode 775448918, size 19, `atomic replacement\n` | Exit 1, `cat: /data/nested/value: No such file or directory` |
| Replace the nested directory within the attached root | New directory's `value` contains `new directory\n` | Exit 1, ENOENT at `/data/nested/value` |
| Delete `created` | Host path absent | Fresh `stat` still succeeds; fresh `cat` fails ENOENT |

The nested case produced the same outcomes. Its post-rename host inode
was 775448926 and its atomically replaced file inode was 775448996.
The phase 4 candidate also showed both fresh-open failures in both layouts.

Guest writes, creates, renames, deletions and mkdir through the mount all
failed with `Read-only file system`. Final host reads confirmed unchanged
bytes and no new guest-created paths. The same guest user successfully
created, wrote, moved, read and removed a file and created/removed a
directory in `/tmp`. Read-only enforcement passed these bounded probes.

These results establish a fresh-open violation, beyond stale stat data.
They do not establish the stale interval, eventual convergence, other
host/runtime versions, filesystem notifications or root rebinding.
A separate agent read the diagnostic and evidence and agreed with this
bounded conclusion. It did not rerun the probes.

## Supported controls inspected

The installed `container run --help` exposes no cache control.
An actual `container create` request with
`--mount type=bind,source=SOURCE,target=/data,readonly,cache=none` was refused:

```text
Error: unknown directive cache when parsing mount type=bind,source=SOURCE,target=/data,readonly,cache=none
```

The version-pinned [1.6.1 volumes documentation](https://github.com/apple/container/blob/1.6.1/docs/volumes.md)
lists bind/volume/tmpfs, source, destination, readonly, and tmpfs size/mode.
The [1.6.1 parser](https://github.com/apple/container/blob/1.6.1/Sources/Services/ContainerAPIService/Client/Parser.swift#L442-L454)
rejects other directives. The [pinned Containerization 0.53.0 VZ implementation](https://github.com/apple/containerization/blob/0.53.0/Sources/Containerization/VZVirtualMachineInstance.swift#L504-L513)
constructs shared directories from a URL and read-only flag, with no cache
setting in this path. No supported setting meeting the contract was found.
No custom kernel, runtime replacement or filesystem service was attempted.

## Local reproduction evidence

The full diagnostic scripts record specs, exact argv, exit codes,
stdout/stderr, elapsed time, and host paths/inodes/bytes. They are local
artifacts preserved outside Git in this worktree at
`/Users/adam/.codex/worktrees/phase-four-live-inputs/pinfold/.local-evidence/phase-four-live-inputs/`.
Paths below are relative to that evidence root:

- Candidate: `candidate/evidence.json`, `candidate/reproduce.py` and
  `candidate/diagnostic.log`.
- Clean phase 3: `baseline/evidence.json`, `baseline/reproduce.py` and
  `baseline/diagnostic.log`.
- Inspected upstream sources: `sources/`.
- Post-cleanup inventory: `runtime-after.json`.
- `manifest.json` records original scratch paths, sizes and SHA-256 hashes.
  Each preserved copy was checked against its scratch original.

The initial real e2e visibility test failed after rename (20.43 s).
Clippy with all targets, locked dependencies and warnings denied passed
on the initial candidate. The full Mac and Linux suites were not run:
the feature cannot pass its new guarantee. The new assertions remain
uncommitted and have unresolved audit findings, including Pi-specific
Linux refusal evidence and complete positive controls. No passing
cross-platform feature gate is claimed.

All diagnostic owners exited with `stdin-closed`, exit 0. Their temporary
disks were removed through `pinfold disk rm`. Runtime inventory afterward
held only the pre-existing `buildkit` service. The Mac test slot is released.

## Decision needed

Recommend deferring strict live inputs until a supported Apple path can
pass the unchanged contract. Preserve the incomplete candidate for reuse.
An alternative is to approve a separate, weaker visibility contract after
measuring it. These probes establish no safe bound for eventual visibility.
That would require an explicit product/spec decision and new evidence.
An alternative filesystem implementation is outside this phase's scope.
