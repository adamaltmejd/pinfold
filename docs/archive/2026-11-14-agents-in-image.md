# Standalone agents and fresh managed builds: PR 2

2026-11-14. Ticket #278. Starting source `837e16a`. The operator dropped download reuse; follow-up verification passed. No test, dependency,
release, push or PR. Evidence: [manual collectors and selected output](2026-11-14-agents-in-image/).

## Operator decision

Identity freezing is a Linux mechanism. Apple records `dev.pinfold.base` but its native builder resolves the profile tag, as before this PR.
No host-wide lock or extra Apple source binding is added. A concurrent rebuild uses that user's profile image; a changed profile digest is
already reported as `image-outdated` at launch. The spec states the platform distinction.

## Implementation

- All built-ins contain Pi/assets, native Claude and the complete same-release
  Codex package on PATH; procps is added and hardening stays last.
- Stable metadata is fresh; publisher SHA-256 verifies version-addressed assets.
  Missing assets fail. A receipt records resolution time, versions and downloads.
- Missing-only seeds hold Pi trust/settings, Claude permission/sandbox and Codex
  approval/sandbox/version-check/daemon-updater defaults. Pi/Claude switches
  are in recipe environment. Old mounts, pins, auth files/helper remain.
- Managed RUN reruns; projects rebuild profiles first and report publication.
  Linux binds the result. Managed no-cache is removed; caller caching/flag
  and Context::Alone remain. README/spec describe these changes.
- Separate Linux budgets cover inputs, conversion, transfer, publication/reuse.
  Aggregate preflight before READY retains independent streaming/host checks.
  Build hosts add only api.github.com/downloads.claude.ai; agent hosts stay.

## Measurements and bounds

Fresh read-only size queries are retained with exact image identities in the results files. Mac is runtime variant size; Linux OCI is
index.json plus all admitted blobs, and Linux root length matches the host file. These are not unique storage costs.

| Profile | Mac variant | Linux OCI | Linux root |
|---|---:|---:|---:|
| default | 470400578 | 462738281 | 1262831616 |
| documents | 541090658 | 533170725 | 1428272128 |
| full | 557764880 | 552973697 | 1487130624 |

The separate original Linux build cohort is supported by retained catalog snapshots and sampler output. Peaks are the largest st_blocks ×
512 sample every 0.1 s, not guest used-space measurements. Build times exclude launch.

| Profile | Original catalog root | Build s | Sampled allocated peak |
|---|---:|---:|---:|
| default | 1262822400 | 150.138 | 5051985920 |
| documents | 1428290560 | 174.487 | 5724315648 |
| full | 1487130624 | 178.668 | 5958373376 |

Mac original build times were 38.269/44.216/46.978 s for default/documents/full. Original Mac size and Linux OCI outputs were not retained;
their historical values are replaced by fresh queries above. Retained project output shows 279.037 s for its profile-plus-project build and
a maximum sample of 5959266304 across both conversions. The documents remeasurement built in 170.722 s, sampled 5725097984, and launched;
its first attempt ran out of host disk, then passed unchanged after own-caller cleanup.

Fixed GiB bounds: work 7, root 2, OCI 1, selected bases 1; 7 is not proven minimal. Six-GiB exhaustion was observed during development; the
retained 76.892 s tar ENOSPC excerpt does not log the work-disk size, so that configuration is not retained proof and was not rerun. Seven
GiB reached `resource: converted root size` for the 2049 MiB fixture. Two 576 MiB random layers and two 634022340-byte local bases reached
their respective aggregate guards. All three guards preserved refs and passed corrected controls. Context/artifact downloads remain 512 MiB,
cache 8 GiB; JSON/object limits and deadlines remain unchanged.

## Commands and outcomes

Use `PROBE_ROOT=/tmp/pfi.pr2mac` or `/tmp/pfi.pr2linux`: isolated config/state, a fresh maintenance stamp, real cache and
PINFOLD_NO_UPDATE_CHECK=1. Mac checks `df -g /` before each build; stop below 12 GiB. Linux checks `/` and 2 GiB tmpfs `/tmp`, uses only
`labvm exec -- bash -lc '…'`, setsid/nohup for long jobs, 2 CPUs and 1 GiB boxes. Credentials/homes/fixtures are synthetic; no real login
was read, and no lease acquired/released.

