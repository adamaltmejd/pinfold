# Issue 212: project discovery proof

G13's existing test now runs the real CLI from `.git/objects`, a bare
repository's `objects` directory and a linked worktree's nested folder.
Host Git creates all three fixtures. The existing first A launch is the
main-checkout control. No extra box start or production change is needed.

The added block is `crates/e2e/tests/e2e/pi.rs:259-301` beside
`crates/pinfold/src/pi/context.rs:18-45` and `pi/launch.rs:27-46`.
Removing either metadata recognizer or allowing a `.git` file to fall back
to scratch makes the named refusal assertions fail. Existing blocks only
started ordinary checkouts or folders without Git markers.

Expected reasons come from the Pi contract. The host snapshots state,
including project, run and box ownership records, then compares it after
each refusal. The fresh test environment's disk inventory must stay empty, including
Apple volumes outside that state tree. This is not an independent native
runtime box inventory. Discovery runs before project recording and box
creation. The fixtures exercise ancestor discovery rather than only
starting at each marker's own directory. No assertion checks prose,
private label names or argv shapes.

Independent correctness/security and lean/test-admission reviews passed.
Format and locked all-target Clippy passed. The ordinary Mac suite passed
26 tests, with the existing slow test ignored, in 154.35 seconds.
