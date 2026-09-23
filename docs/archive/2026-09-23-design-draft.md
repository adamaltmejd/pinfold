> Historical: the design draft as of 2026-09-23, before it was cut down to
> `docs/ARCHITECTURE.md`. Kept verbatim; it has known slips ("rootful" where
> rootless was decided, `refs/agentbox/` in test 13) and some claims the
> evidence in `2026-09-22-spikes.md` does not support. The spec wins.

# pinfold design

pinfold is the rewrite of agentbox. The name is from a village pound where
strays are held.

Status: draft. pinfold is a new repository. agentbox keeps working unchanged
until pinfold replaces it for daily use (phase 2 of the migration).

## What it is

pinfold runs a coding agent in a disposable box: an Apple `container`
micro-VM on macOS, a rootful podman container on Linux. The box sees the
project and nothing else of the host. Its only way out is an allowlisting
proxy.

Two uses:

1. **Interactive.** `pi` on the host is a shim for `pinfold run`. It takes
   the same arguments and behaves the same way, but runs in a box.
2. **Programmatic.** Switchyard drives the core through one `pinfold box`
   process per box. The boxes hold its workers (pi, Codex, Claude) and its
   gates.

## Layers

| Layer | Knows about | Used by |
|---|---|---|
| core | runtimes, images, mounts, the box's proxy socket, pinned artifacts | Switchyard, the pi layer |
| pi | pi's CLI, per-project state, git handoff, the shim | the `pinfold` CLI (interactive use) |
| profile | tools in the image, pi packages, default allowlist | the pi layer, as data |

The core has no knowledge of pi or git. The profile contains no code except
a pi package.

## Security boundary

The threat is a prompt injection turning a capable agent into an
exfiltration path, or into code running on the host.

| Layer | What it does |
|---|---|
| No network | The box has only loopback. Its one way out is a unix socket to its own proxy. |
| Egress proxy | Default deny. Allowlist by host name. Every decision is logged. |
| Kernel boundary | A VM per box on Apple `container`. Namespaces only on podman, with a warning on every run. |
| No capabilities | `--cap-drop ALL` leaves an empty bounding set, so nothing in the box can gain a capability, even through setuid. |
| Git handoff | The box never writes the host's `.git`. Commits come back as data. |

With no network interface, the box cannot reach anything the spikes on
2026-09-22 found reachable from an internal network:

- host listeners on the LAN, Tailscale and IPv6 addresses
- services on the gateway
- Tailscale's MagicDNS, a DNS exfiltration channel
- other boxes

Not claimed:

- **Project contents.** They are not protected. An agent with a route to a
  model API can put source code in a prompt.
- **CDN fronting.** An allowlisted name on a shared CDN reaches other sites
  on that CDN. squid had the same problem. A TLS name check closes the
  simple form. Host-header fronting (TLS name `pypi.org`, `Host:
  www.python.org` on Fastly) needs TLS interception, which is out of scope.
- **Allowlisted services that accept writes are exfil channels by design.**
  Examples are GitHub with a token, and package registries.
- **Image builds.** They are not egress-restricted. A build is a trusted
  operation run by the user, not the agent.

## Core

### Runtimes

Apple `container` on macOS, and rootless podman on Linux. The runtime is
detected, or picked with `PINFOLD_RUNTIME`.

The isolation class is a fixed table, not a setting:

| Runtime | Class | Behaviour |
|---|---|---|
| Apple `container` | vm | run |
| rootless podman with the systemd cgroup manager | namespace | run, with a warning |
| anything else, including rootful podman and docker | n/a | refuse |

**Why not podman on the Mac as well.** One adapter would be simpler, but
podman on macOS runs every box in one shared Linux VM. That gives up four
things:
- **Isolation.** There would be no VM per box: boxes share a kernel with
  each other.
- **Speed.** It is no faster. Host files cross into its VM through virtiofs
  as well.
- **Transport.** A host unix socket cannot be bind-mounted through its VM,
  so the proxy would need networking again.
- **Memory.** It keeps a VM running and holding memory.

Apple `container` costs a second adapter, macOS 26 or later, and a younger
runtime with open bugs. It keeps the strongest isolation on the platform
used every day.

**Rootless only on Linux.** Measured on Debian 13 with podman 5.4 on
2026-09-23.

- **What a breakout gets.** In rootless mode podman, conmon and crun all run
  as the user, so a breakout from the box or the runtime gets the user's
  uid. In rootful mode a runtime breakout gets host root, and pinfold would
  need root itself.
- **Signals.** Rootful is also broken on Debian 13: its AppArmor profiles
  deny signals between exec'd processes, so `kill` fails and tools hang.
- **Same behaviour otherwise.** Ownership and timing are identical in both
  modes.
- **Kernel bugs are the exception.** A kernel privilege escalation reaches
  the whole host either way, which is why nested user namespaces are
  blocked (see Always applied).

**Preflight.** `podman info` must report `rootless=true` and
`cgroupManager=systemd`, or pinfold refuses to start the box. Under the
cgroupfs fallback, podman accepts `--cpus` and `--memory` and silently
enforces neither. That fallback happens when the user has no systemd manager
or D-Bus session. Long-lived boxes, such as Switchyard's, need
`loginctl enable-linger`, because logging out stops the user manager and
kills every rootless box.

### Box lifecycle

Every box has the same shape. PID 1 is `pinfold init`, which idles, and work
arrives by `exec`. Switchyard already works this way. It also means the
interactive layer can run a step after pi exits, the git bundle.

