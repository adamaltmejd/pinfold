# pinfold

Run a coding agent in a disposable micro-VM: Virtualization.framework on
macOS, Firecracker on Linux. By default a macOS project shares your folder with
the box, `.git` included, so the agent commits with ordinary Git. A Linux
project uses a private disk that starts empty (macOS can use one too).
Network egress goes through an allowlisting proxy.

- **Interactive:** `pinfold -- pi`, `pinfold -- claude` or `pinfold -- sh`.
- **From another agent:** `pinfold -- pi -p` (see below).
- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

With a share, the box can delete or corrupt shared files and Git history,
and host tools can run box-written content, Git hooks and configuration
included. See
[Mounts](docs/ARCHITECTURE.md#mounts) and the
[threat model](docs/ARCHITECTURE.md#threat-model). A linked worktree under
a shared main root uses that root's configuration; external worktrees and
separate Git directories refuse.

## Requirements

| Host | Runtime |
|---|---|
| macOS 26+, Apple silicon | none; the state root must be on APFS |
| Linux x64, kernel 6.10+ | `/dev/kvm` for the user (the `kvm` group), unprivileged user namespaces, Landlock and e2fsprogs |

A disk project's Git remote expects host Git 2.29 or later.

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
cat > /tmp/pinfold.entitlements <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>com.apple.security.virtualization</key><true/>
</dict></plist>
PLIST
codesign --force --sign - --entitlements /tmp/pinfold.entitlements target/release/pinfold
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

Use `pinfold -- sh` for a shell. Bare `pinfold --` exits with a usage error.

On the first interactive run, of `pinfold -- CMD` or `pinfold start`,
choose the profile and confirm the settings. If your shell sets
`PINFOLD_PROFILE`, that selects the profile. Unset it to choose a
different one. On macOS you also choose how the box sees the folder:
share it (the default), so the box edits it and its Git metadata, or use
an isolated project disk, which starts empty. Linux always uses a project
disk. Before saving, setup shows the settings file it will write and the
lines it will hold. A project with that file gets no setup questions. If
the selected managed image is missing, pinfold offers to build it, which
needs network access, and then starts your command. An explicit `image`
must already be available locally. Rebuild with `pinfold build` if a later
launch reports `image-outdated`.

Run `pinfold config` to see which folder, profile and image the project
uses, `pinfold` for the box status and project settings (before setup, a
preview of what the first run asks to confirm), `pinfold doctor` to check
for problems and `pinfold --help` to list the commands.

For scripts, prepare the project first with `pinfold config edit` and
`pinfold build`. `config edit` opens
`~/.config/pinfold/projects/<id>/pinfold.toml`. When the file is missing
it first creates it with the default mount (share on macOS, disk on
Linux) and commented examples of the other keys. A project recipe is a
`Containerfile` beside it. Moving or renaming a project gives it a new
configuration path.

Box work runs as your host uid. The project home, `/home/pinfold`, is a disk that persists.
A disk project also retains its project disk. Both are created at 64G
unless `home_disk_size = "128G"` or `project_disk_size = "…"` is set in
the project's configuration before the first start (a whole number and `G`,
at least 1G; a share has no project disk); a disk never grows. On Linux,
free space inside a disk is returned to the host when pinfold stops the box.
`pinfold disk list` shows sizes and originating folders. `disk rm --all`
removes unattached disks in the current state root.

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

Or read it from 1Password, with a reference in `from`:

```toml
[routes.anthropic]
login = "claude"
from = "op://Private/Claude setup token/credential"
```

The 1Password CLI (`op`) must be installed and signed in. Only the launch
that starts the box asks 1Password; a session joining it does not.

Agents reach their real hosts; the proxy terminates TLS there and
adds the host credential, so a key the box sends to that host is
replaced. A box with a login route or an `https` injecting route starts
only if its image has a regular file at
`/etc/ssl/certs/ca-certificates.crt`, which Debian's `ca-certificates`
package provides. Codex uses HTTP, not WebSockets. Launch agents through PATH.
An explicit project `routes` table replaces the profile's routes.

A login route grants every process in the box the host login's authority
over every path at its origin; see [Login routes](docs/ARCHITECTURE.md#egress-proxy)
and the [threat model](docs/ARCHITECTURE.md#threat-model).

Each `pinfold -- CMD` starts or joins a session; the box ends when no session, Git crossing or `start`
keep remains, and `pinfold stop` ends everything. When a client leaves early (a signal, a
closed terminal, a lost connection), its command's process group gets
TERM, then KILL. Joining keeps the running policy and credentials; changed
host roots or effective configuration refuse as `config-changed`, so stop the box
first. `start` and `stop` take no arguments, print nothing to stdout, and
on success print one status line on stderr: `start` says `started` or `already
running`, `stop` says `stopped` or `no box running`. Edits to a share remain
after stop. A kept box protects its project from cleanup and executable
replacement; stop it before cleanup or an update.

Arguments after `--` pass literally to the command, including `--help`.
Command exit codes pass through. A `start` or `stop` infrastructure failure,
a project setup failure and an external teardown before observed completion
exit 125. CMD owns stdout; pinfold diagnostics use stderr.

Project reports use readable text, including when piped. Add `--json` to
`config`, `doctor` and `disk` for structured output. Use
`pinfold doctor --artifacts --json` for artifact pins.

A managed build of a chain that contains the bundled `default` recipe
re-resolves stable agent releases every time. Installation reruns when a
resolved version changes or with `--no-cache`, which forces the full
recipe; other steps use the layer cache. Update with `pinfold build` or
`pinfold build --profile NAME`. Launch never installs agents. A build
first rebuilds the profiles beneath it, root first. Other projects and
profiles on a rebuilt image report `image-outdated` at their next launch
if its identity changed.

Every built-in image includes standalone Pi, native Claude and Codex with
its code-mode host on PATH. Built-in profiles seed `.pi/agent/APPEND_SYSTEM.md`,
`.claude/CLAUDE.md` and `.codex/AGENTS.md` into a home only when missing, so
edited files and older seeds stay. Home settings follow the same rule.
Instructions do not enforce containment.

Build egress has its own allowlist. Add exact hosts with
`pinfold build --allow HOST`.

For a share, host edits are live and the box stages and commits in the
host's `.git`; a box commit is the host's commit. Host and
box Git at the same moment can lose commits or record files as deleted
(see the threat model). Export `GIT_OPTIONAL_LOCKS=0` in the shell
that runs `pinfold` and in host tools that poll `git status`; unless it is
exactly `0`, `pinfold -- CMD` warns `git-optional-locks` on stderr. It does not cover
host `git diff` or host commits while the box commits. There is no crossing;
the Git remote refuses as `share-no-crossing`. To select disk behavior on
macOS, set this in the project’s configuration:

```toml
mounts = [{ host = ".", kind = "disk" }]
```

A project disk appears in the box at `/project`, where a share appears
at the folder's own path; `$PINFOLD_PROJECT` holds that path unless your `env` sets it.
Clone inside the box, or send branches yourself. Only committed branches cross; commit local work (a
WIP commit is enough) to send it in. After the first start has created the disk:

```sh
git remote add box pinfold::"$(pwd -P)"   # once, at the project root
git push box main                         # a host branch into the box as host/main
git fetch box                             # guest branches as box/*, to merge, and new tags
```

Crossing never touches a working tree. The first push into a disk
without a repository creates an empty one there. To check a branch out,
run `git switch -c main host/main` in the box. Pushed tags (`--tags`,
`push.followTags`) land as the box's own tags. `git fetch box` brings
the box's new tags back as yours. A fetched tag is yours to publish, and one
named like a branch makes `git push box BRANCH` ambiguous until you delete it.

`git pull box`, a refspec such as `git fetch box main:other` and `--prune`
let the box's branches reach or delete your refs.
Without a running box, crossing starts a transient one on the project
disk, without egress.

Outside Git on macOS, a share exposes the folder’s existing host files. On Linux a folder outside Git
is refused as `not-git`. See
[Configuration](docs/ARCHITECTURE.md#configuration) for precedence and recipes.

To save a stopped disk into a new host directory, run
`pinfold disk export NAME DEST [--image IMAGE]`. It uses the local default
profile image unless an image is named; build that image explicitly if absent.
An attached or unclean disk is refused, and an existing destination is never
replaced. Exported files are untrusted: inspect them before opening or
executing them. Symlinks remain symlinks.

Optional profiles add tools. Each builds on the one above it:

| Profile | Base | Adds |
|---|---|---|
| `documents` | `default` | Bun, AnyDoc for local document conversion, Poppler for PDFs |
| `adam` | `documents` | gh, fff (pi extension) and ponytail; allows `github.com` and `api.github.com` |

Each one's directory under [profile/](profile) holds only what it adds.
Every profile seeds these
[Pi settings](profile/default/home/.pi/agent/settings.json).
In a running box, `cat /opt/pinfold/agents/install.txt` shows the agent versions
installed at build time.

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

`image` can also be a profile default; a project `image` overrides it. An
explicit image cannot be combined with a project Containerfile. With an
explicit image, `pinfold build` reports that there is no selected recipe
to build.

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

Keep Pi settings, extensions and skills in their native files under
`home/` and `share/`. Shared files are read-only at every start.

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
   Use `disk export` for other disk files, including an explicit macOS disk
   project outside Git. Share edits already reside on the host; stop revokes
   box access and preserves those edits.
2. Run `pinfold config --json`. Read the exact names from `project.disks.home`
   and `project.disks.project` (null for a share).
3. Run `pinfold disk rm NAME` for each disk you want to clear. It refuses
   attached disks; the removals are separate operations, so serialize access
   to the project. Never delete state or disk files directly.
4. Launch `pinfold -- pi` or `pinfold -- pi -p "TASK"` again. No image rebuild
   is needed; profiles, project configuration and the selected image remain intact.

For a share, removing the home never clears host shared files. For a disk
project, remove both disks for a fresh home and project. Removing only the
home preserves the guest repository; removing only the project preserves
sessions, settings and other home files. A deleted disk loses everything on
it, including uncommitted work; `git fetch box` brings back commits only.

`pinfold clean [--dry-run] [--idle AGE [--all-projects]]` reclaims disk
space: it runs the automatic maintenance now and removes the unused build
cache. `--dry-run` shows the measured storage per category, and what the
same flags would remove, without cleaning. `--idle AGE` also removes the
state and disks of the current folder's project if it has not run for that
long and has no live box; `--all-projects` widens that to every such
project, named by its folder. AGE is a whole number and a unit, `s`, `m`,
`h` or `d`, such as `30d`. That is never automatic. A project whose checkout
was moved or deleted keeps its state and disks; they go only with
`--idle AGE --all-projects` or `disk rm`.

## Configuration

See [Configuration](docs/ARCHITECTURE.md#configuration) for layers, keys and defaults.

A profile's `pinfold.toml` or the project's configuration sets the box's
environment with `env`. A literal is plain configuration and shows in
`pinfold config --json`. `from` reads a host variable, or a 1Password
reference (`from = "op://…"`), when the box starts, and neither pinfold
nor the box runtime writes it down.
Every `env` value is readable by any process in the box, so credentials
belong in a route ([GitHub access](#github-access)).

```toml
[env]
GIT_AUTHOR_NAME = "my-agent"
GIT_AUTHOR_EMAIL = { from = "MY_AGENT_EMAIL" }
```

Layers replace `env` name by name: the project over the profile, the
profile over its base. A missing `from` variable refuses the start. A second command
joining a running box with a different value for that variable is refused
as `config-changed`; stop the box to change it.

For a single command, forward host variables with `PINFOLD_ENV_<NAME>`.
It wins over `env`.

The proxy lets through only the hosts in `allow` and the configured
routes. A request to any other host gets a 403 carrying `Proxy-Status: pinfold`
with the reason; `curl -v` in the box shows it.

To let the box read host files outside the project, such as notes or a
dataset's documentation, add read-only folders after the project mount in
the project's configuration. `host` is absolute or `~/…`, a real path
with no links, outside the project; `guest` is where the box sees it.
The box can read everything in the folder and send it to any allowed
host. A change applies at the next box start. On Linux each start copies
each folder, taking time and host disk space in proportion to its size, and
the project mount is `kind = "disk"`.

```toml
mounts = [{ host = ".", kind = "share" }, { host = "~/notes/dataset", guest = "/notes" }]
```

On macOS a folder with `kind = "share"` is also writable by the box,
such as an outbox you open in Finder while the box runs.

For disposable Rust build outputs:

```sh
PINFOLD_ENV_CARGO_TARGET_DIR=/tmp/pinfold-target pinfold -- pi
```

`/tmp` is a 64 GiB scratch disk, empty at each box start and removed
with the box. Save deliverables in the
project’s actual cwd; commit them for disk-project crossing.

### GitHub access

Keep the token at the proxy with a GitHub login route:

```toml
[routes.github]
login = "github"
from = "op://…"   # or a host variable's name
```

The box's `git`, `gh` and `curl` then reach `github.com`,
`api.github.com` and `uploads.github.com` by their real names, and
`git@github.com:` remotes are rewritten to HTTPS. The proxy replaces any
credential the box sends to these hosts with the token. They need no `allow`
entry. The box can use the token but never read it. Prefer a fine-grained
token scoped to the one repository the box works on, with only the permissions it needs.

Release download (public and private repositories), upload and
`--archive`, and `gh api --paginate` work. `gh run download` does not: its artifacts redirect to varying
Azure hosts. A Java client fails at these hosts, since Java does not
read the box's CA bundle.

## Limits

Read the [threat model](docs/ARCHITECTURE.md#threat-model) and [disks](docs/ARCHITECTURE.md#disks) before use.
