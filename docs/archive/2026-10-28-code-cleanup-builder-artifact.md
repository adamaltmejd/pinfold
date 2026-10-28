# Builder-root artifact release preparation audit

Date: 2026-10-28. Baseline and proposed artifact target:
`ff6276cb832dfc78e4cb716857e33af1f00f12b8` (phase 6a / PR 249).
Last CLI release: `v0.3.3`, `74c1fc2f20d78a4493f5335e3e3266420f7d188b`.
This is an audit and review candidate, not publication authority.

The managed worktree is
`/Users/adam/.codex/worktrees/builder-artifact-release-audit/pinfold`, branch
`codex/builder-artifact-release-audit`. The original checkout's three
untracked files, active phase 6b source, operator notes, ignored evidence
and runtime/image stores were left alone. The audit reads current main,
not phase 6b's unmerged implementation.

## Measurements

Before is producer main. After is the five cleanup commits through
`13e5f0f`, excluding this audit's new dated records. The artifact publication
still targets producer main, not this cleanup branch. The comparison to
the last CLI release reports all intervening main work; this operation
does not release those CLI changes.

| Measure | Before | After |
|---|---:|---:|
| Commits since v0.3.3 | 22 | 27 |
| Release additions / deletions, all tracked files | 6367 / 720 | 6375 / 736 |
| Release delete/add ratio, all files | 0.1131 | 0.1155 |
| Release additions / deletions, crates | 1116 / 695 | 1119 / 706 |
| Release delete/add ratio, crates | 0.6228 | 0.6309 |
| New dated audit/readiness/proposal/note lines | 0 | 503 |
| Release additions / deletions, including new records | 6367 / 720 | 6878 / 736 |
| Release delete/add ratio, including new records | 0.1131 | 0.1070 |
| Rust lines, all tracked .rs including dated prototypes | 24231 | 24223 |
| Current crates' Rust lines | 23142 | 23134 |
| Binary Rust lines, including build.rs and init crate | 14814 | 14806 |
| E2E Rust lines | 8328 | 8328 |
| Largest Rust source, including tests | box_.rs, 4368 | box_.rs, 4368 |
| Largest binary source | cli.rs, 1304 | cli.rs, 1298 |
| ARCHITECTURE.md lines | 1300 | 1300 |
| Written rework proposals | 0 | 3 |
| Active tickets opened | 0 | 0 |
| Fresh Mac runtime suite wall time | not run | waiting for exclusive slot |

The admitted code-comment cuts have possible net -8 lines and landed net
-8. The complete five-file cleanup is 8 additions / 16 deletions. README
and help remove repeated or misplaced information without a net line
change. Most additions since v0.3.3 are preserved dated evidence. No
performance improvement is claimed. New audit/readiness/proposal records
are measured separately from these fixed implementation figures.

## Whole-tree read

Three read-only agents covered bounded scopes in successive assignments
within the available three worker slots. Review workers used
gpt-6.1-sol and gpt-6.1-terra; trace/spec/docs were assigned to the latter. They read
AGENTS.md and the binding architecture first. None edited files or ran
builds, suites, sabotage or runtime operations. The operator performed
builds, provenance checks and judgment of candidate cuts.

Complete scopes: CLI and main; box/plan/ownership/network/pipe/core mod;
all eight runtime files; proxy/login/TLS; clean/image/image service/Podman/
artifacts/download/profile/disk; config/dirs/trust/init and Linux init;
pi; crossing/run records/update; E2E helpers and every test file, release
fixture and both slow-proxy scripts; architecture and all profile files;
README and built host help; builder-root; guest-kernel; manifests, build
script, toolchain, ancillary scripts and all six workflows. The operator
read bump-pins.py, e2e-linux.sh, e2e-budget.py and run-mac-e2e.sh. Dated
archive evidence is preserved history, not a current-contract deletion
target. Relevant phase 6a evidence and the exact proposed publication
commit were read separately for provenance.

Dependencies match the nine crates named under Code: tokio, httparse,
serde, serde_json, sha2, toml, nix, rustls and rustls-native-certs.
The dead-code Clippy pass has no warnings. No new dependency is proposed.
No TODO or FIXME was found. The requested `share` path has moved under
`profile/share`; the scan was applied there too. The three config/CLI/
guarantee trace tables are in `/private/tmp/pinfold-builder-artifact-traces.md`.
XXX matches are mktemp
patterns and Public Suffix List data. Login's getAuthStatus revisit and
crossing's git-fsck revisit remain conditional; neither trigger was shown
to fire. The local Git version alone cannot establish behavior on every
supported host.

