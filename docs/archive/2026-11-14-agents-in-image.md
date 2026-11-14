# Agents in the image: PR 2 draft stopped at Apple source binding

2026-11-14. Ticket #278, PR 2. Starting source `837e16a`.
This is incomplete work, not an accepted implementation or measured capacity.
The product changes remain uncommitted. No test, dependency, release or push.
Evidence: [collector and selected output](2026-11-14-agents-in-image/).

## Required decision

The plan requires rebuilding a selected profile and freezing its resulting
identity for the project build. The draft CLI does both on Linux: its
`dev.pinfold.base` value binds native source discovery to that identity.
Apple records the same label but its builder still resolves the recipe's
mutable reference. A concurrent profile publication can move `:latest`
before the project builder resolves it. The label is not source binding.
This was found by source review, not reproduced with a racing CLI run.

Apple container 1.7.1's `build --help` offers build arguments but no
named-context or source-policy binding. Its native resolver fetches the
recipe reference ([source](https://github.com/apple/container/blob/1.7.1/Sources/ContainerBuild/BuildImageResolver.swift)).
A literal rewrite cannot safely handle arbitrary existing recipes:
ARG expansion, continuations, stage shadowing, external COPY/mount sources
and heredocs need native parsing.

Proposed decision: a host-wide per-profile publication lock across the
profile rebuild and dependent project build. Every managed publisher of
that profile must use it. It prevents concurrent pinfold retagging, not
direct `container` retagging. Strict identity binding against external
retagging needs a larger native resolver/buildctl design or an approved
recipe restriction. Neither alternative was implemented. The brief says
to stop rather than improvise a different design; an operator question
was submitted. No further image build was started after this finding.

## Draft changes

- Agent recipe fragment: complete standalone Pi assets, native Claude,
  Codex and code-mode host from one resolved release, PATH links.
- Fresh stable metadata resolution, publisher SHA-256 verification,
  version-addressed cache downloads, nonsecret installation receipt.
- All built-ins include agents; hardening stays last; base adds procps.
- Missing-only Claude/Codex convenience seeds, existing Pi settings,
  recipe updater defaults. Route authentication files are unchanged.
- Managed builds rerun RUN, rebuild dependent profiles, report the
  separately published profile, and remove managed `build --no-cache`.
  Caller build cache semantics are unchanged.
- Linux root/OCI/selected-base budgets are separate from context and
  artifact downloads; converter work is enlarged. Build hosts add exactly
  `api.github.com` and `downloads.claude.ai`.
- Shared image limits live in init data so both the host and standalone
  init compile. Their target condition excludes Apple's ARM init.
- README/spec/guarantees are draft updates, including numerical bounds.
- Manual collector covers planned cases but only the runs below occurred.

## Commands and results

All pinfold runs used isolated config/state at `/tmp/pfi.pr2mac` or
`/tmp/pfi.pr2linux`, the real cache and a fresh maintenance stamp.
No login route or real credential was used. Linux used the existing lease
only through `labvm exec -- bash -lc '...'`; no acquire/release occurred.
The VM's pre-existing PR 1 dirty patch matched the local files by SHA-256
and was saved separately before the PR 2 patch was applied.

| Command/case | Result |
|---|---|
| `cargo build --locked` on Mac | Passed. Embedded ARM musl init built. |
| `cargo build --locked --target x86_64-unknown-linux-musl` on VM | Passed. `readelf -l` has no INTERP; file reports static PIE. |
| `cargo fmt --check`; `git diff --check` | Passed. |
| `cargo clippy --all-targets --locked -- -D warnings` | Passed after correcting shared-constant target gating. |
| Linux Clippy with `--target x86_64-unknown-linux-musl` | Passed. |
| Collector `build default` on Mac | Passed, 62.412 s. Preflight `df -g /`: 49 GiB free. |
| Collector `start default` on Mac | Three PATH version commands passed. Installed-binary writes refused. All three old mounted `--harness` version commands passed. |
| TTY no-login starts | No useful screen captured. Timeout/termination is not proof of normal agent operation or companion-dependent startup. |
| Linux collector `build default` | Cancelled deliberately at the design gate after 124.064 s; exit 1, `image-output cancelled`. No completed Linux image measured. |
| Cleanup | Mac owned box ended and owned disk removed; isolated disk inventory empty. Linux build/VMM ended; no Linux box/disk was started. Lease left active. |

The first Linux compile exposed an invalid `crate::core` import from
standalone init. Sharing the constants through init fixed it. Mac Clippy
then exposed unused Linux constants in ARM init; narrowing their target
condition fixed that. These were draft implementation faults.

## Measurements, retained image and gaps

Mac default runtime variant size: 421,490,285 bytes (401.96 MiB).
Guest `du -sk`: `/usr` 220,028 KiB; `/opt` 768,516 KiB. These are different
measurements, not completed Linux root size or unique host storage cost.
Installed versions: Pi 1.5.2, Claude stable 2.1.311, Codex 0.169.0.
Mounted controls: Pi 1.5.2, Claude 2.1.322, Codex 0.167.1.

Retained new Mac image: `localhost/pinfold/profile-default:latest`, unique
ref `localhost/pinfold/profile-default:18e80232dff943d0-42611`, id
`c76d1021b67074b528e5734fb11d3fa150be82fced4ade36aaa7be58a1a1940a`.
No other image was explicitly removed. Shared service/cache remain.

Draft bounds remain proposals: work 8 GiB; raw root, aggregate OCI and
selected bases 2 GiB each; caller context 512 MiB. They are not established
as adequate or minimal. No completed Linux measurements are available.
Linux audited discovery contacted exactly the seven hosts/counts retained
in `linux-results.txt`; no complete cold download-chain claim is made.

Not run: documents/full builds on either platform, successful Linux
build/agent launch, repeated freshness/cache controls, full-derived
project, settings preservation, failed-install corrected controls,
enlarged-bound failures, cache-removal launch, denied/admitted host pair,
caller cache controls, successful companion operation or updater-traffic
proof. The existing mounted path was rechecked only on Mac.

Caller changes in the draft: larger built-in images; managed RUN executes
every time; a project may rebuild/publish two images independently;
managed `build --no-cache` is removed. No CLI JSON/schema change or guest
launch-path removal. These changes are not ready for acceptance.
