# Yard v3 handoff, 2026-09-29

Y-74 was the remaining deferred ticket when the project retired Yard v2.
It is saved here for admission under Yard v3. It has not been implemented.
Its old workflow was `heavy`; it was parked, waiting on Switchyard issues
[122](https://github.com/adamaltmejd/switchyard/issues/122) or
[123](https://github.com/adamaltmejd/switchyard/issues/123). Those v2-specific
prerequisites and paths need reassessment against v3 before admission.

## Saved ticket: Y-74

Sabotage as a merge-queue gate: the candidate carries a patch, a host gate applies it

Waiting on Switchyard: adamaltmejd/switchyard#122 (gates learn the candidate's base and changed paths) and #123 (a proof directory the worker writes and gates read, never merged). Decided 2026-09-25 with Adam: sabotage is an authoring-time proof that the merge queue verifies, never an operator step, and nobody maintains sabotages after they land. Unpark when either issue ships in the Yard release this project runs; #123 is the cleaner base, #122 is enough.

## Mechanism

AGENTS.md Tests rule 3: a test's comment names the change to the binary that makes it fail. Nothing runs it. Evidence for keeping it: 2 of ~10 fresh lane tests did not bite under sabotage (Y-69, Y-70), while 15 of 15 reruns on assertion-preserving edits did (docs/archive/2026-09-25-code-cleanup.md, "Sabotage reruns").

## Consumer

The merge queue: a candidate that adds or changes an end-to-end test carries proof that the test can fail, and the gate checks it without an operator.

## Behavior

- A candidate that adds or changes a test carries a sabotage: a patch against `crates/pinfold/src` only, one per changed test, in the proof directory (#123) or under `crates/e2e/sabotage/<test>.patch` (#122).
- `scripts/sabotage.sh PATCH...` applies each patch, builds, runs the one named test, requires it to fail within a timeout, reverts. A patch that does not apply fails the script.
- A host batch gate `sabotage` in `.yard/config.toml` runs the script over the candidate's patches only: the proof directory's contents, or `git diff --name-only "$YARD_BASE" -- crates/e2e/sabotage`. Around 25 s per patch.
- AGENTS.md rule 3 says the candidate carries the patch and the gate applies it; the prose sabotage comment is no longer required. The review seat's instructions gain: a sabotage breaks the mechanism the row names, not the test's entry.
- The `test-audit` skill's approval question 2 asks for the patch, and its "nobody runs it by hand" paragraph is replaced by the gate.
- With #122 only: consumed patches are deleted by the release pass (add one line to `code-cleanup` step 5).

## Proof

The gate itself, on one existing test: convert one guarantee's prose sabotage to a patch and show the gate fails when the patch is made a no-op (the gate's own positive control). No new e2e test.

## Out of scope

Converting the existing ~70 prose sabotages; the Linux runtime for the sabotage gate (the Mac gate is enough); any model on the gate floor.
