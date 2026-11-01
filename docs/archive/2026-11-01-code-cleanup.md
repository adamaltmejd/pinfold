# Release 0.4.1 audit

Baseline: main `50fed703e68747a5402118dc167b8c412581468b`.
Independent whole-file passes covered CLI/update, box/common helpers, all runtime
adapters, proxy/TLS, images/clean/artifacts, config/dirs/trust/init, pi, E2E
helpers and every test, scripts/workflows, builder sources, profiles and docs.

The image pass removed two obsolete trial/admission-status comments in
`425f989`. All numerical limits remain unchanged. No assertion, control,
dependency or feature was removed. No new written ticket was needed.

Rejected: sharing init's hex encoder with `core` ignores the standalone init
crate boundary; removing the small builder-connect wrapper exposes a lower
level helper without a useful consolidation. Keep the recent Podman migration
instructions, which remain actionable for users upgrading from 0.3.

Direct dependencies match ARCHITECTURE. Format, locked all-target Clippy with
warnings denied and dead-code lint passed. No actionable TODO/FIXME was found.
Existing upstream-auth and Git revisit conditions do not justify a change.

Configuration, CLI and guarantee trace tables were checked against current
source. All 28 guarantee rows map to existing tests or documented gates:
34 is ignored and 36 uses the separate slow-proxy proof. Existing direct
scenario gaps include attach/push, --no-cache, --check, --tty and short aliases;
they do not justify argument-shape tests. All assertion blocks were retained.
README, profiles and built CLI help agree with the contract. Pi help passes
through to its harness and was excluded from the CLI-help check. No open
question is resolved by this pass. No unmeasured optimization was admitted.

| Metric | Before audit | After audit |
| --- | ---: | ---: |
| Rust lines | 27,783 | 27,781 |
| Largest source file | box_.rs: 4,496 | box_.rs: 4,496 |
| Largest product source | image/linux/job.rs: 1,546 | image/linux/job.rs: 1,545 |
| ARCHITECTURE lines | 1,423 | 1,423 |
| Possible / landed cleanup | 0 | 2 / 2 lines deleted |
| New dependencies / written tickets | 0 / 0 | 0 / 0 |

Before this report and release notes, the release delta from v0.4.0 is
32 additions / 18 deletions (delete/add ratio 0.5625); crates: 9 / 14 (1.556).
The normal pin refresh moved Claude 2.1.314 to 2.1.316 and the Public Suffix
List to 2026-10-31. Rust, Cargo dependencies and other tool pins are unchanged.

The full real Mac suite passed 24/0/1 ignored in 89.42 seconds, including
live login. Native Mac release build passed. Final Linux CI, its unchanged
300-second budget, release-build results and publication receipts belong
to the release PR and workflow. The earlier hostname-fix CI passed all 26
Linux tests in 296.983396652 seconds; it does not substitute for the final gate.
