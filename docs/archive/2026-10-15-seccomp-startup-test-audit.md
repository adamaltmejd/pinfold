# Seccomp startup selection test audit

2026-10-15. Isolated candidate based on main
`a1be9213efc3585d75bead7f2c2de4073167fc7d`.

## Scope

Each Linux startup writes its derived seccomp profile into its claimed
box state directory. Existing ownership keeps that path exclusive through
runtime removal and cleans it afterward. The shared output path and its
atomic replacement are removed. Policy transformation and all containment
controls are unchanged. No caller API, flags, JSON fields or dependencies
are added. The separate policy-attestation request remains paused.

The patch includes the implementation, the binding contract and guarantee
37, one Linux end-to-end regression, and this audit. The original dirty
checkout was left intact. Read-only extraction review found no blocking
issue and no unrelated changes.

## Assertion audit

- Row 37: concurrent startups apply their own selected host policy.
- Sabotage: restore a shared derived-profile pathname. Both real Podman
  consumers wait until both policies have been derived; the first then
  consumes the second's restriction and fails its allowed-syscall assertion.
- Existing row 5 only checks privilege controls common to both profiles.
  Two FIFO source rendezvous and the real image-store lock force the
  overwrite window before consumption. No sleeps, retries or runtime stubs.
- Raw syscall return values and errno come from the host fixture's policy
  and Linux semantics. The stricter box also runs an allowed raw syscall.
  FIFO peers and kernel lock waiters provide fixture readiness. No derived
  filename, argv shape, BPF digest or implementation layout is asserted.

The test adds 217 lines, including its host fixtures, alongside the small
runtime change. No existing assertion block is changed or deleted.

## Validation

The earlier working-tree candidate passed format and Clippy on macOS and
Linux, 33 macOS tests and 34 Linux x64 tests, with one slow gate ignored
per platform. The deterministic Linux regression passed independently.
Those results precede this extraction onto current main.

The extracted candidate passed `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, and the full macOS
suite: 33 passed, 0 failed, 1 ignored, 288.43 seconds. Linux x64 and arm64
run in the PR workflow. No installed binary or service was changed.