| Command/case | Mac | Linux |
|---|---|---|
| `cargo build --locked` | Passed, embedded ARM musl init | Musl target passed; no ELF INTERP |
| `cargo fmt --check`; `git diff --check` | Passed | Passed |
| `cargo clippy --all-targets --locked -- -D warnings` | Passed | Musl target passed |
| `cargo zigbuild --all-targets --locked --target aarch64-unknown-linux-musl` | Full ARM musl cross-build passed | Not run on VM |
| `probe.py build default/documents/full` | All passed | All passed |
| `agents.py default/documents/full` | Nine tool exchanges passed | Nine passed |
| Codex companion; binary writes | Real companion, fallback disabled; writes refused | Same, passed |
| `probe.py start`, each profile | Pi UI, Codex sign-in; Claude connectivity | Same, passed |
| Existing mounted `--harness` | Pi 1.5.2, Claude 2.1.322, Codex 0.167.1 | All three/profile passed |
| `probe.py home default` | Four edited seeds preserved | Four preserved |
| `probe.py project` | Full first, project built/launched | Same; recorded base identity matches |
| `probe.py fail` or split failure modes | Intended failures, refs preserved, corrections passed | Both intended failures, preserved refs and corrected launches passed |
| `probe.py caller` | Cached timestamps/id equal; no-cache changes both | Cached timestamp/id equal; no-cache changed both |
| `limits.py root/oci/bases` | Linux-only | All guards, refs and corrected controls passed |
| `limits.py cache` | Shared Mac cache untouched | Exact idle cache removed under actual lock; refs unchanged; full launched |
| `probe.py egress` | Linux-only | 403/reason/no publication; admitted build passed |
| `probe.py pi` | Actual project entrypoint version passed | 1.5.2 through actual entrypoint |
| `probe.py updater full` | Six minutes completed | 360.024 s completed; same initial/analytics pattern |
| Post-decision `build default; start default; agents.py default` | Build 41.581 s; all three agents started and completed fixture commands | Build 152.558 s; all three passed |

Fixture success means a valid tool response, a named guest file and a second model request containing its result, never HTTP 400. Full loads
ffgrep/fffind. Source patch SHA-256 matches on both hosts: 319a1afda7d3b20198de96c79dc4f004d67db7fb08de85fed84f2db9d04198af.

## Findings and freshness decision

The operator dropped the plan's unchanged-version download reuse case: measured uncached build time is acceptable, and repository rules
forbid unmeasured speedups. Managed builds keep the existing uncached path and download agents on every run; the inert cache mount and
cache-directory logic are removed, publisher integrity checks remain, and the BUILDKIT_SANDBOX_HOSTNAME patch is not applied.

Codex exec worked with two binaries but its TUI required the complete native package. Upstream copies it into home. TUI checks and daemon
updater are off. Six-minute onboarding on both hosts showed initial plugin hosts, then only ab.chatgpt.com analytics at 1/2/5 minutes, no
later version host. Initial TLS paths remain unobserved; plugin bootstrap uses GitHub/API/ChatGPT too
([source](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/core-plugins/src/startup_sync.rs)).

The OCI refusal initially became invalid JSON after READY; preflight fixed its diagnostic. Linux hit cache/host-disk exhaustion without
publication. Idle-cache removal and own-image retirement allowed unchanged cases to pass. Three Codex homes filled tmpfs: mounted full
failed twice, and a later VMM was OOM-killed (kernel log). Removing only those homes made the same full mounted case pass. No product
packing or capacity case was weakened. Mac reached its disk threshold; home cleanup/storage reclamation restored room. No manual Mac cache
prune occurred. Initial cancellation published nothing.

## Retention and caller changes

Mac owned boxes/disks are removed; its images/shared builder remain. Names: profile-default/documents/full, profile-probe-pr2-failure,
project-project-1766f2850109 and image-probe-pr2-caller (localhost/pinfold). Linux owned boxes/disks are removed. It retains
profile-default, profile-documents, profile-full, project-project-d7f66ec20ade and image-probe-pr2-egress under localhost/pinfold. Earlier
own default/documents/failure and obsolete full/project revisions were retired under actual catalog/image locks. The original
profile-probe-linux294 identity 7a76df9a… remains unchanged. Earlier Linux cases finished by 22:36 UTC. The follow-up finished by 22:50 UTC,
before the 00:28 cutoff; review checks and the documents remeasurement also finished before that cutoff. The owned caller fixture was
removed for space; boxes/disks are cleaned and the lease remains active.

Review restored the separate catalog manifest/config and selected-config caps to 512 MiB: measured catalog metadata is 44864 bytes, so no
case justifies 1 GiB. Shared root/OCI constants now compile once per binary at its consumer's platform boundary. Guarantee 23 and README
state download-on-every-build and the profile publication warning; no other managed download-reuse claim remains.

Caller changes: larger built-ins and permission/updater defaults; fresh managed RUN, profile-first builds and stderr publication report;
managed no-cache flag removal; 7/2/1/1 Linux bounds and two build hosts. No JSON key or box-schema change. Mounted launches remain. Commits:
15da4ca, 5d688ab, a61c787, 85981a1; 2359a04 started the record; 0f0badc adds collectors and 7af6811 records their results. This follow-up
drops download reuse by operator decision. Review source fixes are 940313d; metadata remains bounded at the original 512 MiB.