1. **Start the box's proxy** on a new unix socket in pinfold's state
   directory. macOS limits socket paths to 104 bytes, so the path must be
   short.
2. **Start the box** with `--network none` and the socket carried in (see
   Transport). PID 1 is `pinfold init`, running as the host uid. pinfold
   runs one short root exec that makes the socket usable. The init then
   relays `127.0.0.1:3128` to it, reaps children, and reports ready.
3. **Wait for ready.**
4. **`exec` the work** as the host uid, with
   `HTTPS_PROXY=http://127.0.0.1:3128` in its environment.
5. **Remove the box,** close the proxy, and delete the socket.

**One `pinfold box up` process owns one box** for its lifetime.
- It does steps 1 to 3, then holds the proxy.
- It does step 5 when told to stop, when its stdin closes, or on SIGTERM.
- If it dies, the socket goes dead and the box has no way out, so it fails
  closed. `box prune`, or Switchyard's reconcile, then removes the leftovers
  by label.

The CLI uses the same code in-process. The process interface is the boundary
Switchyard uses:

```
pinfold box up < spec.json         # prints {"event":"ready","box":…}; holds the box
pinfold box exec BOX [--tty] [--workdir D] -- argv…   # stdio passed through, exit code returned
pinfold box down BOX               # same as closing box up's stdin
pinfold box list --label k=v       # JSON lines
pinfold box prune                  # remove boxes whose `up` is gone
```

```json
{
  "name": "yard-3f2a…-e12-g1",
  "labels": { "dev.yard.lane": "…" },
  "image": "…",
  "mounts": [{ "host": "/…/clone", "guest": "/workspace", "readonly": false }],
  "user": { "uid": 501, "gid": 20 },
  "env": { "HOME": "/yard/state/home", "OPENROUTER_API_KEY": { "from": "OPENROUTER_API_KEY" } },
  "egress": { "allow": ["openrouter.ai"], "routes": { "yard.internal": "127.0.0.1:7777" } },
  "cpus": 4,
  "memory": "8G"
}
```

- Leaving out `egress` gives a box with no way out at all. That is for gates.
- `env` is exact, and nothing is inherited from the host.
- An entry like `{ "from": "NAME" }` is read from the calling process's own
  environment, so secret values never appear in argv or in the spec.
- On Apple `container`, mounts must be directories. Nested read-only mounts
  are how a layer protects subpaths.

Internally, runtime command lines are built as data, which keeps the
runtime adapters small and easy to read.

**Always applied:**

- **`--network none`**
- **`--cap-drop ALL` with no additions.** Apple `container` gives exec'd
  processes the container's capability set, so their bounding set is empty.
  It has no way to set `no_new_privs` on them, so an empty bounding set is
  the control that matters. podman also gets
  `--security-opt no-new-privileges`.
- **`--read-only` root filesystem** with a tmpfs `/tmp`.
- **Nothing long-lived runs as uid 0.**
  - PID 1 and all work run as the host uid and gid.
  - On Apple `container`, the forwarded socket arrives owned by root with
    mode 000. One transient `exec --user 0:0` changes its mode at start and
    exits. The socket is not on a mount.
  - On podman, the bind-mounted socket already belongs to the user, so it
    needs no root step. That matters: under keep-id, container uid 0 is a
    sub-uid that cannot reach a 0600 socket at all.

  The reason is the ownership spike (2026-09-23). On Apple `container`, a
  uid-0 process that touches a mount can make the box user's operations fail
  at random with EACCES. It can also turn a host-set setuid bit into euid 0.
  Read-write mounts cannot be `nosuid`: the option is silently ignored.
- **podman only:**
  - `--userns=keep-id`, which makes ownership seamless
  - `--security-opt no-new-privileges`
  - a seccomp profile that also blocks `CLONE_NEWUSER`. podman's default
    lets the box user create nested user namespaces, which is kernel attack
    surface on a shared kernel. Blocking it was tested and breaks nothing.
  - `--no-hosts` and `--dns none`, so `/etc/hosts` and `resolv.conf` don't
    leak the host's name and nameservers
  - `--memory-swap` equal to `--memory`, and a `--pids-limit`
  - `rm -f -t 0` at teardown
  - `HOME` set explicitly, because keep-id's passwd entry gives `/`
- **Secrets are passed by name** (`--env NAME`), with values in the child's
  environment, never in argv.
- **`NODE_USE_ENV_PROXY=1`**, because Node's fetch ignores `HTTPS_PROXY`
  without it.
- **`GIT_HTTP_PROXY_AUTHMETHOD=basic`**, which saves git one 407 round trip.

### Process supervision

Measured on 2026-09-22, Apple `container` 1.4.1, with a Rust parent driving
`container exec`. Through the parent, a TTY session behaves exactly as
`container exec -it` does from a shell:
- Ctrl-C and Ctrl-\ work.
- Window size and resizes reach the box.
- pi's TUI works.
- Exit codes come through.
- The terminal is restored afterwards.

Without a TTY, pipes are clean: binary data, EOF, and separate stdout and
stderr. `pi --mode rpc` answers over them.

The parent adds no measurable latency: 56 ms to the first byte on a running
box.

What pinfold must do:

- **TTY.** Pass `-t` only when both stdin and stdout are terminals, and
  always pass `-i`. With `-t`, also pass the host `TERM` and `COLORTERM`,
  because the runtime defaults to `xterm`. In TTY mode, `container exec`
  stays in pinfold's foreground process group: it sets raw mode itself and
  gets SIGWINCH directly. Without a TTY, it runs in its own process group,
  and pinfold relays SIGINT and SIGQUIT.
