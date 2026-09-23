# pinfold architecture

pinfold runs a coding agent in a disposable box: an Apple `container`
micro-VM on macOS, a rootless podman container on Linux. The box sees its
mounts and nothing else of the host. Its only way out is its own allowlisting
proxy.

- **Interactive:** `pi` on the host is a shim for `pinfold run`.
- **Programmatic:** Switchyard drives boxes through `pinfold box`, JSON on
  stdio.

This is the current spec. Evidence and rationale are in `docs/archive/`.

## Layers

| Layer | Owns | Used by |
|---|---|---|
| core | runtimes, box lifecycle, transport, proxy, forwards, images, pinned artifacts | Switchyard, pi layer |
| pi | shim, per-project state, seeding, git handoff, herdr | the `pinfold` CLI |
| profile | image, pi package, seed settings, default config | pi layer, as data |

Core knows nothing of pi or git. The profile holds no code except a pi
package.

## Threat model

A prompt injection turns the agent into an exfiltration path or into code
running on the host.

| Control | Effect |
|---|---|
| No network | Loopback only. One unix socket to the box's own proxy. |
| Egress proxy | Default deny, allowlist by name, every decision logged. |
| Kernel boundary | A VM per box (Apple). Namespaces (podman), warned on every run. |
| No privilege | Empty capability bounding set, nothing long-lived as uid 0, read-only rootfs, no setuid files. |
| Git handoff | The box never writes the host's `.git`. |

Not protected:
- Project contents: an agent with a model API can put them in a prompt.
- CDN fronting beyond the SNI check (Host-header fronting needs TLS
  interception).
- Allowlisted services that accept writes (GitHub with a token, registries).
- Image builds: user-run, trusted, unrestricted egress.

## Runtimes

| Runtime | Class | Behaviour |
|---|---|---|
| Apple `container`, macOS 26+ | vm | run |
| rootless podman, systemd cgroup manager | namespace | run, warn |
| anything else (rootful podman, docker) | — | refuse |

Detected, or set with `PINFOLD_RUNTIME`. podman preflight requires
`rootless=true` and `cgroupManager=systemd`; under cgroupfs, `--cpus` and
`--memory` are silently not enforced. Long-lived boxes need
`loginctl enable-linger`.

## Box

### Lifecycle

1. Start the box's proxy on a new unix socket in the state dir. Keep the path
   under macOS's 104-byte limit.
2. Run the box with `--network none` and the socket carried in (Transport).
   PID 1 is `pinfold init`, as the host uid.
3. Apple only: one transient root exec makes the socket connectable.
4. `pinfold init` relays `127.0.0.1:3128` to the socket, reaps children, and
   reports ready.
5. `exec` work as the host uid:gid, with `HTTPS_PROXY=http://127.0.0.1:3128`.
6. Remove the box, close the proxy, delete the socket.

One `pinfold box up` process owns one box. It does steps 1–4, holds the
proxy, and does step 6 on `down`, stdin EOF or SIGTERM. If it dies, the
socket dies and the box has no way out (fails closed); `box prune` removes
leftovers by label. The CLI runs the same code in-process.

### Process interface

