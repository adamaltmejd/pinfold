# Current-target builder artifact audit

Date: 2026-10-29. Baseline/current producer main:
`5c0ffa2287a51d0fa1a8575181a0a32664b6ef3f`.
Local branch: `codex/builder-artifact-audit-current`.
Implementation scope is frozen at `987db2d`; later commits preserve and
record evidence. The public PR 250 branch remains at `d76ea72`.

This is a fresh whole-tree source/test/spec read. It does not inherit a
release audit from the producer-delta review or claim any runtime pass.
The operator owns final independent reviews and test-slot coordination.
Nothing here authorizes push, merge, tagging or artifact publication.

## Measurements

Before is producer main. After below includes the five local source/docs
commits through `987db2d`, excluding this branch's dated audit records.
Both compare against v0.3.3. New archive records are evidence additions,
not a claimed reduction in implementation complexity.

| Measure | Before | After |
|---|---:|---:|
| Commits since v0.3.3, through source scope | 23 | 28 |
| Additions / deletions, excluding new audit records | 7425 / 720 | 7436 / 739 |
| Delete/add ratio | 0.09697 | 0.09938 |
| Crates additions / deletions | 1116 / 695 | 1119 / 706 |
| Crates delete/add ratio | 0.62276 | 0.63092 |
| Current crates' Rust lines | 23142 | 23134 |
| Largest Rust source, including tests | box_.rs, 4368 | box_.rs, 4368 |
| Largest binary source | cli.rs, 1304 | cli.rs, 1298 |
| ARCHITECTURE.md lines | 1307 | 1307 |
| Written test-proof tickets | 0 | 3 carried forward |
| New implementation tickets opened | 0 | 0 |
| Candidate Mac suite wall time | not run | pending coordinated gate |

The candidate changes five operative files: 11 additions / 19 deletions,
net -8. All eight deleted net Rust lines are repeated comments. Fresh
reviewers suggested 27 code lines of possible reduction; two overlap the
approved config/trust cuts, and the other 25 are rejected or deferred below.
The six approved CLI comment lines were retained from the earlier review.
No performance saving is claimed. The six older dated records are copied
byte-for-byte, 850 lines; none is rewritten to track the present.

## Actual read coverage

Fresh read-only module agents worked in three-worker batches. Every
assigned source/test file was read whole, not only its diff. Code/test
agents used gpt-6.1-sol; trace/spec and user-facing docs used gpt-6.1-terra.
They ran no build, test, sabotage, runtime or VM. The main session ran
compile-only mechanical checks and read ancillary tooling/spec documents.

| Reader | Whole-file module coverage |
|---|---|
| current_cli | cli.rs, main.rs |
| current_box | core box, plan, ownership, network, pipe, mod |
| current_runtime | runtime mod, Apple, five Firecracker files: seven files total |
| current_proxy | proxy, login, TLS |
| current_maintenance | clean, image, artifacts, download, profile, disk, image service and Podman adapter |
| current_config_init | config, dirs, trust, shared init, Firecracker init and init-crate main |
| current_pi | pi mod, launch, state |
| current_support | crossing, run records, update |
| current_test_box | e2e lib.rs and box_.rs; all assertion blocks |
| current_test_pi | pi.rs and cli.rs; all assertion blocks |
| current_test_remaining | runtime/runs/update/main.rs, HTTPS release fixture, CONNECT fixture and slow setup |
| current_trace_spec | full architecture and every profile file; three trace tables |
| current_userdocs | README and built help after every host verb/subverb; pi pass-through excluded |
| current_producer | full Python/Go producer, README, pins, package closure, workflow and five relevant producer/component receipts |
| Main session | full architecture/README; six workflows; all remaining scripts; guest-kernel build/pins; build.rs, manifests, toolchain and delegation guidance |

`2026-10-29-builder-artifact-audit/coverage.jsonl` accounts for all 104
operative baseline paths, with baseline/candidate SHA-256, reader and
method. It distinguishes whole-file reads from three alias validations
and four pinned-data validations. Historical archives/investigations are
dated evidence, not current source deletion targets; relevant producer
receipts were read separately. Public-key identities were checked with
GnuPG. Cargo.lock's 76 registry packages have checksums; PSL identity and
content classification were inspected. No pin was moved.

The baseline and candidate hashes differ only for the five approved
operative files. The final independent reviewers must recheck those full
changed files and this record's delta at the final metadata head. The
fresh whole-main reads do not substitute for that final-head decision.

## Mechanical checks and traces

