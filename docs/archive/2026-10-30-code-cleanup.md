# Release 0.4.0 cleanup and spec audit, 2026-10-30

Baseline: merged main `d71efe95e76a3849752eaeb063f6cef35d5684a7`.
Reviewed implementation and validation candidate: `972229be2e2a90343ebb145f99e2ab8e246d1806`.
The final evidence/notes commit is documentation only. No tag or release is authorized by this report.

## Scope and findings

Independent module passes read the whole CLI, box/common helpers, both runtime adapters and VMM jail, proxy/TLS, image/catalog/jobs/clean/artifacts, config/dirs/trust/init, pi, E2E helpers and every test file. Additional passes covered producer/helper sources, kernel builder, scripts/workflows, profiles, README and ARCHITECTURE. Public keys/package pins were treated as data. The test audit retained all assertion blocks and controls.

| Disposition | Result |
| --- | --- |
| Image module | `a54a44a`: reuse catalog resolution under its existing lock before acquiring the image lease; reuse existing absent-file handling during claimed cache purge. Removes two duplicate paths, net −10 lines. |
| Init module | `91b5f40`: remove one obsolete candidate-policy comment; retain its 512 MiB cap. Net −1 line. |
| README | `972229b`: remove one internal builder-mechanism restatement. Retain actionable Podman migration instructions. Net −1 line. |
| Pins/version | `e931199`: workspace and lockfile version 0.4.0; eligible Bun/Claude/Codex/PSL updates. No Rust, Cargo dependency, Firecracker, kernel or bootstrap pin change. |
| Test audit | No assertion changes, deletions, new tests or test-only paths. G28 is already included through cli.rs's nested module. |
| Spec/control questions | None. No control or feature removal proposed. |
| Written tickets opened by this pass | 1: graceful Apple box shutdown, after the upstream revisit condition fired. The existing caller-label-output ticket remains open. |

Rejected cuts: keep the named probe constructor; its tiny infallible wrapper is insufficient reason for a release edit. Keep exact unknown-key diagnostics, both lifecycle/ownership rechecks, explicit Git fsck, host-controlled transfer cancellation/bounds, TLS byte retention and tunnel deadlines. Capability deduplication would rework shared security code. Keep the secret scan's coverage checks, which prevent a vacuous observer. No unmeasured speedup was admitted.

## Mechanical and spec pass

Direct normal dependencies exactly match ARCHITECTURE: tokio, httparse, serde, serde_json, sha2, toml, nix, ring, rustls and rustls-native-certs. Format, locked all-target Clippy with warnings denied, and dead-code lint pass. No actionable TODO/FIXME was found; two revisit comments remain conditional on upstream auth API/Git support and no cut was admitted without evidence that a trigger has fired.

The config-key, CLI and guarantee trace tables were reconciled against source. All seven TOML keys and PINFOLD_ENV_* have scenario coverage through G18/G39. Built help matches current syntax. Every guarantee maps to a test or its explicit separate/ignored gate; no row is deleted. G34 remains intentionally ignored, G36 is the separate slow-proxy gate, and G43/G44 are Linux only. No new assertion of help text, argv shape or file layout was added.

Existing scenario gaps include explicit attach/push, --no-cache invocation, box exec --tty, update --check and interactive notices. They do not justify detector tests or invent new guarantees. No README contract drift was found. No open question is resolved by present evidence. The two future-work lines for the now-fixed apple/container#1941 move from ARCHITECTURE to the [written follow-up](2026-10-30-apple-graceful-shutdown-ticket.md); current process supervision is unchanged. No graceful-shutdown claim is added. Prior accepted resource and handoff evidence limits remain explicit in the builder records.

## Metrics

Measured at the reviewed candidate before adding this report and release notes:

| Metric | Before pass (d71) | After pass (972) |
| --- | ---: | ---: |
| All Rust lines | 27,793 | 27,782 |
| Largest Rust file | box_.rs: 4,496 | box_.rs: 4,496 |
| Largest product Rust file | image/linux/job.rs: 1,545 | image/linux/job.rs: 1,545 |
| ARCHITECTURE lines | 1,424 | 1,424 at tested 972; 1,422 after the documentation pass |
| Possible / landed cleanup | — | −12 / −12 lines |
| New dependencies / tickets | 0 / 0 | 0 / 1 |

Since v0.3.3: all tracked changes 14,740 additions / 1,931 deletions (delete/add ratio 0.131); crates 6,034 additions / 1,760 deletions (0.292). The inherited refactor and retained evidence dominate those totals. The release pass itself changes 9 files, 33 additions / 45 deletions, net −12. Evidence/notes are accounted separately from these implementation measurements.

## Validation and publication hold

The full Mac suite passed 24/0/1 ignored in 81.73 seconds, including the existing self-update and live-login cases. A prior attempt failed before tests because the runner path was relative. Another run lost its checkout externally and failed 23/1/1 with getcwd errors; neither is counted as a pass. The dedicated recovered release checkout remains retained.

Native Mac release build and Linux x64 musl cross-build pass. Linux CI independently builds the native release musl CLI before its unchanged AppArmor and E2E checks. Local build hashes are retained as local build receipts; they are not hashes of CI's unuploaded native binary or promised future publication bytes.

[Linux CI 41128517067](https://github.com/adamaltmejd/pinfold/actions/runs/41128517067), job 124313146903: 26 passed, 0 failed, 1 ignored. Test body 298.01 seconds; unchanged budget clock 302.079920692 seconds; e2e_exit=0. Overall CI fails only the 300-second gate by 2.079920692 seconds. Native release build, restriction=1 AppArmor setup, format and lint pass. This failure stands; no retry or new exception is assumed.
[Slow proxy proof 41128520013](https://github.com/adamaltmejd/pinfold/actions/runs/41128520013), job 124313155521: PASS. The ordinary separate debug CLI and matching AppArmor profile exercise real CONNECT traffic and the production idle deadline. This is separate from the ordinary suite budget.

The previous one-time 309.900624968-second exception was bound to 4f87/37ac; it does not admit this changed release candidate. CI failures remain failures. No retries, budget weakening, control changes or diagnostic substitution are authorized by this report.

The publishing workflow is deliberately not dispatched: it can publish automatically. No version tag, GitHub release, release upload or main-branch merge has been performed during preparation. Final publication requires Adam's approval of the candidate, notes and any unresolved gate.
