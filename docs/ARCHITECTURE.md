# pinfold architecture

pinfold runs a coding agent in a disposable box: an Apple `container`
micro-VM on macOS, a rootless podman container on Linux. The box sees its
mounts and nothing else of the host. Its only way out is its own allowlisting
proxy.

- **Interactive:** `pi` on the host is a shim for `pinfold pi`.
- **Programmatic:** a caller such as a CI system or an agent orchestrator
  drives boxes through `pinfold box`, JSON on stdio.

This is the current spec. Evidence and rationale are in `docs/archive/`.

## Terms

**Box**: one disposable container or micro-VM, described by a box spec and
owned by one `box up` process.
_Avoid_: sandbox, container, VM, run

**Box spec**: the JSON a caller gives `box up`. It is the whole box: nothing
is added that the spec does not name.

**Project**: a checkout, identified by the path of its root. A moved
checkout is a new project.
_Avoid_: repo, workspace

**Profile**: data that shapes a box: an image, seeds, read-only shared
files, and config defaults.

**Project home**: a project's `$HOME`, a host directory outside the
checkout. It belongs to the project from its first start.

**Seed**: a profile file copied into a project home only when missing.
Deleting it there reseeds it at the next start.

## Layers

| Layer | Owns | Used by |
|---|---|---|
| core | runtimes, box lifecycle, transport, proxy, images, profiles, pinned artifacts, maintenance | programmatic callers, pi layer |
| pi | shim, per-project state, config, trust, `.git` protection, herdr | the `pinfold` CLI |
| profiles | box defaults, an image, pi's user-level config | applied as data |

Core knows nothing of pi or git; to core, a profile's pi config is files.
Core never reads a profile's `pinfold.toml`; the pi layer does, when it
writes a box spec. Profiles hold no code except pi packages.

## Threat model

A prompt injection turns the agent into an exfiltration path or into code
running on the host.

| Control | Effect |
|---|---|
| No network | Loopback only. One unix socket to the box's own proxy. |
| Egress proxy | Default deny, allowlist by name, every decision logged. |
| Kernel boundary | A VM per box on macOS. On Linux, boxes share the host kernel. |
| No privilege | Empty capability bounding set, nothing long-lived as uid 0, read-only rootfs, no setuid files. |
| Read-only `.git` | The box never writes the host's `.git`. |
| Protected editor config | `protect` directories, which the host runs on open, are read-only. |
| Profiles | Nothing a box can write maps to a profile. Only host-side commands change one. |

Not protected:
- Project contents: an agent with a model API can put them in a prompt.
- Files the host runs by explicit command: build scripts, tests, package
  scripts.
- A planted repo (`sub/.git`, or `.git` in a project that has none) runs
  code if a host tool runs git in it (VS Code does by default).
- CDN fronting beyond the SNI check (Host-header fronting needs TLS
  interception).
- Allowlisted services that accept writes (GitHub with a token, registries).
- Image builds: user-run, trusted, unrestricted egress.

## Runtimes

One runtime per OS, not configurable:

| Host | Runtime | Kernel |
|---|---|---|
| macOS 26+ | Apple `container` | one VM per box |
| Linux | rootless podman | the host's, shared |

Anything else is refused: podman on macOS, rootful podman, docker. Nothing
warns at run time; `doctor` reports the runtime.

podman preflight requires `rootless=true` and `cgroupManager=systemd`; under
cgroupfs, `--cpus` and `--memory` are silently not enforced. Long-lived boxes
need `loginctl enable-linger`.

## Box

### Lifecycle

1. Start the box's proxy on a new unix socket in the state dir. Keep the path
   under macOS's 104-byte limit.
2. Run the box with `--network none` and the socket carried in (Transport).
   PID 1 is `pinfold init`, as the host uid.
3. Apple only: one transient root exec makes the socket connectable.
4. `pinfold init` relays `127.0.0.1:3128` to the socket, reaps children, and
   reports ready.
