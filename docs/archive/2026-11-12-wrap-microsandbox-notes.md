# wrap and microsandbox: what was read

Date: 2026-11-12. Evidence for `2026-11-12-wrap-shaped-proposal.md`.
Diagnostic only. Nothing here was built or run.

Two research subagents read the sources. The operator did not re-read
them. Marks: [S] read from source, [D] read from the project's own docs,
[I] inferred, [W] through a summarising fetch and not checked directly.

## wrap

`github.com/tobi/wrap`, default branch, shallow clone, 2026-11-12. Files
read: README.md, resources/default.yml, src/config.rs, src/main.rs,
src/skill.md, Containerfile, resources/mise.toml.

- One microVM per workspace, cached between starts. The current
  directory is mounted writable at `/home/user/workspace`. [D]
- `wrap -- CMD` runs one command. Bare `wrap` is a shell. `wrap -c DIR
  read|write|bash …` lets a host agent act in a running VM. [D]
- Engine: the `microsandbox` crate, pinned `0.6.17`. [S]
- macOS: the README says the build is "tracked separately while the
  upstream microsandbox dependency is repaired". Published images are
  amd64 only. [D]
- Guest user has passwordless sudo. The desktop image runs Chrome with
  `--no-sandbox`. [D]
- Egress: default deny, TCP 80 and 443 to an allowlist, DNS to the host.
  The DNS rule allows any query name. [S] [I] That is a channel to an
  attacker's resolver.
- Secrets: every guest variable holds `NOT-AN-ACTUAL-KEY`. The real
  value is substituted by microsandbox on traffic to the secret's hosts.
  Variables are set for the whole VM; no per-agent scoping exists. [S]
- Defaults: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`,
  `GH_TOKEN`. No `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_BASE_URL` or
  Codex login handling anywhere in the tree. [S]
- Subscription login: only an opt-in `host-copy: ~/.claude` (or
  `~/.codex`, `~/.pi`), which copies the whole host directory into the
  guest unfiltered. [S]
- Agents: installed at build as mise packages at `@latest`; the resolved
  version is pinned in the image. The stage is cached and refreshes on
  `wrap --rebuild` or a configuration change. Nothing disables an
  agent's own updater. [S]

## microsandbox

`github.com/superradcompany/microsandbox` (formerly `zerocore-ai`). Tag
`v0.6.17` and `main` at 0.7.15 read.

- libkrun through `msb_krun`: Hypervisor.framework on Apple Silicon, KVM
  on Linux x64 and arm64. No daemon; one host process per VM holds the
  VMM, the network stack and the file share. [S] [D]
- README: "still beta software". 0.6 to 0.7 changed the secret API and
  the configuration database. [S]
- Network: the guest has a virtio network device. A userspace smoltcp
  stack on the host applies policy. The library default allows the
  public internet. Name rules match on SNI plus an address the guest
  resolved for that name. QUIC is not intercepted. [S] [D]
- Secrets: TLS is intercepted with a generated CA whose certificate is
  installed in the guest. In 0.6.17 any request header carrying the
  placeholder is substituted. A value passed with `.value()`, as wrap
  does, is stored in plaintext in a database under `~/.microsandbox`. [S]
  [D]
- Advisories: 12 published. Several are allowlist bypasses. GHSA-3mpp
  (Host and authority mismatch) and GHSA-cg9x (NAT64) are reported to
  affect 0.6.17. GHSA-cvr8 (the world-readable database) had no patched
  version listed. [W]
- OAuth: nothing for Claude or Codex subscriptions. The documented
  recipe logs in inside the guest. A running VM's secret value can be
  replaced from the host with `modify().secret(…)`. [S] [D]
- File share: an in-process virtiofs server with default entry and
  attribute timeouts of 5 s. [S] [I] No setting was found, and the stale
  window was not measured.
- Guest: work runs as root by default. An optional restricted profile
  sets `no_new_privs`. [D]
- Host: no seccomp, Landlock, sandbox-exec or namespace hardening of the
  VMM process was found in the repository. `msb_krun` was not read. [S]
- Owner death: the VM stops when its creator's pipe closes, unless
  detached. [S]

## Not established

- The advisory version ranges and the open-issue details, beyond [W].
- Whether a Claude or Codex subscription login works through
  microsandbox's interception at all.
- wrap's behaviour on this Mac. It was not installed.
