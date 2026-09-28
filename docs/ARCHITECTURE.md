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
  scripts, and scripts a hook calls from the project, such as husky's
  `.husky/pre-commit`.
- A planted repo (`sub/.git`, or `.git` in a project that has none) runs
  code if a host tool runs git in it (VS Code does by default).
- CDN fronting beyond the SNI check (Host-header fronting needs TLS
  interception).
- Allowlisted services that accept writes (GitHub with a token, registries).
- Image builds: user- or caller-run, trusted, unrestricted egress.

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

1. Claim the name: create the state dir `boxes/<key>` exclusively and write
   the owner's pid, and hold an exclusive lock on that `pid` file for `up`'s
   whole life. `<key>` is the first 16 hex digits of the sha256 of the name,
   so the socket path's length does not depend on the name. The owner is
   alive while the lock is held. A live owner already there is `name-in-use`;
   a dead owner's dir is reclaimed. Nothing (seeds, proxy, box) is created
   before the claim.
2. Start the box's proxy on a new unix socket in the state dir. Keep the path
   under macOS's 104-byte limit.
3. Run the box with `--network none` and the socket carried in (Transport).
   PID 1 is `pinfold init`, as the host uid.
4. Apple only: one transient root exec makes the socket connectable. It
   waits until the runtime lists the box running: Apple records that only
   after the box's first process has started, and refuses an exec before.
5. `pinfold init` relays `127.0.0.1:3128` to the socket, reaps children, and
   reports ready.
6. `exec` work as the host uid:gid, with `HTTPS_PROXY` and `http_proxy` set
   to `http://127.0.0.1:3128`.
7. Remove the box and delete the socket.

One `pinfold box up` process owns one box. It does steps 1–5, holds the
proxy, and does step 7 on `down`, stdin EOF or SIGTERM. If it dies, the
socket dies and the box has no way out (fails closed); `box prune` removes
leftovers host-wide. The CLI runs the same code in-process.

### Process interface

```
pinfold box up < spec.json        # prints ready, holds the box, ends with down
pinfold box exec BOX [--tty] [--workdir D] -- argv…   # stdio through, exit code back
pinfold box stat BOX              # one JSON object of the box's memory, pids and OOM kills
pinfold box down BOX              # signals up to tear down; idempotent: an absent box exits 0
pinfold box list --label k=v      # JSON lines; repeat --label to AND filters
pinfold box prune                 # remove boxes whose `up` is gone, print each removed
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]… [--no-cache]   # one JSON line
pinfold image rm NAME             # one JSON line
```

`image build` builds a caller image (see Images) and prints one line on
stdout; the build's progress never reaches stdout. On success, exit 0:

```json
{"event":"built","image":NAME,"ref":"pinfold/image-<NAME>:<build>","latest":"pinfold/image-<NAME>:latest","id":ID,"labels":{…},"base":DIGEST|null}
```

`id` is the image's runtime id, the same value a box's `ready` and `list`
report as `image.id`; it is null when the runtime cannot resolve the built
ref after a successful build. `labels` is every label the build put on the
image. A failed build exits 1 with the last 40 lines of the runtime's
build output:

```json
{"event":"failed","image":NAME,"log":[LINE,…]}
```

A refusal before the build (a bad name or argument, a missing context or
Containerfile, a `dev.pinfold.` label, a missing runtime) exits 1 and
builds nothing; `image` is null when no name was given:

```json
{"event":"refused","image":NAME,"reason":"spec"|"runtime","detail":TEXT}
```

`image rm` (see Maintenance) exits 0 with the ids it untagged and the ids
whose last tag it kept because a box uses them, either list possibly
empty; an untagged id another name still tags stays; a bad name is refused as
`spec` like a build's:

```json
{"event":"removed","image":NAME,"ids":[ID,…],"in_use":[ID,…]}
```

On a box that is absent, `exec` exits 3 with `pinfold box exec: no box
named ...`; any other exit code is the command's.

`stat` prints one JSON object with every key present:

```json
{"box":NAME,"oom_kills":N|null,"memory":{"current":BYTES|null,"peak":BYTES|null,"limit":BYTES|null},"pids":{"current":N|null,"limit":N|null}}
```

On podman every field comes from the box's cgroup, found through the
runtime. Apple answers the limits from what it reports for the box and null
for the rest. A field the runtime cannot answer is null. `oom_kills` is
monotonic for the box's life; the caller keeps its own baseline. On an
absent box, `stat` exits 3 with `pinfold box stat: no box named ...`, like
`exec`.

`up` prints one `ready` line once the box is up:

```json
{"event":"ready","box":NAME,"owner":PID,"labels":{…},"image":{"id":ID,"ref":REF},"egress_log":PATH|null}
```

