# Decisions, 2026-09-22 to 2026-09-23

Historical record of the decisions behind pinfold's design, with the reason
and what was rejected. The current spec is `docs/ARCHITECTURE.md` and wins
where they differ. Evidence is in `2026-09-22-spikes.md`; the full draft the
spec was cut from is `2026-09-23-design-draft.md`.

## Scope

- **Rewrite agentbox from scratch** (2026-09-22). agentbox.sh (~2.8k lines
  of shell) grew by patching. Goals: modular, simpler, faster, minimal. Two
  uses: interactive coding with pi, and a programmatic box API for
  Switchyard that replaces its Docker layer. agentbox stays unchanged until
  pinfold replaces it.
- **Core, pi layer, profile.** The core is harness-agnostic because
  Switchyard runs pi, Codex and Claude workers and gates. Git handoff is in
  the pi layer, not core: it is needed for interactive pi and not for
  Switchyard, whose lanes are clones.
- **Features move to pi extensions or data where possible.** The profile is
  data plus one pi package.
- **No config or GitHub tooling.** Replaced by `PINFOLD_ENV_<NAME>`, a
  project `.pinfold.toml`, and defaults. Credentials come from `op run`,
  `GH_TOKEN` and direnv, with no pinfold code.

## Language and name

- **Rust** over TypeScript (2026-09-22). The in-box init must be a small
  static binary that works in any image; the Linux musl build doubles as it.
  The rest is systems glue: processes, a VM, a TTY, sockets, an adversarial
  protocol parser. Switchyard stays TypeScript behind a process boundary
  (JSON on stdio), not a library import.
- **Name: pinfold** (2026-09-22), a village pound where strays are held.
  Free on crates.io, PyPI, npm and Homebrew. glovebox, hotcell, vivarium and
  hutch were taken in the agent-sandbox space. `box` was considered first,
  then dropped for a name free on crates.io.
- **Windows** would be the Linux build in WSL2. Not in v1. Native Windows
  (one shared Hyper-V VM) would be namespace class.

## Runtimes

- **Apple `container` on macOS, not podman everywhere** (2026-09-23). podman
  on macOS runs every box in one shared VM: no per-box kernel, no faster
  file sharing (virtiofs either way), no host unix socket bind mount through
  its VM, and a VM always holding memory. The cost is a second adapter and
  a younger runtime.
- **Rootless podman only on Linux** (2026-09-23). A breakout gets the
  user's uid, not root, and pinfold needs no root. Rootful podman on
  Debian 13 also breaks signals between exec'd processes (AppArmor).
  docker is refused.
- **Nothing long-lived runs as uid 0** (2026-09-23). On Apple `container` a
  uid-0 process touching a mount caused random EACCES for the box user and
  could turn a host-set setuid bit into euid 0. PID 1 moved to the host uid;
  only a transient root exec remains, to chmod the forwarded socket.

## Egress

- **Built-in proxy, one per box** (2026-09-22), replacing the shared squid
  and its problems: the pidfile, the reconfigure race between projects, and
  the Linux `shm_open` clash.
- **`--network none` plus a socket transport** (2026-09-22), over a
  per-box internal network. With no interface the box cannot reach host
  listeners (LAN, Tailscale, IPv6), gateway services, Tailscale MagicDNS
  (a DNS exfiltration channel) or other boxes. It also removes the macOS
  firewall prompt (no listener) and the per-box token. The internal network
  with a route-lockdown init is the fallback.
- **Routes for HTTP host services, forwards for sockets.** Routes cover
  Switchyard's MCP endpoint. Forwards were added (2026-09-23) for tools that
  need a host socket: Switchyard's `daemon.sock` for an operator agent, and
  herdr later. herdr's socket is never raw, because `pane.run` is host
  command execution.

## Box and processes

