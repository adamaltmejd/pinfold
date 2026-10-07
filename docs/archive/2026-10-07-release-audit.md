# v0.2.0 release audit — 2026-10-07

The whole-tree cleanup and test audit is complete. This is its dated
snapshot after local validation. Linux runtime and publication gates are
pending; this report does not declare the release ready.

## Scope

The `code-cleanup` whole-tree read and `test-audit` sweep covered the CLI,
configuration, initialization, trust, directory operations, core box and
ownership, all runtime adapters, proxy and network controls, TLS and pipes,
downloads, cleanup, images, artifacts, profiles, login, update, pi, shared
e2e helpers and every guarantee test. The remaining build files, scripts,
workflows, images, profiles and shipped extensions were read separately.
The operator pass read ARCHITECTURE.md and README.md in full.

The mechanical prepass found nine production dependencies, matching the
spec, and no dead-code warnings. Configuration, CLI and guarantee trace
tables found no unlisted interface or guarantee without a named gate.
Scratch module reports and observations are in
`/private/tmp/pinfold-release-audit/`; that directory is local evidence,
not a release artifact.

## Size snapshot

The audit entry is `a51fa8ddf9787d51d7f4d63becd92cb753c3cb9d`.
The snapshot below is committed HEAD `d2a1383`, excluding this report.

| Measure | v0.1.9 | Audit entry | Audit close |
| --- | ---: | ---: | ---: |
| Tracked Rust lines | 13,828 | 16,497 | 16,905 |
| Production Rust source lines | 7,930 | 9,060 | 9,043 |
| E2E Rust lines | 5,839 | 7,378 | 7,803 |
| Largest source file, e2e `box_.rs` | 3,492 | 4,373 | 4,820 |
| ARCHITECTURE.md lines | 867 | 959 | 972 |

The largest production source file is `cli.rs`, at 1,115 lines.
Since v0.1.9: fourteen commits; 6,812 added and 1,229 deleted lines across
69 changed/new files; delete/add ratio 0.180. Since the audit entry:
eleven commits; 1,218 added and 157 deleted lines across 19 files;
delete/add ratio 0.129. These totals include the feature, lifecycle,
test and fixture work in this release, not just cleanup cuts.

## Cleanup dispositions

- CLI: remove repeated implementation comments, net 15 lines
  (`1f0ecbd`). No behavior or assertion changes.
- Login: inline the single-use helper exchange, net 10 lines
  (`0a3839b`). Preserve helper lifetime, lock, deadline and cancellation.
- Runner maintenance: remove a discarded read-only artifact lookup and
  its scratch state, net nine lines (`6c8c6d4`). Keep maintenance commands.
- Documentation: shorten README by six lines and clarify ignored profile
  selectors and optional tools (`553b281`). Keep the current contract.
- Update test: remove the unrelated whole-process four-second assertion
  and orphaned timing import, net five lines (`94618d1`). Keep the actual
  stalled-request bound and successful fixture control.
- New CONNECT fixture: remove three lines retaining unused worker
  references. Daemon threads and their start calls remain.

The other production modules were lean already. The proposed runtime
PATH-scanner deletion was rejected: changing executable resolution has
semantic and control risk, with no measured benefit that warrants it.
Native runtime parsing, ownership and cleanup guards, trust-boundary
validation, protected paths and positive controls remain.

No written tickets were opened by this pass. The final local full suite
passed; the earlier failures and their dispositions are recorded below.

## Test and spec pass

The sweep deleted duplicated pi seeding, project-marker and easy allowlist
blocks. It removed static prompt word bags (`403`/`final` and
`commits`/`host`/`.git`); changing host-selected allowlists in real model
requests still prove the extension loads. Saved host settings, fixture
skills and repeated default/full/default selection remain observable.

G9 now requires the spec-named `image-changed` reason at the held cold
artifact download, including cleanup bookkeeping after failed preparation.
G15 holds a real pi startup after project ownership but before box creation.
Host checkout removal makes cleanup eligible without a clock wait;
cleanup must preserve the existing home while held, then remove it idle.
The redundant `state.json` existence assertion was deleted.
G11 covers late `path-not-utf8` refusal and protected-directory rollback.

G26 uses a held real Codex refresh with two independent owners
sharing CODEX_HOME. The documented `login-busy` signal exposes contention;
the host fixture supplies only one refresh, and both owners must complete.
The real 0.161.0 helper, contention scenario and live Mac login passed in
87.30 seconds. Its tagged upstream protocol still provides getAuthStatus;
the removal-based revisit trigger has not fired.

