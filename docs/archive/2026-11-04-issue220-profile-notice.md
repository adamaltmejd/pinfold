# Issue 220: bundled-profile advice

The notice points to the existing `image-outdated` rebuild command and
explains that `profile new --from` copies the effective user-or-bundled
profile. Fingerprint changes alone do not imply changed image inputs.

A normal binary copied outside Cargo's development path ran through a real
PTY with isolated XDG roots and a recent, old-fingerprint update record.
The notice appeared before the fixture's Git-metadata refusal, then stayed
silent on the second invocation. Profile copies matched the bundled recipe
sources without an override and a literal user recipe with one; the user
recipe remained unchanged. Independent correctness/lean review, format and
locked all-target Clippy passed. No test assertion changed.
