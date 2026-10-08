# Apple builder reclamation: issue 86

The patch extends guarantee 15. Explicit Apple cleanup prunes unused
BuildKit records, then requests online filesystem reclamation. It keeps
the same builder and active builds. The missing-builder return and the
stopped-builder start path are unchanged. Native prune or trim failures
propagate through `CleanPlan::remove`; neither is ignored. No runtime
version fallback or change to Podman was added.

Apple Container 1.5.0's [runtime clean operation](https://github.com/apple/container/blob/1.5.0/Sources/Services/RuntimeLinux/Server/RuntimeService.swift#L792-L841)
trims writable block filesystems. It does not delete the container or its
allocated files. Its [integration test](https://github.com/apple/container/blob/1.5.0/Tests/IntegrationTests/Containers/TestCLIClean.swift#L84-L99)
measures allocated host blocks after a guest file is deleted and confirms
the container remains running.

## Mechanism evidence

The shared builder began stopped, with creation time
`2026-10-07T12:45:26Z`. Starting that instance left zero BuildKit cache
records. A read-only inventory saved 19 existing image references and IDs.

A real CLI build wrote, synced and deleted 64 MiB of random data, then
synced again. Native pruning selected only the six cache IDs created by
that fixture, using exact `id==` filters. Host allocated-block measurements
separated pruning from trimming:

| Measurement | Allocated bytes |
| --- | ---: |
| Before fixture-only prune | 1,897,185,280 |
| After prune and guest sync | 1,897,197,568 |
| After online trim | 1,657,491,456 |

Pruning reclaimed no host blocks. Online trim reclaimed 239,706,112 bytes.
This includes fixture base-cache data; it is not a measurement of only the
64 MiB file. The builder retained its creation time and remained running.
All 19 preexisting image references still named their original IDs. The
fixture's two image tags were removed afterward.

The first proposed probe included unfiltered cleanup and was rejected by
automatic approval review. It did not run. The approved probe used exact
fixture cache IDs and verified preservation. Two scratch-fixture errors
were corrected before the measurements: Apple interpreted a bare digest
as a registry name, and BuildKit's JSON uses lowercase `id`. Neither was a
product regression or an E2E retry.

## Test audit and checks

- Guarantee 15, `box_.rs:2020-2087`, covers the addition at
  `core/runtime/apple.rs:161-164`. A bounded real RUN writes and deletes
  64 MiB while the existing second caller remains held in RUN.
- Native pre-trim removes earlier free blocks before the fixture writes.
  Host `du` supplies the expected allocation: the fixture must increase it,
  dry-run must preserve it, and real clean must lower it. The existing
  report check and concurrent-build completion checks remain.
- Named sabotage: omit the final online trim. Pruning leaves the deleted
  guest blocks allocated on the host, so the decrease assertion fails.
  The existing blocks measured reported size and build survival, not
  reclamation. No sabotage was run by hand.
- No new guarantee test, mock, binary flag, sleep, retry, dependency or
  shared helper was added. Native failure propagation and unchanged
  missing/stopped-builder handling were reviewed in source, not injected
  as additional runtime scenarios.
- `cargo fmt --check` passed. Strict all-target Clippy passed. Focused G15
  passed: one test, 64.40 seconds. The complete host suite remains for the
  combined candidate.

After focused G15, only the same running builder remained. No issue-86
fixture image remained. The test's normal default-profile build moved
`:latest`; the prior image and its unique tag remained. Other preexisting
image references were unchanged.
