---
name: test-audit
description: Judge pinfold's end-to-end tests — gate a candidate that adds or changes a test, and sweep the suite for assertion blocks that fail the bar in AGENTS.md, deleting by default. Load this when a candidate touches a test, or when asked to audit, prune or review tests.
---

# /test-audit

The Tests section of AGENTS.md is the policy. This file is what the
operator does with it, at two moments. The unit throughout is the
assertion block, because a test is a guarantee row.

## At approval: the candidate touches a test

Four answers, from the diff and the ticket. A missing one sends the
candidate back.

1. Which guarantee row. A new test needs a new row; a bug extends its
   row's test.
2. Which change to the binary makes it fail, named in the test's comment.
3. Why the row's existing blocks do not already catch that failure, and
   whether this is the row's hardest case.
4. Which outside source gives each expected value.

Do not run the sabotage by hand. Review the comment's named change to
the binary as the test's failure proof.

## The sweep

Read-only first. One subagent per test file (`box.rs`; `pi.rs` with
`cli.rs`) on the family's mid tier (in Claude, Opus), told to edit
nothing, given AGENTS.md, the guarantees table, `crates/e2e/src/lib.rs`
and the file whole.

One line per block that fails the bar, no hedging:

`<file>:L<a>-<b>: <tag> <what>. <disposition>. [row <N>]`

- `taut:` expected value from pinfold's own output or a copy of its logic.
- `easy:` the row's simplest case. Name the harder one.
- `unrelated:` a refusal that passes for another reason: a timeout, DNS, a
  different guard than the row names.
- `twin:` the same contract asserted in another block. Name the survivor.
- `detector:` wording, layout, or a count that is not the row.
- `promise:` the name or comment claims more than the scenario exercises.
- `seam:` a helper in `lib.rs`, or anything in the binary, that only this
  block needs.

Disposition is `delete`, or `rewrite` when the block is the row's only
proof of a contract. The agent ends with `<N> blocks, <M> to delete.` or
`Lean already.`

## Judge and land

A deletion that leaves a row without proof is Adam's: the row goes, or the
block is rewritten. That is a spec change. Everything else: delete the
block and the helpers it orphans, one commit per test file. A kept block
that fails on main is a product bug: reproduce it through the CLI and
ticket it, never delete it.

Before each push: the suite on the host. Report in the code-cleanup
archive file when run inside that pass, otherwise in
`docs/archive/YYYY-MM-DD-test-audit.md`: each disposition with its row,
and test lines beside binary lines.