The guarantee table reflects these scenarios. G36 names the exact
`idle timeout` reason and has a separate slow CONNECT gate. Its narrow
Linux fixture exception isolates DNS and TLS endpoints in network/mount
namespaces while retaining the production CONNECT and TLS checks.
Actual rootless Podman feasibility remains a runtime gate.
The four existing open questions remain unresolved by this pass.
Release caller notes must cover optional default image tools and the new
`path-not-utf8` reason and `login-busy` stderr token.

## Measurements

Six alternating warm startup pairs used the same image identity.
Baseline median was 0.684126 s; candidate median was 0.668573 s.
Ranges were 0.651891–0.704374 s and 0.647441–0.707779 s respectively.
They overlap; these observations do not establish a startup speedup.

Three repeated build-log observations reported 935,268 JSON bytes for
the baseline and 65,610 for the candidate. Native peak RSS remained about
40 MB and was dominated by the runtime client. Attribution is inconclusive;
the observations do not establish a memory reduction.

Compressed image snapshots were default 70.9 MB, docs 126 MB and full
145.8 MB, versus the old image's 239.1 MB. Shared builder allocated storage
was 9,087,832 KiB before this validation work. A later snapshot measured
48,309,160 KiB for the shared Apple Container store, including 11,141,596 KiB
for buildkit. Test builds and shared images affect these figures; they are
not per-profile savings. Compressed sizes and builder allocation do not
establish host disk savings.

## Validation at audit close

- Formatting and strict all-target Clippy passed on `d2a1383`.
- G15's claimed-startup cleanup scenario passed in 72.05 seconds. The
  original G9 additions passed in 50.61 seconds. Its corrected native-tag
  fixture passed in 48.46 seconds and received independent review.
- G26 passed in 87.30 seconds, including the real pinned helper and live login.
- G34's separate production-deadline test passed in 350.27 seconds,
  including a 31.87-second binary rebuild.
- All three local release builds pass for the current production source.
  Linux outputs are static arm64/x64 ELF; the Mac output is arm64 Mach-O.
  Zig reports its existing deprecated linker-optimization warning.
- The first full Mac run passed 31 and failed two held-build fixtures in
  429.48 seconds. Native firewall logs show an inbound permission request
  for the new Cargo test identifier `e2e-2cb68f1c8cd47fa4`.
- Reusing the approved path preserved that new embedded identifier and
  did not resolve the firewall gate. That run passed 30 and failed three
  in 431.78 seconds: the held-build fixture, a real artifact connection
  timeout in G9, and G23's missing failed-build marker.
- G9's hold previously included a complete image build, which could exceed
  curl's production connection deadline under load. Both images are now
  built before the hold; only native retagging occurs inside it. The
  production deadline and image-changed assertion remain unchanged.
  Native Apple tagging qualifies the target with `docker.io/`; the fixture
  now uses that explicit alias, cleans it up and observes identities after
  both moves. It still requires different images, refusal for image-changed
  and successful startup reading the second image's file.
- Mac durable recovery passed on the current candidate: host-boundary
  refusals, a boxed command, exclusive writer, crash recovery without unsafe
  replay and whole-box cancellation. Frozen dependency installation and
  TypeScript checking also passed.
- Five isolated CLI failures and four native runtime failures retained
  G23's final marker. Its assertion now includes the bounded JSON log so
  another failure identifies an early runtime error versus lost tail data.
  The unchanged assertions also passed in the final full candidate run.
  The earlier failure remains unexplained; no evidence attributes it to
  the firewall or G9 fixture issue. Further passing repetitions could not
  recover the missing historical output.
- Automatic approval review initially rejected a persistent firewall
  exception without explicit user authorization. The user subsequently
  approved the exact E2E executable exception and authenticated through
  macOS. Incoming fixture connections are now permitted for that executable;
  global firewall settings are unchanged.
- A fresh Sol 6.1 reviewer found no actionable blockers in the changes
  since `a51fa8dd`, including G9, G15, G26, G36, Linux durable wiring and
  caller notes. Runtime signoff remains conditional on the pending gates.
- The final full Mac suite passed on `d2a1383`: 33 passed, zero failed,
  one intentionally ignored slow test, in 589.70 seconds. This is a local
  shared-runtime result; the five-minute Linux CI budget remains a gate.

Both Linux suites including durable recovery, G36 on both architectures,
publication and installation remain pending. Final runtime and release
evidence belongs in a separate dated record. Earlier focused observations
do not substitute for those final gates.
