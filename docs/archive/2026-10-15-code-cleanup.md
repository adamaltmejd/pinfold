# v0.2.5 release cleanup and specification pass

2026-10-15. Release baseline `v0.2.4` is `fd45db2365bcf0f31903e99caac46e828fea6e31`.
The audit began at PR #91 head `8fa870c4`; the combined implementation,
reviewed pins and bounded PR #93 fixture are at `0f10f69`.

## Whole-tree audit

Independent read-only agents read the full modules, not only the diff:

- CLI and main; box lifecycle; all runtime adapters; proxy.
- Cleanup, images and artifacts; config, directories, trust and shared init.
- Pi; update and download; plan, network, TLS and pipe helpers.
- Ownership, login, profiles and core helpers; the E2E harness.
- All E2E test files and auxiliary fixtures; build scripts and manifests.
- Runtime/setup/release scripts, GitHub workflows, Yard configuration.
- Durable prototype source, gates, configuration and documentation.
- ARCHITECTURE.md, README.md, all profile files and CLI usage help.

The direct Cargo dependency tree matches the binding Code list. The initial
dead-code Clippy audit passed. No deferred TODO/FIXME was found in source;
the `xxx` matches in the Public Suffix List are domain data. The login
`getAuthStatus` revisit has not fired because its pin is unchanged. Upstream
apple/container#1941 remains open. The four architecture open questions
remain unanswered; assigning HERDR_AGENT does not establish where herdr
reads it.

All modules except the cuts below were judged lean already. No new
abstraction, dependency or unmeasured performance change was admitted.

## Landed dispositions

| Commit | Module | Removal or correction |
|---|---|---|
| `8c583f3` | core/box | Two-line comment repeating mount selection and function documentation. |
| `f8e5ecd` | core/download | Redundant successful-read `continue`. |
| `0766899` | E2E box | Quotation-format assertion becomes a unique fixture route token; remove an untouched-object byte comparison unrelated to the attempted worktree write. |
| `23fad79` | E2E update | Remove a request-count observation already proved by the fixture version; align its comment with the surviving observer. |
| `38107c3` | README | Remove repeated runtime requirement prose. |

The agents proposed 14 net lines of cuts; comment alignment brought the
landed cleanup to 15 net lines removed. Independent review approved each
implementation. No control, feature, guarantee or meaningful assertion was
removed. The guarantee rows already describe the surviving behavior and
need no deletion.

PR #93 was independently reviewed for production and tests. Its merge
`0f4b8c6` preserves PR #91's state-directory plumbing and resolver, adds the
private seccomp profile and removes the unused directories import. Separate
review approved that actual merge. A second review found an unbounded FIFO
write in the new test fixture. `0f10f69` keeps it nonblocking with a single
60-second transfer deadline; all syscall assertions and runtime controls
remain unchanged. Both production and test reviews approved the result.

## Test audit and written work

Rows 17, 22 and 28 retain external expected values, refusal causes, positive
controls and sabotage. The two removed observations did not prove additional
behavior. All other existing assertion blocks were retained except the
format-only route expectation, whose cause proof is stronger with a unique
fixture token. G35/G36 remain lean. New G37 uses two real concurrent Podman
startups, kernel lock waiters and FIFO rendezvous, raw syscalls, explicit
EPERM and a successful syscall in the stricter box.

One written ticket: [#94](https://github.com/adamaltmejd/pinfold/issues/94).
G12 has four failure-only refusal blocks. Each has a subsequent authorized
positive control, but no stable spec-named trust refusal token exists. Keep
the blocks and the guarantee; defining that diagnostic and extending the
existing test is bounded contract rework. Independent review confirmed this
is a pre-existing proof limitation, not a regression introduced here.

No control or feature-removal question is awaiting approval. No speculative
speedup is pending. The prior rejected tag-loop rewrite and deferred proxy
buffering remain outside this release; representative cost evidence is
required to reconsider them. Fine-grained CLI/config trace gaps are not
grounds for new help, argv-shape or duplicate tests under the fixed-list rule.

## Specification pass

Configuration keys, CLI verbs/flags and all 37 guarantees are traced in the
companion tables. Rows 1–34 map to named E2E tests; row 34 runs separately,
row 35 to the durable prototype on three hosts, row 36 to both Linux slow
gates, and row 37 to the Linux seccomp race. Pi's help is passed through and
can perform working-command preparation; the audit stopped that invocation
after a sandbox refusal instead of treating it as a read-only help surface.

README states current usage and links to the spec. Required runtime versions
remain in its table. Architecture controls, threat model, trust and caller
schemas remain intact. G37 records the additional Linux startup guarantee. The spec introduction
now names that Linux-only Rust E2E test too.
Release notes include the unreleased main changes since v0.2.4, including
minimum runtime versions and inventory timeout/interruption exits, as well
as external hook-alias refusal. No box-spec keys, JSON fields or CLI flags
were added or removed.

## Measurements

The following diff counts are from v0.2.4 to the named code candidate,
before adding this audit record and its trace tables.

| Measure | Before audit (`8fa870c4`) | Combined candidate (`0f10f69`) |
|---|---:|---:|
| Added/deleted lines, all tracked files | 1,389 / 240 | 1,704 / 295 |
| Delete/add ratio | 0.1728 | 0.1731 |
| Added/deleted lines, crates | 897 / 222 | 1,141 / 257 |
| Crates delete/add ratio | 0.2475 | 0.2252 |
| Largest Rust file, E2E box | 5,274 | 5,501 |
| Largest production source, cli.rs | 1,135 | 1,135 |
| ARCHITECTURE.md lines | 1,017 | 1,022 |
| Rust test/harness lines | 8,332 | 8,550 |
| Production Rust lines | 9,498 | 9,489 |
| Findings recorded as written tickets | 0 | 1 |

PR #91's paired memory/size measurements precede this release's pin refresh.
They are evidence for the selected reductions, not a new release-wide speed
claim. Pin refresh `a2d36e4` and both lockfiles received independent review;
the prototype typecheck passed. The original primary-checkout tracked diff
remains `d553421a48a119d7d6eef39fa85b7aff691441b0dca05800c9ef2624008cc9c7`.

## Gates

Formatting and strict Clippy pass for the combined source. The first local
Clippy invocation failed before tests because its optional compiler-cache
daemon could not write an output. The accepted invocation disables that
wrapper only for the command; global settings were unchanged. The failed
log is retained separately. The full Mac suite passed 33 tests, failed none and ignored the separately
run slow gate in 252.93 seconds. Linux, slow-gate and release-build results
follow in the release evidence bundle.

Full release diff including this audit record and trace tables: 1935 added, 296 deleted; delete/add ratio 0.1530.