5. `exec` work as the host uid:gid, with `HTTPS_PROXY` and `http_proxy` set
   to `http://127.0.0.1:3128`.
6. Remove the box, close the proxy, delete the socket.

One `pinfold box up` process owns one box. It does steps 1–4, holds the
proxy, and does step 6 on `down`, stdin EOF or SIGTERM. If it dies, the
socket dies and the box has no way out (fails closed); `box prune` removes
leftovers by label. The CLI runs the same code in-process.

### Process interface

```
pinfold box up < spec.json        # prints ready, holds the box, ends with down
pinfold box exec BOX [--tty] [--workdir D] -- argv…   # stdio through, exit code back
pinfold box down BOX              # same as closing up's stdin
pinfold box list --label k=v      # JSON lines; repeat --label to AND filters
pinfold box prune                 # remove boxes whose `up` is gone, print each removed
```

On a box that is absent, `exec` exits 3 with `pinfold box exec: no box
named ...`; any other exit code is the command's.

`up` prints one `ready` line once the box is up:

```json
{"event":"ready","box":NAME,"owner":PID,"labels":{…}}
```

`owner` is the `box up` process; `labels` is the box's full label set. At
`up`, the image's `dev.pinfold.*` identity labels are copied onto the box;
the spec's labels win on a clash.

The caller keeps `up`'s stdin open for the life of the box; closing it is
`down`. The stream ends with one `down` line after teardown:

```json
{"event":"down","box":NAME,"reason":REASON}
```

`REASON` is one of `stdin-closed`, `signal` (SIGTERM or SIGINT, which is
also what `box down` sends), or `exited` (the box's init ended on its own;
`detail` carries its exit status). `up` exits 0 for `stdin-closed` and
`signal`, and 1 for `exited`.

When `up` refuses, it prints one JSON line instead of `ready` and exits 1:

```json
{"event":"refused","box":NAME,"reason":REASON,"detail":TEXT}
```

`REASON` is `spec`, `profile`, `runtime`, `image-missing` or `name-in-use`;
`box` is null when the spec did not parse. Refusals are decided before
anything is created; a refused `up` leaves nothing.

`list` prints one JSON line per box whose labels match every `--label`.
`--label KEY=VALUE` matches that value; `--label KEY` matches any value of
`KEY`. Each line carries the box's labels, its `dev.pinfold.owner` pid (or
null), whether that process is alive, and the runtime's RFC 3339 `created`
time and `state` (`running` or `stopped`):

```json
{"name":NAME,"labels":{…},"owner":PID,"owner_alive":true,"created":RFC3339,"state":"running"}
```

`prune` removes each pinfold box whose `up` is gone and prints one line per
removed box, nothing when there was none:

```json
{"event":"pruned","box":NAME,"owner":PID}
```

The box spec `up` reads from stdin:

```json
{
  "name": "job-3f2a…",
  "labels": { "dev.example.job": "…" },
  "profile": "builder",
  "image": "…",
  "mounts": [{ "host": "/…/clone", "guest": "/workspace", "readonly": false }],
  "user": { "uid": 501, "gid": 20 },
  "env": { "HOME": "/state/home", "OPENROUTER_API_KEY": { "from": "OPENROUTER_API_KEY" } },
  "egress": { "allow": ["openrouter.ai"], "routes": { "api.internal": "127.0.0.1:7777" } },
  "cpus": 4,
  "memory": "8G"
}
```

- No `egress`: no way out at all (gates).
- `env` is exact; nothing is inherited. `{ "from": "NAME" }` is read from
  the caller's environment and passed as `--env NAME`, so values never reach
  argv.
- `profile` applies the profile's `home/` and `share/` (see Profiles), and
  its image if `image` is absent. Core never reads the profile's
  `pinfold.toml`: egress, env and resources come only from the spec.
- Apple: mounts are directories. Nested read-only mounts protect subpaths.
- `.git` protection belongs to the pi layer; a box spec gets only the
  mounts it names.