- **The box is the unit of lifetime.** pinfold never signals a single
  process in the box:
  - With a TTY, Ctrl-C and Ctrl-\\ travel as bytes and reach pi directly.
  - Anything that ends the session removes the box, which kills everything
    in it: a closed terminal, SIGTERM or SIGHUP to pinfold, or Ctrl-C
    without a TTY. `container rm -f` also ends a stranded host
    `container exec`.
  - The trade-off is that the process in the box is killed, not asked to
    stop. pi appends to its session file as it goes, so at most the turn in
    flight is lost.
  - `container exec` 1.4.1 cannot forward signals anyway
    (apple/container#1941; fixes open in #1997 and #1778). Once that is
    fixed, a graceful SIGTERM before removal is a small addition.
- **After a SIGKILL of pinfold,** `box prune` removes boxes by label whose
  owner is gone. Nothing can restore the terminal then; that is documented
  (`stty sane`).
- **Exit codes.** The exec's code is passed through, and `128+n` for signal
  deaths.

### Shared files

The user edits files in the IDE while the agent edits the same files in the
box. Ownership needs no work:

- **Box to host.** Every file the box creates lands on the host owned by the
  user, with mode 644 or 755 (umask 022). As always on macOS, its group
  comes from the parent directory. The executable bit and symlinks survive.
- **Host to box.** Apple `container` reports every file as owned by
  whichever box process asks. So the box user can do exactly what the host
  user can, and host flags such as `uchg` still apply.
- **git.** It needs no `safe.directory`.
- **The IDE** sees box changes at once, through FSEvents.
- **Rootless podman on Linux** with `--userns=keep-id` gives the same
  ownership result. It also shares inotify events and locks in both
  directions, because it is the same kernel. The limitations below are
  specific to Apple `container`.

Known limitations on Apple `container`, all documented for users:

| Limitation | Effect | Handling |
|---|---|---|
| About 1 s of metadata and name caching in the box | After a host atomic save, delete or rename, the box can get ENOENT, or stale `stat` results, for up to 1 s. Not reported upstream yet. | If it bites in practice, a profile extension retries pi's file reads once after 1 s. |
| No inotify for host changes | Watch-mode tools in the box miss IDE edits. | Use polling (`CHOKIDAR_USEPOLLING`, `WATCHPACK_POLLING`). |
| Locks are not shared across the boundary | `flock` and `fcntl` locks don't cross it. O_EXCL lockfiles, which git uses, are safe. | Don't open one SQLite database from both sides at once. |
| Case-insensitive host volume | A case-only rename in the box silently does nothing. | Rename in two steps. |
| Small-file I/O on the shared mount is 4–10× slower than on the host | 5k files: create 1.00 s against 0.25 s on the host, read 0.63 s against 0.07 s | Accepted for v1. `/tmp` is a fast tmpfs for throwaway scratch work. |
| Ownership shown in the box is fictional | The box cannot set setuid or setgid bits; the server strips them. | Harmless. |

### Transport

The box has no network. The proxy's unix socket is carried in:

- **podman:** a bind mount of the socket, mode 0600 in a 0700 state
  directory. It is the same kernel, so the socket works as-is. Measured
  rootless: 0.13 ms round trip, 243 MB/s per stream, and 32 parallel
  streams fine. The host proxy can also read the connecting process's uid
  and pid (`SO_PEERCRED`).
- **Apple `container`:** `--ssh`, with `SSH_AUTH_SOCK` set to the proxy
  socket for that one `container run`. The runtime forwards the socket over
  vsock to `/var/host-services/ssh-auth.sock`, owned by root with mode 000.
  A one-off root exec changes its mode, so the box user can connect. As a
  side effect the box gets no real SSH agent, which is wanted.

`pinfold init` relays each TCP connection on `127.0.0.1:3128` to the socket,
because clients only know how to reach a proxy over TCP. Every connection
over the socket starts with a one-line header naming its target: the proxy,
or a forward (see Forwards). One socket carries everything, and Apple
`container` only gives one.

Measured on 2026-09-22, Apple `container` 1.4.1:

- a connect and round trip takes 1.5 ms
- one stream runs at 484 MB/s
- 32 parallel 20 MB streams all completed
- 57,600 short exchanges on 32 parallel live connections showed no hangs

**Risks:**

- **`--ssh` is used off-label.** A future release could check that the
  socket is an SSH agent.
- **Apple's open relay bug** (apple/container#2247). Parallel clients on
  live connections can lose data and freeze `exec`. It did not reproduce
  with this traffic. If it does, the relay moves every stream onto one
  multiplexed connection.

**Fallback** if `--ssh` stops working: an internal network per box, with the
proxy on the gateway. That also needs a root init that deletes the default
routes and flushes IPv6, and needs NET_ADMIN, SETUID and SETGID, all of
which this design avoids.

### Egress proxy

The proxy is in-process, one per box, and lives as long as the box. It
listens only on that box's socket, so it has no network listener and never
triggers the macOS firewall. The socket identifies the box, so no token is
needed. A TypeScript spike proved the approach against real boxes:

- It starts in under 0.1 ms.
- curl, git, gh, bun, python and uv all work through it.
- Request smuggling probes were refused.

The Rust version keeps its behaviour.

This removes squid and every problem that came from sharing one: the
pidfile, the reconfigure race between projects, and the Linux `shm_open`
clash.

**What gets through:**

- **CONNECT** is allowed only to port 443 on an allowlisted host: an exact
  name, or `.suffix` for the name and its subdomains. The TLS server name in
  the client's first message must match the CONNECT host.
- **Plain HTTP** is allowed only to port 80. It is one request per
  connection, framed by Content-Length only. Ambiguous framing, folded
  headers and bare LF get a 400.
- **Refused:** IP literals, and names that resolve to loopback, private,
  link-local, CGNAT (`100.64/10`) or benchmark (`198.18/15`) addresses. The
  name is resolved once, and the proxy dials the address it checked.
- **Routes** map a name to a host-local service, for example
  `yard.internal → 127.0.0.1:7777`. They carry plain HTTP only, and the
  proxy rewrites the Host header. CONNECT to a route is refused, because it
  would be a raw TCP pipe to the host service. The host service must still
  authenticate its callers. Routes replace socket forwarding and cover
  Switchyard's MCP endpoint.

**Limits and logging:**

- A cap on connections, a header timeout, and an idle timeout on tunnels, so
  a box cannot exhaust the pinfold process.
- Each decision is written as a JSON line to the box's egress log.

A later option, not in v1: model API credentials injected at the proxy, so
the box only ever sees a placeholder key.

### Forwards

A forward exposes one host unix socket at a guest path. It is for tools that
talk to a host daemon over a socket rather than over HTTP. It rides the
same transport:
- `pinfold init` listens at the guest path.
- It relays each connection with a header naming the forward.
- The host side connects to the host socket.

The bytes pass through raw, with no allowlist. A forward grants whatever the
host service grants, so:
- Forwards are opt-in, listed in config, covered by trust, and printed at
  every start.
- The pi layer can put a filter in front of one. pinfold then serves its own
  host socket, and passes on only allowed messages to the real one.

| Forward | What it grants | Default |
|---|---|---|
| Switchyard `daemon.sock`, for an operator agent in a box | full control of the board, including approve and land | off; opt-in per project, and a filter can restrict it, for example to read-only status |
| herdr's socket | `pane.run`: running commands on the host | never raw; only through the herdr filter (see pi layer) |

Switchyard's workers need no forward. They reach its MCP endpoint over HTTP
through a route, with a bearer token scoped to the lane.

### Images

An image is a toolchain. The harness is not baked in (see Pinned artifacts),
so any project's Containerfile works.

- The default image comes from the profile's Containerfile.
- A project can name its own Containerfile, typically `FROM` the profile
  image.

**Profile base: `debian:trixie-slim`.** Decided on 2026-09-22.

- **Current over pinned.** The base is referenced by tag, and each build runs
  `apt-get upgrade`, so security fixes land on every rebuild. A build records
  the resolved base digest as an image label. The previous image is kept
  for rollback. The image is not a security boundary: the agent already runs
  code in the box, and containment is the VM, the network, the proxy and the
  git handoff.
- **Tools are minimal to start.** This profile is for building with pi, and
  more tools are added when a real need shows up. There is no R, shellcheck,
  shfmt, uv, python or ast-grep in v1.
- **From Debian:** ca-certificates, git, curl, ripgrep, fd, jq and less. pi
  uses ripgrep and fd itself; without them it downloads its own copies.
- **From GitHub's signed apt repository:** gh, which stays current without a
  pin.
- **Pinned with `ADD --checksum=sha256:…`:** bun (pi's package manager, and
  JS/TS work) and rtk (a profile extension), which Debian lacks. Apple's builder enforces the checksum; a wrong digest
  fails the build. A short bump script updates the pins. This replaces
  `vendor.lock`, `vendor/`, and `vendor check`/`vendor update`.
- **setuid and setgid bits stripped** in the build. `no-new-privileges`
  already disables them; stripping is the second layer.

**Alternatives considered:**

| Option | Why not |
|---|---|
| Wolfi | Current packages and no setuid, but Chainguard controls it: community contributions are closed, and the free tier is `:latest` only. |
| Docker Hardened Images | The best provenance, but pulling needs a Docker login. |
| Nix | Non-FHS layout, so prebuilt binaries the agent downloads fail to start: manylinux wheels, `uv python install`, npm native packages. Building on macOS also needs a Linux builder and a format conversion for Apple `container`. |
| Alpine | musl cannot run pi's glibc build or rtk's arm64 build. |
| Ubuntu chisel | Cannot pin package versions. |
| UBI | Missing tools, and UBI 9's glibc is too old for rtk. |

### Pinned artifacts

A small cache of checksummed release binaries per platform, at
`~/.cache/pinfold/artifacts/<name>/<version>/<os-arch>/`. Artifacts are
mounted read-only into the box. The pinned binaries are:

- pi
- the Linux `pinfold` binary, used as the init (`pinfold init`)
- Switchyard's pinned codex and claude, which replaces its own download code

## pi layer

For interactive use. Built on the core box lifecycle, in-process.

- **Harness.** pi is a pinned artifact, mounted read-only at
  `/opt/pinfold/pi`.
- **State.** Each project gets one host directory,
  `~/.local/state/pinfold/projects/<name>-<hash>/home`, mounted as the box's
  `$HOME`. pi's agent dir, sessions, `~/.config` and tool caches all live
  there at their default paths, with no env overrides.
  - Everything is visible on the host, and pinfold seeds it without
    starting a box.
  - Nothing is shared between projects, and `git clean` in the checkout
    does not touch it.
  - It is on the shared mount, so caches get its small-file speed. A
    per-project ext4 volume measured faster than the host's own disk, but it
    only helps caches. The agent mostly writes to the project, which must
    stay on the shared mount for the IDE. So the volume was rejected as
    complexity without a noticeable gain (2026-09-23). Revisit only if a
    measured workload says otherwise.
- **Seeding.** When a project's agent dir is created, `settings.json` is
  copied from the profile. After that the file is pi's, and pinfold never
  touches it again. There is no marker file and no merging.
- **No argument rewriting.** `pi <args>` means exactly what it means on the
  host. pinfold adds no `--continue`, provider or model. The project is
  mounted at its own absolute path, so paths inside it work. Paths outside it
  fail, and pi reports them.
- **Environment in the box:** `PI_TELEMETRY=0`, `PI_SKIP_VERSION_CHECK=1`,
  the core's proxy variables, and whatever `PINFOLD_ENV_*` provides.
- **TTY.** pi runs through `exec` with a TTY whenever the host has one.
  Without it, pi falls into print mode.
- **OAuth.** Because state is per project, an OAuth `/login` (Claude
  Pro/Max, Codex) is done once per project. API keys come through the
  environment.
- **Protected paths.** Extra read-only directory mounts from `protect`, for
  example `.claude/` (its settings can hold hooks) or `.vscode/`.

### herdr

herdr runs agents in terminal panes. It learns the agent and its state from,
in order:
1. lifecycle hooks, when installed
2. the pane's foreground process
3. reading the screen

**v1: no socket.** The TTY passes through untouched, so screen reading works
as-is. The shim sets herdr's wrapper hint, `HERDR_AGENT=pi`, so the pane
shows pi and not `pinfold` or `container`. Where herdr reads that variable
still needs verifying.

**Later: hook fidelity.** This needs herdr's pi integration in the box
(seeded from the profile) and a filtered forward of herdr's socket. The
filter is small, because the protocol is newline-delimited JSON. It passes
only `pane.report_agent` for the box's own pane, and refuses `pane.run`,
`pane.split` and everything else. pinfold passes through the pane-identity
variables herdr sets.

### Git handoff

The box must never write the host's `.git`. The mounts spike (2026-09-22,
Apple `container` 1.4.1, host git 2.55) found:

- With the plain mount, the box plants a hook or `core.fsmonitor`, and the
  next host `git status` or `git checkout` runs it.
- `--read-only-path` on `.git/config` and `.git/hooks` blocks writes and
  renames of those two paths. Renaming `.git` itself still works, and a
  poisoned copy put back in its place runs on the host.
- With `.git` as its own mount and config and hooks read-only, all of the
  above is blocked. But the box can still create `.git/commondir` pointing at
  a poisoned copy inside the working tree, and host git follows it. git must
  be able to create files in `.git` to commit, so no mount layout closes
  this.

git's own SECURITY section agrees: running git in a `.git` that came from an
untrusted source is unsafe.

So the box gets its own git dir, and its commits come back as data:

1. **Before the run, on the host.** Read the repo's state with host git: the
   branch, HEAD, remotes and objects dir. This is safe because it is the
   user's own repo. Then build a fresh git dir for this run under the
   project's state, before any box can touch it:
   - a template config with the remote URLs, the gh credential helper and
     the identity. It also sets `core.checkStat=minimal`, because inode
     numbers differ between host and box and would otherwise force a full
     rehash in every new box. It copies `core.ignorecase` and
     `core.precomposeunicode` from the host repo.
   - HEAD and refs
   - a copy of the index
   - `objects/info/alternates` pointing at the host's objects, mounted
     read-only

   Record a hash of the host's index.