```
pinfold box up < spec.json        # prints {"event":"ready","box":…}; holds the box
pinfold box exec BOX [--tty] [--workdir D] -- argv…   # stdio through, exit code back
pinfold box down BOX              # same as closing up's stdin
pinfold box list --label k=v      # JSON lines
pinfold box prune                 # remove boxes whose `up` is gone
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

- No `egress`: no way out at all (gates).
- `env` is exact; nothing is inherited. `{ "from": "NAME" }` is read from
  the caller's environment and passed as `--env NAME`, so values never reach
  argv.
- Apple: mounts are directories. Nested read-only mounts protect subpaths.

### Always applied

- `--network none`, `--cap-drop ALL`, `--read-only` with a tmpfs `/tmp`.
- PID 1 and all work run as the host uid:gid.
- `NODE_USE_ENV_PROXY=1`: Node's fetch ignores `HTTPS_PROXY` without it.
- podman adds: `--userns=keep-id`, `--security-opt no-new-privileges`, a
  seccomp profile that also blocks `CLONE_NEWUSER`, `--no-hosts`,
  `--dns none`, `--memory-swap` equal to `--memory`, `--pids-limit`, an
  explicit `HOME`, and `rm -f -t 0` at teardown.

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
- Every connection starts with a one-line header naming its target: the
  proxy or a forward. One socket carries everything.
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
- **Refused:** IP literals, and names resolving to loopback, private,
  link-local, CGNAT (`100.64/10`) or benchmark (`198.18/15`) addresses.
  Resolve once; dial the checked address.
- **Routes:** a name maps to one host service, e.g.
  `yard.internal → 127.0.0.1:7777`. Plain HTTP only, Host header rewritten.
  CONNECT to a route is refused. The host service authenticates its callers.
- **Limits:** a connection cap, a header timeout, an idle timeout on
  tunnels.
- **Log:** one JSON line per decision in the box's egress log.

## Forwards

A host unix socket exposed at a guest path. `pinfold init` listens there and
relays each connection with a header naming the forward; the host side
connects to the host socket. Bytes pass raw.

- Opt-in, in config, covered by trust, printed at every start.
- A filter can front a forward: pinfold serves its own host socket and passes
  only allowed messages to the real one.

| Forward | Grants | Default |
|---|---|---|
| Switchyard `daemon.sock` | full board control, including approve and land | off; opt-in per project; a filter can restrict it |
| herdr socket | `pane.run`, i.e. host command execution | never raw; the filter passes only `pane.report_agent` for the box's own pane |

## Images

An image is a toolchain; the harness is not baked in. The default comes from
the profile's Containerfile. A project can name its own, typically `FROM` the
profile image.

Profile image:
- `debian:trixie-slim` by tag, `apt-get upgrade` at every build. The
  resolved base digest is recorded as a label; the previous image is kept for
  rollback.
- Debian: ca-certificates, git, curl, ripgrep, fd-find (as `fd`), jq, less.
- gh from GitHub's signed apt repository.
- `ADD --checksum=sha256:…`: bun, rtk, and the ponytail pi package. A bump
  script updates the pins.
- setuid and setgid bits stripped.

## Pinned artifacts

Checksummed release binaries in
`~/.cache/pinfold/artifacts/<name>/<version>/<os-arch>/`, mounted read-only:
pi, the Linux `pinfold` (as the init), and Switchyard's codex and claude.

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

## pi layer

- **Harness:** pi, a pinned artifact at `/opt/pinfold/pi`.
- **State:** `~/.local/state/pinfold/projects/<name>-<hash>/home`, mounted as
  `$HOME`. pi's agent dir, sessions, `~/.config` and caches sit at their
  default paths. One per project, outside the checkout.
- **Seeding:** when a project's agent dir is created, the profile's
  `settings.json` is copied in. pinfold never touches it again.
- **Arguments** pass through unchanged. The project is mounted at its own
  absolute path; the box starts in the invoking directory. Paths outside the
  project fail, and pi reports them.
- **Environment:** `PI_TELEMETRY=0`, `PI_SKIP_VERSION_CHECK=1`, the core's
  proxy variables, and `PINFOLD_ENV_*`.
- **TTY** whenever the host has one; otherwise pi runs in print mode.
- **Auth:** OAuth `/login` once per project; API keys through the
  environment.
- **`protect`:** extra read-only directory mounts, e.g. `.claude/`,
  `.vscode/`.
- **herdr:** no socket in v1. The TTY passes through, so screen detection
  works, and the shim sets `HERDR_AGENT=pi`. Later: herdr's pi integration in
  the box plus the filtered forward.

### Git handoff

The box never writes the host's `.git`. Host git never runs in a git dir the
box could write.

1. **Host, before the run.** Read branch, HEAD, remotes and objects dir with
   host git. Build a fresh git dir for this run under project state:
   - config: remote URLs; `!gh auth git-credential` for
     `https://github.com`; identity; `core.checkStat=minimal`; the host's
     `core.ignorecase` and `core.precomposeunicode`
   - HEAD, refs, a copy of the index
   - `objects/info/alternates` to the host objects, mounted read-only

   Record a hash of the host index.
2. **Run.** The git dir is mounted read-write at `<repo>/.git`.
3. **After pi exits.** `git bundle create` in the box, for everything new
   since the start refs. If the box is gone, in a fresh box on the same git
   dir.
4. **Host.** `git -c transfer.fsckObjects=true fetch <bundle>` into
   `refs/pinfold/<run>/*`. If host HEAD and index are unchanged, fast-forward
   with a compare-and-swap `update-ref` and reset the index. Otherwise move
   nothing and print the refs to merge. Delete the run's git dir.

One git dir per run, so concurrent runs don't collide. Residual risk: a
planted nested repo (`sub/.git`) runs code if a host tool runs git in it
(VS Code does by default).

## Profile

`profile/` in this repo, or `PINFOLD_PROFILE`:

```
profile/
  pinfold.toml    same schema as .pinfold.toml
  Containerfile   the default image
  settings.json   seed for a new project's pi settings; names rtk and ponytail by path
  pi/             pi package: operating-context extension, skills
```

The operating-context extension writes the box's facts (the live allowlist;
a 403 is final) into the system prompt at `before_agent_start`. Extensions
run in the box; nothing security-relevant lives in one.

## Configuration

Precedence: environment > `.pinfold.toml` > profile `pinfold.toml` >
defaults.