### Always applied

- `--network none`, `--cap-drop ALL`, `--read-only` with a tmpfs `/tmp`.
- PID 1 and all work run as the host uid:gid.
- `NODE_USE_ENV_PROXY=1`: Node's fetch ignores `HTTPS_PROXY` without it.
- podman adds: `--userns=keep-id`, `--security-opt no-new-privileges`, a
  seccomp profile that also blocks `CLONE_NEWUSER`, `--no-hosts`, an empty
  read-only `/etc/resolv.conf`, `--memory-swap` equal to `--memory`,
  `--pids-limit`, an explicit `HOME`, and `rm -f -t 0` at teardown.

Runtime command lines are built as data.

### Process supervision

- Always `-i`. `-t` only when stdin and stdout are both terminals, with the
  host `TERM` and `COLORTERM`.
- TTY: the runtime's exec stays in pinfold's foreground process group and
  handles raw mode and SIGWINCH itself. No TTY: its own process group.
- The box is the unit of lifetime; pinfold never signals a process in it.
  With a TTY, Ctrl-C and Ctrl-\ reach pi as bytes. A closed terminal,
  SIGTERM, SIGHUP, or SIGINT without a TTY removes the box.
  Revisit when apple/container#1941 lands: add a graceful SIGTERM before
  removal.
- Exit codes pass through; `128+n` for a signal death.
- After a SIGKILL of pinfold, `box prune` cleans up; the terminal needs
  `stty sane`.

## Transport

- **podman:** the socket is bind-mounted, mode 0600 in a 0700 dir.
- **Apple `container`:** `--ssh` with `SSH_AUTH_SOCK` set to the proxy socket
  for that one `container run`. It appears at
  `/var/host-services/ssh-auth.sock`, root-owned and not connectable by the
  box user; the root exec in step 3 chmods it. This is off-label use of
  `--ssh`.
- The socket carries only proxy connections. Init and the CLI come from one
  build (see Pinned artifacts), so this protocol has no versioning.
- Fallback if `--ssh` breaks: a per-box internal network with the proxy on
  the gateway and a root init that deletes routes (needs NET_ADMIN, SETUID,
  SETGID).
- apple/container#2247 (relay data loss under parallel live connections)
  did not reproduce. If it does, multiplex all streams over one connection.

## Egress proxy

In the pinfold process, one per box, listening only on the box's socket. No
network listener, no token: the socket identifies the box.

- **CONNECT:** port 443 only, to an allowlisted host (exact name, or
  `.suffix` for the name and its subdomains). The ClientHello SNI must equal
  the CONNECT host.
- **Plain HTTP:** port 80 only, one request per connection, Content-Length
  framing only. Ambiguous framing, folded headers and bare LF get 400.
- **Refused:** IP literals, and names resolving to loopback, unspecified,
  private, link-local, CGNAT (`100.64/10`) or benchmark (`198.18/15`)
  addresses. Resolve once; dial the checked address.
- **Routes:** a name maps to one host service, e.g.
  `api.internal → 127.0.0.1:7777`. Plain HTTP only, Host header rewritten.
  CONNECT to a route is refused. The host service authenticates its callers.
- **Limits:** a connection cap, a header timeout, an idle timeout on
  tunnels.
- **Log:** one JSON line per decision in the box's egress log at
  `~/.local/state/pinfold/egress/<box>.jsonl` (`$XDG_STATE_HOME` is
  honored). It names the host, the decision and its reason; no header value
  is ever written.

## Images

An image is a toolchain; the harness is not baked in. The default comes from
the profile's Containerfile. A project can name its own, typically `FROM` the
profile image.

- Images are built only by `pinfold build`. `pinfold pi` refuses when the
  image is missing and names the command.
- Every build gets an empty context: the Containerfile alone. For a project
  build, trust then covers every input. Files come in by `ADD --checksum` or
  from the profile image.
