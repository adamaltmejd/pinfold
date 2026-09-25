---
name: code-cleanup
description: Audit pinfold's whole tree for yagni, duplication, wrong-altitude fixes and unmeasured cost, then land the deletions — directly, one commit per module, or as a Yard ticket when a change is a rework. Hands every control-touching or feature-removing finding to Adam as a spec question. Load this when asked for a cleanup, yagni, simplification or consolidation review of pinfold, and before every release, where it is followed by the spec pass and the tag.
---

# /code-cleanup

AGENTS.md says the tree stays lean; this file is the recipe. Run it before
a release and whenever the tree feels fat. Preconditions: main is green,
`yard status` shows no open lane (the Mac suite and a merge-queue gate
share one image store), and the operator has the sitting.

The pass reads the whole tree, not a diff. A diff-only review finds little
here and was rejected as a review seat on 2026-09-24. Do not re-propose one.

## 0. Measure

Take the numbers before anything moves, so the report can show what the
pass changed. Repeat at the end.

```sh
last=$(git describe --tags --abbrev=0)
git log --oneline "$last"..HEAD | wc -l
git diff --shortstat "$last"..HEAD
git diff --shortstat "$last"..HEAD -- crates
wc -l $(git ls-files '*.rs') | sort -rn | head -5
wc -l docs/ARCHITECTURE.md
yard ticket list --status done --json
```

Tickets landed is the count of done tickets above the last report's
high-water mark. Proposal-born tickets are the ones `yard proposal accept`
created since the last report. The suite's wall time is in the last
`e2e-macos` gate log.

## 1. Mechanical checks

These answer without judgment. Run them first.

- **Dependencies.** `cargo tree -p pinfold --depth 1 -e normal` against the
  list under `## Code` in ARCHITECTURE.md. A crate in one and not the other
  is a finding.
- **Dead and over-public code.**
  `cargo clippy --all-targets --locked -- -W dead_code -W unreachable_pub`.
  Every warning is a finding.
- **Trace tables.** Three, built with `rg`, kept in the scratchpad, by
  one subagent on a small model (the work is lookup, not judgment): every key
  `config.rs` reads, to its line under `## Configuration`, to the test
  that sets it; every verb and flag `cli.rs` parses, to its line under
  `## CLI`, to a test; every guarantee row, to its test. A row with a gap
  is a finding: code without a spec line is yagni, and a spec line without
  a test is either an untested guarantee or a line to cut.
- **Deferrals.** `rg -n -i 'TODO|FIXME|XXX|revisit' crates share` and the
  `## Open questions` list in ARCHITECTURE.md. A TODO is a violation
  (follow-up work is a ticket). A revisit trigger that has fired, or one
  that names no trigger, is a finding. An open question the suite now
  answers is closed in the spec pass.

## 2. Read, one agent per module, in parallel

Do not read the modules yourself. Launch one subagent per module through
the harness's subagent tool (in Claude Code, the Agent tool with the
`general-purpose` type; the Explore type locates code and does not audit
it), all in one message so they run concurrently, told to edit nothing.
Set the model on every call: a subagent that names none inherits the
session's, and the session's model is too costly for a read. The code
modules take the family's mid tier, the one below its largest model; the
docs take a small model. Modules:
`cli`, `core/box`, `core/runtime` (all three files), `core/proxy`,
`core/clean` with `core/image` and `core/artifacts`, `config` with
`dirs`, `trust` and `init`, `pi/`, the e2e crate, and the docs
(ARCHITECTURE.md, README.md, `share/`).

Each prompt carries: the module's file paths; an instruction to read
ARCHITECTURE.md and AGENTS.md first and the module's files whole; the
step 1 findings that touch it; and the rest of this section verbatim,
from "The agent hunts" to the closing `Lean already.` The agent hunts
what the spec does not ask for, what the platform already does, and what
is said twice. Its best outcome is a shorter module.

One line per finding, no hedging:

`<file>:L<line>: <tag> <what to cut>. <replacement>. [spec: <line or none>] [-<N>]`

Tags:

- `delete:` dead code, an option nothing sets, a fallback for a state the
  spec says cannot occur, a comment restating the code. Replacement: nothing.
- `yagni:` a trait with one implementation, a helper with one caller, a
  layer that only delegates, a compatibility path. Inline it.
- `stdlib:` a hand-rolled thing `std`, `tokio` or `nix` ships. Name it.
- `native:` code doing what the runtime or the platform already does
  (podman, Apple `container`, the kernel, git). Name the feature.
- `dup:` the same logic in two adapters, two modules, or an e2e helper
  copied into a test; one thing the spec states twice. Name the survivor.
- `shrink:` same logic, fewer lines, including a special case on a shared
  path that a general change to the mechanism removes. Show the form.
- `spec:` a cut that touches a control (the Always applied list, the proxy
  rules, the threat model's controls, trust), removes a feature, or deletes
  a guarantee row. Not the agent's call; it goes to Adam.

Two more for the e2e crate, where the unit is the assertion block and the
bar is the one in AGENTS.md (a spec line, an outside observer, an expected
value from outside pinfold, no twin):

- `taut:` an expected value from pinfold's own output or a copy of its
  logic. Name the outside source, or delete.
- `easy:` a scenario that is the guarantee's simplest case. Name the
  harder one.

A change-detector assertion (wording, layout, a count that is not the
guarantee) is `delete:`.

Examples, invented to show the shape:

`core/runtime/apple.rs:L210: dup: image inspect parsed twice, once per adapter. Keep runtime/mod.rs::inspect. [spec: none] [-31]`

`config.rs:L140: delete: retention.keep_failed read, set by no test and no spec line. Nothing replaces it. [spec: none] [-18]`

`core/proxy.rs:L402: spec: the literal-IP refusal also covers link-local v6. Cut would weaken proxy rule 3. [spec: L338] [-9]`

A speedup is a finding only with a before-number. The numbers that exist:
suite wall time per test file from the gate log, image build time, binary
size (`docs/archive/2026-09-24-injecting-routes-binary-size.md`).
Without one it is dropped, per AGENTS.md.

Out of scope: correctness bugs (a bug goes through the CLI and gets its
own ticket), test assertions, log and help wording. A test's positive
control is never bloat.

The agent ranks its list biggest cut first and ends with
`net: -<N> lines, -<M> deps possible.` Nothing to cut: `Lean already.`

## 3. Judge

Merge the lists, dedup findings on the same mechanism, keep the ranking.
Then sort each into one of three piles.

- **Land it.** It fits the admission rule: one module, no test assertion
  changes, and it names what it removes. Group by module.
- **Adam's.** Every `spec:` finding, and anything an agent mistagged that
  touches a control, removes a feature, or deletes a guarantee row. These
  are spec changes. Put them in one list with the spec line each would
  change and the reason, and stop there until he answers. A yes lands as
  an ARCHITECTURE.md edit in the same commit as the deletion.
- **Rejected.** Everything else, each with the condition that would re-admit
  it. This list goes in the report; it is what stops the same finding
  returning next pass.

## 4. Land

Edit directly by default. The audit's context is in this session, and a
lane would spend a container and review rounds rediscovering it. One
commit per module, the message naming what was removed. Before each push:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test -p e2e --locked
```

The Mac suite runs here; CI runs the Linux suite on the push. A commit
that touches a test body reruns that test's named sabotage first, and a
sabotage that no longer bites is a finding of its own.

A Yard ticket instead, filed `--parked` through `yard-file` with
`--workflow heavy`, when the change is a rework: it redesigns a module's
internals rather than deleting from them, or it is more than one sitting.
The review seat and the merge queue are what the ticket buys.

## 5. Report, and at a release

Repeat step 0. Write `docs/archive/YYYY-MM-DD-code-cleanup.md`: the numbers
table with before and after columns, the possible net from step 2 beside
the landed net, each commit or ticket and what it removed, Adam's list
with his answers, and the rejected list with its conditions.

Before a release, the same file then carries the spec pass from AGENTS.md.
Then bump the workspace version, tag `v<version>` and push the tag; the
release workflow refuses a tag that does not match the version.