The nine normal dependencies match Code. Baseline dead-code Clippy passed
in 2.85 s; baseline format and Mac CLI/embedded-init build passed, the
build in 2.61 s. Source-candidate strict locked all-target Clippy passed
in 3.67 s; format passed; Mac CLI/embedded-init build passed in 1.85 s.
Dedicated Cargo/Zig caches under /private/tmp and no compiler wrapper
avoid unrelated cache permissions. These are development/embedded-init
builds, not CLI release builds or runtime-suite results.

The fresh three trace tables are preserved as
`2026-10-29-builder-artifact-audit/traces.md`. References were corrected
against actual numbered source after review. All 27 guarantee rows map to
tests; G36 is the separate slow gate. No config key lacks a spec row.
Direct-test gaps remain for attach, explicit push, update --check, --tty,
-V/-h and PINFOLD_NO_UPDATE_CHECK; setup uses are distinguished from proof.
These are not grounds for prohibited argv/help tests or feature removal.

No TODO/FIXME was found in crates, profile/share or builder-root. XXX
matches are mktemp patterns and PSL data. The getAuthStatus pin-removal
and all-supported-host Git-fsck triggers were not established as fired.
The Apple signal revisit has newer upstream evidence, recorded below.

## Disposition of the five earlier cleanup commits

| Old PR 250 commit | New local commit | Disposition |
|---|---|---|
| b3760ab | 96675b6 | Retain: six repeated CLI comment lines removed; existing disk help moved to top level |
| a439158 | 5117c53 | Retain: repeated config load description removed |
| 0ce4e8a | fdf3371 | Retain: repeated current-hashes description removed |
| 067fb57 | 3ad8dbd | Retain duplicate-profile/AppArmor corrections; add truthful README clarification that startup does not require an initial commit |
| 13e5f0f | 987db2d | Retain: remove false comment that gh release create refuses an existing tag |

The README clarification follows launch.rs:117-130, :44-81 and :482-485:
repository admission checks `.git`, and no branch refs means a silent
sync skip. The architecture explicitly permits it. Host content still
requires committed branches. No refusal wording or implementation changed.

All producer inputs, Rust behavior apart from help grouping, tests,
assertions, guarantees, proxy rules, trust, resources, pins, dependencies
and version remain unchanged. Current main still does not contain these
cuts; they are only in the local audit candidate. No remote update occurred.

## Test sweep and retained tickets

The fresh sweep found the same three proof limitations. Zero assertion
blocks were deleted or changed. The three open written tickets are in
`2026-10-29-builder-artifact-audit-tickets.md`:

- T1/G9: observe stdout/stderr while the guest is still alive.
- T2/G2: correlate the ordinary HTTP positive control with its own allowed
  audit decision and upstream response.
- T3/G39: replace private basename coupling while retaining actual scan
  coverage; observer design remains unresolved.

They are not reproduced product bugs or scheduled work. Positive controls,
teardown checks, specified output comparisons and coverage witnesses stay.
No guarantee or Shown by is narrowed. Release disposition of these open
tickets remains an operator/Adam decision; a missing proof is not fixed by
a later producer/runtime suite passing.

## Rejected/deferred findings and control questions

- Additional config/dirs/trust comment cuts: reject where they preserve
  digest-input meaning, source attribution, layer precedence, parsing
  syntax or directory conventions. Only the already-approved two comment
  cuts land. Re-admit a genuinely redundant comment in an authorized cut.
- init/firecracker.rs:790 Option/Result match shortening, net -3: reject
  as syntax-only outside the approved source scope. Re-admit with a named
  mechanism removal in a substantive admitted module change.
- artifacts.rs:227 hash-expression reuse, net -1: defer. It is a tiny
  duplication; no additional source module is changed for it. Re-admit
  when an authorized artifacts-module consolidation warrants the review.
- Q1, plan.rs:689,723-737 duplicate-login trackers, possible net -6:
  no change. The suggested shared set touches the proxy's duplicate-login
  validation (ARCHITECTURE.md:617-619). Treat it as a control-review
  question for Adam, not an automatically admitted code deletion.
- Q2, Apple graceful-stop revisit: issue apple/container#1941 is completed
  and fix PR1778 merged as 1ebb9d07 on October 22. GitHub comparisons show
  supported 1.6.0 is two commits behind that fix; 1.6.1 includes it. The
  upstream revisit is therefore relevant, but it does not make a graceful
  path safe across the whole supported range. Keep the current contract.
  Any version-floor or shutdown change needs separate explicit design
  review; this audit changes no signal/lifetime control.