2. **The run.** This git dir is mounted read-write at `<repo>/.git`, over
   the host's. The agent commits, branches and pushes as usual.
3. **After pi exits.** `exec git bundle create` in the same box, covering
   everything new since the recorded start refs. If the run was
   interrupted, the box is already gone. The git dir is a host directory,
   so the commits survive, and the bundle step runs in a fresh box on the
   same git dir.
4. **On the host.** `git -c transfer.fsckObjects=true fetch <bundle>` into
   `refs/pinfold/<run>/*`. Then:
   - The current branch is fast-forwarded, with a compare-and-swap
     `update-ref`, only if the host HEAD and index are unchanged since
     step 1. The index is then reset to the new HEAD, and the working tree
     already matches.
   - Otherwise nothing moves, and pinfold prints the refs to merge.
   - The run's git dir is deleted.

Host git only ever reads a bundle file the box wrote. It never runs inside a
git dir the box could write, and never runs `upload-pack` against one. One
git dir per run means two boxes on one project don't collide.

Residual risk: a nested repository planted in the working tree (`sub/.git`)
runs code if a host tool discovers it and runs git there. VS Code does this
by default.

## Profile

A directory, `profile/` in this repo by default, or `PINFOLD_PROFILE`:

```
profile/
  pinfold.toml    # same schema as a project file: allow, image, cpus, ...
  Containerfile    # the default toolchain image
  settings.json    # seed for a new project's pi settings
  pi/              # a pi package: operating-context extension, skills
```

The operating context becomes an extension that writes the box's facts into
the system prompt at `before_agent_start`, for example the live allowlist and
that a 403 is final. A static `AGENTS.md` copied on every run is no longer
needed. rtk and ponytail are installed by the profile image and listed in the
seed settings.

