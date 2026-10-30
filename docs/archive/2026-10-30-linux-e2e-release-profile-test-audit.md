# Linux e2e release profile and test audit, 2026-10-30

This change starts from `a314baf243b4d0037c39ed1a93ad06474724cafe`.
Its Ubuntu suite passed 26 tests, failed none and ignored one. The body took
358.53 seconds; the wrapper took 361.893367932 seconds with e2e_exit 0. The
unchanged 300-second budget failed. The bootstrap job passed. The preceding
c6cc1be wrapper took 353.927805619 seconds. These separate whole-suite runs
show no evidenced gain from the last fixture change. Fixture trimming stops.

On Linux only, the e2e helper adds Cargo's ordinary --release option beside
its existing musl target. Cargo's JSON compiler-artifact message still selects
the actual executable. Mac selection and the test harness's Cargo test profile
are unchanged. There is no custom profile, environment selector, stripping,
alternate binary or test-only runtime flag.

The main CI workflow's existing AppArmor prebuild uses the same --release
option. It generates and loads the profile from
target/x86_64-unknown-linux-musl/release/pinfold, matching the executable Cargo
will report to the Linux helper. The sysctl check still requires the kernel's
unprivileged user-namespace restriction to be 1. G27 still copies that binary
outside its exact profiled path, observes userns-apparmor refusal and shows
the profiled binary reaching ready. The update fixtures still copy the
selected binary and derive their replacement from its bytes.

The CLI was already prebuilt outside the timer for AppArmor. This changes the
profile in that same step. The helper's Cargo query remains inside the measured
suite. No harness precompilation moves outside the timer. e2e-budget.py, its
timer boundary and its 300-second limit are unchanged. The separate slow-proxy
workflow explicitly retains its debug binary; this does not make all gates
release-profile gates.

Source inspection finds current_exe copied into each agent VM's init disk in
runtime/firecracker/vm.rs and into each image job's init tree in image/linux/job.rs.
The operator retained older d6c4b59 measurements of 79,158,648 bytes for the
debug CLI and 7,080,264 bytes for release. Those are prior-head observations,
not current sizes, an isolated cost measurement or a savings prediction.
The sole explicit debug_assert in production source checks an internal run-ID
invariant in runs.rs. No trust-boundary validation or security gate is changed.

## Candidate audit

All guarantee rows, tests, assertions, sabotage comments, outside expected
values, fault observers and positive controls are unchanged. In particular,
G27's real AppArmor refusal/readiness pair and G28's real update fixture remain
their existing hard cases. No new assertion block or guarantee is needed, and
no existing proof is deleted. The helper changes one line; the workflow changes
two. Binary source changes are zero. The guarantees table needs no edit.

No runtime source, artifact, pin, dependency, cap, resource policy, protocol or
fixture changes. The worker performs source review only, with no formatting,
compilation, tests, CI push or VM use. The operator owns formatting, lint,
platform checks and real release-binary Ubuntu validation, including the exact
AppArmor path and full-suite budget. No new pass or speedup is claimed. Runtime
merge, a CLI release, numeric approval and a budget waiver remain unapproved.
