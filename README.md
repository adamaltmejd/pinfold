# pinfold

Run a coding agent in a disposable box: an Apple `container` micro-VM or
podman on macOS, rootless podman on Linux. The box sees the project and
nothing else of the host. Its only way out is an allowlisting proxy.

Isolation differs by runtime, and pinfold says so here rather than on every
run. Apple `container` gives each box its own VM. podman boxes share a
kernel: the host's on Linux, and on macOS the podman machine's VM, which
mounts your home directory by default.

**Status:** design only; nothing works yet. pinfold will replace agentbox.

- Spec: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- History: [docs/archive/](docs/archive/)
- Working rules: [AGENTS.md](AGENTS.md)

## Roadmap

1. Core, proxy, pi layer with git handoff, and CLI, in Rust. Parity with
   agentbox for interactive use on macOS. Done when it has been used daily
   for a week.
2. Linux podman end-to-end suite in GitHub CI, and a real README. Then
   daily use moves from agentbox to pinfold, and agentbox is archived.
3. Switchyard on pinfold: its engine interface over the core, worker
   allowlists, and `docker.ts` removed.

pinfold is built with Switchyard (`.yard/`). Until phase 3 its lanes run in
Docker with unrestricted network.