`owner` is the `box up` process; `labels` is the box's full label set, as
the runtime reports them, image labels included. `image.id` is the image the
runtime resolved `image` to; `image.ref` is the reference as the spec (or its
profile) gave it. `list` spells `ref` as the runtime records it (podman adds
`localhost/`), so a caller compares images by `id`. At `up`, the image's
`dev.pinfold.*` identity labels are copied onto the box, unless pinfold set
that label itself (a pi box's `dev.pinfold.project`). `egress_log` is the
absolute path of the box's egress log (see Egress proxy), null without
`egress`.

The caller keeps `up`'s stdin open for the life of the box; closing it is
`down`. The stream ends with one `down` line after teardown:

```json
{"event":"down","box":NAME,"reason":REASON}
```

`REASON` is one of `stdin-closed`, `signal` (SIGTERM or SIGINT, which is
also what `box down` sends), or `exited` (the box's init ended on its own;
`detail` carries its exit status). `up` exits 0 for `stdin-closed` and
`signal`, and 1 for `exited`; `box` is null when a signal ended `up` before
its spec parsed.

When `up` refuses, it prints one JSON line instead of `ready` and exits 1:

```json
{"event":"refused","box":NAME,"reason":REASON,"detail":TEXT}
```

`REASON` is `spec`, `profile`, `runtime`, `image-missing`, `name-in-use`
or `login`;
`box` is the `name` of stdin's first JSON value when that is an object with
a string `name`, else null.
Refusals are decided before anything is created; a refused `up` leaves
nothing. `up` asks the runtime to resolve `image`, so any reference the
runtime resolves locally is accepted; `image-missing` means the runtime could
not.

When `up` fails after it began creating, it removes what it made, prints one
`failed` line instead of `ready` or `down`, and exits 1:

```json
{"event":"failed","box":NAME,"detail":TEXT}
```

A SIGTERM or SIGINT before `ready` tears down the same way and ends with
`down`, reason `signal`, exit 0. Every `up` ends its stdout with exactly one
of `refused`, `failed` or `down`.

`list` prints one JSON line per box whose labels match every `--label`.
`--label KEY=VALUE` matches that value; `--label KEY` matches any value of
`KEY`; every box carries `dev.pinfold.owner`, so `--label dev.pinfold.owner`
lists them all. Each line carries the box's labels and the image it runs, as the
runtime records it, then its `dev.pinfold.owner` pid (or null), whether it still holds the lock on the box's `pid` file (for a box
from another state dir, whether that pid is alive), and the runtime's RFC
3339 `created` time and `state` (`running` or `stopped`):

```json
{"name":NAME,"labels":{…},"image":{"id":ID,"ref":REF},"owner":PID,"owner_alive":true,"created":RFC3339,"state":"running"}
```

