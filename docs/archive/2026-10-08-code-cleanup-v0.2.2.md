# 0.2.2 release cleanup and test audit

The pass started at `6d43934`, with green main CI and no competing Mac
runtime work. It audited the whole tree before preparing candidate
`467cb02`. Issue fixes 85 and 86 were already merged; issue 78's released
fix had fresh real-runtime verification. Earlier dated reports remain
historical snapshots.

## Scope and mechanical checks

Independent read-only agents covered CLI/main/update; box/plan/ownership/
pipe; all runtime adapters and network; proxy/TLS/login/download; clean/
image/artifacts/profile; config/dirs/trust/init; pi; E2E helpers and every
Rust test and Python fixture; the durable prototype and its gate; build
support, scripts, workflows and manifests; architecture, profiles, README
and safe built help. Reviews ran in bounded parallel groups.

All nine production dependencies match the spec. Clippy with `-W dead_code`
passed. No TODO or fired revisit trigger was found. Public Suffix List
`xxx` matches are data. The pinned Codex 0.161.0 protocol still supplies
`getAuthStatus`, `include_token` and `auth_token`; its login note remains.

The three trace tables connect every configuration key, CLI verb/flag and
all 36 guarantee rows to the spec and exercising scenarios. Unasserted
argument aliases and optional argument variants are not new guarantees;
AGENTS.md forbids argument-shape tests. Scratch tables are retained at
`/private/tmp/pinfold-release-022/trace-tables.md`.

## Size snapshot

The close measurements are at `467cb02`, excluding this report. They
include the version bump, which changes no line count.

| Measure | Entry | Close |
| --- | ---: | ---: |
| Tracked Rust lines | 16,959 | 16,917 |
| Production Rust source lines | 9,071 | 9,032 |
| E2E Rust lines | 7,829 | 7,826 |
| Largest source, E2E box_.rs | 4,883 | 4,879 |
| Largest production source, cli.rs | 1,115 | 1,112 |
| ARCHITECTURE.md lines | 973 | 973 |

Reviewers estimated 70 possible net deletions. The admitted cleanup landed
58 net deletions: 117 removed and 59 added, a delete/add ratio of 1.983.
Binary source has 17 added and 56 removed lines beside 2 added and 5 removed
test lines. The remaining changes consolidate CI setup and shorten README.
No dependency changed. Zero findings became written tickets.

The complete release diff from v0.2.1, before this report, has 478 added and
196 deleted lines across 27 files: delete/add ratio 0.410. It includes the
issue fixes and their evidence, not just cleanup. The crates-only release
diff has 120 added and 112 deleted lines.

## Landed dispositions

- `d48abf4`: remove six redundant Serde defaults on optional box fields.
  Missing fields still deserialize as None; collection defaults remain.
- `5d27dcd`: remove the obsolete comment claiming owner PIDs determine
  liveness across state roots. Lifetime locks retain that authority.
- `4c14eae`, `90fe20b`, `6ab21fe`, `0160166`, `4137855`: remove repeated
  runtime, maintenance, artifact and relay comments, and CLI parser
  rationale. Each module has its own commit.
- `0295263`: inline the sole build-tag helper, retaining ordering and
  cache-identity rationale. Remove repeated repository examples.
- `6f40f76`: remove the empty-output fallback after successful Git root
  lookup and repeated pi comments. Git-error fallback, canonicalization
  and containment checks remain.
- `61deba1`: inline the sole release-target helper. Supported targets and
  unsupported-host refusal are unchanged.
- `4869c69`, guarantee 23: delete the duplicate nonempty failed-log check.
  Array extraction and the stronger external final-marker assertion prove
  the same shape and nonemptiness. The guarantee row remains fully covered.
- `f318cf8`, guarantee 31: retain the network-request count and name its
  distinct sabotage. Resetting the daily timestamp while saving a changed
  profile fingerprint triggers a second request; guarantee 29's unchanged
  binary cannot expose that state interaction. No assertion changed.
- `42d70c9`: shorten programmatic use, update and profile introductions.
- `50dabb1`: replace duplicate rootless Linux runner provisioning with
  `scripts/setup-linux-runner.sh`. Preserve AppArmor, cgroup/session,
  D-Bus and environment setup; the slow workflow installs its fixture tools
  separately. ShellCheck, shell syntax and shfmt checks passed. Independent
  review confirmed environment propagation and ordering.

The complete production cleanup diff passed independent review. The
removed assertion has a surviving stronger proof. Guarantees 15 and 36's
recent regression scenarios passed the fresh static test audit unchanged.
Other assertion blocks and modules were lean already. No proposed deletion
requires a decision about controls, features or guarantee rows.

## Rejected and retained findings

- Keep the raw nonblocking receive in CONNECT. Replacing it with nix's
  socket wrapper requires a new feature and transitive dependency. Revisit
  only if socket support is independently required.
- Keep the ownership PID record. Guarantee 8 uses a misleading live PID
  as an outside fixture; lifetime-lock ownership remains authoritative.
- Keep three profile comment cuts out of this release. They would change
  embedded profile/image fingerprints solely for five comment lines.
  Reconsider alongside a necessary functional profile edit.
- Keep README's host boundary introduction, contributor-rules pointer,
  rebuild state-preservation explanation and built-in profile inspection
  command. These help a user act or choose safely; their presence in the
  spec or runtime notice is not enough reason to remove them. Re-admit
  only if a shorter replacement retains that guidance.
- Keep guarantee 12's trust refusals. Same-project before/after controls
  isolate approval as the cause; the spec defines no refusal token.
  Restoring assertions of remediation prose would violate the test policy.

## Spec and pin pass

README and all 22 safe built-help invocations agree with the CLI and
process contracts. `pi --help` delegates to the harness and was not used
as a supposedly runtime-free help query. No restatement or rationale cut
from ARCHITECTURE.md survived the contract review. All four open questions
remain unanswered; none was closed without evidence. Guarantee 23's
external base-refresh scenario remains a documented manual verification,
not claimed permanent coverage.

The required pin refresh made no changes. Ponytail 5.0.0 remains too new
for the seven-day image-tool policy; the 4.12.0 pin stays. Workspace and
lockfile are 0.2.2. Caller changes: None. No box schema, JSON-line field,
exit code or CLI flag changed.

## Validation before publication

Candidate `467cb02` passed the complete Mac E2E suite: 33 passed, zero
failed, one separate slow test ignored, in **293.35 seconds**. Formatting,
diff whitespace and strict all-target locked Clippy passed. The local
release builds for Mac arm64 and Linux arm64/x64 passed; the Linux outputs
are static ELF binaries and the Mac executable reports 0.2.2. Zig emitted
a deprecated linker-optimization-setting warning in the local Linux cross
builds; no source warning or build failure remained.

The separate Mac blocked-response and durable gates, both Linux suites,
the expanded Linux CONNECT proof and tag-built release assets remain to be
validated before publication. No failed runtime test was retried in this
pass.

Scratch output and measurements are under
`/private/tmp/pinfold-release-022/`. Linux and release-artifact results
will also be retained by their GitHub runs and the release pull request.
