# v0.1.0 release cleanup and spec pass

The whole-tree pass read every Rust module, the test harness and all test
files, the profile, README, CLI help, binding spec, and release workflows.
The dependency list matches the spec. Clippy has no dead-code warnings.
The only XXX search hits are literal Public Suffix List data entries.

Metrics below describe candidate `98805e2`, before adding release evidence.
The before column is the landed baseline `b76ea41`. Release ratios compare
the candidate with `v0.0.9`.

| Metric | Before | After |
|---|---:|---:|
| Rust lines | 12,700 | 12,789 |
| Largest Rust file, box_.rs | 3,304 | 3,389 |
| Largest binary source, cli.rs | 1,136 | 1,136 |
| ARCHITECTURE.md lines | 852 | 855 |
| Guarantee tests | 27 | 27 |
| New written finding tickets | 0 | 0 |

The release deletes/adds 119/274 binary Rust lines: ratio 0.434.
All Rust deletes/adds 252/726: ratio 0.347. All text deletes/adds
1,289/18,197: ratio 0.071; this includes the 16,501-line pinned PSL.
The cleanup itself proposed and landed nine deletions. The increase is
the issue #77 concurrency witness and CI runner correction.

## Landed findings

- `0d2e8ff`: remove the unset image-retirement label parameter and its
  speculative profile/project path, including the sole CLI caller.
  Net seven lines removed; no assertion changes.
- `ae3740f`: remove the profile's restating privilege-strip comment.
  Its chmod control remains. One line removed.
- `e8ad2e3`: remove the future Windows aside, correct the box-spec link,
  name login.rs in the source map, and show repeated/presence-only box
  list filters in help. Net one line removed.

All other modules were already lean. No dependency, control, feature or
guarantee was removed. No new finding needed a rework ticket. Existing
Y-74 remains in the Yard-v3 handoff, outside the retired Yard integration.

## Test audit and issue #77

No pre-existing assertion block failed the policy bar. Rows 15, 17 and 27
retain their positive controls, outside observations and named sabotage.
All configuration keys map to spec lines and test settings; all 27 rows
map to tests. Unexercised CLI spellings remain lookup observations, not
syntax-only tests.

`d2ebbcc` removes both color variables from Apple build subprocesses.
Row 23 now holds a real build inside RUN while a caller in another state
directory builds with opposite color settings. Both must succeed and
their refs must exist in the runtime. Restoring either inherited color
variable is the named sabotage. No new test function or builder mutex.
The held HTTP fixture is shared with row 15; its assertions remain in
their respective tests. Existing dead-box fixture protection covers only
initial maintenance, not the concurrent builds.

`98805e2` gives the Mac suite a stable executable path for its firewall
exception. Cargo metadata supplies the build directory when that test
executable is relocated. No runtime stub, binary flag or test-only path
was added to pinfold.

## Spec pass and rejected findings

README and the spec state the current contract. The guarantees table is
reconciled with all 27 tests, including row 23's caller-color case. None
of the four open questions is resolved; CI cache cleanup is not a runtime
disk cap. No spec/control deletion needed operator approval.

Retain previously rejected optional-login tuple changes, DNS streaming,
mount/TLS/address guard removal, seed-walk replacement, config path and
route-name guards, cache repair, git merging and operating-context
splitting. Reconsider only with the earlier admission evidence. No
measured cost justifies a speedup. The unused-label finding's previous
condition is now satisfied by its mechanical caller update.

## Validation

Candidate `98805e2` passes all 27 Mac tests in 235.89 seconds, including
live Codex login, both held-build cases, and pre/post-job cleanup.
[Mac gate](https://github.com/adamaltmejd/pinfold/actions/runs/36690309990).
Linux x64 passes 27 in 113.31 seconds; ARM64 passes 27 in 91.63 seconds.
[Linux gate](https://github.com/adamaltmejd/pinfold/actions/runs/36690306004).

Focused Mac cleanup passes in 78.08 seconds; the caller-color regression
passes in 29.09 seconds. Formatting, Clippy with warnings denied,
Shellcheck, shfmt, actionlint and diff checks pass.

The initial pin candidate's Mac run passed 26/27 but timed out at the
new gateway-bound fixture. A direct reproduction also failed. Macmini's
firewall was enabled; Adam allowed the stable CI executable, leaving the
firewall enabled. The corrected runner and focused/full suites then
passed. The first wrapper attempts exposed its relative-path and
executable-layout assumptions; both were fixed before the final gate.

The pin-update run checked pi, Claude, Codex, Bun, RTK and Ponytail.
Claude moved 2.1.284 to 2.1.285 and Codex 0.159.0 to 0.159.2; the other
pins were current. The workspace minor version is 0.1.0.