`prune` removes every pinfold box on the host whose `up` is gone and prints one line per
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
  "harness": "pi",
  "image": "…",
  "mounts": [{ "host": "/…/clone", "guest": "/workspace", "readonly": false }],
  "env": { "HOME": "/state/home", "GH_TOKEN": { "from": "GH_TOKEN" } },
  "egress": {
    "allow": ["api.github.com"],
    "routes": {
      "api.internal": "127.0.0.1:7777",
      "openrouter": {
        "to": "https://openrouter.ai",
        "headers": { "Authorization": { "from": "OPENROUTER_API_KEY", "prefix": "Bearer " } }
      }
    }
  },
  "cpus": 4,
  "memory": "8G"
}
```

- No `egress`: no way out at all (gates).
- `env` is exact; nothing is inherited. An env name must match
  `[A-Za-z_][A-Za-z0-9_]*`; any other name is refused as `spec`, naming it.
  `{ "from": "NAME" }` is read from the caller's environment and passed as
  `--env NAME`, so values never reach argv. The runtime executable is found
  on the host's `PATH`, never the spec's.
- A label key starting with `dev.pinfold.` is refused as `spec`, naming
  it: the namespace is pinfold's.
- `profile` applies the profile's `home/` and `share/` (see Profiles), and
  its image if `image` is absent; egress, env and resources come only
  from the spec.
- `harness` is `pi`, `claude` or `codex` (see Pinned artifacts); any other
  value is refused as `spec`, naming it and the accepted names. It mounts
  that harness's directory read-only at `/opt/pinfold/<name>`, its
  executable `/opt/pinfold/<name>/<name>`, and sets its env defaults (pi:
  `PI_TELEMETRY=0`, `PI_SKIP_VERSION_CHECK=1`; claude:
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`; codex: none) and
  `PINFOLD_ALLOW` (the spec's `egress.allow`, empty without egress). The
  spec's own `env` wins; a spec mount at `/opt/pinfold/<name>` is refused.
  claude is the glibc build, so it needs a glibc image (the default profile
  is Debian). codex's `codex-code-mode-host` sits beside it: codex spawns it
  from there for every MCP tool call. codex's own sandbox does not start in
  a box; a caller runs codex with `sandbox_mode = "danger-full-access"`. It needs no profile.
- Mounts are directories. A host path that exists and is not a directory
  is refused as `spec`, naming it. Both runtimes apply a mount nested in
  another inside it whatever the spec's order, so a read-only `REPO/.git`
  inside a writable `REPO` protects the repository on both runtimes; two
  mounts at one guest path are refused.
- A mount path holding `,` or an ASCII control character is refused as
  `spec`, naming the path.
- `memory` is a whole number followed by `M` or `G`, at least `256M`;
  anything else is refused as `spec`, naming it.
- A string route value must be `host:port`, the port a number in
  1–65535; anything else is refused as `spec`, naming the route. Only the
  object form `{ "to": … }` takes an `http://` or `https://` origin.
- An unknown key at any level of the spec is refused as `spec`, naming the
  key; pinfold never applies a spec partially.

### Always applied

- `--network none`, `--cap-drop ALL`, `--read-only` with a tmpfs `/tmp`.
- PID 1 and all work run as the host uid:gid.
- `NODE_USE_ENV_PROXY=1`: Node's fetch ignores `HTTPS_PROXY` without it.
- podman adds: `--userns=keep-id`, `--security-opt no-new-privileges`, a
  seccomp profile that also blocks `CLONE_NEWUSER`, `--no-hosts`, an empty
  read-only `/etc/resolv.conf`, `--memory-swap` equal to `--memory`,
  `--pids-limit`, `--http-proxy=false`, an explicit `HOME`, and `rm -f -t 0`
  at teardown.

Runtime command lines are built as data.

### Process supervision

- Always `-i`. `-t` only when stdin and stdout are both terminals, with the
  host `TERM` and `COLORTERM`.
- TTY: the runtime's exec stays in pinfold's foreground process group and
  handles raw mode and SIGWINCH itself. No TTY: its own process group.
- The box is the unit of lifetime; pinfold never signals a process in it.
  Once pi runs on a TTY, Ctrl-C and Ctrl-\ reach it as bytes. A closed
  terminal, SIGTERM, SIGHUP or SIGINT removes the box.
  Revisit when apple/container#1941 lands: add a graceful SIGTERM before
  removal.
- Exit codes pass through; `128+n` for a signal death.
- Every process started in a box through `exec` runs with `oom_score_adj`
  1000, so the kernel's OOM killer takes the box's own processes before its
  init.
- After a SIGKILL of pinfold, `box prune` cleans up; the terminal needs
  `stty sane`.

## Transport

- **podman:** the socket is bind-mounted, mode 0600 in a 0700 dir.
- **Apple `container`:** `--ssh` with `SSH_AUTH_SOCK` set to the proxy socket
  for that one `container run`. It appears at
  `/var/host-services/ssh-auth.sock`, root-owned and not connectable by the
  box user; the root exec in step 4 chmods it.
- The socket carries only proxy connections. Init and the CLI come from one
  build (see Pinned artifacts), so this protocol has no versioning.

## Egress proxy

In the pinfold process, one per box, listening only on the box's socket. No
network listener, no token: the socket identifies the box.

- **CONNECT:** port 443 only, to an allowlisted host (exact name, or
  `.suffix` for the name and its subdomains). The ClientHello SNI must equal
  the CONNECT host.
- **Plain HTTP:** port 80 only, one request per connection, Content-Length
  framing only. Ambiguous framing and bare LF get 400, as does any head the
  parser rejects, folded headers included; a CONNECT head is parsed the
  same way.
- **Refused:** IP literals, and names resolving to loopback, unspecified
  (`0/8`), private, link-local, CGNAT (`100.64/10`), benchmark
  (`198.18/15`), IETF protocol (`192.0.0/24`), multicast, reserved (`240/4`,
  with broadcast), site-local (`fec0::/10`) or NAT64 local-use
  (`64:ff9b:1::/48`) addresses. An IPv6 address that embeds an IPv4 one
  (IPv4-mapped, IPv4-compatible, NAT64 `64:ff9b::/96`, 6to4) is checked as
  that IPv4 address. Resolve once; dial the checked address.
- **Routes:** a name maps to one host service, e.g.
  `api.internal → 127.0.0.1:7777`. Plain HTTP only, Host header rewritten,
  `Upgrade` dropped (no WebSocket). CONNECT to a route is refused. The host
  service authenticates its callers.
- **Injecting routes:** a route whose value is `{ "to": ORIGIN, "headers":
  { NAME: { "from": VAR, "prefix": "…" } } }`. `to` is an `http://` or
  `https://` origin. The box sends `http://<route>/path` as for any route;
  the proxy dials `to`, over TLS for `https` with SNI and the certificate
  checked against the host's roots, sets `Host` to `to`'s authority, and
  sets each named header to `prefix` plus `$VAR` from `up`'s own
  environment, replacing any the box sent under that name. Values are read
  once at start; a missing variable refuses `up`. An `https` target is
  resolved and checked like an allowlisted host; an `http` target is a host
  service. The value never reaches argv, the box, the spec or the log. The
  box can use the credential but not read it, unless `to` echoes request
  headers back. Header names that frame the request (`Host`,
  `Content-Length`, hop-by-hop) are refused.
- **Login routes:** a route whose value is `{ "login": HARNESS }` carries
  a subscription login for the box's harness; pinfold owns the origin, the
  headers and the harness's config for the pinned version. `login` must
  name the spec's `harness`, and a spec has at most one login route; else
  `spec`. An optional `to` replaces the harness's origin, checked like an
  injecting route's. The box gets only placeholders; the spec's own `env`
  wins.
  - `claude`: `{ "login": "claude", "from": VAR }`, where `$VAR` is a
    `claude setup-token` token read once at start like an injecting route's.
    The route goes to `https://api.anthropic.com` with `Authorization:
    Bearer`. The box gets `ANTHROPIC_BASE_URL=http://<route>` and a
    placeholder `CLAUDE_CODE_OAUTH_TOKEN`.
  - `codex`: `{ "login": "codex" }` uses the host's own Codex login, in
    whatever store Codex keeps it. The proxy gets the access token from
    the pinned host helper (Pinned artifacts): `codex-app-server` with the
    built-in `openai` provider forced and `up`'s environment, sent
    `initialize` and `getAuthStatus { includeToken: true }` on stdio. Codex
    refreshes and writes its own store; pinfold never reads it, and holds
    the token in memory only. The token's JWT must carry a future `exp`
    and an account id; the route goes to `https://chatgpt.com` with
    `Authorization: Bearer` and `ChatGPT-Account-ID`. Within 5 minutes of
    `exp`, the proxy asks the helper again under one host-wide lock, at
    most once a minute per box; a request on a lapsed token is refused and
    logged `login lapsed`. The box gets `/etc/codex/config.toml`, mounted
    read-only, selecting a custom provider with `base_url =
    "http://<route>/backend-api/codex"`, `wire_api = "responses"` and
    `requires_openai_auth = false`; it is Codex's system layer, so the
    caller's own config still overrides it.
  - `up` refuses a login it cannot use (no token, or not a usable JWT) as
    `login`, naming the harness.
  - `getAuthStatus` is deprecated at this pin. When a pin drops it, the
    likely replacement is `account/read` plus reading Codex's file store,
    which would stop supporting a Keychain store: a spec change.
- **Limits:** a connection cap and a header timeout. A tunnel is idle, and
  closed, only when neither direction has carried bytes for 5 minutes.
- **Log:** one JSON line per decision in the box's egress log at
  `~/.local/state/pinfold/egress/<box>.jsonl` (`$XDG_STATE_HOME` is
  honored). It names the decision's UTC time (RFC 3339 to the second), the
  host, the decision and its reason. A route request's line also names the
  method, the request path without its query, and the upstream's status
  (null when no status line arrived). No header value is ever written.

## Images

An image is a toolchain; the harness is not baked in. The default comes from
the profile's Containerfile. A project can name its own, typically `FROM` the
profile image.

- Images are built only by `pinfold build` and `pinfold image build`.
  `pinfold pi` refuses when the image is missing and names the command.
- A profile or project build gets an empty context: the Containerfile
  alone. For a project build, trust then covers every input. Files come in
  by `ADD --checksum` or from the profile image. Every step reruns at every
  build (no layer cache).
- A build's identity is its unique tag, never an image label. A caller
  build whose layers and labels are unchanged returns the existing image.
  Every build sets both family labels, `dev.pinfold.profile` and
  `dev.pinfold.project`, and `dev.pinfold.base`, the other family empty,
  so an image inherits no family and no base from the image it builds on.
- A profile build is tagged uniquely `pinfold/profile-<name>:<build>` and
  moves the stable `pinfold/profile-<name>:latest` to it. The stable ref is
  what a project Containerfile `FROM`s and what `doctor` compares against.
  Images carry `dev.pinfold.profile=<name>` and `dev.pinfold.base=<digest>`
  (the resolved base) so Maintenance can find them.
- A caller image is built by `pinfold image build NAME` from a context
  directory the caller vouches for, as it vouches for its mounts: no trust
  record, no project, no `.pinfold.toml`. The Containerfile may lie outside
  the context. NAME is validated like a profile name. The build is tagged
  uniquely `pinfold/image-<NAME>:<build>` and moves the stable
  `pinfold/image-<NAME>:latest` to it; a box spec's `image` names either.
  NAME lives only in these tags, never in the image, so identical inputs
  under two names build one image. It carries
  `dev.pinfold.base` (the first `FROM`'s digest when it resolves to one,
  else empty; a base pinfold built is read from local storage, never
  pulled), both family labels empty, and
  the caller's `--label`s, none of which may start with `dev.pinfold.`. It uses
  the runtime's layer cache unless the caller passes `--no-cache`; podman
  labels the cache's intermediate images `dev.pinfold.layer`.
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
- Debian: ca-certificates, git, curl, ripgrep, fd-find (as `fd`), jq, less,
  and unzip for bun's release archive.
- gh from GitHub's signed apt repository.
- `ADD --checksum=sha256:…`: bun, rtk, and the ponytail pi package. A bump
  script updates the pins.
- setuid and setgid bits stripped.

## Pinned artifacts

`crates/pinfold/harnesses.toml`, embedded at build time, pins each harness
(pi, claude, codex): a version, env defaults, and per box `os-arch`
(`linux-arm64`, `linux-x64`) its release assets, each a literal URL, its
sha256, and how it installs (a `tar.gz` whose one top-level entry is
installed, or a bare executable) and at which path. On a box's first use of
a harness, its assets are downloaded, checked and installed into
`~/.cache/pinfold/artifacts/<name>/<version>/<os-arch>/`; its `<name>/`
directory there is mounted read-only at `/opt/pinfold/<name>`. An install
records its files; a use that finds one missing reinstalls it. A checksum
mismatch fails the start, naming the harness, version and asset. A harness
no box asks for is never downloaded. A harness moves only with a pinfold
release.

codex also pins a host helper, `codex-app-server`, per host `os-arch`
(`darwin-arm64`, `linux-arm64`, `linux-x64`), installed the same way into
`~/.cache/pinfold/artifacts/codex/<version>/host-<os-arch>/` on the first
`up` with a codex login route. It never enters a box.

The init is not an artifact: it always comes from the CLI's own build. The
macOS CLI embeds the `aarch64-unknown-linux-musl` build and writes it to the
cache once; a Linux CLI mounts its own executable.

## Profiles

A profile works the same for interactive runs and programmatic boxes,
with or without a TTY.

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
- `pinfold profile new` (CLI) writes a copy of `default` (or another
  profile) to be edited as files; `--from-project [PATH]` (PATH is the
  current project root by default) merges the project's `home/.pi/agent/`
  into the copy, minus `auth.json`, `sessions/`, `npm/` and caches.
- Selected by `profile` in config or `"profile"` in a box spec.
- `home/` needs `$HOME` on a read-write mount.
- pinfold does not manage pi settings: `pi install` in a box changes that
  project home only.

pi's two config levels both load in every run:
- **User level:** pi's agent dir under `$HOME`, seeded from `home/`, with
  packages from `share/`.
- **Project level:** the project's own `.pi/` and `.agents/skills`,
  pi-native.

The default profile's seed settings name its `share/pi` package, rtk and
ponytail, and set `defaultProjectTrust: "always"`: the box, not pi's prompt,
is the boundary. Its `share/pi` is a pi package: the operating-context
extension, which writes the box's facts into the system prompt (the
allowlist from `PINFOLD_ALLOW`, that a 403 is final, and that commits are
made on the host), and a `skills/` directory for the profile's skills, live
and read-only in every box.

