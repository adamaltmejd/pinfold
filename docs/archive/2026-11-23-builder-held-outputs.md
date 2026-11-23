# Apple's builder holds every build's export, 2026-11-23

Issue #440. macOS, Apple `container` 1.7.1, builder shim 0.13.3
(BuildKit 0.34.1), pinfold 0.6.3.

## The leak

Each `container build` writes
`~/Library/Application Support/com.apple.container/builder/<UUID>/out.tar`,
loads it and unlinks it. The builder's host process keeps the unlinked
file open, so its blocks stay allocated until the builder stops.

The builder had run since 2026-11-22 14:45. `lsof +L1` showed 295
deleted `out.tar` files of 520 to 608 MB each, held by one process:
153.9 GB in total, with 11.8 GiB free on the volume. Nearly all came
from Mac suite runs. `container builder stop` released them: free space
rose to about 96 GiB within seconds and the count of held files went to
0. The space is in neither the builder's backing storage nor the image
store, so `doctor` and `clean` did not see it.

## Decision

`clean` ends its Apple build-cache step by stopping the builder when no
build is running. The automatic pass and the post-build prune never stop
it. `doctor` does not measure the held space: the only observer found
is `lsof` on the builder's host process.

## Deciding that no build is running

- Pinfold's builds hold a new host lock, `builder:apple`, shared from the
  base refresh through the post-build trim. `clean` takes it exclusive
  and non-blocking for the check and the stop.
- Other callers' builds: `buildctl debug histories` inside the builder
  lists a build record without `CompletedAt` while a build runs.
  Measured with a `RUN sleep 60` probe started by `container build`: the
  record appeared at the first poll, 0.3 s after the client started, and
  stayed active until the client exited. A client killed by SIGKILL
  completed its record within 0.12 s, so a dead client does not block
  the stop.
- With a 200 MB `dd` probe the record completed 0.8 to 1.1 s before the
  client exited, while the client loaded the export. Stopping the
  builder at that moment did not fail the build (rc 0, image tagged).
- The residual window is a build started outside pinfold between the
  check and the stop, about the time `container builder stop` takes
  (0.4 s measured).

## Verification of the change

- `container builder stop` keeps the cache: `buildctl du` read 559.28 MB
  before the stop and after the restart, and the probe's `RUN` step was
  `CACHED` after the restart.
- Three `--no-cache` 200 MB probe builds left 4 held `out.tar` files.
  `clean` stopped the idle builder and the count went to 0. The next
  probe build restarted the builder and succeeded.
- With a foreign probe build in flight, and separately with another
  process holding `builder:apple` shared, `clean` left the builder
  running, printed the warning and exited 0. The foreign build
  completed.
- With the builder already stopped, `clean` started it, pruned, trimmed
  and stopped it again, silently, exit 0.
