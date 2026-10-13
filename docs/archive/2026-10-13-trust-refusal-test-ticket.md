# Written ticket: distinguish trust refusals in guarantee 12

2026-10-13. Release test-audit finding. Not implemented in v0.2.3.

## Contract and mechanism

Guarantee 12 protects the caller when the agent creates or changes
`.pinfold.toml` or changes the project Containerfile. `trust::check` rejects
changed bytes, but its error has no spec-named reason token. The four
refusal blocks in `a_changed_project_file_stops_the_run` assert unsuccessful
exit only. An unrelated startup guard could satisfy each block.

## Requested observable guarantee

The caller can distinguish a trust refusal through a stable, spec-named
`trust-changed` reason token. Preserve the current trust check, error detail,
exit code, allowed controls and fixed test count. Document the token in the
binding spec and in that release's Caller changes.

Extend the four existing refusal assertions to observe that token on
stderr. The expected refusal comes from fixture-created or guest-modified
files; the allowed controls must still launch or build after `pinfold allow`.
Name removal of trust checking or hashing as the sabotage that fails them.

No product bug is claimed: this finding is from source review, without a
CLI reproduction of a bypass. No new test, hashing rewrite, permission
change, startup change or broader error taxonomy is in scope.