The default profile's `pinfold.toml` sets no config, so it gets the
built-in defaults. `PI_OFFLINE` is unset, so pi's package installs go
through the proxy.

## Shared files

Box-created files land on the host as the user's, 644 or 755; the exec bit and symlinks survive. Apple `container` can show
the top directory of a mount as root-owned inside the box, so git needs
`safe.directory` for a mounted repository: the pi layer sets it for the
project root in the box's environment; a caller-owned box sets it in its
spec's `env` or on git's command line. The box user can do whatever the
host user can. Rootless podman with keep-id is the same, and also shares
inotify and locks.

Apple `container` limitations, documented for users:

| Limitation | Handling |
|---|---|
| ~1 s metadata and name cache: ENOENT or stale `stat` after a host atomic save | Not handled; a reader retries. |
| A mount's top directory may be root-owned inside the box | `safe.directory`, as above. |
| No inotify for host changes | The box polls (`CHOKIDAR_USEPOLLING`, `WATCHPACK_POLLING`); pinfold sets neither. |
| `flock`/`fcntl` locks not shared; O_EXCL lockfiles are safe | Don't open one SQLite database from both sides. |
| A case-only rename is a no-op | Rename in two steps. |
| Small-file I/O 4–10× slower | Accepted. `/tmp` is tmpfs. |
| setuid and setgid bits cannot be set | Harmless. |

