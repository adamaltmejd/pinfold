# Disposable prototype B source

Only `/private/tmp/pd-1015/src` changed. Main session owns builds and experiments.

New ordinary scratch CLI operations:

```
pinfold box inspect-run RUN_ID --limit N
pinfold box check-run RUN_ID HOST PORT CONNECT|HTTP
```

`N` is 1..100. Inspection reads at most 64 KiB of snapshot, terminal record,
and egress independently. Egress is oldest-first and explicitly truncated.
Queries skip maintenance and runtime calls. An unavailable snapshot is a JSON
result, exit 1. Invalid arguments remain normal CLI errors.

At ready, core saves selected resolved facts under state/runs/RUN_ID.json:
name, generation, timestamp, runtime name, image id, allow entries, route names,
and requested CPU/memory. No env values, headers, prefixes, arbitrary failure
details, command arguments, or route targets are captured. Egress filename uses
the generation. Existing egress contents and fail-closed audit behavior remain.
The ready stream adds run_id; normal ownership, stdin and exec channels remain.

Terminal records are separate from the immutable ready snapshot. CLI hold records
the observed shutdown reason after teardown; Pi records only pi-ended,
pi-exec-failed, or teardown-failed. SIGKILL leaves terminal not-observed. Inspection
always labels current liveness not-probed, so an old ready snapshot is not a
claim the box remains alive.

Static authority checks use one helper called by both proxy enforcement and
check-run. Existing CONNECT/plain HTTP order and reasons are preserved. A positive
result means passes-static only. Request framing, DNS and SNI are unevaluated;
delivery is not tested. There is no request-path check or traffic attribution.

Limitations deliberately retained: no startup snapshots, config provenance,
full effective runtime-control manifest, DNS probing, live subscription,
cursor/pagination, bounded retention policy, source hashes, first-class generation
on old down/stat output, or structured upstream failure causes. New snapshots are
best-effort and are not a new fail-closed control. A disk failure prints a generic
diagnostic and leaves unavailable data. Existing closed-owner-stdout behavior is
untouched. Resource values are requested values, not observed enforcement.
Malformed or missing snapshot/terminal files intentionally produce availability
states instead of guessed facts. Terminal unreadable and absent are conflated in
this prototype. Path validation prevents traversal; production durability and
host-file permissions/symlink protection need separate design review.

No tests, builds or runtime runs were executed by this subagent. `cargo fmt --all`
was run. The parent should build and exercise real runtime success, refused
network requests, changed configuration, name reuse, missing terminal and bounded
queries before drawing empirical conclusions.
