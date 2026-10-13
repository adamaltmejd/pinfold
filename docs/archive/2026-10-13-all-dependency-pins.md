# Dependency pin updater, 2026-10-13

The nightly updater covered harnesses and profile image downloads. It now also
updates Rust and the matching Yard Rust image, Cargo requirements and lockfile,
the durable example's exact npm pins and lockfile, CI Node and Bun, Yard
TruffleHog, and the vendored Public Suffix List. Cargo and npm requirements
may cross incompatible versions. Stable releases, existing artifact integrity
checks, publication waits, and the release gates remain required.

GitHub Action commits remain manual pins. This change updates checkout to
v7.0.1, setup-node to v7.1.0, upload-artifact to v7.0.2 and download-artifact
to v8.0.2. setup-bun's existing SHA already resolves to v2.2.0; its comment
now names that release. Rust-cache was current. Each SHA was resolved through
GitHub's release/tag metadata, including annotated tags.

Node and Bun pins moved to `.github/tool-versions.toml`. The reusable CI
workflow reads candidate data after checkout. This avoids modifying workflow
definitions during a nightly run or requiring a workflow-writing credential.
The Mac job installs candidate Rust and now runs the prototype typecheck and
durable recovery gate, matching the two Linux gates.

## Validation

- `actionlint` passed, ignoring only its unsupported existing runner labels.
- Ruff lint and formatting, `cargo fmt --check`,
  `cargo clippy --all-targets --locked -- -D warnings`, and whitespace checks
  passed in the working checkout.
- The actual updater ran against publisher endpoints in a scratch copy. It
  selected Rust 1.99.0, verified image/harness and TruffleHog assets, and
  refreshed the Public Suffix List. Crate selection handles stable versions
  with build metadata, including `toml`'s `+spec` suffix.
- Cargo regenerated the scratch lockfile under an isolated Rust 1.99.0
  installation. Bun 1.4.2, downloaded and checked against its publisher's
  SHA-256, regenerated the npm lockfile and accepted a frozen installation.
- The refreshed prototype failed its real typecheck at `prototype.ts:247`.
  Direct and nested `pi-ai` model types were incompatible. This candidate
  would fail CI and cannot reach release. No compatibility fix or dependency
  downgrade was added to conceal that failure.

The scratch pin changes were not applied to the working checkout. Hosted
CI, Mac boxes and release builds were not run for this updater edit. No
repository credential was uploaded, and no commit, push or release was made.
