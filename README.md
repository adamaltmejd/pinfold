# pinfold

Run a coding agent in a disposable box: a micro-VM, Apple `container` on
macOS and Firecracker on Linux. The box works on its own
copy of the project; `pinfold fetch` brings its commits back as branches.
Its only way out is its own allowlisting proxy.

- **Interactive:** `pi` on the host is a shim for `pinfold pi`.
- **Programmatic:** drive boxes through `pinfold box` (JSON on stdio),
  and read project settings with `pinfold config`.
- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

## Requirements

| Host | Runtime |
|---|---|
| macOS 26+ | Apple `container` 1.6.0+ (stable) |
| Linux x64, kernel 6.10+ | `/dev/kvm` for the user (the `kvm` group), unprivileged user namespaces and Landlock, e2fsprogs, and rootless podman 6.1.4+ (stable) for image builds; pinfold fetches its pinned Firecracker |

On Ubuntu 24.04+, AppArmor restricts user namespaces. Load pinfold's
profile once instead of turning the restriction off:

```sh
pinfold apparmor-profile | sudo tee /etc/apparmor.d/pinfold >/dev/null
sudo apparmor_parser -r /etc/apparmor.d/pinfold
```

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

Run `pinfold update --check` to check for a stable release, or `pinfold
update` to download, verify and install it. Close boxes and other pinfold
commands first; the install directory must be writable. Interactive `pi`
and `pinfold attach` show at most one daily update notice.
`PINFOLD_NO_UPDATE_CHECK=1` disables automatic checks.

The macOS binary is not notarized. Clear a browser download's quarantine
with `xattr -d com.apple.quarantine pinfold-*`.

Linux, a static musl binary:

```sh
sha256sum -c --ignore-missing SHA256SUMS
mkdir -p ~/.local/bin
install -m 0755 pinfold-*-x86_64-unknown-linux-musl ~/.local/bin/pinfold
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
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
mkdir -p ~/.local/bin
install -m 0755 target/x86_64-unknown-linux-musl/release/pinfold ~/.local/bin/pinfold
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
   creates the project home and project disks.
4. Run `/login` in pi once per project: state is per project. API keys
   arrive through the environment instead.

The first start copies the checkout into the box, which keeps that copy.
`pinfold fetch` brings the box's commits back as `pinfold/*` branches for
you to merge; `pinfold push` sends your branches in as `host/*`.

Interactive launches report `bundled-profiles-changed` once without changing
saved settings or user profiles. To inspect or edit a built-in, run
`pinfold profile new NAME --from full --builtin`.

The default has coding tools; optional profiles add more without changing egress:

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

For disposable Rust build outputs in guest memory:

```sh
PINFOLD_ENV_CARGO_TARGET_DIR=/tmp/pinfold-target pi
```

Keep outputs within the guest's available memory and `/tmp` capacity. Copy
deliverables into the project and commit them before the box exits;
`/tmp` is discarded.

## Limits

Read the [threat model](docs/ARCHITECTURE.md#threat-model) and
[disks](docs/ARCHITECTURE.md#disks) before using pinfold with a repository.

The experimental [Pi Durable example](examples/pi-durable/README.md)
provides a host adapter for boxed execution and recovery.
