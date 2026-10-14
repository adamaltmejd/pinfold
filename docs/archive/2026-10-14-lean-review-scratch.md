# Scratch base review — 2026-10-14

Approved for runtime gates. No blockers in the actual uncommitted `core/image.rs` and ARCHITECTURE.md diff at HEAD `01563845f9b915c2495301e45e986e58630f7e6d` in `/Users/adam/.codex/worktrees/pinfold-lean-implementation/pinfold`. Main authored the candidate; I reviewed it independently. Untracked investigation documents were untouched.

`crates/pinfold/src/core/image.rs:253` returns `Ok(None)` for exact bare lowercase `scratch` before either runtime lookup or image pull. This is the virtual empty base, so it has no registry digest. It leaves actual native image building and validation to the builder. Existing first-FROM parsing, unresolved-variable/platform handling, local Pinfold image resolution, floating-tag refresh, explicit platform selection and Apple's native-platform default remain byte-identical. Tagged or qualified references (`scratch:tag`, `library/scratch`, registry paths) and differently cased arguments are not caught by this exact branch. No timeout, retries, concurrency or dependency changes.

Primary sources support the narrow case:

- [Docker base image documentation](https://docs.docker.com/build/building/base-images/) says the reserved `scratch` base starts the first filesystem layer and cannot be pulled, run or tagged.
- [BuildKit convert.go](https://github.com/moby/buildkit/blob/master/frontend/dockerfile/dockerfile2llb/convert.go#L559-L566) compares the base token directly with `emptyImageName`, defined as `scratch` at line 42, then selects `llb.Scratch()` and skips image resolution. This is exact equality, not a suffix or case-insensitive name test.

`ARCHITECTURE.md:525-529` adds the current contract: the empty scratch base has no digest and is not pulled. This agrees with the existing base-label meaning and the runtime's builder semantics; it weakens no control. Comments now correctly distinguish the virtual base from other unresolved references.

Whole image.rs was read, including both callers: caller image builds at line 201 and profile builds through `cli.rs:891`. Returning no base digest still results in the existing empty `dev.pinfold.base` label; family labels, Containerfile fingerprints, build output, retention and image identity remain unchanged.

G15's existing fixture `box_.rs:2028-2044` builds exact `FROM scratch` repeatedly, observes runtime images and controls retention with real live boxes. Its later native scratch build at lines 2132-2156 remains an independent runtime fixture. No assertions were changed. This existing CLI path reproduces the reported problem, so no new guarantee, synthetic timing assertion or test seam is needed for this fix.

Limitations: source review only, no build, test or runtime command run by this reviewer. The 45-minute stalled pull and successful continuation after main stopped that pull are evidence supplied by main, not independently reproduced here. They establish an unnecessary native operation in the existing test path; they are not a comparative performance benchmark. The assisted first full-suite run is not a gate. Main must rerun the full suite without intervention after the fix and preserve the existing platform/build gates. This review does not claim other real-base pulls are bounded or safe to interrupt.