- Combining Apple allocated-block measurement with logical-size walks:
  reject. Units, error handling and directory accounting differ.
- Replacing the manual Env deserializer: reject if it loses the offending
  nested key required by the spec. Generic copying of proxy/TLS streams
  similarly loses framing/deadline/audit behavior.
- Inlining substantive one-caller pi helpers or merging pi/transient
  supervision: reject; no mechanism is removed and contracts differ.
- Performance changes without before measurements: reject. Re-admit only
  with demonstrated cost and preserved boundaries.

No control change is proposed for landing. Q1/Q2 stay explicit questions;
neither authorizes a weakening, new compatibility path or supported-version
removal. No open question was answered by current test evidence.

## Spec pass and the 093 publication plan

The operator read the architecture whole; fresh spec/profile/docs reviews
were also completed. All 27 rows and all five open questions remain.
No spec text is cut merely to erase a coverage gap. README/help drift is
resolved within the approved docs scope. The producer paragraph accurately
describes standalone tooling and no installed CLI consumption.

`2026-10-29-builder-artifact-readiness.md` is the operative twelve-file
proposal. It targets main 5c0ffa2 and preserved archive 09393a02.../root
ef1aaa0f..., with recipe/helper identities, all twelve hashes, reviewed
notes, unknown-outcome inspection and all-asset publisher/download checks.
The older e7dd/eight-file plan is history, not a fallback. No rebuild,
new dependency, version/pin bump or CLI release accompanies this preparation.

## Actual validation inventory before a publication decision

| Boundary | Required evidence/disposition | Current status at this snapshot |
|---|---|---|
| Local cleanup candidate | Format, strict locked all-target Clippy, actual source/help review | Compile checks passed; parent correctness/security and distinct lean/spec/test reviews assigned at source 987db2d + preservation ae0c3ff; final records need delta review |
| Local cleanup candidate | One coordinated full Mac suite, including live login | Not run yet on this candidate; permitted only after this scope/coverage commit, with ownership/space checks |
| Linux cleanup candidate | Actual Linux x64 suite, format/Clippy/AppArmor and <300s budget | Pending later authorized source publication/CI; no dispatch/push in this continuation |
| Producer main | Exact-tree Mac/Linux and bootstrap evidence | Producer 784988b tree equals 5c0ffa2; Mac: 24 passed/1 ignored, 120.75s test, 125.957s command; Linux: 25 passed/1 ignored, 184.682s command; bootstrap 40768192630 passed |
| Preserved artifact bytes | Exact twelve-file inventory, raw root, gzip/source/provenance checks | Reconciliation verified retained 093 bytes; refresh immediately before an approved promotion; no CI filesystem substitution |
| CLI release builds | Mac arm64 CLI and Linux x64 static-musl release builds | Not run on this candidate. Commands are in release.yml; never dispatch its publishing workflow merely to build |
| G34 | Real blocked-route response proof at the production five-minute deadline | Existing ignored test is outside the normal suite; no fresh result on this candidate |
| G36 | Real CONNECT activity proof in private Linux network/mount namespaces | Separate slow-proxy gate, outside normal CI; no fresh result on this candidate |
| Runtime PR 252 | Consumer, cache/lifecycle/fault, audited egress/jail, preservation and resource policy | Worker/operator owned; its b70f323 Mac pass is not an audit-head pass; no VM lease or duplicate suite here |
| Publication | Final review, open-ticket/control/gate dispositions, exact twelve-asset approval | Not granted; any narrowed artifact-only gate inventory or exception must be explicit |

The current supported Linux runtime is x64; Linux arm64 is still an open
question. No obsolete three-host prototype rule is invented for this
artifact operation. Conversely, artifact-only status supplies no automatic
exemption from the release audit or remaining validation. The operator
must decide which binary release-build and separate slow-gate results are
required before the final question, or identify an explicit proposed
artifact-only exception. Neither old CI nor component proofs satisfy them.

The old Mac run at 3d6b106 remains 23 passed/1 failed/1 ignored, exit 101,
438.252s. Its separate upstream 429 observation and intentional box stop
causing exit 137 remain distinct. No later producer or runtime pass changes
that history. A new candidate-head run will have its own receipt/result;
there is no automatic retry or test weakening.

At this snapshot, no shared Mac suite, slow proof, release build, VM lease,
source push, PR merge, tag or release has run. The worker's Mac reservation
was released; the parent permits one candidate run after this final scope
is committed and reviewed/assigned. No global cache cleanup or shared-store
pruning is authorized by this audit.
