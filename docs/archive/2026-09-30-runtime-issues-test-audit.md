# Runtime issues 71–75

## Scope

Issue 71 was already fixed by `c8de207`: missing mount hosts and dangling
symlinks are refused as `spec` before the box claim. Issue 73 was already
fixed by `a10143c`: doctor runs no maintenance. That commit also added most
of issue 72's failed-Podman host diagnostics. This change adds the missing
`/dev/net/tun` access diagnostic.

For issues 74 and 75, Apple clean now asks BuildKit to prune unused cache
records instead of deleting the shared builder. It starts an existing
stopped container with `container start buildkit`; `builder start` can
replace an instance when its settings changed. No new lock or dependency
is needed. BuildKit retains active references under its own locks.

Doctor and dry-run measure allocated builder backing storage. The exports
mount identifies the runtime's actual data root. Sparse file length and
logical cache bytes are unsuitable: the observed builder occupied
8,863,576,064 host bytes while BuildKit reported about 212 MB of records.
The new report matched host `du -sk` (8,655,836 KiB).

Issue 74's minimum diagnostic remedy is implemented. There is no new
automatic cache quota. Pruning may not release all sparse backing storage;
clean reports measured use, without claiming it is all reclaimable.

Sources: [Apple builder mounts](https://github.com/apple/container/blob/main/Sources/ContainerCommands/Builder/BuilderStart.swift),
[Apple allocated container storage](https://github.com/apple/container/blob/main/Sources/Services/ContainerAPIService/Server/Containers/ContainersService.swift),
[BuildKit cache pruning](https://github.com/moby/buildkit/blob/master/cache/manager.go).

## Candidate test audit

- Row 15: extend cleanup with another process's build, in another state
  directory, held active inside RUN by a host HTTP fixture. Restoring forced
  builder deletion makes the build fail. Existing cleanup blocks never held
  a build active. Readiness comes from the host request, and success from
  the CLI result and the runtime's image listing. No sleeps or retries.
  The build caller's daily pass runs before the dead-box fixture is made,
  so that caller cannot remove it ahead of the explicit clean.
- Row 15: dry-run must count at least the backing filesystem's allocated
  bytes, measured by host `du`. Omitting builder bytes fails this block.
  The lower bound allows other concurrent builds to allocate more space.
- Remove the suite's builder mutex and its five holders. They concealed
  the cross-process failure and are unnecessary with native pruning.
- Row 27: extend the failed-runtime case to real Podman with a missing
  runtime directory. A private Linux user and mount namespace overlays
  `/dev/net` with tmpfs when present, so the missing tun device is a known
  fixture rather than a conditional check on a healthy host. Real Podman
  stays on PATH; other devices remain available inside the namespace.
  Omitting the independent host checks loses the named reasons.
  Assertions use the spec tokens and device path, not diagnostic punctuation.

No new test or guarantee row. Rows 15 and 27 describe the added scenarios.
Test files: box_.rs 3,304 lines; cli.rs 143 lines. Binary files: apple.rs
377 lines; podman.rs 505 lines; cli.rs 1,136 lines.

## Validation

- Real CLI: missing host and dangling symlink both refused as `spec`, named
  the host path, and left no named box state.
- Focused Apple row 15: passed, including the held build across clean.
- Full Apple suite: 27 passed in 107.55 seconds, with the builder mutex
  removed and builds running concurrently with clean.
- After the daily-pass fixture ordering correction: focused row 15 passed
  in 74.50 seconds; full Apple suite passed all 27 in 83.01 seconds.
- [Linux CI on 3e15836](https://github.com/adamaltmejd/pinfold/actions/runs/36681570564):
  27 passed on x64 in 104.63 seconds and 27 on arm64 in 81.37 seconds.
  Both exercised the missing-tun namespace scenario.
- `cargo fmt --check`, Clippy with warnings denied, and `git diff --check`:
  passed. The sandboxed Zig cache was unwritable; task-scoped caches under
  `/private/tmp` allowed the cross-build without changing build code.