## Maintenance

pinfold only removes what it created: its images and boxes carry `dev.pinfold.*` labels,
and its state lives under its own dirs. Each project's state records its
checkout path and last run.

Automatic, never prompting:
- After a build: keep the newest two builds per source (a profile, a
  project or a caller image name), the second for rollback, ordered by
  their tags. Remove older builds' tags; an image goes with its last tag,
  and its dangling layers with it. An image a box still uses keeps its
  last tag, and pins only itself. A caller build in the last hour stays, so the ref its
  `built` line named still comes up; past the hour the newest two rule
  applies.
- At most once a day, at the start of any working command (not
  `--version`, `--help` or `init`): prune boxes whose owner
  is gone (nothing holds the lock on its `pid` file), leftover sockets,
  artifact `<name>/<version>` directories no pin names (`init/` aside),
  builds beyond each source's newest two (the after-build rule, applied
  even when no build follows), and egress logs older than 14 days.

A caller image name's builds are its `pinfold/image-<NAME>` tags; the
rules above never touch another name's tags.

`pinfold image rm NAME` retires a caller image name: it removes every
tag of NAME, whatever its age, and no other name's, except an image's
last tag while a box uses it. An image goes with its last tag.

`pinfold clean` lists sizes, then removes:
- everything automatic, now
- the runtime's build cache (Apple: the builder container; podman: the
  `dev.pinfold.layer` intermediate images no image builds on)
