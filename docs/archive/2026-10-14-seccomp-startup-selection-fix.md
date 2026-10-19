# Shared seccomp policy selection fix

2026-10-14. The operator authorized fixing the reproduced startup race and
paused the earlier policy-attestation request. Yard will rely on pinfold's
documented contract. No caller reporting API is added.

## Change

Podman's derived profile now lives in the claimed box state directory.
Core passes that existing directory to the runtime adapter. Apple ignores
it. The profile is completely written before Podman starts. The existing
name claim keeps its path exclusive until runtime removal; existing owner
cleanup then removes the state directory.

The shared seccomp cache, temporary PID-named file and rename are removed
from this path. The host profile transformation, namespace restrictions,
keep-id, exit status and teardown behavior are unchanged. No dependencies,
CLI flags or JSON fields are added.

ARCHITECTURE.md states independent startup selection and adds guarantee 37.
The implementation and contract change belong together. Unrelated dirty
changes in this checkout were retained.

## Regression and test audit

`linux_boxes_apply_their_own_seccomp_profile` uses two real CLI owners with
different process-local Podman configurations. Both source profiles are
host FIFOs. Writer rendezvous places both owners after preflight and
preparation, before derivation finishes. The test then holds the real
Podman image-store POSIX lock, supplies the first profile and observes its
runtime lock waiter, then does the same for the second. Releasing the lock
lets both consumers proceed after both profiles have been derived.

The normal profile allows raw `getppid`. The second profile additionally
returns `EPERM` for it. A static C fixture executes the raw syscall inside
each box; raw `getpid` succeeds in the stricter box as its positive control.
The fixture compiler, musl-gcc, is already installed by Linux CI.

The candidate passes test-audit's four questions:

- Guarantee 37 covers policy selection. Guarantee 5's privilege controls
  are common to both profiles and cannot catch this mismatch.
- The test comment names restoring the shared derived pathname as its
  sabotage. With both consumers held, the first box then consumes the
  second profile and its allowed-syscall assertion fails.
- This forces the reproduced overlap before consumption. It does not
  depend on a favorable scheduler race, sleeps or timing retries.
- Expected return values and errno come from the supplied profiles and
  Linux syscall semantics. Readiness comes from host FIFO and kernel lock
  observations. No derived filename, argv shape, OCI JSON or BPF digest is
  asserted.

Read-only review found no blocking issue. Its one spec correction removed
a duplicated normal-teardown claim from row 37; lifecycle teardown remains
guarantee 9's responsibility.

## Validation

Format, Clippy and whitespace checks pass. Clippy ran on macOS and Linux
with `--all-targets --locked -- -D warnings`.

The deterministic Linux regression passed: 1 passed, 0 failed, 31.77 seconds
including the candidate build and default image build. It used Podman 6.1.4
and an isolated native-overlay image store on yard-sthlm. The image-store
lock fixture therefore could not block Yard's operational store.

The macOS suite passed: 33 passed, 0 failed, 1 ignored, 202.91 seconds.
The first sandboxed attempt could not access Apple container and was not a
runtime validation. The complete real-runtime run used normal host access.

The full Linux x64 suite passed: 34 passed, 0 failed, 1 ignored, 314.81
seconds. This isolated LXC host run does not qualify the separate
five-minute CI runner budget. Linux arm64 CI was not dispatched.

Remote source, compiler and logs are retained at
`/home/yard/validation/pinfold-seccomp-fix-20261014/`. The Linux compiler's
Debian musl packages were extracted into that validation directory;
no system packages or installed pinfold executable were replaced. The
macOS suite log is `/private/tmp/pinfold-seccomp-macos-e2e.log`.

No installed pinfold or Yard service was updated, and no public posting,
push or release was performed.
