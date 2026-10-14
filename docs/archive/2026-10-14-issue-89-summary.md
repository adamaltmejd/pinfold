# Issue 89: investigation and validation, 12–14 October 2026

Issue #89 remains open. Stock Podman storage contention was reproduced;
no complete fix within the agreed complexity and security constraints was
validated. PR #90 retains bounded, truthful inventory failure handling.

## Brief work log

- **12 October:** Reviewed the original unprivileged-LXC workload on
  `yard-sthlm`: cold 72/72 passed; base-image-prepared 71/72 passed, with
  an unassigned EAGAIN failure. Neither run reproduced the historical
  30-second listing failures. Only the base image was prepared.
- Validated candidate `70c18327c811c7e9eae92778dcb365acec5e9ca8`
  against a real cold ownership conversion lasting 241.273 seconds.
  Listing returned exit 1, empty stdout and `runtime-timeout` in 20.033
  seconds. Kernel lock/descriptor identities proved waiter removal while
  the writer continued and ultimately succeeded. A subsequent listing
  succeeded in 0.247 seconds with the same live box, generation and owner.
  A separate caller probe retained capacity during failed inventories and
  admitted the waiting worker after recovery; its final workflow failed,
  so successful workflow completion was not established.
  This deliberately large image caused high IO pressure; its timings are
  correctness evidence, not ordinary workload performance.
- Reproduced a teardown socket-read timeout returning EAGAIN while removal
  waited on storage. This does not assign the original workload's EAGAIN
  or explain its two listings without proven lock-wait chains.
- **13 October:** Compared stock Podman 5.4.2 and 6.1.4: both retained
  listing timeouts (70/72 and 68/72 workload passes). These runs do not
  establish a throughput improvement. Upgraded Yard to unchanged 6.1.4
  and added the requested startup floors: Podman 6.1.4, Apple container 1.5.0.
- An experimental initialized API inventory passed one workload comparison
  (72/72 versus native 69/72), but did not establish a complete contention
  fix. Runtime services, storage scheduling and per-box stores were not
  adopted. Exact-image warming was tested; derived images still contended.
  A mapped-ingestion experiment also failed its full workload (68/72).
- **14 October:** Native ID-mapping returned EPERM on the existing ext4
  store but succeeded on namespace-owned tmpfs; ordinary NOSUID controls
  succeeded on both. This supports a filesystem/user-namespace permission
  boundary, not a demonstrated persistent-storage configuration remedy.
- Three independent reviews covered runtime ownership, teardown/tests and
  version/CI changes. Review found an orphaned waiter on SIGINT; cancellation
  now cleans up before returning 130/143 and received an additional review.
  This was a targeted review, not a whole-tree release audit.

## Evidence for the remaining runtime problem

Stock Podman 6.1.4, commit `464c15fec06f0074c916f9eff87c4806869c2833`,
reproduced contention without Pinfold or Switchyard. Native overlay reported
`Supports shifting: false`. Actual POSIX records, descriptor device/inode
identities and ancestry linked the listing waiter to a storage writer with
a `storage-chown-by-maps` descendant. The writer held the image/layer locks.

| Synthetic image/run | Writer seconds | Listing seconds |
|---|---:|---:|
| Base, cold | 73.069 | 30.238 |
| Base, warm | 0.149 | 0.093 |
| Derived, cold | 85.718 | 85.615 |
| Derived, warm | 0.234 | 0.155 |

All writers and ownership checks passed. The cold base listing came from
the corrected all-thread observer; the initial observer missed the helper.
These are single synthetic trials on a shared host, not throughput estimates.
No fork, FUSE change, security weakening, concurrency reduction, deadline
increase or upstream filing was adopted. Disposable stores were removed
after reference/mount checks; active stores and unrelated services were preserved.

## Final implementation and checks

Implementation: `d27e75707dcffe5c3b2c9012095156fb17eb3b99`.
One shared inventory helper buffers output, bounds waiting at 20 seconds,
and kills/reaps its owned runtime client on failure. Successful owner
teardown acknowledgement avoids another inventory; recovery checks remain.
The unreaped leader protects cleanup against PID reuse. Runtime floors use
existing preflight queries. No dependency or runtime service was added.

Guarantee 9's existing real-runtime test covers timeout, empty output,
waiter removal, SIGINT/SIGTERM, preserved ownership and successful recovery.
Expected exits come from the spec; waiter identities come from the kernel.
The test names its sabotage and retains positive controls; no test seam was added.

- Final Yard private-store probe: timeout exit 1 in 20.004 seconds with
  `runtime-timeout`; SIGINT 130; SIGTERM 143. Every case had empty stdout
  and no remaining runtime waiter. This probe contained no live box;
  preservation evidence comes from the earlier cold probe and lifecycle test.
- Full suites: Mac 33 passed/1 existing ignored, 254.82 seconds; Linux x64
  33/1, 296.90 seconds; Linux arm64 33/1, 249.61 seconds. Durable recovery
  gates passed on all three. x64 is close to the five-minute suite budget.
- Formatting, strict all-target Clippy, shell checks and Linux musl build passed.
  Initial Mac checks hit an unwritable Zig cache; writable caches corrected
  the environment without changing assertions.
- [Hosted validation](https://github.com/adamaltmejd/pinfold/actions/runs/38534580320)
  passed on `1b6465b85a5719f5036b56b984ac79ca16ff9e12`, whose production
  and test files are unchanged by this documentation consolidation.

## Retained evidence

Full reports and probe source are preserved locally on
`codex/issue-89-investigation-evidence` at commit
`1b6465b85a5719f5036b56b984ac79ca16ff9e12`, under `docs/archive/`.
That branch is not part of PR #90. Raw evidence remains on `yard-sthlm`:

- `/home/yard/validation/pinfold-89-lxc-20261012/`: original report,
  candidate identities/manifests, `candidate-probe-1/`, stock comparisons
  and `stock-minimal-20261013/`.
- `/home/yard/validation/pinfold-89-idmap-20261014/`: syscall controls.
- `/home/yard/validation/pinfold89-signal-review-20261014/` and
  `/home/yard/validation/pinfold89-signal-final-20261014/`: cancellation
  regression and corrected probe; final binary SHA-256
  `bfbed9a7cc80ad9366921fedcc014c3540282807b5c6855562721cc156001b97`.

Checksum-verified evidence exports in `/private/tmp/` (temporary copies):

| Archive | SHA-256 |
|---|---|
| `pinfold89-stock-minimal-evidence-20261013.tar.gz` | `1711411991eb13c6ab0b9708d10340c84f520fbf267ab1c2bc90a7d03bf69b3e` |
| `pinfold89-idmap-evidence-20261014.tar.gz` | `a87eb679ef86ca17c59233d8aaca38df4af5409187dbb4c540f698dc464a3cff` |
| `pinfold89-final-signal-evidence.tar.gz` | `fba984c0dabf5449973b3c829c027a9f63d2f18e056a23141f3891d75d5ed530` |