- caches in project homes (`~/.cache`), except a project's with a live box
- state of projects whose checkout is gone
- with `--unused AGE`, state of projects not run for that long, except a
  project's with a live box. Never automatic: state holds sessions and
  logins.

`--dry-run` only lists. `doctor` shows disk use per category and suggests
`clean` above 20 GB.

## pi layer

- **Harness:** pi, a pinned harness at `/opt/pinfold/pi`; `pinfold pi`
  asks core for it with `"harness": "pi"`.
- **State:** `~/.local/state/pinfold/projects/<name>-<hash>/home`, mounted as
  `$HOME`, where the hash is of the canonical project root path. pi's agent
  dir, sessions, `~/.config` and caches sit at their default paths. One per
  project, outside the checkout.
- **Arguments** pass through unchanged. The project is mounted at its own
  absolute path; the box starts in the invoking directory. Paths outside the
  project fail, and pi reports them.
- **Environment:** the harness's variables (Box spec, `harness`), the
  core's proxy variables, `HERDR_AGENT`, git's `safe.directory` entry
  (Shared files), and `PINFOLD_ENV_*`.
- **TTY** whenever the host has one.
- **Auth:** OAuth `/login` once per project; API keys through the
  environment.
- **`protect`:** read-only directory mounts for editor config the host runs
  on open. Always `.vscode/`, `.claude/` and `.idea/`, plus the configured
  list. One that is absent is created empty on the host before the run, so
  the box cannot create it, and removed after the run if still empty. One
  that is a symlink, or is reached through one, refuses the run.
- **herdr:** no socket in v1. The TTY passes through, so screen detection
  works, and the shim sets `HERDR_AGENT=pi`.
- **Attach:** `pinfold attach [--box NAME] [cmd…]` execs bash (or cmd) in
  this project's running pi box. It fails if none is running and asks for a
  box name if several are; `--box NAME` selects one. Its processes end with
  the box.

### Git

`<root>/.git` is mounted read-only at its own path, and the mount point
cannot be renamed. A `core.hooksPath`
inside the project, as host git resolves it (global config included), is
read-only too. The agent reads history and diffs;
commits are made on the host. A caller-owned box protects its repository
the same way by mounting `.git` read-only; its commits are the caller's own
host-side operation, hooks and signing off.

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
| `routes` | `PINFOLD_ROUTES` | `{}` | Proxy routes: `name = "host:port"` for a host service, or an injecting route `name = { to = "https://…", headers = { Authorization = { from = "VAR", prefix = "Bearer " } } }` (see Egress proxy) |
| `protect` | `PINFOLD_PROTECT` | `[]` | Read-only directories in the box, beyond the always-protected ones |
| `containerfile` | — | the profile's image | The project's Containerfile, a path relative to the project; typically FROM the profile image |
| `cpus` | `PINFOLD_CPUS` | 4 | |
| `memory` | `PINFOLD_MEMORY` | `8G` | |
| — | `PINFOLD_ENV_<NAME>` | — | `<NAME>` in the box; the only way host env enters |

The list keys take comma-separated values in the environment:

- `PINFOLD_ALLOW`: host names; a leading `.` makes a suffix entry.
- `PINFOLD_ROUTES`: `name=host:port` pairs; an injecting route needs a file.
- `PINFOLD_PROTECT`: project-relative directory paths.

