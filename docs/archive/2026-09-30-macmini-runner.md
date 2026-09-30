# Macmini runner readiness, 2026-09-30

GitHub runner `Macmini` is online, on Apple Silicon, with labels
`self-hosted`, `macOS`, `ARM64`, `pinfold-nightly`. Its service runs as
`adam` on macOS 27.0.1. Automatic pin releases remain disabled: the
`PINFOLD_AUTO_RELEASES` variable is unset, and main still has unreleased
implementation changes since v0.0.9.

## Setup

The first real runner check found Homebrew and Apple `container`, but no
Cargo, rustup, Zig or cargo-zigbuild. The manual setup mode installed the
missing Homebrew formulas and Rust 1.98.1, including rustfmt, Clippy and
`aarch64-unknown-linux-musl`. Verified versions: Zig 0.16.0,
cargo-zigbuild 0.23.4, container 1.5.0. The container service runs as the
same user as the Actions runner.

Manual setup and checks are separate from automatic pin updates:

```sh
gh workflow run bump-pins.yml --ref main -f check_runner=true -f install_tools=true
```

Once tools are installed, omit `install_tools`:

```sh
gh workflow run bump-pins.yml --ref main -f check_runner=true
```

Both modes skip pin preparation and release. The suite runs through
`rustup run` with the project's pinned toolchain. No additional login
was needed: the runner user's existing Codex login passed the live test.

## Container networking

The first suite built the real binary, but image dependency downloads
failed on DNS timeouts at the default gateway, 192.168.64.1. It was not a
passing suite: 2 tests passed and 25 failed. A disposable container could
resolve api.github.com using the existing LAN resolver, 10.0.40.10.
Restarting the idle builder with that resolver restored dependency
access; its cache was retained.

Mullvad's app and daemon were already absent, but Homebrew retained an
old cask record. Forced removal reached privileged cleanup; Adam ran
`brew uninstall --force --cask --zap mullvad-vpn` locally with his admin
password. Afterwards, Homebrew no longer listed Mullvad, and checked app,
CLI, daemon, cache and settings paths were absent. Tailscale and Little
Snitch were retained.

After cleanup and a normal builder stop/start, the builder returned to
its default resolver, 192.168.64.1. HTTPS probes from the builder to
api.github.com and registry.npmjs.org succeeded. The temporary LAN DNS
override is no longer needed. This establishes working networking after
cleanup; it does not isolate which stale VPN state caused the original
failure.

## Verification

- [Full Mac runner check](https://github.com/adamaltmejd/pinfold/actions/runs/36675036110)
  passed all 27 tests, including the live Codex tool round trip, in
  247.48 seconds, at commit 6756684. That suite used the temporary LAN
  builder resolver. The later default-resolver probes passed separately.
- [Linux CI](https://github.com/adamaltmejd/pinfold/actions/runs/36672298251)
  passed on x64 and arm64 for the same commit.
- Actionlint and diff checks passed for the workflow changes. The initial
  readiness typo, `cargo zigbuild --version`, was corrected to
  `cargo-zigbuild --version` after the real runner rejected it.
- No pins were bumped and no release was published during these checks.
  The remaining activation prerequisite is a manually reviewed release
  baseline; no Mac installation or login prerequisite remains.