Extensions run inside the box. Nothing security-relevant lives in one.

## Configuration

Precedence: environment > project `.pinfold.toml` > profile `pinfold.toml`
> defaults.

| Key | Env | Default | Meaning |
|---|---|---|---|
| `allow` | `PINFOLD_ALLOW` | `[]` | Domains added to the allowlist |
| `routes` | `PINFOLD_ROUTES` | `{}` | Proxy routes to host services |
| `forwards` | `PINFOLD_FORWARDS` | `{}` | Host unix sockets exposed at guest paths (see Forwards) |
| `protect` | `PINFOLD_PROTECT` | `[]` | Directories mounted read-only in the box |
| `image` | `PINFOLD_IMAGE` | profile image | Image ref, or a Containerfile path relative to the project |
| `cpus` | `PINFOLD_CPUS` | 4 | |
| `memory` | `PINFOLD_MEMORY` | `8G` | |
| — | `PINFOLD_RUNTIME` | detected | `container` or `podman` |
| — | `PINFOLD_PROFILE` | `profile/` | |
| — | `PINFOLD_ENV_<NAME>` | — | Becomes `<NAME>` in the box. The only way host environment enters. |

**Trust.** The project file lives inside the mount, so the agent can edit
it. A project file, and the Containerfile it names, are used only if their
hash matches the one `pinfold allow` recorded in
`~/.local/state/pinfold/trust`. If either changes, the run stops until the
user allows it again. This is direnv's model.

**Credentials use no pinfold code:**

- 1Password: `op run -- pi …` resolves `op://` references in
  `PINFOLD_ENV_*`.
- GitHub: `PINFOLD_ENV_GH_TOKEN`. `gh` reads `GH_TOKEN`, and the git dir's
  template config sets `!gh auth git-credential` as the credential helper
  for `https://github.com`.
- A per-repository token comes from direnv on the host.
- Commit identity comes from `PINFOLD_ENV_GIT_AUTHOR_NAME` and the other
  `GIT_*` variables.

## CLI

```
pinfold run [pi args…]   pi in a box for this project; `pi` is a symlink to this
pinfold shell [cmd…]     bash (or cmd) in a fresh box with the same mounts, no pi
pinfold build            build this project's image
pinfold allow            trust this project's .pinfold.toml and Containerfile
pinfold doctor           runtime, isolation class, image, artifacts, trust, effective config
```

The project root is the git top level, or `$PWD` outside a repository. The
box starts in the directory the command was run from.

## Switchyard

Switchyard would use core only:

- runtime detection and the isolation class
- `pinfold box up`/`exec`/`down`, spawned through its own spawn gate
- label-based `list` and `remove` for reconcile
- boxes with no `egress` for gates
- a proxy per worker box, with a route for its MCP listener
- pinned artifacts for the pi, codex and claude binaries

It needs no git handoff. Its lanes are standalone clones that the daemon
fetches from.

**Reviews** move from `autoreview` on the host to a pi review extension
(inspired by autoreview) running in a pinfold box. The box has:
- the candidate mounted read-only
- no forwards
- egress limited to the model API

A reviewer reads agent-written diffs, which can carry prompt injections, so
it gets the smallest box of all.

What it gains: Apple `container` on macOS instead of Docker Desktop, and an
egress allowlist for workers, which have none today.

What changes on its side:

- DESIGN.md fixes the Docker CLI and says the pinned CLI is never an image
  build step. Both clauses need amending.
- `src/docker.ts` and the worker-CLI download code, about 4.8k lines, are
  replaced.
- The adapters (argv construction, frame parsing, Codex credential handling)
  stay in Switchyard.
- Codex's credential is a single read-only file mount, which Apple
  `container` cannot do. It moves to a read-only directory.
- Memory-share and OOM detection are Docker-specific. On podman,
  `memory.events` shows `oom_kill`, and an exec'd process that is OOM-killed
  exits 137 while the box keeps running. Apple `container` still needs an
  answer.

## Invariants

Each one has an end-to-end test (see Testing):

1. The box has no network interface except loopback. This is verified from
   inside the box, not assumed.
2. Its only way out is its own proxy socket. A box without egress has none.
3. The proxy refuses:
   - hosts not on the allowlist
   - a TLS server name that doesn't match the CONNECT host
   - IP literals and non-public addresses
   - CONNECT to routes
   - ambiguous HTTP framing
4. Everything long-lived in the box runs as the host uid, with an empty
   capability bounding set. The image has no setuid or setgid bits.
5. No secret value appears in any argv.
6. Rootless runtimes and unknown OCI runtimes are refused. A namespace-class
   runtime warns on every run.
7. The box never has the host's `.git` writable. Host git never runs inside
   a git dir the box could write. It only fetches a bundle.
8. An untrusted or changed project file stops the run.
9. The environment in the box is exactly the spec's. Nothing from the host
   leaks in except `PINFOLD_ENV_*`.

## Testing

Policy, written to be copied into the repo's `AGENTS.md`:

**Only end-to-end tests.** No unit tests, no mocks, no runtime stubs, and no
test-only code paths or flags in the binary. A test builds the real binary,
starts real boxes, and observes from outside: exit codes, output, host
files, the egress log. Or it acts from inside a box through
`pinfold box exec`. The e2e crate never imports pinfold's internals; the
only seams are the ones a user has: the CLI, env vars, `.pinfold.toml`, and
the box spec.

