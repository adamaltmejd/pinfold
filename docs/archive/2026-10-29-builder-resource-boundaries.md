# Builder resource boundary observations

Date: 2026-10-29 local time. Raw timestamps are UTC on 28 October.
These are finite manual CLI trials, not numeric approval, new e2e tests or
the full CI suite. No resource limit was changed for the trial.

The production binary was
`fdefe37d46cade9d2e4b85c55b2f48ba63b0ad0e94cbd84fcd68090779e602b7`,
source `c4bcd4d`, using the ordinary verified 093 builder cache. The
driver ran as the normal disposable VM user with default XDG paths and a
file-backed temporary directory. It read only host-written catalog and
marker metadata; it did not read, mount or unpack a guest-writable disk.

| Boundary | Negative input and observation | Smaller or finite control |
|---|---|---|
| 512 MiB frozen input | A new sparse ordinary file nested in the context had 513 MiB logical size. CLI failed/exit 1 with `resource: build input size` in 5.643 s. | The same command with that file truncated to zero built in 15.894 s. |
| 512 MiB completed root | Normal RUN wrote 500 MiB of zeros, keeping the compressed OCI small. Actual conversion exceeded the root bound and failed/exit 1 with `resource: converted root size` in 30.513 s. | The same command with RUN writing 1 MiB built in 16.765 s. |
| 300 s job deadline | Normal RUN blocked on a FIFO. CLI failed/exit 1 with `image-output deadline exceeded` in 308.851 s. The driver's 420 s timeout did not fire. | A finite COPY recipe under the same image name and CLI command built in 16.537 s. |

All three failures left catalog bytes and public references unchanged.
The input and deadline failures retained the unclean cache marker; root
conversion failed after the build cache had reached its clean barrier and
left that marker absent. Each successful control left it absent. The
deadline duration is whole-command wall time, including preparation and
teardown, not an independent measurement of the internal timer instant.

Inputs were preserved before changing them for their controls. The
copied-out bundle contains each recipe/context, actual command/stdout/
stderr, elapsed time, catalog bytes/hash and references, marker metadata,
driver identity and observed executable identity. Its sparse negative
context remains sparse in the archive.

Evidence is under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
`resource-boundaries-success.tar.gz` has SHA-256
`dd341380fc5f04afc1f1e1a4edebba4e30a04e7d0a9be500b66e6ef42beb59f7`.
The driver SHA-256 is
`2db07c45cf345b128ca89647fab40c9010592195d2585c9d41e48b7f9801d183`.
Raw progress is in `linux-resource-boundaries-1.log`. The exact production
source and binary remain in the successful lifecycle bundle.

These observations do not measure peak CPU, memory, cache or scratch
allocation, worst-case concurrency, output-payload/JSON/object-count/header
boundaries, or malformed graph/hash/URL rejection. The separate truncated
output receipt covers one genuine incomplete stream. Candidate allocations
remain unapproved; shipped-profile fit and boundary enforcement do not
alone establish suitable headroom for arbitrary projects.
