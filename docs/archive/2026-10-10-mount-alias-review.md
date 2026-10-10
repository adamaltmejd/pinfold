# Y-12 operator clarification and proof repair

The operator identified a global runtime observation in the Pi admission
scenario. The cleanup assertion now selects `dev.pinfold.project` with the
fixture's existing `project_id` helper. Other legitimate Pinfold and Yard
boxes do not affect it. The state observation remains in the fixture's
private TestEnv. No runtime cleanup was broadened.

The binding lifecycle prose and API comments now name the late refusal
precisely. `mount-alias` occurs after preparation, before runtime spawn or
ready. Existing failed-start cleanup removes claimed state. Pi's Git guard
removes newly created empty protection directories. Fetched artifacts and
ordinary seeded files may already exist; they are not rolled back. Existing
early refusals and guarantee 17 retain their meaning. Production scan and
cleanup behavior did not change.

The earlier admission archive records copied-function worker timings and
fixture entry counts. Those timings do not satisfy the requested matched
baseline/candidate startup measurement. The operator's real Mac fixture
measurement remains pending. The Mac and both Linux suites also remain
host landing checks. No worker measurement was rerun for this repair.

Execution 82 failed proof collection because the transient copied
`y12-cost/init/pinfold` exceeded the 67,108,864-byte proof limit. The repaired
`/yard/proof/y12-cost` retains only `scanner.rs`, `cost.rs`, `commands.txt`,
`result.txt` and `result-debug.txt`. Commands describe the original inputs,
compilation and timing calls. The result files retain original driver
output, including counts and timing summaries; individual iteration samples
were never written.

The copied executable, compiled timing drivers and generated fixture trees
were removed from that proof directory. The earlier archive's description
of retained fixtures refers to the bundle before this packaging repair.
No actual Pinfold runtime artifact, Yard-owned state or clone was removed.
The compact evidence remains reproducible from its recorded commands and
driver, with build-tree counts dependent on the original workspace state.
The repaired bundle totals 11,077 bytes.

Worker validation uses `CARGO_HOME=/usr/local/cargo`: offline locked build,
format check and offline locked all-target clippy with warnings denied pass.
Runtime suites remain pending on the host.
