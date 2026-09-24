# pinfold

Run a coding agent in a disposable box: an Apple `container` micro-VM on
macOS, a rootless podman container on Linux. The box sees its mounts and
nothing else of the host. Its only way out is its own allowlisting proxy.

Isolation differs by platform: on macOS each box is its own VM; on Linux,
boxes share the host kernel.

- **Interactive:** `pi` on the host is a shim for `pinfold pi`.
- **Programmatic:** a caller such as a CI system or an agent orchestrator
  drives boxes through `pinfold box`, JSON on stdio, and reads a project's
  configuration with `pinfold config`.

- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- History: [docs/archive/](docs/archive/)
- Working rules: [AGENTS.md](AGENTS.md)

## Requirements

| Host | Runtime |
|---|---|
| macOS 26+ | Apple `container` |
| Linux | rootless podman |

Anything else is refused: podman on macOS, rootful podman, docker.

On Linux, podman must report `rootless=true` and `cgroupManager=systemd`,
or pinfold refuses to start a box. Under the cgroupfs fallback, `--cpus`
and `--memory` are silently not enforced. Long-lived boxes need
`loginctl enable-linger`.

## Install

Each release carries a binary per host and `SHA256SUMS`. Download them
from the release page, or on a private repository with
`gh release download v<version> -R <owner>/pinfold`. Then verify and
install. macOS (Apple silicon):

```sh
version=0.0.2
shasum -a 256 -c --ignore-missing SHA256SUMS
mkdir -p ~/.local/bin
install -m 0755 "pinfold-$version-aarch64-apple-darwin" ~/.local/bin/pinfold
```

The macOS binary is not notarized. `gh` and `curl` downloads run as they
are; a browser download carries the quarantine attribute and Gatekeeper
refuses it until `xattr -d com.apple.quarantine` clears it.

Linux, a static musl binary:

```sh
version=0.0.2
sha256sum -c --ignore-missing SHA256SUMS
mkdir -p ~/.local/bin
install -m 0755 "pinfold-$version-$(uname -m)-unknown-linux-musl" ~/.local/bin/pinfold
```

Or build from a checkout of this repository; the Rust toolchain is pinned
in `rust-toolchain.toml`.

macOS needs `zig` and `cargo-zigbuild` (`cargo install cargo-zigbuild`):
the build cross-compiles the Linux init the macOS CLI embeds.

```sh
rustup target add aarch64-unknown-linux-musl
cargo build --release
mkdir -p ~/.local/bin
install -m 0755 target/release/pinfold ~/.local/bin/pinfold
```

Linux builds the static musl binary, which doubles as the box's init:

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

`pi` must resolve to pinfold before any other `pi` on `PATH`. The shim
shadows the host `pi` for every tool that spawns one, not only your shell:
such a tool then runs pi in a box for the project it invokes `pi` from,
with that project's pinfold state and logins.

## First run

In a project:

1. If it has a `.pinfold.toml`, run `pinfold allow` once. The file and the
   Containerfile it names are used only after that; a change stops the next
   run until allowed again.
2. Build the image: `pinfold build`. Without a project Containerfile it
   builds the selected profile's image; with one it builds the project's.
   The default profile image pulls `debian:trixie-slim` and runs
   `apt-get upgrade` at every build, so the first build needs network.
3. Start pi: `pi`. The first run downloads the pinned pi release and
   creates the project home under `~/.local/state/pinfold/projects/`.
4. Run `/login` in pi once per project: state is per project. API keys
   arrive through the environment instead.

To reuse one project's pi configuration in another, `pinfold profile new
NAME --from-project [PATH]` copies its settings, provider and skills into a
profile; `auth.json`, `sessions/`, `npm/` and caches stay behind.

## Configuration

Layers, highest first: the environment, `.pinfold.toml`, the selected
profile's `pinfold.toml`, built-in defaults. The keys, their environment
variables and defaults are the table under Configuration in
[ARCHITECTURE.md](docs/ARCHITECTURE.md#configuration). A list set in a
layer replaces the lists below it.

Credentials need no pinfold code: `op run -- pi …` for 1Password,
`PINFOLD_ENV_GH_TOKEN` for gh and git, direnv for per-repo tokens, and
`PINFOLD_ENV_GIT_*` for identity. `PINFOLD_ENV_<NAME>` is the only way a
host environment variable enters the box.

## What pinfold does not protect

- Project contents: an agent with a model API can put them in a prompt.
- Files the host runs by explicit command: build scripts, tests, package
  scripts, and scripts a hook calls from the project, such as husky's
  `.husky/pre-commit`.
- A planted repo (`sub/.git`, or `.git` in a project that has none) runs
  code if a host tool runs git in it (VS Code does by default).
- CDN fronting beyond the SNI check (Host-header fronting needs TLS
  interception).
- Allowlisted services that accept writes (GitHub with a token,
  registries).
- Image builds: user- or caller-run, trusted, unrestricted egress.

## Shared files

Box-created files land on the host as the user's, 644 or 755; the exec bit
and symlinks survive. Apple `container` can show the top directory of a
mount as root-owned inside the box, so git needs `safe.directory` for a
mounted repository; `pinfold pi` sets it for the project root.

Apple `container` limitations:

| Limitation | Handling |
|---|---|
| ~1 s metadata and name cache: ENOENT or stale `stat` after a host atomic save | Not handled; a reader retries. |
| A mount's top directory may be root-owned inside the box | `safe.directory`, as above. |
| No inotify for host changes | The box polls (`CHOKIDAR_USEPOLLING`, `WATCHPACK_POLLING`); pinfold sets neither. |
| `flock`/`fcntl` locks not shared; O_EXCL lockfiles are safe | Don't open one SQLite database from both sides. |
| A case-only rename is a no-op | Rename in two steps. |
| Small-file I/O 4–10× slower | Accepted. `/tmp` is tmpfs. |
| setuid and setgid bits cannot be set | Harmless. |

## Roadmap

1. Core, proxy, pi layer, and CLI, in Rust. Code landed; a week of daily
   use on macOS is the exit criterion.
2. Linux podman end-to-end suite in GitHub CI, and this README. Both
   landed. Daily use then moves from agentbox to pinfold, and agentbox is
   archived.