- A profile build is tagged uniquely `pinfold/profile-<name>:<build>` and
  moves the stable `pinfold/profile-<name>:latest` to it. The stable ref is
  what a project Containerfile `FROM`s and what `doctor` compares against.
  Every build is a distinct image even on a full cache hit, through the
  unique `dev.pinfold.build` label. Images carry `dev.pinfold.profile=<name>`
  and `dev.pinfold.base=<digest>` (the resolved base) so Maintenance can find
  them.
- A project build is tagged uniquely `pinfold/project-<id>:<build>` and
  moves the stable `pinfold/project-<id>:latest` to it, where `<id>` is the
  project's state id. A project image carries `dev.pinfold.project=<id>`,
  and records the digest of the profile image it was built from as
  `dev.pinfold.base`. `pinfold pi` prints one line when that is no longer
  the current profile image.
- A build never touches project homes.

The default profile's image:
- `debian:trixie-slim` by tag, `apt-get upgrade` at every build. The
  resolved base digest is recorded as a label.
- Debian: ca-certificates, git, curl, ripgrep, fd-find (as `fd`), jq, less.
- gh from GitHub's signed apt repository.
- `ADD --checksum=sha256:…`: bun, rtk, and the ponytail pi package. A bump
  script updates the pins.
- setuid and setgid bits stripped.

## Pinned artifacts

Checksummed release binaries in
`~/.cache/pinfold/artifacts/<name>/<version>/<os-arch>/`, mounted read-only:
pi.

The init is not an artifact: it always comes from the CLI's own build. The
macOS CLI embeds the `aarch64-unknown-linux-musl` build and writes it to the
cache once; a Linux CLI mounts its own executable.

## Profiles

A profile is data: box defaults, an image, and pi's user-level config. It
works the same for interactive runs and programmatic boxes, with or
without a TTY.

```
~/.config/pinfold/profiles/<name>/
  pinfold.toml    config defaults (the .pinfold.toml schema, less containerfile)
  Containerfile   the profile image
  home/           seeds for $HOME: a file is copied only if missing, then left alone
  share/          mounted read-only at /opt/pinfold/profile; live, never copied
```

- `default` is built into the binary (from `profile/` in this repo) unless
  the user has their own. User profiles are the user's files and need no
  trust.
- `pinfold profile new NAME [--from PROFILE]` writes a copy of `default`
  (or another profile) to be edited as files.
- Selected by `profile` in config or `"profile"` in a box spec.
- `home/` needs `$HOME` on a read-write mount. Seeds are copied at start,
  never at build. To reseed a file, delete it from the project home.
- pinfold does not manage pi settings: `pi install` in a box changes that
  project home only.

pi's two config levels both load in every run:
- **User level:** pi's agent dir under `$HOME`, seeded from `home/`, with
  packages from `share/`.
- **Project level:** the project's own `.pi/` and `.agents/skills`,
  pi-native.

The default profile's seed settings name its `share/pi` package, rtk and
ponytail, and set `defaultProjectTrust: "always"`: the box, not pi's prompt,
is the boundary, and without it pi's non-interactive modes silently skip
project config. Its `share/pi` holds skills and the operating-context
extension, which writes the box's facts into the system prompt: the
allowlist from `PINFOLD_ALLOW`, that a 403 is final, and that commits are
made on the host. Extensions run in the box; nothing security-relevant
lives in one.

The default profile's `pinfold.toml` sets no config, so it gets the
built-in defaults. `PI_OFFLINE` is unset, so pi's package installs go
through the proxy.

## Shared files

Ownership needs no work. Box-created files land on the host as the user's,
644 or 755; the exec bit and symlinks survive; git needs no
`safe.directory`. The box user can do whatever the host user can. Rootless
podman with keep-id is the same, and also shares inotify and locks.

Apple `container` limitations, documented for users:

