# Stock Podman 5.4.2 and 6.1.4 on the original LXC

Stock 6.1.4 did not resolve issue #89. The original workload reproduced
20-second listing timeouts on both versions. Actual POSIX lock records
on 6.1.4 again linked blocked listings to container starts with
`storage-chown-by-maps` children. The newer runtime also reproduced an
exec-session removal failure, `openByHandleAt: operation not permitted`.

## Completed workload results

| Observation | Stock 5.4.2 | Stock 6.1.4 |
|---|---:|---:|
| Tests passed / failed | 70 / 2 | 68 / 4 |
| Cargo workload wall time, seconds | 934.288 | 824.121 |
| Completed observed listings | 665 | 859 |
| Listing timeouts, exit 1 | 4 | 6 |
| Median listing, seconds | 0.131 | 0.093 |
| Longest listing, seconds | 20.023 | 20.023 |
| Sampled ownership helpers | 11 | 10 |
| Sum of sampled helper spans, seconds | 166.630 | 172.389 |
| Host CPU pressure, some | 63.078% | 21.551% |
| Host I/O pressure, some | 19.662% | 15.869% |
| Final private native inventory | empty | empty |
| Observer records without completion | 7 | 5 |

Both Cargo invocations exited 101. The time difference is not a causal
speed estimate: runs were sequential, host pressure differed, failures
changed execution paths, and one 6.1.4 test waited on its existing
600-second event deadline. Counts include polling during those paths.
Helper spans are sampled lifetimes, not CPU time. Unfinished observer
records have unknown outcomes. Completed listing and exec records reported
no EAGAIN; this does not assign or exclude the historical EAGAIN failure.

## Conditions and identities

The unchanged original Switchyard source was
`23e55b2d0f7d38bfd14a6ce11ba6dcc837cebf5e`, running all 72 tests with
`cargo test -p e2e --locked` and default concurrency. Source status was
clean before and after. Assertions and workload deadlines were unchanged.

Both arms used the retained Pinfold binary from
`cdf2c41970e82e41a453c5c55164d9a9bc3982ff`, including bounded inventory
and acknowledgement teardown changes. Using this same artifact avoids
confounding the comparison with the new minimum-version admission check,
which would refuse 5.4.2.

The original OCI archive was loaded into a fresh native-overlay graph/run
store for each arm. Both reported `Native Overlay Diff: true` and
`Supports shifting: false`. No FUSE, mapped-image ingestion, extra image
preparation, Podman patch, security relaxation or concurrency reduction
was used. Both used private file locks to avoid the active per-user shared
memory locks; this is an isolation difference from the original host
default. Host page caches and unrelated services were not reset.

5.4.2 was Debian package `5.4.2+ds1-2+b2`. 6.1.4 was built from unchanged
upstream commit `464c15fec06f0074c916f9eff87c4806869c2833`, with default
native build tags, in the disposable Debian 13 VM. It used Yard's installed
crun 1.21 and conmon 2.1.12. The VM was released after exporting artifacts.

| Artifact | SHA-256 |
|---|---|
| Retained Pinfold | `85a75dd43f5a197be7e6df1a24e32f45f3291838f19dcef0d0c3c7c710a3d5ca` |
| Stock 5.4.2 binary | `6e4993e061eb88205003892252e2e13bbf49c12a6f67f519881b9b3c94e980e6` |
| Stock 6.1.4 binary | `11326740634a0929f438256c4e3ff90273c4399a279c68e9fde02be25f75c2a5` |
| Original OCI archive | `08ad0be8b6e6275c58229c82517c95fb42cdede1e11b25b527f4b4fe60bf53ee` |
| Initial comparison driver | `77617caa938381403c8a6b02b74dcd96b0ecc4c32cadf57a31937edf44a87123` |
| Corrected comparison driver | `f6b303ef17be69e66d166687dca561279df3aa02661113455ccb2a01cf5a493d` |
| Build archive | `0fbad2eb5484ab69b8676a526280ad94e2017d587ea769685af8c2486919c2c3` |
| Workload evidence archive | `a268d30dccaa561d14db94cf618f44df02df573b580f24fbec98ae14e1d3ee73` |

## Actual lock chains

In the baseline, native listing processes 761232 and 761335 waited on
`images.lock`, device/inode `fc:08:527703`. The holder was `podman run`
761031, start ticks 125841300, with direct ownership-helper child 761058,
start ticks 125841308. Each native listing had 33 sampled POSIX wait edges.
Each corresponding Pinfold listing exited 1 with `runtime-timeout` after
approximately 20.02 seconds.

Three of the six corrected 6.1.4 listing timeouts were traced individually:

| Native listing / start ticks | Holder / start ticks | Direct helper / start ticks | Wait-edge samples |
|---|---|---|---:|
| 959345 / 125968725 | 959256 / 125968640 | 959266 / 125968650 | 34 |
| 959365 / 125968832 | 959256 / 125968640 | 959266 / 125968650 | 34 |
| 960102 / 125971410 | 960065 / 125971391 | 960081 / 125971399 | 35 |

All three waited for WRITE access to `overlay-images/images.lock`,
device/inode `fc:08:708736`. Descriptor identities and sampled
`fcntl(F_SETLKW)` calls agree with the actual POSIX wait records.
CLI records `call-959343.json`, `call-959363.json` and `call-960095.json`
report exit 1, `runtime-timeout`, and approximately 20.02 seconds.

Subsequent lock/process samples show the timed-out waiters gone while the
holders and helpers remain alive. Later listings succeed. For the first
6.1.4 pair, see `locks.jsonl` lines 655 and 659 and `resources.jsonl`
line 189. For the third, see `locks.jsonl` line 704 and `resources.jsonl`
line 201. Recovery records include candidates 959596 and 960536.

This proves contention, bounded failure, sampled waiter removal and later
listing recovery. It does not prove empty stdout, writer exit success or
preservation of a particular live box/owner. Yard logs name capacity
retention, but these observations are not a complete external validation
of the fail-closed capacity guarantee. Issue #89 remains unresolved.

## Failures remain separate

5.4.2 failed the durable-audit iteration assertion (0 instead of 12) and
the gate-error scenario, which reported unknown ownership and retained
capacity. The former was not assigned a specific runtime cause.

6.1.4 failed:

- The gate-error scenario: ownership attention instead of landing attention,
  after the listing stalls.
- The silent-reviewer scenario: runtime removal failed while reopening an
  exec PID handle, with `openByHandleAt: operation not permitted`.
- The legacy-planner scenario: the same removal error is recorded in its
  execution row; subsequent ownership refusal prevented the expected event.
- The prompt-update scenario: observed `first, retry` instead of
  `first, restart`. Its precise cause was not established.

The PID-handle error is distinct from storage contention. Stock 6.1.4
[only handles ESTALE at this point](https://github.com/podman-container-tools/podman/blob/464c15fec06f0074c916f9eff87c4806869c2833/pkg/pidhandle/pidhandle_linux.go#L86-L134);
EPERM propagates. No PID-reuse protection was bypassed.

## Excluded first 6.1.4 run

The original trial configuration redundantly set both `graphroot` and
`rootless_storage_path` to the same private path. 6.1.4 warns about that
combination. Its ownership helper inherits the warning, and the parent
treats any combined output as an error even when the helper succeeds.
A direct native startup reproduced exit 125; removing only the redundant
setting made the identical command exit 0.

The mechanism is visible in upstream
[configuration handling](https://github.com/podman-container-tools/podman/blob/464c15fec06f0074c916f9eff87c4806869c2833/vendor/go.podman.io/storage/types/options.go#L235-L254)
and [ownership-helper handling](https://github.com/podman-container-tools/podman/blob/464c15fec06f0074c916f9eff87c4806869c2833/vendor/go.podman.io/storage/drivers/chown.go#L85-L102).
This diagnostic/error-channel coupling is separate from issue #89.

The invalid run was interrupted through PID handles scoped to its own
process tree. Its native inventory ended empty. A separate startup probe
ran during that invalid run; neither run contributes performance evidence.
The corrected run used a new empty store and unchanged workload. The
baseline's redundant field was accepted by 5.4.2; effective root checks
and native-overlay settings make the normalized comparison defensible.

## Preservation and evidence

No release, GitHub comment or issue closure occurred. Yard's installed
Podman remains 5.4.2 with the checksum above. Installed Pinfold remains
`7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`.
Operational Yard PID 965905 retains start ticks 89301792 and its executable
identity. Active runtime storage was not reset or used by these commands.

Remote evidence:
`/home/yard/validation/pinfold-89-lxc-20261012/stock-version-20261013/`.
The completed arms are `stock-5.4.2-1` and `stock-6.1.4-2`.
`stock-6.1.4-1` is explicitly excluded; `probe-6.1.4-1` captures the
configuration reproduction. Logs, identities, CLI records, lock observations,
resource samples and driver scripts are retained there.

Local exports are `/private/tmp/pinfold89-stock-version-20261013-evidence.tar.gz`
and `/private/tmp/pinfold89-stock614-build.tar.gz`. The latter includes the
binary, stock storage template, build log, source identity and dependency
report. The support-floor change is not evidence of a contention fix;
these results do not support upgrading Yard as a remedy for #89.