**A fixed, short list.** Each test proves one guarantee, and its name states
the guarantee (`box_has_no_route_to_host`, not `test_network_3`). A new test
needs a new guarantee, or a bug to reproduce: a bug fix comes with a test
that reproduces the bug through the CLI. Nothing tests argv shapes, file
layout, help text or log wording.

**Tests must be able to fail.** Agents write tests that pass because
nothing ran. Three rules against that:

1. **Positive controls.** Every "this is refused" test also shows that the
   allowed version succeeds in the same box. A denied host is checked next
   to an allowlisted one, and a blocked host route next to the proxy route
   to the same fixture. A failure for an unrelated reason then fails the
   test instead of passing it.
2. **Assert the reason, not only the failure.** For example: a 403 from the
   proxy, together with the egress-log entry that names why. A timeout or a
   DNS error is not a pass.
3. **Seen failing once.** Before a test is merged, it has been run against a
   deliberately broken build or setup, and it failed. The test's comment
   names that sabotage, for example "box started with a network: this test
   must fail".

**Deterministic.** No sleeps; wait on the readiness signals pinfold already
emits. No retries: a flaky test is a bug to fix or delete. Fixtures run on
the host:
- an HTTP service reached through a proxy route
- a fake OpenAI-compatible model server, so pi runs a full turn without a
  real model

The only public endpoints are one allowed host and one denied host
(`api.github.com` and `example.com`), for the allowlist tests.

**Budget.** The whole suite runs in under 5 minutes in CI.

**Where it runs:**
- **Linux with podman** runs in GitHub CI on every push and PR. That is the
  required gate.
- **macOS with Apple `container`** runs the same suite as a Switchyard
  host gate (`environment = "host"`, darwin/arm64), with
  `stage = "batch"`. Hosted macOS runners cannot run Apple `container`,
  because they have no nested virtualization.
  - A batch-stage gate runs only in the merge queue, after you approve the
    exact head. So agent-written code runs on the host only after review.
  - Never make it a candidate-stage gate: that would run unreviewed code
    on the host.

**The list:**

| # | Guarantee | Shown by |
|---|---|---|
| 1 | The box has no network but loopback | From inside: only `lo`. Connections to the host's addresses, to `1.1.1.1` and to `100.100.100.100:53` fail. Control: the same fixture is reachable through a proxy route. |
| 2 | Only allowlisted hosts get through | `api.github.com` answers. `example.com` gets a proxy 403, and the log says "not allowlisted". |
| 3 | The proxy refuses the tricks | From inside, each with a control: an IP literal, a name resolving to loopback, a TLS server name different from the CONNECT host, CONNECT to a route, and ambiguous HTTP framing. |
| 4 | Routes reach exactly one host service | `http://fixture.internal/` works. The fixture's port on the host is unreachable directly. |
| 5 | Nothing in the box can gain privileges | The exec'd process has `CapBnd` 0. The image has no setuid or setgid files. The root filesystem cannot be written. |
| 6 | The environment is exactly the spec | An unprefixed host variable is absent. `PINFOLD_ENV_X` arrives as `X`. The secret's value never shows in host `ps` output during the run. |
| 7 | A box without egress has no way out | With no `egress` in the spec, nothing gets out, not even through a route. |
| 8 | Losing the owner fails closed | After `SIGKILL` of `box up`, the box has no egress, and `box prune` removes it. |
| 9 | The box lifecycle works for a caller | `box up` reports ready. `exec` streams output and returns the exit code. `list` finds the box by label, and `down` removes it. |
| 10 | Host and box share files seamlessly | Box-created files and directories land on the host owned by the user with mode 644 or 755, the executable bit intact. Host files, including 0600 and 0700, are writable in the box. A read-only mount rejects writes. |
| 11 | The box cannot plant code for host git | In the box: write a hook, `core.fsmonitor` and `commondir` into `.git`. After exit, the host `.git` is unchanged, and host `git status` runs nothing (no marker file appears). |
| 12 | Commits come home | A full pi turn through the `pi` shim, against the fake model, edits a file and commits. The commit lands on the host branch. |
| 13 | A moved host branch is never overwritten | The host commits during the run. Afterwards the branch is untouched, and the box's commit is waiting under `refs/agentbox/`. |
| 14 | A changed project file stops the run | The agent edits `.pinfold.toml` to add a domain. The next run refuses until `pinfold allow`. |
| 15 | Project state persists and stays separate | pi settings are seeded once and survive across runs. Two projects don't see each other's state. |

## Code

Rust. The tool is systems glue around processes, a VM, a TTY, sockets and
an adversarial protocol parser. The in-box init has to be a small static
Linux binary that works in any image.

**Builds.** Each Linux build is a static musl binary and doubles as the init
(`pinfold init`). On a Mac, building the Linux binary uses `cargo zigbuild`.
CI builds all three.

| Target | Role |
|---|---|
| `aarch64-apple-darwin` | macOS CLI |
| `aarch64-unknown-linux-musl` | Linux arm64 CLI, and the init for Mac and arm64 hosts |
| `x86_64-unknown-linux-musl` | Linux x64 CLI, and the init for x86 hosts |

**Dependencies.** Kept small:
- `tokio` for the proxy and the process supervision
- `httparse` for request heads
- `serde`, `serde_json` and `toml`
- `nix` for signals, TTY and setuid in the init

TLS server names are read from the ClientHello by a small parser.

**Tests.** End-to-end only. See Testing.

