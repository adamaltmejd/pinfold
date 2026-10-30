# Linux builder validation, 2026-10-30

This is the compact evidence record for PR252's reviewed runtime
4f87c982bbf0c1a13f29a22f0e3278774d32fd2f. Historical observations retain
their actual source/binary/bootstrap scope. Complete originals, raw bundle
identities and recovery instructions are in [provenance](2026-10-30-builder-provenance.md);
none is replaced by an invented current-head result.

## Final checks and decisions

| Observation | Result and boundary |
| --- | --- |
| Exact-source reviews | Independent correctness/security and lean/spec/test reviews accepted 4f87. Format and locked all-target Clippy passed. |
| Mac c6cc1be | 24 passed,0 failed,1 ignored;83.42s body,84.739844667s wrapper. Later fixture/profile/guard changes are Linux-only; do not describe this as a Mac execution of 4f87. |
| [Ordinary Ubuntu41057246696](https://github.com/adamaltmejd/pinfold/actions/runs/41057246696), job124067777615 | Exact4f87:26 passed,0 failed,1 ignored;306.57s body;309.900624968s unchanged budget clock,e2e_exit0. Format/lint/release AppArmor passed. CI failed 300s by 9.900624968s. Adam explicitly admitted this exact result once. |
| Earlier ordinary Ubuntu41030723453 | Exact4f87:26/0/1;316.39s body,319.768852424s wrapper. Budget failed; bootstrap41030723468 passed. Both failures remain recorded. |
| [Diagnostic41050343930](https://github.com/adamaltmejd/pinfold/actions/runs/41050343930), source3cf608b | Unchanged runtime behavior:26/0/1;291.11s body;294.130789542s budget clock versus294.150628605s observer envelope. Instrumented timing is not normal-budget admission or causal improvement. |

The diagnostic passed readiness, native framing, loss and cleanup checks.
It retained 9366 sanitized rows/1418015 bytes; no recorded parser gaps,
overflow or identity collision. There were 589 excluded unknown-ancestry
events; attribution remains incomplete and every record is AMBIGUOUS.
Raw traces stayed private. Overlapping process lifetimes cannot be summed
or called boot cost, lock wait or guest CPU. Earlier observer attempts
1433a06/e12d596 refused before tests and removed their owned instances.

Final ordinary logs/result/metadata are retained under the operator root
.local-evidence/final-signoff-2026-10-30/. Log SHA-256:
2aa508464f29b38952de273dfacb9cf36c6cc8d8b39f6336bccef46cbcbb79be.
Diagnostic log/result/timeline are under
.local-evidence/ubuntu-timing-observer-2026-10-30/. No further optimization
or repeat-until-green run was adopted.

## Current coverage, including deliberate reductions

Adam approved five critical G23/G15 build cases: three caller builds A/B/F
for success, change and failure, and two cleanup builds. Those are five
cases in that pair of guarantees, not five builds in the entire suite.
The dedicated ONBUILD/composition/repeat/no-cache/profile/color and
retention/sibling/shared-tag matrices were coverage cuts, not duplicates
or removed product behavior. Postcommit-EIO and actual live-root lifetime
observations remain. Apple foreign-resource checks use a separate fixture.

G44 keeps four real cancellation, exhaustion, recovery and completed-image
launch cases. Its initial Debian base contains only needed shell/text/HTTPS
tools. G27 uses a genuine scratch readiness image and retains copied-binary
AppArmor refusal plus profiled READY. G17 reuses the managed tag with private
configuration. Its obsolete exclusive guard was removed only after its
dedicated build/cleanup disappeared. Other maintenance/dead-owner/disk guards
remain. The earlier G23 shared-lock downgrade was unsafe and retracted.
G12's actual tools checks were not replaced with minimal fixtures.

Linux E2E uses Cargo's ordinary release musl binary and the same release
path for AppArmor. Product prebuild was already outside the timer; harness
compilation/helper query remain inside. Mac and separate slow-proxy selection
are unchanged. No mock, unit, runtime stub or binary test flag was added.
The final fixture reductions leave13 Linux source build invocations.

## Historical failure, recovery and positive controls

Common c4bcd4d binary:
fdefe37d46cade9d2e4b85c55b2f48ba63b0ad0e94cbd84fcd68090779e602b7.
Selective-prune recovery binary:
b529085013cb25ab86cb06ea7dfc98578f3746eb91ceaf370fee9173a1cb698a.
These and the093 bootstrap identify historical observations, not4f87 runs.
Each linked original preserves exact fixtures, receipts and finer hashes.

| Boundary or regression | Retained observation and limitation |
| --- | --- |
| [Native resolver](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-28-builder-helper-api-proof.md) | Component aliases, ARG/COPY, scratch/unreachable and zero-layer/layered controls bypassed VMM/jail/proxy/claims. Later real CLI config-only input launched expected ENV; ONBUILD parent succeeded and actual child RUN false failed. A partial local-zero driver failed its aggregate assertion; continuation is separate evidence. |
| [Inherited ONBUILD conversion](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-28-builder-metadata-boundary-results.md) | Pre-fix FROM-only conversion executed inherited triggers. Private converter configuration was corrected while published triggers stayed intact. Apple native omission was observed, not repaired with a parser or generalized to all Apple builds. This historical scenario is no longer dedicated suite coverage. |
| [Metadata fidelity](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-28-current-root-metadata.md) | GNU tar --acls overwrote raw ACL xattrs when textual ACLs were absent; removing that option corrected the component path. Reproduced 093 component checked nine invariants plus hardlink/decoded capability on a 114542592-byte root. It bypassed production admission/jail/claims. Ordinary BuildKit RUN export had already dropped ACL/user attrs: this is not general end-to-end ACL support. Early lost artifacts/failed trials remain unavailable/failed. |
| [Selected input and context size](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-28-builder-metadata-checkpoint.md) | Unrelated633240702-byte stored OCI once broke tiny builds; selection fixed that regression. Over-limit caller context refused without reference changes; smaller control succeeded. Empty null layer lists are decoded empty while graph/count checks remain. |
| [Publication durability](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-28-builder-runtime-receipts.md) | One post-rename directory-sync EIO retained successful CLI/ref/launch. Not crash durability or cancellation-order proof. Earlier supervisor_errno5 failed even with child exit0. |
| [Recipe failure/cache reuse](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-28-cache-failure-recovery.md) | RUN false originally lost a cached random-stamp layer. Before/after controls established the clean-cache fix; later restoration-fallback faults were source-reviewed, not injected by that trial. |
| [SIGINT, owner death, audit failure](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-builder-lifecycle-receipts.md) | Initial SIGINT hung 60s because Interrupted was retried; correction yielded terminal cancelled/dead pidfd/unchanged refs and same-cache recovery. Owner SIGKILL and audit-I/O failure also recovered. Not PID1 reaping, late-after-commit cancellation or selected-handoff midpoint proof. |
| [Selected-base lease](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-builder-lifetime-and-maintenance.md) | Removing a selected base's last reference during a build did not remove the leased data; cancellation released it. Not the acquisition race, clean/publication race or a real old-store preservation trial. |
| [Interrupted output](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-builder-output-truncation.md) | Real received-index fsync barrier, owned VMM kill, incomplete output/no refs, identical ordinary success control. Exact EOF offset is unknown. Not selected-input handoff evidence. |
| [Resource rejection/control](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-builder-resource-boundaries.md) | Historical 513MiB input, oversized converted-root and FIFO deadline refusals had smaller successful controls and unchanged references. Role deadline is not a whole-command deadline. |
| [JSON and payload](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-builder-json-and-payload-receipts.md) | 20 large labels refused/12 succeeded; two 300MiB payloads refused/two 50MiB succeeded, refs preserved. Not header, wrong-hash or object-count runtime proof. |
| [Cache exhaustion](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-full-cache-recovery.md) | Real ENOSPC recovery first failed; a zero-available predicate also failed. Selective native mutable-cache prune passed recovery and then-retained layer controls, without arbitrary free-space thresholds. Later G23 retention coverage was narrowed. |
| [Installed-bootstrap integrity](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-builder-verification-ref-map-supplement.md) | Same-size corruption of the installed093 bootstrap raw root refused on digest; restoration followed by an ordinary build succeeded. Seven retained byte-identical raw ref maps close an earlier map-retention gap; do not rewrite the earlier run's limitation or call this completed-image launch coverage. |
| Later completed-image integrity | A separate d6c operator trial corrupted the task-owned completed image root: box launch failed with image-output/digest mismatch, restoring the byte and independently checked hash restored the same marker launch; catalog bytes stayed equal. The first observer expected the wrong event and restored in finally; corrected integrity-only continuation completed without repeating the three builds. The preserved operator note and receipts are indexed in provenance. This is distinct from the093 bootstrap-build control above. |
| [Public installation](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-public-builder-installation.md) | Historical 093 fresh public download verified; transport denial and offline corruption refused; restoration succeeded. Not malicious-server proof or a fresh1e9 runtime capture. G44's real cache obstruction keeps builder-independent launch meaningful. |

Earlier Linux 309.25s/supervisor_errno5 and340.52s/cache-loss failures, later
Mac ENOSPC 21-pass/3-failure results, and earlier missing-artifact Linux cascades
remain historical failures. They are not replaced by later successful runs.
No old store, managed disk or persistent volume was deleted to obtain passes.

## Capacity and latency limits

Older documents/full roots were392686592/451499008 bytes. The latter is
81.417MiB nominally below512MiB; that is emitted-root arithmetic, not measured
filesystem free space. Roots were streamed/hash-checked, not mounted or
launched in that measurement. Binary association used compile order/behavior,
not an invocation-time hash. Child CPU deltas and cumulative max single-child
RSS are not aggregate/per-phase memory peaks; du after completion is not
per-build/peak storage. A later roughly 3.006GB charged full-profile observation
still does not establish arbitrary-project or concurrent capacity.

The 4100-object attempt reached the deadline before the object guard;
its 50-object control succeeded. Header 8 KiB, wrong hash and malformed
graph/media/URL/attachment rejection have source review without malicious-
producer runtime injection. Peak guest occupancy/inodes and concurrent
capacity margins are unestablished.

The optional final-head full-profile trial never built: normal-user KVM and
compiler prerequisites were missing; the old root-sibling accounting wrapper
would bypass a finite ancestor PID bound and inherit different sudo limits.
No repairs/limit changes/retry occurred; the disposable VM was released idle.
Approval accepts the numerical policy with these disclosed evidence limits.

Prior actual default cached CLI samples: Mac 1.51s, trial Linux 8.416s.
Mac first fresh-XDG 14.75s used Apple image/layer cache. Small Linux caller
14.25s scoped cold/4.83s repeat excludes preseeded bootstrap download.
Real profile UUID first=repeat and --no-cache changed it; project reuse is
shared-source reasoning, not a separate manual observation. Fully cold
universal latency and a few-second default Linux build remain unproved.

## Unresolved follow-up

The [caller label-output ticket](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-29-caller-label-output-bug.md)
remains open: accepted OCI retained all 12 ordinary caller labels, but CLI
image output exposed only four host metadata labels. Linux reproduced it;
the shared Apple path was source-observed, not runtime reproduced.
Compaction neither fixes nor silently drops this ticket.