## Admitted cuts

Each source module is a distinct local commit. No test assertion changes.

- `b3760ab`: CLI, remove six lines restating the crossing/inspect/query/
  age-parser bodies. Move the existing disk syntax into top-level usage,
  removing it from box-only help. No new flag or operation.
- `a439158`: config, remove the repeated load-function description, -1.
- `0ce4e8a`: trust, remove the repeated current-hashes description, -1.
- `067fb57`: README, remove the duplicate full-profile allowlist sentence;
  qualify Ubuntu AppArmor instructions for a host where the restriction
  is actually enabled.
- `13e5f0f`: guest-kernel workflow comment, remove the false statement that
  `gh release create` refuses an existing tag. Existing releases and tags
  are different; `--target` only governs creation of a missing tag. No
  workflow behavior or control was changed.

The rest of the production modules were lean already under the admission
rule. Platform-specific readiness, storage and transport are distinct
contracts, not duplicate policy to erase. No abstraction was added.

## Test sweep and written proposals

All 27 guarantee rows map to their tests or the separate slow gate.
Every E2E assertion block was reviewed for outside expected values,
observer, named sabotage, positive control, hardest case and twins.
Three findings require reviewed rewrites, recorded with source locations,
observable outcomes and exclusions in
`2026-10-28-builder-artifact-test-proposals.md`:

1. Guarantee 9: collected exec output after exit does not establish
   streaming while the guest remains alive. Preserve the existing test
   until a live-output observation is specified and validated.
2. Guarantee 2: the ordinary plain-HTTP positive control can accept a
   proxy 403. Correlate that request's own allowlisted audit decision in
   a reviewed rewrite; a previous CONNECT/route control is not enough.
3. Guarantee 39: private filenames couple a scan-coverage guard to layout.
   Keep it until an outside observer preserves actual artifact coverage.
   The initial deletion recommendation was withdrawn after independent
   judgment. Its shared `scanned` output is not orphaned.

These are inherited proof limitations, not reproduced product bugs. No
failure was hidden by deleting a test. Zero assertion blocks were deleted
or changed. Three written proposals were recorded; no ready/scheduled
ticket was opened for an unresolved observer design. The `yard-file`
admission/body criteria were read for these proposals; Yard was not run.
Existing open issues 211 and 212 already record other trace/test gaps.

## Rejected findings and conditions

- Delete seven further config/trust comment lines: rejected. They describe
  diagnostic attribution, precedence, parsing syntax or digest inputs.
  Re-admit when they only repeat the code and lose no useful convention.
- Delete guarantee 39's filename coverage checks: rejected. It makes the
  negative secret scan less informative. Re-admit only with demonstrated
  equivalent outside coverage, or Adam's explicit guarantee narrowing.
- Treat normal teardown status checks as twins: rejected. They establish
  the scenario's successful lifetime/cleanup and prevent silent failures.
- Treat the root-write probe as an ineffective permissions check:
  rejected. It asserts `Read-only file system`, distinguishing a missing
  read-only mechanism from ordinary ownership refusal.
- Inline single-caller substantive pi helpers: rejected. It moves bodies
  without removing a concept/path/duplicate. Re-admit a named removal.
- Merge different backend paths, replace atomic CLOEXEC behavior or
  generalize filesystem walks: no current measured/safe deletion was
  established. Re-admit only a concrete removed mechanism with preserved
  ownership, isolation, cancellation and trust boundaries.
- Add independent tests for every CLI trace gap: rejected under the fixed
  guarantee list. Extend the existing guarantee only with an admitted
  missing scenario; do not remove promised behavior without approval.
- Performance cleanup without before measurements: rejected. Re-admit
  with an actual cost and a measured reduction.

## Spec pass

ARCHITECTURE.md was read whole and left unchanged. Controls, proxy rules,
trust, features and guarantee rows are unchanged. The guarantee/test map
is complete; the three proof limitations above remain explicit rather
than narrowing Shown by. No control/feature removal is proposed and no
substantive control answer was inferred from artifact approval.

README and built help now agree on the disk command scope. Ubuntu setup
is conditional as in the runtime spec. Pi help remains pass-through and
was not invoked as a host help probe. Other host verbs/subverbs' help was
read from the real built binary; no wording assertions were added.