| Limitation | Handling |
|---|---|
| ~1 s metadata and name cache: ENOENT or stale `stat` after a host atomic save | If it bites, a profile extension retries pi's reads once after 1 s. |
| No inotify for host changes | Polling (`CHOKIDAR_USEPOLLING`, `WATCHPACK_POLLING`). |
| `flock`/`fcntl` locks not shared; O_EXCL lockfiles are safe | Don't open one SQLite database from both sides. |
| A case-only rename is a no-op | Rename in two steps. |
| Small-file I/O 4–10× slower | Accepted. `/tmp` is tmpfs. |
| setuid and setgid bits cannot be set | Harmless. |

## Maintenance

Disk use stays bounded without the user thinking about it. pinfold only
removes what it created: its images and boxes carry `dev.pinfold.*` labels,
and its state lives under its own dirs. Each project's state records its
checkout path and last run.

Automatic, never prompting:
- After a build: keep the newest two images per source (a profile or a
  project), the second for rollback. Remove older ones and their dangling
  layers.
- At most once a day, at the start of any command: prune boxes whose owner
  is gone, leftover sockets, artifact versions no pin names, and egress
  logs older than 14 days.

`pinfold clean` lists sizes, then removes:
- everything automatic, now
- the runtime's build cache (Apple: the builder container)
- caches in project homes (`~/.cache`)
- state of projects whose checkout is gone
- with `--unused AGE`, state of projects not run for that long. Never
  automatic: state holds sessions and logins.

`--dry-run` only lists. `doctor` shows disk use per category and suggests
`clean` above 20 GB.

## pi layer

- **Harness:** pi, a pinned artifact at `/opt/pinfold/pi`.
- **State:** `~/.local/state/pinfold/projects/<name>-<hash>/home`, mounted as
  `$HOME`, where the hash is of the canonical project root path. pi's agent
  dir, sessions, `~/.config` and caches sit at their default paths. One per
  project, outside the checkout.
- **Arguments** pass through unchanged. The project is mounted at its own
  absolute path; the box starts in the invoking directory. Paths outside the
  project fail, and pi reports them.
- **Environment:** `PI_TELEMETRY=0`, `PI_SKIP_VERSION_CHECK=1`, the core's
  proxy variables, `PINFOLD_ALLOW` (the effective allowlist, for the
  operating-context extension), and `PINFOLD_ENV_*`.
- **TTY** whenever the host has one; otherwise pi runs in print mode.
- **Auth:** OAuth `/login` once per project; API keys through the
  environment.
- **`protect`:** read-only directory mounts for editor config the host runs
  on open. Always `.vscode/`, `.claude/` and `.idea/`, plus the configured
  list. One that is absent is created empty on the host before the run, so
  the box cannot create it, and removed after the run if still empty.
- **herdr:** no socket in v1. The TTY passes through, so screen detection
  works, and the shim sets `HERDR_AGENT=pi`.
- **Attach:** `pinfold attach [--box NAME] [cmd…]` execs bash (or cmd) in
  this project's running pi box. It fails if none is running and asks for a
  box name if several are; `--box NAME` selects one. Its processes end with
  the box.

### Git

The box never writes the host's `.git`. `<root>/.git` is mounted read-only
at its own path, and the mount point cannot be renamed. The agent reads
history and diffs; commits are made on the host.

Worktrees are refused: their `.git` is a file whose `gitdir:` line host git
follows, and Apple `container` cannot mount a file read-only.

## Configuration

Layers: environment > `.pinfold.toml` > the profile's `pinfold.toml` >
built-in defaults. Each key takes the highest layer that sets it, lists
included: a list replaces the ones below it, and `allow = []` allows
nothing. An environment variable that is set, even empty, sets its key.

