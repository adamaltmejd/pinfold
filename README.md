# pinfold

Run a coding agent in a disposable micro-VM: Apple `container` on macOS,
Firecracker on Linux. New macOS project configurations share host files
directly, `.git` included, so an agent commits with ordinary Git. Linux projects use a private
disk copy of committed branches; macOS also supports an explicit disk.
Network egress uses the box’s allowlisting proxy.

- **Interactive:** `pinfold -- pi`, `pinfold -- claude` or `pinfold -- bash`.
- **From another agent:** `pinfold -- pi -p` (see below).
- **Programmatic:** drive boxes through `pinfold box` (JSON on stdio, the
  [box spec](docs/ARCHITECTURE.md#process-interface)), and read project settings with `pinfold config --json`.
- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

Core `box up` specs can declare an Apple writable host mount with
`{"host":"/path/to/folder","guest":"/work","kind":"share"}`.
The default mount kind is `readonly`; Linux refuses `share` as `spec`.
A share is writable throughout, `.git` included. Shared working files and
Git history can be deleted or corrupted, and host tools can execute
box-written content, Git hooks and configuration included.
See [Mounts](docs/ARCHITECTURE.md#mounts) and the
[threat model](docs/ARCHITECTURE.md#threat-model). A linked worktree beneath
a shared main root uses that root’s configuration and owner, at its invoking
cwd. External worktrees and separate Git directories refuse. Independent
nested repositories retain their own project discovery.

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
ln -sf pinfold ~/.local/bin/git-remote-pinfold
```

The `git-remote-pinfold` link is a disk project's Git remote (below); Git
finds it on PATH.

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

On the first interactive run, choose the profile and confirm the project
folder. If your shell sets `PINFOLD_PROFILE`, that selects the profile.
Unset it to choose a different one. On macOS the box can edit files in
that folder, including its Git metadata. Linux uses a private project disk. If the selected
managed image is missing, Pinfold offers to build it and then starts your
command. A build needs network access. An explicit `image` must already
be available locally. Rebuild with `pinfold build` if a later launch
reports `image-outdated`.

Run `pinfold config` to see which folder, profile and image the project uses.
Run `pinfold` to see the box status and project settings. Before setup it
previews the initial profile, file sharing and image. Run `pinfold doctor`
to check for problems or `pinfold --help` to list the commands.

For scripts, prepare the project first with `pinfold config edit` and
`pinfold build`. `config edit` opens
`~/.config/pinfold/projects/<id>/pinfold.toml`. A project recipe is a
`Containerfile` beside it. Moving or renaming a project gives it a new
configuration path.

A guest `/login` persists in the project home. A disk project also retains
its project disk. `pinfold disk list` shows sizes and originating folders.
`disk rm --all` removes unattached disks in the current state root. An API
key or a Claude or Codex subscription can stay on the host behind a login
route in `routes` (ARCHITECTURE.md, Egress proxy). An API key can also
arrive through the environment. Using a Claude subscription through Pi is
subject to Anthropic's terms: Pi presents the token as Claude Code and may
bill it as extra usage.

One detached owner serves each canonical project host-wide, independently of
HOME and XDG roots. Each `pinfold -- CMD` starts or joins a session. Ending or
disconnecting one client leaves other sessions running. The box ends when no
session, crossing lease or keep remains. When a client departs, its guest helper
sends TERM to that command's process group, then KILL after two seconds if needed.
This includes client signals, terminal closure, SIGKILL and control connection
loss. `pinfold start` keeps the box, not a departed session's command group.
A terminal departure first hangs up its guest terminal. Normal command
completion preserves background children. Output from background children is
delivered for up to one second after the command exits. A process that leaves
the command's process group, or for a terminal session ignores hangup after
leaving it, may keep running. `pinfold stop` ends everything. Public `box exec`
sends no signal on departure.

`pinfold start` keeps the box without clients. Matching repeated starts
succeed. `pinfold stop` stops its observed generation, without loading current
configuration, and cannot stop a replacement. From the recorded main root,
it also works with a hostile Git marker added during a share run. Share edits remain after stop; it neither
commits nor rolls them back. On Apple an orphan box may retain share access
after owner death until checked removal. Both take no arguments and
print nothing to stdout on success; infrastructure failures return 125.
Admission and silent-owner control waits have a ten-second deadline; timeout
leaves ownership intact. Waiting for a busy launcher has no startup deadline
and remains signal-cancellable. The last client and stop wait for acknowledged
teardown to finish, beyond the control deadline, and remain signal-cancellable.
A kept owner protects its project from cleanup and executable replacement;
stop it before cleanup or an update.

Joining or keeping preserves the running policy and credentials. Changed
host roots or effective configuration refuse as `config-changed`, naming
only differing fields. Stop the owner before applying those changes. Command
metadata, recipe freshness and seed/shared-file contents do not affect
comparison. Mount kind and canonical paths are startup configuration;
working files, HEAD, refs, index and mtimes are not. Host and box edits and
commits permit joining. A share whose root `.git` appears, disappears or
becomes a non-directory refuses new joins and starts as `config-changed`
until stop; the owner checks it on every join. Joining does not reseed,
synchronize Git or refresh login.

Arguments after `--` pass literally to the command, including `--help`. Command exit codes
pass through; project setup failure or external teardown before observed completion uses 125. CMD owns stdout;
pinfold diagnostics use stderr.

Pinfold owns the caller's terminal settings and forwards input and window
changes. Native Apple clients hold no caller-terminal descriptors. Without a
TTY, stdout and stderr stay separate.
The box-owning client's startup diagnostics stay in memory, bounded to 64 KiB,
and are escaped when printed. After ready, its latest 64 KiB of stderr stays
in memory and is printed, escaped, only if the box ends unexpectedly.
Cancellation and teardown close its stderr pipe.

Runtime sockets use private directories under `/tmp/pinfold-<uid>` (`/private/tmp` on macOS),
independently of XDG state-root length. Unsafe socket paths refuse as
`runtime-path`.

For reusable Codex subscription login, sign in with Codex on the host and
put this in the selected profile's `pinfold.toml`:

```toml
[routes.openai]
login = "pi"
provider = "openai-codex"
```

The token and refresh stay on the host across independent Pi homes.
Login routes remove guest authentication headers and Connection-nominated
fields before adding host authentication. Named credential query parameters
are refused; malformed query escapes and Connection options are refused too.
Login routes generate native configuration for the whole box. Claude and Codex
use system configuration; Pi autoloads a read-only extension under its writable
home. Agent configuration precedence still applies. Launch agents through PATH.

Each login route grants every process in the box the host login's authority
over every path at its origin. It does not inspect request bodies or prevent
a reflecting upstream from returning credentials. See
[Login routes](docs/ARCHITECTURE.md#egress-proxy) for the exact policy.
An explicit project `routes` table replaces the profile's routes.
Project reports use readable text, including when piped. Add `--json` to
`config`, `doctor`, disk operations or caller image operations
for structured output. Use `pinfold doctor --artifacts --json` for artifact pins.
The `box` process interface retains its JSON records. Guest commands own
their stdout and stderr.

Managed builds of a chain that contains the bundled `default` recipe
re-resolve stable agent releases through it every time. Installation
reruns when a resolved version changes; other steps use the layer cache.
Use `pinfold build` or `pinfold build --profile NAME` to update.
Add `--no-cache` to force the full recipe to run. Launch never installs agents.
A build first rebuilds every profile beneath it, root first: a project
build its selected profile and that profile's bases. Other projects and
profiles on a rebuilt image report `image-outdated` at their next launch
if its identity changed. Each image publishes separately; a failed build
can leave the rebuilt images beneath it published. Caller `image build` retains its layer cache
and `--no-cache` option.

Every built-in image includes standalone Pi, native Claude and Codex with
its code-mode host on PATH. Built-in profiles seed `.pi/agent/APPEND_SYSTEM.md`,
`.claude/CLAUDE.md` and `.codex/AGENTS.md` for both mount kinds only when missing,
including Linux and explicit macOS disk homes. Existing instruction files,
including earlier seeds, stay untouched and can therefore miss updated guidance.
Instructions do not enforce containment. Home settings are also missing-only;
rebuilding preserves edited settings.

Linux build egress has its own allowlist. Add exact hosts with
`pinfold build --allow HOST` or `pinfold image build … --allow HOST`.
Apple builds reject `--allow` additions.

Rebuild old Podman-only Linux images explicitly with `build` or `image build`.
Old Podman storage, derived roots and managed agent disks remain untouched.

For a share, host edits are live and the box stages and commits in the
host's `.git`; a box commit is the host's commit. Pinfold reads share Git
metadata as data and never executes Git on the host for that project.
Host and box Git at the same moment can lose commits or record files as
deleted (see the threat model). Export `GIT_OPTIONAL_LOCKS=0` in the shell
that runs `pinfold` and in host tools that poll `git status`; unless it is
exactly `0`, `pinfold -- CMD` warns `git-optional-locks` on stderr. It does not cover
host `git diff` or host commits while the box commits. There is no startup synchronization
or crossing; the Git remote refuses as `share-no-crossing`. A share launch never
deletes an old project disk or copies it back. To select disk behavior on
macOS, set this in the project’s configuration:

```toml
mounts = [{ host = ".", kind = "disk" }]
```

A disk project keeps its own repository. Only committed branches enter it;
commit local work (a WIP commit is enough) to send it in. Its first start
pushes every host branch in as `host/*`, checks out the current one, and
adds the Git remote `box` (`pinfold::` and the project root) to the host
repository unless a `box` remote exists. After that:

```sh
git fetch box             # guest branches as box/*, to inspect and merge
git push box main         # a host branch into the box as host/main
```

Every byte from the box is checked before Git sees a ref, but the
remote ends at refs: `git pull box`, a refspec on the command line
(`git fetch box main:other`) and `--prune` let the box's branches reach
or delete your refs, and a box branch named `HEAD` makes `box` name it.
Without a running box, crossing starts a transient one on the project
disk, without egress.

Outside Git on macOS, a share exposes the folder’s existing host files and
retains its home. An explicit disk instead starts empty at the folder’s
canonical path and imports no host files. On Linux a folder outside Git is
refused as `not-git`. See
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

Their [image recipes](profile/image) each add only their own tools. All
three seed these [Pi settings](profile/home/.pi/agent/settings.json).
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
a project Containerfile. Explicit images resolve locally. With an explicit
image, `pinfold build` reports that there is no selected recipe to build.

### Your own profile

A profile lives in `~/.config/pinfold/profiles/NAME/`. To add to a
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
`pinfold config --json` reports `image_source` and the selected recipe, even without
a runtime installed.

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
starts Rex in the box and prints a `unix://` address. Add it once in the
Rex app under Add Remote Host…. The host is reachable while the box runs.
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
For disk projects, a `sync-failed` warning means the first start's push
failed even if the command starts, and it is not retried. Check that
`git remote get-url box` prints `pinfold::` and the project root (or
`git remote add box pinfold::ROOT`), then `git push box BRANCH`.

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
no fixed recovery delay is guaranteed. `git status` on a very large shared
tree measured 10–15 seconds in the guest against about 0.1 seconds on the
host. This is a known cost. Shares have no cross-box coordination.