`pinfold config` prints the effective configuration, spec-shaped.

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
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]… [--no-cache]   a caller's image from its own context; one JSON line
pinfold image rm NAME            retire a caller image name; one JSON line
pinfold allow                    trust this project's .pinfold.toml and Containerfile
pinfold profile new NAME [--from PROFILE] [--from-project [PATH]]   copy a profile to edit as files
pinfold clean [--dry-run] [--unused AGE]   reclaim disk (see Maintenance)
pinfold doctor                   runtime, kernel, image, artifacts, trust, config, disk use
pinfold artifacts                the pinned harnesses as JSON: name, version, path, cached, assets
pinfold config [ROOT]            the effective configuration and project facts as JSON, for callers
pinfold box …                    the process interface
pinfold init                     PID 1 in the box (Linux builds)
```

`artifacts` prints one JSON array, an object per harness: `name`, `version`,
`path` (the host directory mounted at `/opt/pinfold/<name>`), `cached`,
and `assets`, a list of `{url, sha256}` for this host's `os-arch`. Nothing
is downloaded. `doctor` reports the same list.

`pinfold --version` and `pinfold --help` answer without touching the runtime
or the state dir; `--help` or `-h` after a subcommand, before any `--`,
prints that subcommand's syntax line and exits 0, touching neither. `pi`
passes it to pi, and `attach` to its command once one is given.

The project root is the git top level, else `$PWD`. Inside a git directory,
the command refuses.

## Guarantees

Each has one end-to-end test. Testing policy is in `AGENTS.md`.

| # | Guarantee | Shown by |
|---|---|---|
| 1 | No network but loopback | Inside: only `lo`; `1.1.1.1` unreachable. Control: the fixture answers through a route. |
| 2 | Only allowlisted hosts get through | `api.github.com` answers. `example.com` gets a proxy 403, logged "not allowlisted"; `ready` names that log's path. |
| 3 | The proxy refuses the tricks | Each with a control: IP literal, name resolving to loopback, SNI ≠ CONNECT host, CONNECT to a route, ambiguous framing. |
| 4 | A route reaches exactly one host service | `http://fixture.internal/` works; the request's route line names the DELETE method, the queryless path and the fixture's non-200 status, with the query sentinel absent. |
| 5 | Nothing can gain privileges | `CapBnd` 0 in exec'd processes; PID 1 runs as the host uid; no setuid or setgid files; rootfs not writable; on Linux `unshare -U` fails. |
| 6 | The environment is exactly the spec | The host's unprefixed proxy variables are absent; `PINFOLD_ENV_X` arrives as `X`; the secret never shows in host `ps`; a `PINFOLD_ENV_` name that is not a POSIX name is refused; a spec `PATH` that omits the runtime's directory still comes up, and the box sees exactly that `PATH`. |
| 7 | No egress means no way out | Without `egress`, nothing gets out, not even through a route. |
| 8 | Losing the owner fails closed | After SIGKILL of `box up`, the box has no egress. With its `pid` file naming a live process, `list` reports the owner gone, `box prune` removes it, and the name can be used again. |
| 9 | The lifecycle works for a caller | `up` reports ready; `exec` streams and returns the exit code, and exits 3 on an absent box; an orphan in the box is reaped; `list` finds by label; `down` removes, and closing `up`'s stdin tears the box down with reason `stdin-closed`. `ready`'s labels equal `list`'s and its `egress_log` is null without `egress`; `down` on an absent box or an empty name exits 0, prints nothing and leaves live boxes alone. `ready` and `list` name the image's id; an image named by ID (podman) or without its tag comes up. A 60-character name comes up. |
| 10 | Host and box share files seamlessly | Box-created files are the user's, 644/755, exec bit intact. Host 0600/0700 files are writable in the box. |
| 11 | The box cannot write `.git` or protected config | Writing a hook under `core.hooksPath`, `core.fsmonitor`, renaming `.git`, writing `.vscode/`, or creating `.vscode/` in a project without one fails, also under a top level named by a space, and a symlinked protected path refuses the run; starting inside `.git` is refused. Control: a project file is writable. |
| 12 | A changed project file stops the run | The agent adds a domain to `.pinfold.toml`, or changes the project Containerfile; the next run and `pinfold build` refuse until `pinfold allow`. |
| 13 | Project state persists and stays separate | Settings are seeded once and survive runs; a deleted seed returns; two projects don't see each other's state; `profile new --from-project` copies the project's settings but not its `auth.json`. |
| 14 | Both pi config levels load behind a route | `pi -p` through the shim, against a fake model reached through a route: the model's request carries a skill from the profile and one from the project's `.pi/`. |
| 15 | Cleanup removes only pinfold's garbage | After three builds of one source, two images remain; an image a box still uses survives later builds and pins only itself; an image built on a profile's is its own family; a source a build left above two drops to two on a later `pinfold clean` with no build after. `--dry-run` leaves a project state the real clean removes. Two names built from identical inputs share one image id; `image rm` of one removes its tags while a box of the other name runs the image, and leaves the other name's tags, the image and the box; an image's last tag stays while a box uses it. An unlabeled image, a live box, its project's state and `~/.cache` survive `pinfold clean`, also with `--unused 0s`, which removes an idle project's state; a dead box is removed and protects no project's cache. |
| 16 | The highest layer sets the allowlist | Without project config the box's PINFOLD_ALLOW carries the default list's hosts; with PINFOLD_ALLOW=api.github.com over a project's allow = ["registry.npmjs.org"], it is exactly that host, and registry.npmjs.org is refused as not allowlisted. The project's cpus and memory reach the box. |
| 17 | up refuses before it creates | A missing image, a misspelled spec key, a bad env name, a `dev.pinfold.` label, a memory below 256M or without a unit, a malformed string route, a login route naming another harness, a mount path with a comma, a file mount, a mount at a path pinfold mounts and a live name are refused as data, naming the cause; a misspelled key's refusal also names the box, and a missing host path fails after the claim; each leaves no box and no state dir. The box whose name was reused still answers exec, and of two `up`s racing for one name exactly one wins. |
| 18 | A caller reads the effective configuration as data | pinfold config reports a project's allow list, its trust state before and after pinfold allow, and the project home pinfold pi then mounts. |
| 19 | A caller-owned box launches the pinned harness | A spec with each harness (pi, claude, codex) runs `/opt/pinfold/<name>/<name> --version` at the version `pinfold artifacts` pins, also after the host deleted that executable from the cache, and the box's PINFOLD_ALLOW is the spec's allow list. |
| 20 | A caller can tell an OOM kill from a failure | On podman, a command that exceeds the box's memory limit is killed and stat's oom_kills rises; on both runtimes stat reports the limits in force, every field is present, and `exec`'d processes carry `oom_score_adj` 1000, so init is never the victim. |
| 21 | An injecting route keeps the credential on the host | The fixture behind an injecting route receives the header; the box's environment and the egress log never hold the value; an https route reaches api.github.com over TLS. |
| 22 | A caller-owned box cannot write .git | With REPO/.git read-only listed before REPO writable, and safe.directory set by the caller: a worktree write succeeds, git log and git status succeed, and a hook write fails. |
| 23 | A caller builds an image from its own tree | An image built from a caller's context with a COPYed file reaches a box as that file; the built line carries the unique ref, the image's id and the labels; three builds of one name move latest, and the first build's ref still comes up; two builds of unchanged inputs make one image under two refs; a failed build prints its log and makes no image. |
| 24 | Every build reruns its steps | A second build of one source does not reuse the first's `RUN` layer; on podman it leaves no untagged image. |
| 25 | `--version` needs no runtime | `pinfold --version` prints the version, and `pinfold box list --help` exits 0, with no runtime and leaving the state dir untouched. |
| 26 | A login route keeps the login on the host | With `to` at a host fixture, claude: the fixture receives the `from` token as a Bearer header and the box has `ANTHROPIC_BASE_URL` and a placeholder. codex, with `CODEX_HOME` holding a login whose token lapses within 5 minutes and `CODEX_REFRESH_TOKEN_URL_OVERRIDE` at a fixture: the model fixture receives the refreshed token and its account header; the box's environment, files and the egress log hold neither token. An empty `CODEX_HOME` is refused as `login`. On macOS only (the operator's Mac, with a real login), codex completes one tool round trip through a route with no `to`. |

