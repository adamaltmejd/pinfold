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

Original probe cohort, before cache-mount removal. Bytes. Mac is runtime variant size; Linux OCI is index.json plus admitted blobs, root is
completed raw length. These are not unique storage costs. Every row uses Pi 1.5.2, Claude stable 2.1.311 and Codex 0.169.0.

| Profile | Mac variant | Linux OCI | Linux root | Linux s | Converter allocated peak |
|---|---:|---:|---:|---:|---:|
| default | 470400916 | 462738711 | 1262822400 | 150.138 | 5051985920 |
| documents | 541090658 | 533171087 | 1428290560 | 174.487 | 5724315648 |
| full | 557764880 | 552973666 | 1487130624 | 178.668 | 5958373376 |

Mac builds took 38.269/44.216/46.978 s. Linux full/project passed in 279.037 s, with converter peak 5959266304. Individual timings, not
benchmarks. `measure.py` samples backing-file st_blocks without reading guest disks.

Selected whole-GiB bounds: work 7, raw root 2, OCI 1, selected bases 1. All recipes built under 6/2/1/1. The unchanged 2049 MiB root fixture
exhausted 6 GiB scratch before its guard; 7 GiB reached `resource: converted root size`. The OCI fixture has two 576 MiB random layers,
overwriting one file; each blob fits but their aggregate does not. Two separately admitted 634022340-byte bases exceed the combined input
limit. All guards preserved refs and passed corrected smaller build/launch controls. The root control was 704655360 bytes; OCI control
payload 701151536; derived one-base control payload 634022772. Caller context 512 MiB, artifact downloads 512 MiB, cache 8 GiB, deadlines
and JSON/object limits remain unchanged. Snapshot overhead is documented.

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
ffgrep/fffind. Source patch SHA-256 matches on both hosts: 0ea32124f638add74124b38259d237ee5def3a2ac0b3b0fe1447a5f4a2405f50.

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
profile-default, profile-full, project-project-d7f66ec20ade, image-probe-pr2-caller and image-probe-pr2-egress under localhost/pinfold.
Earlier own default/documents/failure and obsolete full/project revisions were retired under actual catalog/image locks. The original
profile-probe-linux294 identity 7a76df9a… remains unchanged. Earlier Linux cases finished by 22:36 UTC. The follow-up finished by 22:50 UTC,
before the 00:28 cutoff; its lease remains active. No case was omitted for time or capacity.

Caller changes: larger built-ins and permission/updater defaults; fresh managed RUN, profile-first builds and stderr publication report;
managed no-cache flag removal; 7/2/1/1 Linux bounds and two build hosts. No JSON key or box-schema change. Mounted launches remain. Commits:
15da4ca, 5d688ab, a61c787, 85981a1; 2359a04 started the record; 0f0badc adds collectors and 7af6811 records their results. This follow-up
drops download reuse by operator decision.