| Key | Env | Default | Meaning |
|---|---|---|---|
| `allow` | `PINFOLD_ALLOW` | `[]` | Domains added to the allowlist |
| `routes` | `PINFOLD_ROUTES` | `{}` | Proxy routes to host services |
| `forwards` | `PINFOLD_FORWARDS` | `{}` | Host sockets at guest paths |
| `protect` | `PINFOLD_PROTECT` | `[]` | Read-only directories in the box |
| `image` | `PINFOLD_IMAGE` | profile image | Image ref, or a Containerfile path relative to the project |
| `cpus` | `PINFOLD_CPUS` | 4 | |
| `memory` | `PINFOLD_MEMORY` | `8G` | |
| — | `PINFOLD_RUNTIME` | detected | `container` or `podman` |
| — | `PINFOLD_PROFILE` | `profile/` | |
| — | `PINFOLD_ENV_<NAME>` | — | `<NAME>` in the box; the only way host env enters |

**Trust.** `.pinfold.toml` and the Containerfile it names are used only if
their hashes match those `pinfold allow` recorded in
`~/.local/state/pinfold/trust`. A change stops the run until allowed again.

**Credentials** need no pinfold code: `op run -- pi …` for 1Password;
`PINFOLD_ENV_GH_TOKEN` for gh and git; direnv for per-repo tokens;
`PINFOLD_ENV_GIT_*` for identity.

## CLI

```
pinfold run [pi args…]   pi in a box for this project; `pi` is a symlink to this
pinfold shell [cmd…]     bash (or cmd) in a fresh box with the same mounts, no pi
pinfold build            build this project's image
pinfold allow            trust this project's .pinfold.toml and Containerfile
pinfold doctor           runtime, class, image, artifacts, trust, effective config
pinfold box …            the process interface
pinfold init             PID 1 in the box (Linux builds)
```

The project root is the git top level, else `$PWD`.

## Switchyard

Uses core only: runtime detection and class, `box up`/`exec`/`down`,
label-based `list`/`prune` for reconcile, no-egress boxes for gates, a proxy
per worker with a route to its MCP listener, and pinned pi, codex and claude.
No git handoff: lanes are clones.

Reviews run as a pi review extension in a box with the candidate read-only,
no forwards, and egress to the model API only.

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
| 11 | The box cannot plant code for host git | A hook, `core.fsmonitor` and `commondir` written in the box leave host `.git` unchanged; host `git status` runs nothing. |
| 12 | Commits come home | A pi turn through the shim, against a fake model, edits and commits; the commit lands on the host branch. |
| 13 | A moved host branch is never overwritten | The host commits during the run; the branch is untouched and the box's commit waits under `refs/pinfold/`. |
| 14 | A changed project file stops the run | The agent adds a domain to `.pinfold.toml`; the next run refuses until `pinfold allow`. |
| 15 | Project state persists and stays separate | Settings are seeded once and survive runs; two projects don't see each other's state. |

Linux (podman) runs in GitHub CI on `ubuntu-26.04` and `ubuntu-26.04-arm` as
the required gate. macOS (Apple `container`) runs as a Switchyard host gate
at `stage = "batch"`, never `"candidate"`: code runs on the host only after
the exact head is approved.

## Code

Rust. Each Linux build is a static musl binary and doubles as `pinfold init`.

| Target | Role |
|---|---|
| `aarch64-apple-darwin` | macOS CLI |
| `aarch64-unknown-linux-musl` | Linux arm64 CLI; init on Macs and arm64 hosts |
| `x86_64-unknown-linux-musl` | Linux x64 CLI; init on x64 hosts |

Linux targets build on a Mac with `cargo zigbuild`.

Dependencies: `tokio`, `httparse`, `serde`, `serde_json`, `toml`, `nix`,
`directories`. SNI comes from a small ClientHello parser.

Portability (Windows later means the Linux build in WSL2):
- Host-to-guest paths map in one function, the identity on Unix.
- Platform dirs come from `directories`.
- Unix-only host code (signals, process groups, TTY, modes) is one module.
- Runtimes are adapters behind a trait, each declaring its class.

```
crates/pinfold/src/
  core/    runtime.rs plan.rs box.rs network.rs proxy.rs tls.rs artifacts.rs
  init.rs  socket mode, TCP relay, reaping, readiness
  pi/      launch.rs state.rs git.rs
  cli.rs config.rs trust.rs
profile/
```

## Open questions

- Not yet run end to end, so phase 1 proves them first: on Apple
  `container`, a client through the init relay, the socket and the proxy;
  PID 1 as the host uid with the transient root chmod exec; the SNI check.
  On podman, `--no-hosts` and `--dns none`.
- Git worktrees: `.git` is a file, which Apple `container` cannot mount
  over. `--read-only-path` plus `GIT_DIR`, or refuse in v1.
- `198.18/15` blocking breaks fake-IP DNS proxies (Surge, Clash).
  Configurable, or documented.
- Nested user namespaces in the Apple `container` guest kernel: can they be
  turned off?
- GitHub runners: AppArmor's unprivileged-userns restriction, and linger for
  the runner user.
- Where an installed binary finds its default profile.
- Where herdr reads `HERDR_AGENT`.
- For Switchyard: memory limits and OOM detection on Apple `container`;
  Codex's single-file credential mount becomes a directory.
- No disk cap on Apple `container`.
- Model credentials injected at the proxy, so the box sees a placeholder.