Both run in the merge queue on the exact ref being merged: Linux (podman)
on GitHub's `ubuntu-26.04` and `ubuntu-26.04-arm` runners, dispatched by
the queue; macOS (Apple `container`) on the operator's Mac. Linux also
runs on every push to main.

## Code

Rust. Each Linux build is a static musl binary.

Every process-interface operation is a `core` function that returns data;
`cli.rs` parses arguments and serializes results. The one exception is
`clean`'s measurement plan, which needs the pi layer's project state and
lives in `cli.rs`.

| Target | Role |
|---|---|
| `aarch64-apple-darwin` | macOS CLI |
| `aarch64-unknown-linux-musl` | Linux arm64 CLI; init on Macs and arm64 hosts |
| `x86_64-unknown-linux-musl` | Linux x64 CLI; init on x64 hosts |

Linux targets build on a Mac with `cargo zigbuild`. The default profile is
embedded with `include_bytes!`.

Dependencies: `tokio`, `httparse`, `serde`, `serde_json`, `sha2`, `toml`, `nix`,
and the TLS client for injecting routes: `rustls` (ring) with
`rustls-native-certs` for the host's roots.
SNI comes from a small ClientHello parser.

Portability (Windows later means the Linux build in WSL2):
- Platform dirs are the literal XDG-style paths on both OSes:
  `~/.config/pinfold`, `~/.local/state/pinfold` and `~/.cache/pinfold`,
  with `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` honored.
- Runtimes are adapters behind a trait, chosen by target OS.

```
crates/pinfold/src/
  core/    runtime/{mod,apple,podman}.rs plan.rs box.rs network.rs proxy.rs
           tls.rs artifacts.rs profile.rs clean.rs image.rs
  init.rs  socket mode, TCP relay, reaping, readiness
  pi/      launch.rs state.rs git.rs
  cli.rs main.rs config.rs trust.rs dirs.rs
crates/pinfold/harnesses.toml  the harness pins
crates/e2e/  the end-to-end suite: src/lib.rs (harness, fixtures, helpers), tests/e2e/{main,box_,pi,cli}.rs
profile/   the built-in default profile
```

## Open questions

- `198.18/15` blocking breaks fake-IP DNS proxies (Surge, Clash).
  Configurable, or documented.
- Nested user namespaces in the Apple `container` guest kernel: can they be
  turned off?
- Where herdr reads `HERDR_AGENT`.
- No disk cap on either runtime.
