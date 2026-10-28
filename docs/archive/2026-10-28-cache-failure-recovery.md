# Ordinary recipe failure and cache recovery

Date: 2026-10-28. Guarantee 23 already repeats an unchanged caller recipe
after a failing recipe. `linux-row23-local-1.log` reached that assertion
and failed in 340.52 seconds. No new test or assertion was needed.

A smaller real CLI reproduction used a local image, an unchanged COPY
fixture and a RUN-generated random stamp. Before the fix, binary SHA-256
`641a3a44fba3449e3062d060d4b62d1088110299a9af9d3c087780fae17c2b85`
produced the same image for its first build and immediate cached control:
`sha256:5c3101dc1f9e4b603bfddf0a6d6defb79a3e2dab2c880bdc8e78a42bfa7f88d0`.
A separate `RUN false` failed and left the catalog unchanged. It left the
cache marked unclean, however. The next unchanged recipe produced
`sha256:c6f26d9d456ef3d3a30cd4f489ad3a7b0c6a617fa95955122ea7c38c5e18e623`.
Allocated cache backing dropped from 1,809,272,832 to 78,970,880 bytes.
Source inspection confirmed that preparation discarded the cache after
the ordinary recipe error omitted its clean completion receipt.

The fix distinguishes a handled recipe error from interrupted work.
After daemon and work teardown, sync and unmount, the guest sends a bounded
failure diagnostic, exact end frame and EOF. The host requires that complete
sequence and VMM reaping, then syncs the backing file and durably removes
the unclean marker. No failed recipe reaches conversion or publication.
Bare or incomplete failure replies do not certify the cache. Stops during
the watched clean transition restore the marker; a restoration I/O error
retires the backing file and propagates the error. Later stops need not
dirty an already-clean, unattached cache.

The first fixed CLI proof used binary SHA-256
`9e60283daf67ab5b297ec310ff07a9002786bde36cd25fb1615e4440b228064d`.
First build, immediate cached control and unchanged build after `RUN false`
all produced the same `c6f26d9d...e18e623` image. The failed recipe again
left catalog bytes unchanged, and the cache stayed clean. Those calls took
15.62, 11.87, 12.00 and 12.02 seconds. This establishes the reproduced
sequence, not final suite performance. The later restoration-error fallback
was source-reviewed after that execution; it was not triggered by this proof.

Evidence is retained under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`:

| Bundle | SHA-256 |
|---|---|
| `cache-failure-before.tar.gz` | `4cc3484179b8824244f16eff95112b1519943d999191a1e07b6ac6deef9f2cae` |
| `cache-failure-after.tar.gz` | `2574a80ce9041dd4defbefd14e086ed5d30be679e0bbedb0932ecd41fead1946` |

The second bundle includes the exact executed host/guest source and binary;
the binary was rehashed at preservation. Neither proof reads or mounts a
guest-writable filesystem on the host. The driver observes normal CLI
results, catalog bytes and host backing-file metadata.

Linux and Mac locked all-target Clippy passed. The final source, including
the restoration fallback, compiles as musl binary SHA-256
`615300f8d7bb13a336666df26bd5a616ff79491b573a65702af33f1aed954f50`.
Cancellation, owner-death, audit failure, the complete revised guarantee
and its suite budget remain separate runtime gates at this checkpoint.