No open question is resolved by current evidence: fake-IP DNS, Apple
user namespaces, Linux arm64 KVM, herdr's reader and freed disk allocation
remain open. The Linux-only unshare assertion does not answer the Apple
question. The standalone builder is documented as tooling-only; current
main still uses Podman and does not consume the root artifact.

## Verification and gate inventory

- Baseline `cargo tree -p pinfold --depth 1 -e normal`: passed, spec agrees.
- Baseline `cargo clippy --all-targets --locked -- -W dead_code`: passed,
  18.51 s with dedicated writable Cargo/Zig caches and compiler wrapper
  disabled. Initial attempts failed at worktree writes, sccache dep-info
  writes and Zig's default cache. These are infrastructure failures, not
  counted passes or product failures.
- Baseline and cleanup `cargo fmt --check` and
  `cargo clippy --all-targets --locked -- -D warnings`: passed.
  Cleanup lint took 2.91 s. Real Mac CLI/embedded init build passed,
  cleanup build 2.09 s. This is a development build, not a release build.
- Main Linux CI [40692195293](https://github.com/adamaltmejd/pinfold/actions/runs/40692195293)
  passed at exact producer main: 25 passed, 1 documented slow test ignored;
  173.79 s test time and 176.026877979 s including Cargo, under 300 s.
  This was refreshed from the live job/log, not an old memory snapshot.
- Phase 6a head `37b55d2` builder workflow
  [40690899152](https://github.com/adamaltmejd/pinfold/actions/runs/40690899152)
  and CI [40690899075](https://github.com/adamaltmejd/pinfold/actions/runs/40690899075)
  passed. Producer files at that head equal merged main. That workflow
  proves the tooling builds; its output is not substituted for retained
  promotion bytes.
- Exact retained archive/provenance verification passed as detailed in
  `2026-10-28-builder-artifact-readiness.md`, including all eight files,
  all 91 retained members, raw root, gzip EOF, source identity, three
  Debian signatures and live BuildKit publisher digest.
- Fresh Mac suite: not started. The operator explicitly retains phase
  6b's shared-store reservation. A coordinated exclusive run is required
  before pushing these cleanup commits under the skill's step 4. No
  prior phase/6b run is claimed as this branch's pass.
- Cleanup-branch Linux CI and independent final-head review: pending
  push/PR, which must wait for that Mac gate. No PR or merge is claimed.
- Separate read-only correctness and lean/spec reviewers found no blocking
  issue in the five cleanup commits and these four prepared records.
  Their source/document reviews do not replace outstanding execution gates
  or the operator's independent final-head decision.
- Ignored slow route and separate CONNECT gates were not rerun. New CLI
  release builds and prototype host gates were not run by this audit.
  An artifact bootstrap/manual converter proof is not a Mac/Linux binary
  suite or a production builder-role proof.

The current-main AGENTS.md release paragraph names Linux x64, full Mac and
release builds for nightly pin releases; the initial checkout's older
session instructions also name two Linux suites and three prototype hosts. This
audit is neither a nightly pin nor CLI release and invokes neither path.
It does not silently waive any gate: the operator must use the applicable
current feature/CLI release contract for those future operations. The
whole-tree audit applies to this artifact-only release in either case.

## Artifact operation and authority

The separate readiness plan carries exact assets, target/tag, recovery on
unknown outcome, all-eight-asset publisher-digest verification, actual
tag resolution, independent download/raw checks and consumer admission.
The preserved proposal and release notes were read through exact committed
git show. No active worker checkout was edited. No rebuild, pin update,
new dependency, version bump, release dispatch, tag, asset, draft or public
release was created. No numeric runtime policy was decided.

The guest-kernel convention supports separate prerelease/latest=false
artifact publication; it supplies no audit exemption. The skill's final
pin/version/v-tag steps would be a CLI release and conflict with this
task's explicit artifact-only boundary. They were not executed.

Publication remains a final explicit Adam decision after the operator
reviews this preparation and its outstanding gate/proof items. An
artifact approval cannot approve phase 6b's consumer or runtime controls.

## Caller changes

None to the released CLI, box spec, JSON lines, exit codes or flags.
The cleanup candidate corrects help grouping only. The proposed artifact
release notes have their own required Caller changes section.
