# Linux run of the project configuration cases (#294), 2026-11-14

Commit 2a9bf28 (main with #299), debug build, on the Linux trial VM:
Debian 13 amd64, kernel 6.12.107 cloud, 2 vCPUs, 4 GB, nested KVM,
`/dev/kvm` world-writable as `scripts/setup-linux-runner.sh` sets it.
Script: `2026-11-14-project-config-linux/linux-cases.sh`. Output:
`linux-output.txt` beside it. No credentials were used.

## Established

- Outside Git, `pi`, `config edit`, `config` and `build` are each refused
  as `not-git` with exit 1. No configuration is created. The only entries
  under the state root are `pinfold/` and its `maintenance` stamp.
- In a Git project with no configuration, `pi` is refused as `no-config`
  and no project state directory exists.
- `config edit` creates the project's `pinfold.toml` under the config root.
- A `mounts` value other than the template is refused as `spec`.
- A cache root that resolves inside the project is refused as
  `host-path-in-project`, and nothing is written inside the project.

## Not established

- A started box. `pinfold build` failed on this VM: `builder exited before
  handshake`, then `the BuildKit build failed`, with no further diagnostic.
  `pinfold doctor` reported `preflight: ok`. So `pi --version`, `push` and
  `fetch` in a configured project were not exercised on Linux. The cause is
  not known: it may be this VM or a fault in the Linux build path.
