# Builder lifecycle receipts

Date: 2026-10-29, local time. Raw trial timestamps are UTC on 28 October.
The evidence directory remains
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.

The first real interrupted RUN exposed a cancellation bug. After its
audited API request, another build was refused as runtime/builder-in-use.
SIGINT then failed to complete within 60 seconds. The external driver killed
the owner; only that later kill ended the VMM. Catalog bytes were unchanged
and the cache remained marked unclean. This is failure evidence, not a
successful SIGINT result. The binary was
`615300f8d7bb13a336666df26bd5a616ff79491b573a65702af33f1aed954f50`.

`JobReader` exposed semantic cancellation as `Interrupted`, which
[`read_exact` retries](https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact).
The adapter now maps only that error to `ConnectionAborted`, preserving its
cause and other error kinds. The underlying pipe code still handles ordinary
syscall interruptions. Both framed reads and BOOT writes use this adapter;
shared download/login pipe behavior is unchanged.

The fixed finite driver used binary
`fdefe37d46cade9d2e4b85c55b2f48ba63b0ad0e94cbd84fcd68090779e602b7`.
All three cases passed with no cleanup error:

| Case | Observed result |
|---|---|
| SIGINT and contention | Audited RUN; competing build refused runtime/builder-in-use; SIGINT returned failed/exit 1; VMM pidfd became readable; catalog unchanged; next identical contender command built successfully. |
| Owner SIGKILL | Owner exited from SIGKILL; VMM pidfd became readable; catalog unchanged; next ordinary build recovered successfully. |
| Audit I/O failure | Atomic exchange replaced the owned live audit file with a directory. The owner returned failed/exit 1 with audit-log, its VMM ended and catalog stayed unchanged. The file was restored and the next build succeeded. |

Each interrupted cache retained its marker; each successful recovery cleared
it. These are external observations of the normal CLI, production audit and
host kernel. No guest-writable disk was read or mounted on the host, and no
runtime or producer was replaced. Pidfd readability proves task death; it
does not independently prove PID-1 reaping or the source's finer
reap-before-claim ordering. No late post-commit cancellation acknowledgement
or physical-crash guarantee is inferred.

The existing guarantee 44 test now contains the blocked-RUN SIGINT case,
unchanged public-reference check and existing ordinary recovery build.
Its private readiness observer uses the already-present Nix dependency's
inotify feature, enabled only by the e2e package. No crate version, binary
test flag or production seam was added. The test passed in **130.19 seconds**
in `linux-row44-cancel-1.log`; the actual binary was rehashed afterward and
matches `fdefe37d...79e602b7`. Linux and Mac locked all-target Clippy and
formatting passed. This is a targeted test pass, not the full Ubuntu suite
or its five-minute budget.

| Retained file | SHA-256 |
|---|---|
| `lifecycle-first-failure.tar.gz` | `4aaece6cc5b4b1e64377e460cfc367f706e5865910cfa96daa9967bc625f907b` |
| `lifecycle-after-success.tar.gz` | `86cf5a34661a2fad36ba584eb6b7170d0f95e32bee36704f9737dc5a5401294b` |
| `cancellation-regression-source.tar.gz` | `d6e156824beb2ed81228d1be6223bc014be7bcff0ee201308a58619e22eca444` |
| `linux-row44-cancel-1.log` | `a9778a85b549b6a9a362fb48aa425cc6a583bce29057891c01d32eed7b2c432e` |

The successful lifecycle bundle includes exact executed source, binary,
driver, raw stdio, kernel receipts, catalog hashes and production audit logs.
Mac runtime validation remains blocked by recorded host ENOSPC failures.
Publication, numeric approval and the remaining admission gates are separate.
