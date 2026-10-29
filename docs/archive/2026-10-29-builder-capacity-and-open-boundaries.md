# Builder capacity and open boundaries

Date: 2026-10-29. This records retained observations, not numeric policy
approval or final runtime admission. No limits were changed for these runs.

The external observer put the normal uid-1000 CLI and descendants in a
new accounting cgroup. Its receipts show zero active/inheritable/ambient
capabilities, unchanged limits/controllers and completed cleanup. All three
groups recorded no host OOM event. `io.stat` was unavailable; no controller
was enabled to obtain it.

| Observed workload | Whole measured run | CPU time | Peak charged bytes | Result |
|---|---:|---:|---:|---|
| Full profile build | 93.057 s | 114.973 s | 3,006,369,792 | Built successfully; completed root is 451,499,008 bytes. |
| 4,100-copy object-limit attempt and 50-copy control | 329.901 s | 551.259 s | 3,000,025,088 | Large attempt hit the job deadline; smaller control built. Object-count rejection remains unproved. |
| Work/cache exhaustion sequence and controls | 119.993 s | 131.799 s | 3,013,230,592 | Work-pressure recovery succeeded; persistent-cache recovery failed. This sequence is failure evidence. |

Memory is the kernel's peak bytes charged to the group, including newly
charged page cache. Reused shared file-cache charges from earlier groups
can be excluded. It is not summed RSS, guest memory, a per-phase peak or a
worst-case bound. CPU time covers the whole measured process tree, not an
instantaneous CPU peak. The object and exhaustion totals include their
driver, negative case, controls and cleanup.

The object attempt returned failed/exit 1 after 310.763 s with
`resource: builder job deadline`; the external 420 s timeout did not fire.
References stayed unchanged. The 50-copy control built in 18.844 s and
owned-image cleanup succeeded. An earlier guard prevented observation of
the 4,096-object guard; this is not a pass of that guard.

The work-pressure recipe created then deleted a 3 GiB zero-filled layer.
Its client-tar-export step returned ENOSPC after 62.955 s without changing
catalog bytes; a 1 MiB variant built in 17.380 s. The pinned init uses that
tar-export path for conversion. This is source/log phase attribution,
not independent phase or peak-disk accounting; the driver deliberately
leaves `phase_established=false`.

The cache-mount recipe then returned ENOSPC after 21.959 s. A following
plain COPY build also failed with ENOSPC after 12.640 s, while the cache
marker stayed absent and public references stayed unchanged. No cache-mount
repair or manual host deletion was attempted. The observer completed its
measurement, but the workload exited 1. That is the reproduced recovery
bug, not a successful exhaustion/recovery result.

These measurements belong to the earlier `c4bcd4d` trial sequence. The
object/exhaustion drivers record executable SHA-256
`fdefe37d46cade9d2e4b85c55b2f48ba63b0ad0e94cbd84fcd68090779e602b7`.
The full-profile observer records an executable path, not its own
per-invocation executable checksum; do not treat that receipt alone as
exact-binary attestation.

The later selective cache-mount cleanup candidate separately passed G44
in 160.14 s. `linux-full-cache-regression-2.log` records source archive
`72af587480693b11ed1f3629cd30025593c31311e4f8c685823b342f4e770c46`
and matching executable checksums before and after the test:
`b529085013cb25ab86cb06ea7dfc98578f3746eb91ceaf370fee9173a1cb698a`.
That recovery pass does not turn the older failure into a pass or make the
older performance measurements measurements of the revised executable.

The full root is 430.583 MiB, leaving 81.417 MiB below the candidate
512 MiB file-size bound. This is nominal root-file headroom, not capacity
for arbitrary packages. Large discarded layers can exhaust converter work
even when the final tree is small. The full-profile group charged about
2.800 GiB on the host; the configured 2.5 GiB guest allocation does not
describe total host memory cost. No concurrent capacity margin is proved.

Remaining boundaries are explicit:

- The input, root, JSON and aggregate-payload receipts have their own
  smaller controls; see [resource bounds](2026-10-29-builder-resource-boundaries.md)
  and [JSON/payload bounds](2026-10-29-builder-json-and-payload-receipts.md).
  They do not prove the object-count or frame-header guards.
- No ordinary-recipe seam was found for an oversized frame header, wrong
  hash, malformed graph, unsupported media, descriptor URL or attachment
  under the pinned sender/exporter. That source assessment is not runtime
  rejection evidence from a compromised producer.
- Peak cache/work disk allocation, inode exhaustion, every form of regular
  layer-cache exhaustion, concurrent capacity and lease-acquisition races
  remain unestablished. Passing mutable-cache recovery does not cover them.
- Full Ubuntu and Mac suites, the five-minute CI budget, normal fresh
  public bootstrap installation, final reviews and numeric policy remain
  separate gates. No arbitrary limit increase follows from these results.

Metadata and raw receipts are retained under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
Both bundle hashes below were independently rechecked for this note.

| Bundle | SHA-256 |
|---|---|
| `object-limit-attempt.tar.gz` | `a8a664ce651e970185a505a08ae8202658acda24bda9133b2383fa25c1468826` |
| `exhaustion-first-failure.tar.gz` | `1815c6cb5e4ecd22bf546fb9827a09773cff3b2bf9a98a48dac58199293772b8` |
