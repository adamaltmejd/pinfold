# Nightly pin releases, 2026-09-30

The nightly workflow covers the existing bump script's pins: pi, Claude,
Codex (including its host helper), Bun, RTK and Ponytail. Rust, Cargo
third-party dependencies, Actions and the Public Suffix List are excluded.

It is scheduled for 02:23 UTC and supports manual dispatch. It does
nothing until the repository variable `PINFOLD_AUTO_RELEASES` is `true`.
No runner or login was installed and that variable was not enabled.

## Activation prerequisites

1. Complete a manual release of the current implementation after the
   repository's cleanup, test audit and spec pass. Nightly pins inherit
   that audited baseline. The workflow refuses a changed production tree
   (`crates`, `profile`, workspace manifests/lock and Rust toolchain) since
   the latest published release. Current main has unreleased changes
   since v0.0.9, so it cannot serve as the initial nightly release.
2. Provision a dedicated Apple Silicon Mac on macOS 26+, with Apple
   `container`, Rust/rustup, Zig, cargo-zigbuild and the Linux arm64 musl
   target. Run its Actions runner as the user whose container service is
   running. Give it labels `self-hosted`, `macOS`, `ARM64`, `pinfold-nightly`.
   Restrict its runner group to this repository. Do not share its
   container builder with another project's tests.
3. Authenticate Codex as that runner user. The suite uses `CODEX_HOME` or
   that user's `.codex` directory for its live-login scenario. Keep the
   login on the runner; do not put credentials in repository variables,
   artifacts or workflow output. Confirm `cargo test -p e2e --locked`
   succeeds as that user, including the live tool round trip.
4. Set `PINFOLD_AUTO_RELEASES=true` only after those checks. Dispatch
   `bump-pins.yml` for the first full run and inspect its result.

## Publication

The updater verifies harness downloads against publisher digests. No pin
change means no version bump or release. A changed candidate increments
only the workspace patch version and the workspace lock entries. Its
unique temporary branch supplies an exact commit to both Linux suites,
the dedicated Mac suite, and all three release builds.

After all checks and builds pass, the release job requires main still to
be the candidate's parent. One non-forced atomic push promotes main and
creates the annotated tag. Existing tags or concurrent main changes abort
it. Notes contain `Caller changes: None`; unreleased production changes
are blocked rather than silently receiving those notes. Candidate
branches are removed after success or failure.

The nightly workflow calls CI and Release directly. A push made with
`GITHUB_TOKEN` does not trigger their push events, so relying on a tag
push alone would not publish. No personal access token is needed.
See [GitHub's reusable workflows documentation](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows).

If publication fails after the atomic push, rerun the failed release job
or dispatch Release on the existing tag:

```sh
gh workflow run release.yml --ref v<VERSION>
```

No-change nights do not retry an already tagged release.

## Verification

- Actionlint passed with an explicit local runner-label list, including
  the existing Ubuntu 26.04 runners and the dedicated Mac label.
- Format and diff checks passed.
- The actual inline patch-bump code ran against a temporary copy of the
  real repository. Only workspace versions changed; Cargo accepted the
  lockfile with `--locked --offline`.
- The actual promotion script passed scratch Git-remote checks: atomic
  success, main advancing before or during push, an existing tag, and a
  candidate with the wrong parent. Failure cases left the remote unchanged
  by the candidate.
- No permanent tests were added: this changes release orchestration, not
  a binary guarantee. The full nightly path needs the dedicated runner
  and its live login before it can be exercised.
