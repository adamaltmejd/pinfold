# Failed cache-mount recovery

Date: 2026-10-29. This fixes the normal CLI reproduction retained in
`exhaustion-first-failure.tar.gz`, SHA-256
`1815c6cb5e4ecd22bf546fb9827a09773cff3b2bf9a98a48dac58199293772b8`.
Filling the finite mutable cache failed with ENOSPC but left it marked
clean. A following plain COPY build then failed too, with public refs
unchanged. The earlier converter-space failure and its smaller control
were separate successful boundary observations, not proof of recovery
from persistent-cache exhaustion.

The first fix tried withholding a clean end only when guest statvfs
reported zero available blocks or inodes after daemon teardown. G44 failed
in 142.91 s: the real RUN reported ENOSPC but the predicate did not trigger.
Raw post-stop counters were not retained. This does not establish an exact
remaining-space quantity or that space changed at a particular instant.
No broader threshold was substituted. Source and executed binary are in
`full-cache-zero-failure.tar.gz`, SHA-256
`50c903c110fa96e4e2673cddc3177f893d7d9bc333cf1cb41114cb66027236f1`.

The final change uses the pinned BuildKit CLI's existing selective prune
operation after failed discovery/build work. Its exact `exec.cachemount`
filter selects unused mutable cache-mount records, leaving regular
image-layer records outside that selection. The original build diagnostic
remains separate from cleanup progress. Daemon teardown still follows;
cleanup failure takes the existing bare-F failure path and leaves the
host marker unclean for replacement by the next build.

Successful work and conversion use their existing paths. Cache retention
still requires the complete shutdown/end/EOF receipt, VMM reaping and host
backing-file sync. No host mount or read of a guest-writable filesystem,
new wire field, dependency, entitlement, user option or limit is added.
Mutable cache mounts are best-effort data and may now be discarded after
failed work. The candidate numerical allocations remain unapproved.

The existing G44 test adds a bounded normal workload compiled with the
already-required musl toolchain. It emits its marker only on actual kernel
ENOSPC; the marker is absent from the Dockerfile command text. That avoids
matching quoted recipe prose as proof of execution. The test preserves
separate SIGINT and exhaustion recovery controls, public-ref checks, the
role/audit/egress observations and the final builder-absent launch.
It passed in **160.14 s**.

G23's existing failed RUN now creates an empty mutable-cache record on
Linux, so filtered cleanup has a real match. Its giant diagnostic and
final caller marker remain, followed by the existing unchanged-image-ID
checks. This strengthens healthy-cache retention without another build
or assertion; Mac recipe bytes are unchanged. G23 passed in **340.44 s**,
including its real post-commit EIO receipt, no-cache/profile rebuild and
caller-image scenarios. An earlier G23 invocation stopped at a missing
host Git fixture prerequisite after 62.06 s. Git was installed before the
successful run; that setup failure is not cache-retention evidence.

Both passing logs record production executable SHA-256 before and after:
`b529085013cb25ab86cb06ea7dfc98578f3746eb91ceaf370fee9173a1cb698a`.
G44 source archive is
`72af587480693b11ed1f3629cd30025593c31311e4f8c685823b342f4e770c46`.
G23 uses that production source with test file SHA-256
`6fdae5b2cd90ef3deac0323d260fa223ee305f6fff5e5008739dbecda9fb2b0a`,
also recorded in its invocation log and copied out separately.
Linux and Mac locked all-target Clippy and formatting passed. Mac runtime
validation, the full Ubuntu suite and its five-minute budget remain open;
these targeted local durations do not establish that budget.

Evidence is under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
`full-cache-prune-pass.tar.gz` retains the source archive and executable.
Its SHA-256 is
`d27ce412c380257a2c80494f0054aec9c970836d1c852bf95e2a90d289f9f358`.
The later retention test file is `full-cache-retention-box.rs`.

| Invocation log | SHA-256 |
|---|---|
| `linux-full-cache-regression-1.log` (failed predicate) | `1e015def1207f2e12f9e47a67ace84201b3bc119442ba0c77197c880ee7531fb` |
| `linux-full-cache-regression-2.log` (G44 pass) | `a26ad4487958398a81c10eab8d46f517ed4915fb73afb56f3612ae6d96b01d0e` |
| `linux-cache-retention-current-1.log` (missing Git) | `5482c451aaf405dabf855250972594213e56eb444e597b601b45450ead2d6db8` |
| `linux-cache-retention-current-2.log` (G23 pass) | `d190a917b50d711af8481a4b8b0d200409a1c52716d808424006b0ed8bada083` |

The tests do not establish every regular-layer/inode exhaustion case,
still-referenced parallel mounts or concurrent capacity. Distribution,
fresh public-cache installation, final independent review and admission
remain separate. No publication or numerical approval is implied.
