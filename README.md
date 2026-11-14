# pinfold

Run a coding agent in a disposable micro-VM: Apple `container` on macOS,
Firecracker on Linux. The box works on its own clone of the project's branches, and
its only way out is its own allowlisting proxy. `pinfold fetch` brings its
commits back as branches.

- **Interactive:** `pi` on the host is a shim for `pinfold pi`.
- **From another agent:** `pinfold pi -p` (see below).
- **Programmatic:** drive boxes through `pinfold box` (JSON on stdio, the
  [box spec](docs/ARCHITECTURE.md#process-interface)), and read project settings with `pinfold config`.
- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

## Requirements

| Host | Runtime |
|---|---|
| macOS 26+ | Apple `container` 1.6.0+ (stable) |
| Linux x64, kernel 6.10+ | `/dev/kvm` for the user (the `kvm` group), unprivileged user namespaces, Landlock and e2fsprogs; pinned Firecracker and isolated BuildKit image jobs |

Linux image builds use the [pinned builder bootstrap](builder-root/README.md).

On Ubuntu 24.04+, AppArmor may restrict user namespaces. If `box up` is
refused as `runtime` naming `userns-apparmor`, load pinfold's profile
once, as root. It attaches to the path of the executable that prints it,
so run these with the installed binary, and again after moving it:

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
update notice a day; `PINFOLD_NO_UPDATE_CHECK=1` turns it off.

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

Put the `pi` symlink before other `pi` executables on `PATH`.

## First run

In your working folder:

1. Create its configuration: `pinfold config edit`. It writes
   `~/.config/pinfold/projects/<id>/pinfold.toml` and opens it in your
   editor. A project without one is refused as `no-config`; so is a moved
   or renamed project. A project recipe is a `Containerfile` beside it.
2. Build the selected recipe: `pinfold build`. Without a project Containerfile
   it builds the profile's recipe; with one it builds the project's.
   An explicit `image` must already be present locally and needs no build.
   Building needs network. Rebuild when a run shows `image-outdated`;
   it keeps project settings, sessions and logins.
3. Start pi: `pi`.
4. In a Git repository, run `/login` once; the home disk retains it.
   Scratch runs have a fresh home each time. After a clean exit, confirmed
   empty work deletes both scratch disks, including guest logins and sessions.
   Runs with work or failed inspection keep their disks. `pinfold disk list`
   shows readable sizes and recorded originating folders; `disk rm --all`
   removes unattached disks in the current state root.
   An API key or a Claude or Codex
   subscription can instead stay on the host behind
   a pi login route in `routes` (ARCHITECTURE.md, Egress proxy); an API
   key can also arrive through the environment.
   Using a Claude subscription through pi is subject to Anthropic's terms:
   pi presents the token as Claude Code and may bill it as extra usage.

For reusable Codex subscription login, sign in with Codex on the host and
put this in the selected profile's `pinfold.toml`:

```toml
[routes.openai]
login = "pi"
provider = "openai-codex"
```

The token and refresh stay on the host across independent Pi homes.
An explicit project `routes` table replaces the profile's routes.
For scripts, add `--json` to `pinfold disk` commands for exact byte counts
and structured results.

To rerun package-update steps, use `pinfold build --no-cache` or
`pinfold build --profile NAME --no-cache`.

Linux build egress has its own allowlist. Add exact hosts with
`pinfold build --allow HOST` or `pinfold image build … --allow HOST`.
Apple builds reject `--allow` additions.

Rebuild old Podman-only Linux images explicitly with `build` or `image build`.
Old Podman storage, derived roots and managed agent disks remain untouched.

In Git, the box keeps its own repository. Only committed branches enter it;
commit local work (a WIP commit is enough) to send it in.
`pinfold fetch` brings guest commits back as `pinfold/*` branches to merge;
`pinfold push` sends host branches in as `host/*`. Each `pi` start also pushes
and fast-forwards the guest branch when clean and strictly behind.

Outside Git on macOS, each invocation starts with fresh home and work disks
and an empty `/work`; the invoking folder is not shared. On Linux a folder
outside Git is refused as `not-git`. See
[Configuration](docs/ARCHITECTURE.md#configuration) for precedence and recipes.

On normal exit, Pi gives a bounded advisory summary of outstanding work,
without file contents. Fetch commits to retain them; uncommitted and untracked
files remain on the guest disk.

To save a stopped disk into a new host directory, run
`pinfold disk export NAME DEST [--image IMAGE]`. It uses the local default
profile image unless an image is named; build that image explicitly if absent.
An attached or unclean disk is refused. Exported files are untrusted: inspect
them before opening or executing them. Symlinks remain symlinks and are never
followed during export. An existing destination is never replaced.

Optional profiles add tools. To edit one, copy it with
`pinfold profile new NAME --from full`:

| Profile | Adds |
|---|---|
| `documents` | Bun, AnyDoc for local document conversion, Poppler for PDFs |
| `full` | documents tools plus gh, fff (pi extension) and ponytail; allows `github.com` and `api.github.com` |

Build and select one explicitly:

```sh
pinfold build --profile documents
PINFOLD_PROFILE=documents pi
```

Set `profile = "documents"` in the project's configuration
(`pinfold config edit`) to keep the selection.

To use another local image with a profile's settings:

```toml
profile = "documents"
image = "localhost/pinfold/image-research-tools:latest"
```

`image` can also be a profile default. A project `image` overrides it;
there is no environment override. An explicit image cannot be combined with
a project Containerfile. Pi never pulls or builds on launch. With an explicit
image, `pinfold build` reports that there is no selected recipe to build.

Custom profiles can omit `Containerfile` when an image or project recipe is
selected. Keep Pi settings, extensions and skills in their native files under
`home/` and `share/`. Home seeds fill missing files; changing profiles keeps
saved settings, sessions and disks. Shared files are read-only at every start.
`pinfold config` reports `image_source` and the selected recipe, even without
a runtime installed.

## From another agent

A coding agent on the host can delegate to pi in a box:

```sh
pinfold pi -p "TASK" </dev/null              # answer on stdout, pi's exit code
pinfold pi -p --continue "FOLLOW-UP" </dev/null
pinfold fetch                                 # box branches as pinfold/*
```

Treat the box's output and branches as untrusted. The
[pinfold-delegate](skills/pinfold-delegate/SKILL.md) skill teaches an
agent this; copy its directory into the agent's skills directory.

## Fresh state

In a Git repository, `pinfold pi` and `pinfold pi -p` reuse the same home
and project disks. To start fresh:

1. Finish Pi after committing guest work you want to keep. Run `pinfold fetch`,
   inspect the fetched commits, then merge or give them a host branch to retain them.
2. Run `pinfold config`. Read the exact names from `project.disks.home`
   and `project.disks.project`.
3. Run `pinfold disk rm NAME` for each disk you want to clear.
4. Launch `pinfold pi` or `pinfold pi -p "TASK"` again. No image rebuild
   is needed.

Remove both disks for a fresh home and project. Removing only the home
preserves the guest repository; removing only the project preserves
sessions, settings and other home files. Everything on a deleted disk is
lost, including uncommitted work and home-only files. Fetch brings back commits,
not uncommitted or home-only files. Later fetches can move `pinfold/*` refs.

Outside Git, each invocation already has fresh disks. Save wanted files with
`disk export`, then remove retained disks using their printed names.

Disk removal refuses attached disks. The two removals are separate operations;
serialize access to the project. Use `disk rm`, never delete state or volume
files directly. Profiles, project configuration and the selected image remain intact.
A `sync-failed` warning means source synchronization failed even if Pi starts.

## Configuration

See [Configuration](docs/ARCHITECTURE.md#configuration) for layers, keys and defaults.

For `pinfold pi`, forward host variables with `PINFOLD_ENV_<NAME>`.

For disposable Rust build outputs in guest memory:

```sh
PINFOLD_ENV_CARGO_TARGET_DIR=/tmp/pinfold-target pi
```

`/tmp` is in guest memory and goes with the box; commit deliverables in
the project first.

## Limits

Read the [threat model](docs/ARCHITECTURE.md#threat-model) and [disks](docs/ARCHITECTURE.md#disks) before use.
