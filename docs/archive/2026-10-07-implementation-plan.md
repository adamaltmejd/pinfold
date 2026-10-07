# Pinfold hardening and optional tooling plan

Adam authorized the full October 7 review, optional tooling, and an
external Pi Durable prototype. Implementation and independent review use
Sol 6.1. Adam is the only user and accepts reinstalling; this work adds no
migration or compatibility layer.

The starting commit is `f9cd183` (0.1.9). Work is isolated on
`codex/pinfold-hardening-profiles`. The earlier review remains in the
original checkout. Existing Yard tickets Y-2 through Y-7 provide prior
acceptance criteria; Y-8's dead-owner repair is already in the starting
commit and must be assessed before any further change.

## Contract decisions

- Keep the Rust isolation core, fixed runtime choice, explicit image
  builds, exact egress controls and host-owned credentials.
- Built-in profiles are `default`, `documents`, and `full`. Default has
  shell, certificates, git, curl, ripgrep, fd, jq and less. Documents adds
  Bun, AnyDoc, Poppler and its local-reading skill. Full adds gh, rtk and
  ponytail. Harnesses retain their existing lazy pinned installation.
- Optional profiles build independently. Fixed shared recipe fragments
  form complete Containerfiles; there is no profile dependency scheduler
  or general package registry. Download only assets needed by the target
  architecture and keep downloaded archives out of the final image layers.
- A user profile overrides a built-in name as a whole. A named built-in
  copy bypasses that override explicitly. Keep one stable seeded pi
  registration pointing at the selected profile's live package; switching
  profiles in one project must work without overwriting personal settings.
- Loading configuration/profile metadata performs no extraction. Actual
  preparation owns filesystem writes and cleanup.
- Host-derived effective configuration remains available without a
  runtime; unavailable image observations are represented explicitly.
- Pinning, checksums, offline execution and release verification apply to
  every supported optional profile.
- The Durable prototype stays outside the Rust binary. Its contract must
  state any limitations instead of claiming full ExecutionEnv conformance.
  Checkpoints stay on the host; model-directed operations stay in boxes.

## Work groups

| Group | Work | Acceptance |
| --- | --- | --- |
| Profiles | Built-in variants, pure metadata, live pi resources, explicit copy/build behavior, updater fragments | Existing project switches default/full/default; selected tools and skills match; optional documents work offline on a fresh image; overrides and built-in copies stay distinct |
| Input and trust | Nested env unknown fields, non-UTF-8 overrides, bounded project IDs, protected-dir cleanup, writable XDG overlap | Real CLI reproductions; extend guarantees 11/12/16/17; no secrets printed or user settings rewritten |
| Ownership | Partial claims, concurrent claim/prune/down, cross-state roots, live project state preservation | Reproduce races where feasible with real CLI/runtime; hold ownership through destructive actions; replacement/live work survives |
| Proxy and login | Write cancellation, relay half-close, visible audit failure, bounded/reaped helper, host-wide login serialization | Host fixtures and real boxes; preserve positive controls and shared tunnel activity; no helper stubs |
| Runtime and images | Guest/client env separation, actual image identity, bounded build output, platform-limited base refresh | Both real runtimes; exact guest env with unchanged host runtime selection; image identity agrees with observed contents; measured resource evidence |
| Downloads and config | One secure downloader; runtime-independent config observations; accurate doctor behavior | Existing harness/update flows, checksums, atomicity, no inspection writes |
| Tests | Update bytes/race/timing, HTTP body control, image attribution, actual guest-home separation, bounded harness waits and clean env | Strengthen existing guarantee tests; no retries, runtime stubs or test-only binary flags |
| Durable | Small external pinned adapter/prototype with recovery and explicit cancellation semantics | Crash around a mutating command; stop old work before recovery; no unsafe automatic replay; no host tool access |

Each source-level suspicion is an investigation until a CLI reproduction
or an explicit new guarantee justifies its implementation. A proposal that
does not survive that check is recorded with its rejection reason. Do not
force a change merely to clear the review list.

## Execution and review

The coordinator owns contract edits and integration. Implementers receive
disjoint file scopes where practical. Independent Sol 6.1 reviewers do not
review their own implementation. A reviewer checks the actual final diff,
spec, failure proof and observations. Blocking findings are repaired and
reviewed again before landing a group.

Run formatting and targeted compile/lint checks while iterating. The
coordinator runs container tests serially with respect to other suites,
preserving the shared builder and unrelated workloads. The real Mac suite
and both Linux CI suites are required before declaring the candidate
verified. Release builds and the whole-tree cleanup/spec pass are required
before a release. Do not bypass failed gates.

The five-minute proxy idle contract cannot fit an entire five-minute test
budget. Define a real slow validation path or another explicit production
contract; never add a test-only timeout override. Performance changes need
before/after observations, not line-count arguments.

Final evidence records changes, rejected hypotheses, exact tested commit,
runtime results, remaining limitations, and the Durable prototype's actual
interface. Installed user state is not deleted as part of accepting a
breaking release.