**Portability rules**, so a Windows target stays cheap later:
- Host-to-guest paths are mapped in one function. It is the identity on
  Unix, and no other code assumes the two paths are the same.
- Platform directories come from the `directories` crate, not hardcoded XDG
  paths.
- Unix-only host code (signals, process groups, TTY, file modes) lives in one
  module.
- Runtimes are adapters behind a trait, and each one declares its isolation
  class.

Windows would first be supported by running the Linux build inside WSL2.

**Other languages.** The profile's pi package stays TypeScript, because pi
extensions are TypeScript. Switchyard stays TypeScript on the other side of
the process boundary.

```
crates/pinfold/src/
  core/     runtime.rs plan.rs box.rs network.rs proxy.rs tls.rs artifacts.rs
  init.rs   socket mode, TCP relay, reaping, readiness
  pi/       launch.rs state.rs git.rs
  cli.rs config.rs trust.rs
profile/
```

Rough size:

| Part | Lines |
|---|---|
| core, including a ~450-line proxy | ~1,800 |
| init | ~150 |
| pi layer, including git handoff | ~600 |
| CLI and config | ~400 |
| README | ~150 |

## Dropped from agentbox.sh

- the `config` store and command
- `github` token minting
- the 1Password code
- the `proxy` subcommand
- `vendor check` and `vendor update`
- socket forwarding and the socat relay
- pi-wrapper settings seeding
- the session lock
- the `--continue`, provider and model defaults
- retired settings
- the builder subnet and DNS settings
- `PINFOLD_ISOLATION`
- the shared squid
- most of `doctor` and `clean`

## Migration

0. **Spikes.**
   - Done: protecting `.git`, `--user` without a passwd entry, and
     `ADD --checksum`.
   - Done: the built-in proxy against real boxes.
   - Done: startup baseline. A warm `container run --rm … true` takes 0.66 s
     and mounts add almost nothing. Today's full path takes 1.8 s warm, and
     16 s cold when an `op read` fails. Target: under 1 s to pi's first
     frame.
   - Done: route lockdown. A root init with NET_ADMIN can remove the routes,
     but the no-network socket transport makes it unnecessary.
   - Done: exec'd processes inherit an empty bounding set under
     `--cap-drop ALL`. A later spike moved PID 1 off uid 0 altogether (see
     Always applied).
   - Done: socket transport. Throughput, concurrency, and the
     apple/container#2247 pattern (no hangs).
   - Done: the macOS firewall. It prompts once per install path, and a
     rebuild at the same path does not prompt again. This is moot now,
     because there is no network listener.
   - Done: TTY, signals and pipes through a Rust parent (see Process
     supervision).
   - Done: ownership and permissions between host and box (see Shared
     files).
**Bootstrap with Switchyard.** pinfold can be built by Switchyard as it is
today:
- Workers run in Docker.
- Models come from OpenRouter or opencode-go (Switchyard 0.17.4), both of
  which carry open models.
- The macOS end-to-end suite is a batch-stage host gate.

Phase 3 then moves Switchyard onto pinfold.

Two cautions until phase 3:
- Workers have unrestricted network.
- Reviews run through `autoreview` on the host, outside any box. The plan
  replaces it with a pi review extension that runs in a pinfold box (see
  Switchyard).

1. **Core, proxy, pi layer with git handoff, and CLI, in Rust.** Parity for
   interactive use on macOS. Exit: daily use for a week.
2. **Linux podman end-to-end in CI, and the README.** Then switch daily use
   from agentbox to pinfold and archive the agentbox repository.
3. **Switchyard.** Implement its engine interface over the core, and run its
   worker and gate live contracts against it. Add worker allowlists. Remove
   `docker.ts`.

## Open questions

- **Git worktrees.** A worktree's `.git` is a file, and Apple `container`
  cannot mount over a file. Two options:
  - `--read-only-path` on the file, plus `GIT_DIR` pointing at the run's
    git dir
  - refuse worktrees in v1
- **Blocking `198.18/15`** breaks hosts whose DNS proxy hands out fake IPs
  from that range (Surge, Clash). Make it configurable, or document it.
- **Unprivileged user namespaces** are enabled in the box kernel. Nothing
  exploitable was found, but they are kernel attack surface. Check whether
  Apple `container` can turn them off.
- **GitHub CI runs on `ubuntu-26.04` and `ubuntu-26.04-arm`,** which ship
  podman 5.7. `ubuntu-24.04` ships 4.9. Both architectures run, because the
  init binary is per architecture. Still to verify in phase 2:
  - Ubuntu's AppArmor restriction on unprivileged user namespaces
    (`kernel.apparmor_restrict_unprivileged_userns`)
  - a user systemd manager for the runner (`loginctl enable-linger runner`)

  The newest podman (6.x) is checked on the lab VM.
- **Nested user namespaces on Apple `container`.** They are enabled in the
  box kernel, and there is no seccomp option. This is less critical there,
  because the kernel belongs to the VM, not the host.
- Apple `container` memory limits and OOM detection, for Switchyard.
- Linux Switchyard hosts move from Docker to rootless podman.
- Disk: Apple `container` has no disk cap.
- Model credentials kept out of the box by injecting them at the proxy.
- **Name.** Decided: pinfold. It is free on crates.io, PyPI, npm and
  Homebrew. glovebox, hotcell, vivarium and hutch are taken by projects in
  the agent-sandbox space.
- **Native Windows** (Docker Desktop or podman machine) would be namespace
  class: one shared Hyper-V VM. Worth doing only if WSL2 is not enough.