| Key | Env | Default | Meaning |
|---|---|---|---|
| `profile` | `PINFOLD_PROFILE` | `default` | Profile name |
| `allow` | `PINFOLD_ALLOW` | `api.anthropic.com`, `platform.claude.com`, `api.openai.com`, `auth.openai.com`, `chatgpt.com`, `openrouter.ai`, `opencode.ai`, `registry.npmjs.org`, `pi.dev` | Hosts the proxy lets through |
| `routes` | `PINFOLD_ROUTES` | `{}` | Proxy routes to host services |
| `protect` | `PINFOLD_PROTECT` | `[]` | Read-only directories in the box, beyond the always-protected ones |
| `containerfile` | — | the profile's image | The project's Containerfile, a path relative to the project; typically FROM the profile image |
| `cpus` | `PINFOLD_CPUS` | 4 | |
| `memory` | `PINFOLD_MEMORY` | `8G` | |
| — | `PINFOLD_ENV_<NAME>` | — | `<NAME>` in the box; the only way host env enters |

The list keys take comma-separated values in the environment:

- `PINFOLD_ALLOW`: host names; a leading `.` makes a suffix entry.
- `PINFOLD_ROUTES`: `name=host:port` pairs.
- `PINFOLD_PROTECT`: project-relative directory paths.

**Trust.** `.pinfold.toml` and the Containerfile its `containerfile` names
are used only if their hashes match those `pinfold allow` recorded in
`~/.local/state/pinfold/trust` for this project. A change stops the run
until allowed again.

**Credentials** need no pinfold code: `op run -- pi …` for 1Password;
`PINFOLD_ENV_GH_TOKEN` for gh and git; direnv for per-repo tokens;
`PINFOLD_ENV_GIT_*` for identity.

## CLI

```
pinfold pi [pi args…]            pi in a box for this project; `pi` is a symlink to this
pinfold attach [--box NAME] [cmd…]   bash (or cmd) in this project's running pi box
pinfold build [--profile NAME]   build this project's image, or a profile's; prints the ref
pinfold allow                    trust this project's .pinfold.toml and Containerfile
pinfold profile new NAME [--from PROFILE]   copy a profile to edit as files
pinfold clean [--dry-run] [--unused AGE]   reclaim disk (see Maintenance)
pinfold doctor                   runtime, kernel, image, artifacts, trust, config, disk use
pinfold box …                    the process interface
pinfold init                     PID 1 in the box (Linux builds)
```

`pinfold --version` and `pinfold --help` answer without touching the runtime
or the state dir.

The project root is the git top level, else `$PWD`.

## Guarantees

Each has one end-to-end test. Testing policy is in `AGENTS.md`.

| # | Guarantee | Shown by |
|---|---|---|
| 1 | No network but loopback | Inside: only `lo`. Host addresses, `1.1.1.1` and `100.100.100.100:53` unreachable. Control: the fixture answers through a route. |
| 2 | Only allowlisted hosts get through | `api.github.com` answers. `example.com` gets a proxy 403, logged "not allowlisted". |
| 3 | The proxy refuses the tricks | Each with a control: IP literal, name resolving to loopback, SNI ≠ CONNECT host, CONNECT to a route, ambiguous framing. |
| 4 | A route reaches exactly one host service | `http://fixture.internal/` works; the fixture's host port is unreachable directly. |
| 5 | Nothing can gain privileges | `CapBnd` 0 in exec'd processes; no setuid or setgid files; rootfs not writable. |
| 6 | The environment is exactly the spec | An unprefixed host variable is absent; `PINFOLD_ENV_X` arrives as `X`; the secret never shows in host `ps`. |
| 7 | No egress means no way out | Without `egress`, nothing gets out, not even through a route. |
| 8 | Losing the owner fails closed | After SIGKILL of `box up`, the box has no egress, and `box prune` removes it. |
| 9 | The lifecycle works for a caller | `up` reports ready; `exec` streams and returns the exit code; `list` finds by label; `down` removes. |
| 10 | Host and box share files seamlessly | Box-created files are the user's, 644/755, exec bit intact. Host 0600/0700 files are writable in the box. A read-only mount rejects writes. |
| 11 | The box cannot write `.git` or protected config | Writing a hook, `core.fsmonitor`, `commondir`, renaming `.git`, writing `.vscode/`, or creating `.vscode/` in a project without one fails; host `git status` runs nothing. Control: a project file is writable. |
| 12 | A changed project file stops the run | The agent adds a domain to `.pinfold.toml`; the next run refuses until `pinfold allow`. |
| 13 | Project state persists and stays separate | Settings are seeded once and survive runs; a deleted seed returns; two projects don't see each other's state. |
| 14 | Both pi config levels load without a TTY | `pi -p` through the shim: the fake model's request carries a skill from the profile and one from the project's `.pi/`. |
| 15 | Cleanup removes only pinfold's garbage | After three builds of one source, two images remain. An unlabeled image, a live box and a project's state survive `pinfold clean`. |
| 16 | The highest layer sets the allowlist | Without project config the box's PINFOLD_ALLOW is the default list; a project's allow = ["api.github.com"] makes it exactly that host, and registry.npmjs.org is refused as not allowlisted. |
| 17 | up refuses before it creates | A missing image and a live name are refused as data, with no box, state dir or seed left; the box whose name was reused still answers exec. |

