# pinfold

Run a coding agent in a disposable micro-VM: Apple `container` on macOS,
Firecracker on Linux. The box works on its own copy of the project, and
its only way out is its own allowlisting proxy. `pinfold fetch` brings its
commits back as branches.

- **Interactive:** `pi` on the host is a shim for `pinfold pi`.
- **Programmatic:** drive boxes through `pinfold box` (JSON on stdio),
  and read project settings with `pinfold config`.
- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

## Requirements

| Host | Runtime |
|---|---|
| macOS 26+ | Apple `container` 1.6.0+ (stable) |
| Linux x64, kernel 6.10+ | `/dev/kvm` for the user (the `kvm` group), unprivileged user namespaces and Landlock, e2fsprogs, and rootless podman 6.1.4+ (stable) for image builds; pinfold fetches its pinned Firecracker |

On Ubuntu 24.04+, AppArmor restricts user namespaces, and `box up` is
refused as `runtime` naming `userns-apparmor`. Load pinfold's profile
once, as root, instead of turning the restriction off. It attaches to the
path of the executable that prints it, so run these with the installed
binary, and again after moving it:

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

`pinfold update` installs the latest stable release, verified; `--check`
only reports it. Close boxes and other pinfold commands first; the install
directory must be writable. Interactive `pi` and `attach` show at most one
update notice a day; `PINFOLD_NO_UPDATE_CHECK=1`
turns it off.

The macOS binary is not notarized. Clear a browser download's quarantine
with `xattr -d com.apple.quarantine pinfold-*`.

Linux, a static musl binary:

```sh
sha256sum -c --ignore-missing SHA256SUMS
mkdir -p ~/.local/bin
install -m 0755 pinfold-*-x86_64-unknown-linux-musl ~/.local/bin/pinfold
```

Or build from a checkout; `rust-toolchain.toml` pins the toolchain:

```sh
mkdir -p ~/.local/bin

# macOS, with zig and cargo-zigbuild (cargo install cargo-zigbuild)
rustup target add aarch64-unknown-linux-musl
cargo build --release
install -m 0755 target/release/pinfold ~/.local/bin/pinfold

# Linux
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
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
   Building needs network. Rebuild when a run shows `image-outdated`;
   it keeps project settings, sessions and logins.
3. Start pi: `pi`. The first run downloads the pinned pi release and
   creates the project's home and project disks.
4. Run `/login` in pi once per project: state is per project. API keys
   arrive through the environment instead.

The first start copies the checkout into the box, which keeps that copy.
`pinfold fetch` brings the box's commits back as `pinfold/*` branches for
you to merge; `pinfold push` sends your branches in as `host/*`.

To edit a built-in profile, copy it:
`pinfold profile new NAME --from full --builtin`.

Optional profiles add tools to the default, not egress:

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

`/tmp` is in guest memory and goes with the box; commit deliverables in
the project first.

## Limits

Read the [threat model](docs/ARCHITECTURE.md#threat-model) and
[disks](docs/ARCHITECTURE.md#disks) before using pinfold with a repository.

The experimental [Pi Durable example](examples/pi-durable/README.md)
provides a host adapter for boxed execution and recovery.
