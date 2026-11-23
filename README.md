# pinfold

Run a coding agent in a disposable micro-VM: Apple `container` on macOS,
Firecracker on Linux. By default a macOS project shares your folder with
the box, `.git` included, so the agent commits with ordinary Git. A Linux
project uses a private disk that starts empty (macOS can use one too).
Network egress goes through an allowlisting proxy.

- **Interactive:** `pinfold -- pi`, `pinfold -- claude` or `pinfold -- sh`.
- **From another agent:** `pinfold -- pi -p` (see below).
- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

A share is writable throughout, `.git` included. The box can delete or
corrupt shared files and Git history, and host tools can run box-written
content, Git hooks and configuration included. See
[Mounts](docs/ARCHITECTURE.md#mounts) and the
[threat model](docs/ARCHITECTURE.md#threat-model). A linked worktree under
a shared main root uses that root's configuration; external worktrees and
separate Git directories refuse.

## Requirements

| Host | Runtime |
|---|---|
| macOS 26+ | Apple `container` 1.6.0+ (stable) |
| Linux x64, kernel 6.10+ | `/dev/kvm` for the user (the `kvm` group), unprivileged user namespaces, Landlock and e2fsprogs |

A disk project's Git remote expects host Git 2.29 or later.
Linux image builds use the [pinned builder bootstrap](builder-root/README.md).

On Ubuntu 24.04+, AppArmor may restrict user namespaces. If a launch is
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
ln -sf pinfold ~/.local/bin/git-remote-pinfold
```

`git-remote-pinfold` is a disk project's Git remote (below); Git finds it
on PATH. `pinfold update` and a disk project's first-run setup also create
it when missing.

`pinfold update` installs the latest stable release, verified; `--check`
only reports it. Close boxes and other pinfold commands first; the install
directory must be writable. Interactive `pinfold -- CMD` launches show at
most one update notice a day; `PINFOLD_NO_UPDATE_CHECK=1` turns it off.

The macOS binary is not notarized. Clear a browser download's quarantine
with `xattr -d com.apple.quarantine pinfold-*`.

Linux, a static musl binary:

```sh
sha256sum -c --ignore-missing SHA256SUMS
mkdir -p ~/.local/bin
install -m 0755 pinfold-*-x86_64-unknown-linux-musl ~/.local/bin/pinfold
ln -sf pinfold ~/.local/bin/git-remote-pinfold
```

Or build from a checkout; `rust-toolchain.toml` pins the toolchain:

```sh
mkdir -p ~/.local/bin

# macOS, with zig and cargo-zigbuild (cargo install cargo-zigbuild)
rustup target add aarch64-unknown-linux-musl
cargo build --release
install -m 0755 target/release/pinfold ~/.local/bin/pinfold
ln -sf pinfold ~/.local/bin/git-remote-pinfold

# Linux
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
install -m 0755 target/x86_64-unknown-linux-musl/release/pinfold ~/.local/bin/pinfold
ln -sf pinfold ~/.local/bin/git-remote-pinfold
```

## First run

Start Pi from your project folder.

```sh
pinfold -- pi
```

Use `pinfold -- sh` for a shell. Anything after `--` is the command and
its arguments. Bare `pinfold --` exits with a usage error.

On the first interactive run, of `pinfold -- CMD` or `pinfold start`,
choose the profile and confirm the settings. If your shell sets
`PINFOLD_PROFILE`, that selects the profile. Unset it to choose a
different one. On macOS you also choose how the box sees the folder:
share it (the default), so the box edits it and its Git metadata, or use
an isolated project disk, which starts empty; in a Git project, `git push
box` and `git fetch box` move branches and tags. Linux always uses a project disk. If the selected
managed image is missing, Pinfold offers to build it and then starts your
command. A build needs network access. An explicit `image` must already
be available locally. Rebuild with `pinfold build` if a later launch
reports `image-outdated`. Before saving, setup shows the settings file it
will write and the lines it will hold. A project with that file gets no setup
questions.

Run `pinfold config` to see which folder, profile and image the project uses.
Run `pinfold` to see the box status and project settings. Before setup it
previews the profile and file mount the first run asks to confirm. Run `pinfold doctor`
to check for problems or `pinfold --help` to list the commands.

For scripts, prepare the project first with `pinfold config edit` and
`pinfold build`. `config edit` opens
`~/.config/pinfold/projects/<id>/pinfold.toml`. When the file is missing
it first creates it with the default mount (share on macOS, disk on
Linux) and commented examples of the other keys. A project recipe is a
`Containerfile` beside it. Moving or renaming a project gives it a new
configuration path.

Box work runs as your host uid; the built-in images name uids 501 and
1000 `pinfold`. The project home, `/home/pinfold`, is a disk that persists.
A disk project also retains its project disk. `pinfold disk list` shows
sizes and originating folders. `disk rm --all` removes unattached disks in
the current state root.

**Where an agent's login lives.**

| | `/login` in the box | Login route in `routes` |
|---|---|---|
| Credential | On the project's home disk | On the host; the box holds a placeholder |
| Box code can read it | Yes | Not directly; it can use it, and an upstream that echoes it returns it |
| Scope | That project; persists across `stop` | Every project selecting the profile |

Either way the agent's traffic goes through the proxy. Put a login route
in the selected profile's `pinfold.toml`. Pi on the host's Codex
subscription (sign in with Codex on the host first):

```toml
[routes.openai]
login = "pi"
provider = "openai-codex"
```

Claude on a `claude setup-token` token in a host variable:

```toml
[routes.anthropic]
login = "claude"
from = "CLAUDE_SETUP_TOKEN"
```

Or read it from 1Password with `op` in place of `from`:

```toml
[routes.anthropic]
login = "claude"
op = "op://Private/Claude setup token/credential"
```

The 1Password CLI (`op`) must be installed and signed in. Only the launch
that starts the box asks 1Password; a session joining it does not.

Login routes generate native configuration for the whole box. Claude and Codex
use system configuration; Pi autoloads a read-only extension under its writable
home. Agent configuration precedence still applies. Launch agents through PATH.
An explicit project `routes` table replaces the profile's routes. An API
key can also arrive through the environment. Using a Claude subscription
through Pi is subject to Anthropic's terms: Pi presents the token as Claude
Code and may bill it as extra usage.

Each login route grants every process in the box the host login's authority
over every path at its origin. It does not inspect request bodies or prevent
a reflecting upstream from returning credentials. See
[Login routes](docs/ARCHITECTURE.md#egress-proxy) for the exact policy.

One detached owner serves each project. Each `pinfold -- CMD` starts or
joins a session; the box ends when no session, Git crossing or `start`
keep remains, and `pinfold stop` ends everything. When a client leaves early (a signal, a
closed terminal, a lost connection), its command's process group gets
TERM, then KILL. Joining keeps the running policy and credentials; changed
host roots or effective configuration refuse as `config-changed`, so stop the box
first. `start` and `stop` take no arguments, print nothing to stdout, and
on success print one status line on stderr: `start` says `started` or `already
running`, `stop` says `stopped` or `no box running`. Infrastructure
failures exit 125. Edits to a share remain after stop. A kept box protects
its project from cleanup and executable replacement; stop it before
cleanup or an update.

Arguments after `--` pass literally to the command, including `--help`. Command exit codes
pass through; project setup failure or external teardown before observed completion uses 125. CMD owns stdout;
pinfold diagnostics use stderr.

Project reports use readable text, including when piped. Add `--json` to
`config`, `doctor` and `disk` for structured output. Use
`pinfold doctor --artifacts --json` for artifact pins.

Managed builds of a chain that contains the bundled `default` recipe
re-resolve stable agent releases through it every time. Installation
reruns when a resolved version changes; other steps use the layer cache.
Use `pinfold build` or `pinfold build --profile NAME` to update.
Add `--no-cache` to force the full recipe to run. Launch never installs agents.
A build first rebuilds every profile beneath it, root first: a project
build its selected profile and that profile's bases. Other projects and
profiles on a rebuilt image report `image-outdated` at their next launch
if its identity changed.

Every built-in image includes standalone Pi, native Claude and Codex with
its code-mode host on PATH. Built-in profiles seed `.pi/agent/APPEND_SYSTEM.md`,
`.claude/CLAUDE.md` and `.codex/AGENTS.md` for both mount kinds only when missing,
including Linux and explicit macOS disk homes. Existing instruction files,
including earlier seeds, stay untouched and can therefore miss updated guidance.
Instructions do not enforce containment. Home settings are also missing-only;
rebuilding preserves edited settings.

Linux build egress has its own allowlist. Add exact hosts with
`pinfold build --allow HOST`. Apple builds reject `--allow` additions.

For a share, host edits are live and the box stages and commits in the
host's `.git`; a box commit is the host's commit. Pinfold reads share Git
metadata as data and never executes Git on the host for that project.
Host and box Git at the same moment can lose commits or record files as
deleted (see the threat model). Export `GIT_OPTIONAL_LOCKS=0` in the shell
that runs `pinfold` and in host tools that poll `git status`; unless it is
exactly `0`, `pinfold -- CMD` warns `git-optional-locks` on stderr. It does not cover
host `git diff` or host commits while the box commits. There is no crossing;
the Git remote refuses as `share-no-crossing`. To select disk behavior on
macOS, set this in the project’s configuration:

```toml
mounts = [{ host = ".", kind = "disk" }]
```

A project disk appears in the box at `/project`, where a share appears
at the folder's own path; `$PINFOLD_PROJECT` holds that path unless your `env` sets it. A new disk starts empty; no start copies code
into it or changes your repository. Clone inside the box, or send
branches yourself. Only committed branches cross; commit local work (a
WIP commit is enough) to send it in. After the first start has created the disk:

```sh
git remote add box pinfold::"$(pwd -P)"   # once, at the project root
git push box main                         # a host branch into the box as host/main
git fetch box                             # guest branches as box/*, to merge, and new tags
```

Git prints `[new reference] main -> host/main` on every successful push of a branch;
`git push box :` sends no branch.

Crossing never touches a working tree. The first push into a disk
without a repository creates an empty one there. To check a branch out,
run `git switch -c main host/main` in the box. Pushed tags (`--tags`,
`push.followTags`) land as the box's own tags. `git fetch box` brings
the box's new tags back as yours, as Git follows tags from any remote. A
fetch never changes or deletes a tag you have, unless a refspec you type
maps a ref to another tag name. A fetched tag is yours to publish, and one
named like a branch makes `git push box BRANCH` ambiguous until you delete it.

`git pull box`, a refspec such as `git fetch box main:other` and `--prune`
let the box's branches reach or delete your refs.
Without a running box, crossing starts a transient one on the project
disk, without egress.

Outside Git on macOS, a share exposes the folder’s existing host files and
retains its home. An explicit disk instead starts empty and imports no
host files. On Linux a folder outside Git is refused as `not-git`. See
[Configuration](docs/ARCHITECTURE.md#configuration) for precedence and recipes.

To save a stopped disk into a new host directory, run
`pinfold disk export NAME DEST [--image IMAGE]`. It uses the local default
profile image unless an image is named; build that image explicitly if absent.
An attached or unclean disk is refused. Exported files are untrusted: inspect
them before opening or executing them. Symlinks remain symlinks and are never
followed during export. An existing destination is never replaced.

Optional profiles add tools. Each builds on the one above it:

| Profile | Base | Adds |
|---|---|---|
| `documents` | `default` | Bun, AnyDoc for local document conversion, Poppler for PDFs |
| `adam` | `documents` | gh, fff (pi extension) and ponytail; allows `github.com` and `api.github.com` |

Each one's directory under [profile/](profile) holds only what it adds:
its recipe adds only its own tools. All three seed these
[Pi settings](profile/default/home/.pi/agent/settings.json).
In a running box, `cat /opt/pinfold/agents/install.txt` shows the agent versions
installed at build time. `cat ~/.pi/agent/settings.json` shows the current guest settings.

Build and select one explicitly:

```sh
pinfold build --profile documents
PINFOLD_PROFILE=documents pinfold -- pi
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
a project Containerfile. With an explicit
image, `pinfold build` reports that there is no selected recipe to build.

### Your own profile

A profile lives in `~/.config/pinfold/profiles/NAME/`. `pinfold profile`
lists every profile, its base, and its folder or that it is built in. To add to a
published one without copying it, give it a `base`. This profile adds one
tool to `adam`:

```toml
# ~/.config/pinfold/profiles/adam-local/pinfold.toml
base = "adam"
```

```dockerfile
# ~/.config/pinfold/profiles/adam-local/Containerfile
FROM localhost/pinfold/profile-adam:latest
RUN apt-get update && apt-get install -y --no-install-recommends tree && rm -rf /var/lib/apt/lists/*
```

`pinfold build --profile adam-local` builds `default`, `documents`,
`adam`, then `adam-local`. The base's settings are defaults the profile's
own keys replace, one key at a time; a list such as `allow` replaces the
base's whole. The profile's `home/` and `share/` files lie over the
base's, file by file. Without a `Containerfile` a profile uses its base's
image. When a base changes, the profiles above it report `image-outdated`
until rebuilt. `pinfold profile new NAME --from adam` copies `adam`'s own
files, so the copy still builds on `documents`. A profile named like a
built-in replaces it, also as a base.

Custom profiles can omit `Containerfile` when an image or project recipe is
selected. Keep Pi settings, extensions and skills in their native files under
`home/` and `share/`. Home seeds fill missing files; changing profiles keeps
saved settings, sessions and disks. Shared files are read-only at every start.

## From another agent

A coding agent on the host can delegate to pi in a box:

```sh
pinfold -- pi -p "TASK" </dev/null              # answer on stdout, pi's exit code
pinfold -- pi -p --continue "FOLLOW-UP" </dev/null
git fetch box                                 # disk-project branches as box/*
```

Treat the box's output and branches as untrusted. The
[pinfold-delegate](skills/pinfold-delegate/SKILL.md) skill teaches an
agent this; copy its directory into the agent's skills directory.

## Rex on macOS

Bundled profiles do not ship Rex. To use it, install the Linux `rex` in
your own profile or project image, from Superlogical's package repository:

```dockerfile
RUN curl -fsSL https://pkgs.superlogical.com/rex/install.sh | sh -s -- --tarball
```

Rebuild the image to update it. Then run `pinfold start` in a Rex pane. It
starts Rex in the box and prints a `unix://` address on its own line. Add
it once in the Rex app under Add Remote Host…. It works only while the
box runs.
Turn off Remote Control in the Rex app first; with it on, `start` sets up
nothing. Read the threat model's Rex entry before use.

## Fresh state

All commands reuse the project home. Disk projects also reuse a project
disk; shares use host files directly. To start fresh:

1. Finish every command. For a disk Git project, commit guest work you want
   to keep, run `git fetch box`, inspect the commits, then merge or give them a host
   branch to retain them. Stop a kept box with `pinfold stop`.
   Use `disk export` for other disk files. Share edits already reside on the
   host; stop revokes box access and preserves those edits.
2. Run `pinfold config --json`. Read the exact names from `project.disks.home`
   and `project.disks.project` (null for a share).
3. Run `pinfold disk rm NAME` for each disk you want to clear.
4. Launch `pinfold -- pi` or `pinfold -- pi -p "TASK"` again. No image rebuild
   is needed.

For a share, removing the home resets only home contents; it never clears
host shared files. For a disk project, remove both disks for a fresh home
and project. Removing only the home preserves the guest repository; removing only the project preserves
sessions, settings and other home files. Everything on a deleted disk is
lost, including uncommitted work and home-only files. Fetch brings back commits,
not uncommitted or home-only files. Later fetches can move `box/*` refs.

Outside Git in an explicit macOS disk project, save wanted files with
`disk export` before removing the persistent disks.

Disk removal refuses attached disks. The two removals are separate operations;
serialize access to the project. Use `disk rm`, never delete state or volume
files directly. Profiles, project configuration and the selected image remain intact.

## Configuration

See [Configuration](docs/ARCHITECTURE.md#configuration) for layers, keys and defaults.

A profile's `pinfold.toml` or the project's configuration sets the box's
environment with `env`. A literal is plain configuration and shows in
`pinfold config --json`; `from` reads the value from your shell when the box
starts; pinfold never writes it down, and neither does the box runtime.
Use `from` for anything secret:

```toml
[env]
GIT_AUTHOR_NAME = "my-agent"
GH_TOKEN = { from = "MY_AGENT_GH_TOKEN" }
```

The project's names replace the profile's one by one, and the profile's
its base's. A missing `from` variable refuses the start. A second command
joining a running box with a different value for that variable is refused
as `config-changed`; stop the box to change it.

For a single command, forward host variables with `PINFOLD_ENV_<NAME>`.
It wins over `env`.

To let the box read host files outside the project, such as notes or a
dataset's documentation, add read-only folders after the project mount in
the project's configuration. `host` is absolute or `~/…`, outside the
project; `guest` is where the box sees it. The box can read everything
in the folder and send it to any allowed host. A change applies at the
next box start.

```toml
mounts = [{ host = ".", kind = "share" }, { host = "~/notes/dataset", guest = "/notes" }]
```

### GitHub access

Give the box a token through `env` with `from`, as `GH_TOKEN` above, and
allow `api.github.com` (the `adam` profile does). Prefer a fine-grained
token scoped to the one repository the box works on, with only the
permissions it needs. Every process in the box can read the token.

For disposable Rust build outputs in guest memory:

```sh
PINFOLD_ENV_CARGO_TARGET_DIR=/tmp/pinfold-target pinfold -- pi
```

`/tmp` is in guest memory and goes with the box. Save deliverables in the
project’s actual cwd; commit them for disk-project crossing.

## Limits

Read the [threat model](docs/ARCHITECTURE.md#threat-model) and [disks](docs/ARCHITECTURE.md#disks) before use.

Shared-file reads can be stale or internally inconsistent after host changes;
no fixed recovery delay is guaranteed. Shares have no cross-box
coordination.

Developers integrating pinfold: see the spec's
[Process interface](docs/ARCHITECTURE.md#process-interface).