Linux (podman) runs in GitHub CI on `ubuntu-26.04` and `ubuntu-26.04-arm` as
the required gate. The workflow installs the pinned toolchain's musl target,
enables linger and a D-Bus user session for the runner user so podman's
cgroup manager is systemd, and clears AppArmor's unprivileged-userns
restriction, which the runner image enables and which denies rootless podman
the user namespace it needs. macOS (Apple `container`) runs on a macOS host
before each merge, on the exact commit being merged.

## Code

Rust. Each Linux build is a static musl binary and doubles as `pinfold init`.

Every process-interface operation is a `core` function that returns data;
`cli.rs` serializes it and nothing more. A Rust caller will link `core` and
see the same operations as one that spawns `pinfold box`, so a new verb is a
`core` function first.

| Target | Role |
|---|---|
| `aarch64-apple-darwin` | macOS CLI |
| `aarch64-unknown-linux-musl` | Linux arm64 CLI; init on Macs and arm64 hosts |
| `x86_64-unknown-linux-musl` | Linux x64 CLI; init on x64 hosts |

Linux targets build on a Mac with `cargo zigbuild`. The default profile and,
in the macOS CLI, the arm64 Linux init are embedded with `include_bytes!`.

Dependencies: `tokio`, `httparse`, `serde`, `serde_json`, `sha2`, `toml`, `nix`.
SNI comes from a small ClientHello parser.

Portability (Windows later means the Linux build in WSL2):
- Host-to-guest paths map in one function, the identity on Unix.
- Platform dirs are the literal XDG-style paths on both OSes:
  `~/.config/pinfold`, `~/.local/state/pinfold` and `~/.cache/pinfold`,
  with `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` honored.
- Unix-only host code (signals, process groups, TTY, modes) is one module.
- Runtimes are adapters behind a trait, chosen by target OS.

```
crates/pinfold/src/
  core/    runtime/{apple,podman}.rs plan.rs box.rs network.rs proxy.rs tls.rs
           artifacts.rs profile.rs clean.rs
  init.rs  socket mode, TCP relay, reaping, readiness
  pi/      launch.rs state.rs git.rs
  cli.rs config.rs trust.rs
profile/   the built-in default profile
```

## Open questions

- Not yet run end to end, so phase 1 proves them first: on Apple
  `container`, a client through the init relay, the socket and the proxy;
  PID 1 as the host uid with the transient root chmod exec; the SNI check.
- A read-only `.git` mount on Apple `container`: writes fail and the mount
  point cannot be renamed, as on podman.
- `198.18/15` blocking breaks fake-IP DNS proxies (Surge, Clash).
  Configurable, or documented.
- Nested user namespaces in the Apple `container` guest kernel: can they be
  turned off?
- Where herdr reads `HERDR_AGENT`.
- Memory limits and OOM detection on Apple `container`.
- No disk cap on Apple `container`.
- Model credentials injected at the proxy, so the box sees a placeholder.
