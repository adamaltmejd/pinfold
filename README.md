# pinfold

Run a coding agent in a disposable box: an Apple `container` micro-VM on
macOS, a rootless podman container on Linux. The box sees its mounts and
nothing else of the host. Its only way out is its own allowlisting proxy.

- **Interactive:** `pi` on the host is a shim for `pinfold pi`.
- **Programmatic:** a caller such as a CI system or an agent orchestrator
  drives boxes through `pinfold box`, JSON on stdio, and reads a project's
  configuration with `pinfold config`.

- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Working rules: [AGENTS.md](AGENTS.md)

## Requirements

| Host | Runtime |
|---|---|
| macOS 26+ | Apple `container` |
| Linux | rootless podman |

On Linux, podman must be rootless with `cgroupManager=systemd`, and
long-lived boxes need `loginctl enable-linger`.

## Install

Each release carries a binary per host and `SHA256SUMS`. Download them
from the release page, or on a private repository with
`gh release download v<version> -R <owner>/pinfold`. Then verify and
install. macOS (Apple silicon):

```sh
shasum -a 256 -c --ignore-missing SHA256SUMS
mkdir -p ~/.local/bin
install -m 0755 pinfold-*-aarch64-apple-darwin ~/.local/bin/pinfold
```

Once installed, run `pinfold update` to download, verify and install the
latest stable release, or `pinfold update --check` to check without
installing. Close running pinfold boxes and let other pinfold commands
finish before updating. The install directory must be writable.
Interactive `pi` and `pinfold attach` show an update notice at most once
a day. Set `PINFOLD_NO_UPDATE_CHECK=1` to disable automatic checks.

The macOS binary is not notarized. `gh` and `curl` downloads run as they
are; a browser download carries the quarantine attribute and Gatekeeper
refuses it until `xattr -d com.apple.quarantine` clears it.

Linux, a static musl binary:

```sh
sha256sum -c --ignore-missing SHA256SUMS
mkdir -p ~/.local/bin
install -m 0755 pinfold-*-"$(uname -m)"-unknown-linux-musl ~/.local/bin/pinfold
```

Or build from a checkout of this repository; the Rust toolchain is pinned
in `rust-toolchain.toml`.

macOS needs `zig` and `cargo-zigbuild` (`cargo install cargo-zigbuild`).

```sh
rustup target add aarch64-unknown-linux-musl
cargo build --release
mkdir -p ~/.local/bin
install -m 0755 target/release/pinfold ~/.local/bin/pinfold
```

Linux builds the static musl binary:

```sh
rustup target add "$(uname -m)-unknown-linux-musl"
cargo build --release --target "$(uname -m)-unknown-linux-musl"
mkdir -p ~/.local/bin
install -m 0755 "target/$(uname -m)-unknown-linux-musl/release/pinfold" ~/.local/bin/pinfold
```

Then make `pi` a symlink to the binary, in a directory on `PATH`:

```sh
ln -s pinfold ~/.local/bin/pi
```

Put the `pi` symlink before other `pi` executables on `PATH`. Every tool
that invokes it runs boxed pi with the invoked project's state and logins.

## First run

In a project:

1. If it has a `.pinfold.toml`, run `pinfold allow`, and again after the
   file or the Containerfile it names changes.
2. Build the image: `pinfold build`. Without a project Containerfile it
   builds the selected profile's image; with one it builds the project's.
   Building needs network.
   Later runs show an `image-outdated` rebuild hint when the Containerfile
   changes. Rebuilding keeps project settings, sessions and logins.
3. Start pi: `pi`. The first run downloads the pinned pi release and
   creates the project home under `~/.local/state/pinfold/projects/`.
4. Run `/login` in pi once per project: state is per project. API keys
   arrive through the environment instead.

To reuse one project's pi configuration in another, `pinfold profile new
NAME --from-project [PATH]` copies it into a profile (see Profiles in the
spec).

Interactive launches report `bundled-profiles-changed` once without changing
saved settings or user profiles. To inspect or edit a built-in, run
`pinfold profile new NAME --from full --builtin`.

The default image has the basic coding tools. Optional profiles add tools
without changing the egress allowlist:

| Profile | Adds |
|---|---|
| `documents` | Bun, AnyDoc for local document conversion, Poppler for PDFs |
| `full` | documents tools plus gh, rtk and ponytail |

Build and select one explicitly:

```sh
pinfold build --profile documents
PINFOLD_PROFILE=documents pi
```

Set `profile = "documents"` in the project's `.pinfold.toml` to keep the
selection, then run `pinfold allow`.

## Configuration

See [Configuration](docs/ARCHITECTURE.md#configuration) for layers, keys and defaults.

For `pinfold pi`, forward host variables with `PINFOLD_ENV_<NAME>`.
Programmatic callers use the [box spec](docs/ARCHITECTURE.md#process-interface).

## Limits

Read the [threat model](docs/ARCHITECTURE.md#threat-model) and
[shared files](docs/ARCHITECTURE.md#shared-files) before using pinfold with a repository.

The experimental [Pi Durable example](examples/pi-durable/README.md)
provides a host adapter for boxed execution and recovery.