- **Box as the unit of lifetime** (2026-09-22). A per-process signal wrapper
  in the box was judged too messy. `container exec` cannot forward signals
  anyway (apple/container#1941), so ending a session removes the box. Cost:
  the process is killed, not asked to stop; pi's session file loses at most
  the turn in flight.
- **Every box idles under `pinfold init` and takes work by `exec`.** This is
  Switchyard's model already, and it lets the pi layer run the git bundle
  step after pi exits.

## Git

- **Git handoff instead of mount protection** (2026-09-22). The mounts spike
  showed no layout protects a writable `.git`: the box can plant
  `commondir` pointing at a poisoned copy, and host git follows it. The box
  gets a fresh git dir per run and returns commits as a bundle that host git
  fetches with `transfer.fsckObjects`.

## State and files

- **Per-project `$HOME` in the host state dir** (2026-09-22), not in the
  checkout. Visible on the host, seedable without a box, untouched by
  `git clean`, not shared between projects.
- **No per-project ext4 volume for `$HOME`** (2026-09-23). It measured faster
  than the host disk, but only caches benefit; the agent mostly writes to
  the project, which must stay on the shared mount for the IDE. Rejected as
  bug surface for a speedup that may not be noticeable. Rule: don't add
  speedups that haven't been measured to matter.
- **Settings seeded once, never merged.** No marker file.

## Images

- **`debian:trixie-slim`, current over pinned** (2026-09-22). The tag
  floats and each build runs `apt-get upgrade`, so security fixes land on
  rebuild. The image is not the security boundary.
- Rejected bases:

  | Option | Why not |
  |---|---|
  | Wolfi | Chainguard controls it: community contributions closed, free tier `:latest` only. Judged non-OSS in style. |
  | Docker Hardened Images | Pulling needs a Docker login. |
  | Nix | Non-FHS: prebuilt binaries (manylinux wheels, `uv python install`, npm natives) fail. Needs a Linux builder and a format conversion on macOS. |
  | Alpine | musl cannot run pi's glibc build or rtk's arm64 build. |
  | Ubuntu chisel | Cannot pin package versions. |
  | UBI | Missing tools; UBI 9's glibc too old for rtk. |

- **Minimal v1 toolset** (2026-09-22): git, curl, rg, fd, jq, less, gh, bun,
  rtk. No R, shellcheck, shfmt, uv, python or ast-grep. Tools are added when
  a real need shows up.
- **Harness as a pinned artifact**, mounted read-only, not baked into the
  image, so any project's Containerfile works. Switchyard already does this.

## Testing

- **End-to-end only** (2026-09-22). Agents write poor unit tests; a short,
  fixed list of real e2e tests shows the fundamentals work.
- **Where it runs** (2026-09-22). Linux podman in GitHub CI is the required
  gate. macOS runs as a Switchyard host gate at `stage = "batch"`, after
  exact-head approval. No pre-push hook, no self-hosted runner. Hosted macOS
  runners cannot run Apple `container` (no nested virtualization).
- **CI runners `ubuntu-26.04` and `ubuntu-26.04-arm`** ship podman 5.7;
  `ubuntu-24.04` ships 4.9. Both architectures, because the init is per
  architecture.

## Switchyard

- **Bootstrap** (2026-09-22): Switchyard as it is today (Docker workers)
  builds pinfold, using open models through OpenRouter or opencode-go
  (Switchyard 0.17.4). Until Switchyard moves onto pinfold, workers have
  unrestricted network and reviews run on the host.
- **Reviews move off `autoreview`** to a pi review extension in a read-only
  pinfold box.
- Switchyard's side: DESIGN.md amendments (it fixes the Docker CLI and forbids
  the pinned CLI as an image build step), `src/docker.ts` and the worker-CLI
  download code (~4.8k lines) replaced, adapters kept.

## Dropped from agentbox.sh

The `config` store and command; `github` token minting; the 1Password code;
the `proxy` subcommand; `vendor check`/`vendor update` and `vendor.lock`;
socket forwarding with socat; pi-wrapper settings seeding; the session lock;
the `--continue`, provider and model defaults; retired settings; the builder
subnet and DNS settings; `AGENTBOX_ISOLATION`; the shared squid; most of
`doctor` and `clean`; the static `AGENTS.md` copied on every run.
