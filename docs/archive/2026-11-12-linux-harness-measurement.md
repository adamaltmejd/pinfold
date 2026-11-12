# Linux harness list and start cost

Date: 2026-11-12. pinfold at 283f052 (`main`, PR #284) on Linux x64. Two
things: step 1's cases from `2026-11-11-harness-list-verification.md`,
which ran only on macOS, and the cost of a box start with one harness
against three. The end-to-end suite is suspended, so this records what the
real binary did. No test was added and no source was changed. Specs,
scripts and raw output: `2026-11-12-linux-harness-measurement/`. The
scripts expect `P` (the binary) and `D` (that directory) in the
environment.

## Environment

- The homelab trial VM, through `labvm`. Debian 13, kernel
  6.12.107+deb13-cloud-amd64, 2 vCPUs of an Intel i5-9500T, 3 GiB RAM.
  `/home` is on the root ext4 virtual disk (`rotational` 0; `dd` of
  800 MiB with `conv=fsync` ran at 1.3 GB/s). `/tmp` is tmpfs.
- Host setup was `scripts/setup-linux-runner.sh` (the KVM udev rule,
  build-essential, e2fsprogs) plus `musl-tools`, `strace`, `curl` and
  rustup. No podman was installed; the script does not ask for one.
  mke2fs 1.47.2, strace 6.13.
- `cargo build -p pinfold --locked --release --target
  x86_64-unknown-linux-musl` with `CC_x86_64_unknown_linux_musl=musl-gcc`,
  installed as `~/bin/pinfold`. `pinfold --version`: 0.5.5.
- `pinfold doctor`: runtime Firecracker 1.17.0+6.18.57, `preflight: ok`.
- `pinfold build --profile default` printed
  `localhost/pinfold/profile-default:latest` and exited 0 after 58 s. Its
  id is `sha256:15ffeacd…cbcd`. Every box below used it.
- The default XDG roots under `/home/agent`, so state, cache and the
  per-start mount images were all on the ext4 disk.
- Login values were dummies set by `cases.sh`: `HL_DUMMY_CLAUDE` and
  `HL_DUMMY_ANTHROPIC`. No real credential was used.
- `box up` was fed the spec on stdin, then held open by a `sleep`
  (`up.sh`); each box was ended with `pinfold box down NAME`.

## A. Step 1's cases

Spec `three.json`, the macOS `hl-three` unchanged: harnesses
`["pi","claude","codex"]`, `env`
`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = "spec-wins"`, allow
`api.github.com`, a claude login route and a pi `anthropic` route. Run by
`cases.sh`; its output is `out-cases.txt`. The commands under "pi differs
from macOS" and "One more observation" below were typed by hand afterwards.

`up` printed `ready`. This first start downloaded the three harnesses into
an empty harness cache and took 24.49 s.

### Mounts and versions

- `ls /opt/pinfold` listed `claude codex login pi`. `/opt/pinfold/login`
  held `pi.ts` and `lost+found`.
- `/proc/mounts` in the box: `/dev/vdc` on `/opt/pinfold/pi`, `/dev/vdd`
  on `/opt/pinfold/claude`, `/dev/vde` on `/opt/pinfold/codex`, `/dev/vdf`
  on `/opt/pinfold/login`, each `ext4 ro,nosuid,nodev`.
- `pinfold artifacts` pins pi 1.5.2, claude 2.1.322 and codex 0.167.1.
- `pinfold box exec hl-three -- /opt/pinfold/<n>/<n> --version`:

  | n | Output | Exit |
  |---|---|---|
  | pi | A Bun stack trace on stderr ending `ENOENT: no such file or directory, uv_os_homedir`. Nothing on stdout. | 1 |
  | claude | `2.1.322 (Claude Code)` | 0 |
  | codex | `codex-cli 0.167.1`, after `WARNING: proceeding, even though we could not create PATH aliases: Could not find home directory` | 0 |

**pi differs from macOS**, where the same command printed `1.5.2` and
exited 0. `--harness pi` on the same command gave the same trace and exit 1.
What a bare exec sees in this box:

```
$ pinfold box exec hl-three -- sh -c 'id; echo HOME=[$HOME]; grep -c . /etc/passwd; grep :$(id -u): /etc/passwd; pwd'
uid=1000 gid=1000 groups=1000
HOME=[]
18
/
```

`HOME` is unset and uid 1000 has no passwd entry. With a home given in the
exec's own argv, pi ran at its pin:

```
$ pinfold box exec hl-three -- env HOME=/tmp /opt/pinfold/pi/pi --version
1.5.2                                                      exit 0
$ pinfold box exec hl-three -- env HOME=/tmp /opt/pinfold/codex/codex --version
codex-cli 0.167.1                                          exit 0
```

(codex's warning then read `Refusing to create helper binaries under
temporary dir "/tmp"`.) Nothing was changed to make the bare command pass.
The spec in use sets no `HOME`. Whether a spec's box should have one is not
answered here.

### Per-exec env

`pinfold box exec hl-three [--harness H] -- sh -c "env | grep -E '…' | sort"`,
filtered to `ANTHROPIC|CLAUDE|PI_|PINFOLD|PROXY|proxy|OPENROUTER|OPENAI|CODEX`.
All exited 0.

| Exec | Variables seen |
|---|---|
| `--harness claude` | `ANTHROPIC_BASE_URL=http://claude`, `CLAUDE_CODE_OAUTH_TOKEN=pinfold-placeholder` |
| `--harness pi` | `ANTHROPIC_API_KEY=pinfold-placeholder`, `PI_SKIP_VERSION_CHECK=1`, `PI_TELEMETRY=0` |
| `--harness codex` | none beyond the box-wide ones |
| no `--harness` | none beyond the box-wide ones |

Every exec also showed the box-wide
`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=spec-wins`,
`PINFOLD_ALLOW=api.github.com`, `HTTPS_PROXY=http://127.0.0.1:3128`,
`http_proxy=http://127.0.0.1:3128` and `NODE_USE_ENV_PROXY=1`. No exec
showed another harness's placeholder. This matches the macOS table.

### Unlisted harness

```
$ pinfold box exec hl-three --harness nope -- true
stderr: pinfold box exec: box "hl-three" has no harness "nope"      exit 3
$ pinfold box exec hl-three --harness gemini -- true
stderr: pinfold box exec: box "hl-three" has no harness "gemini"    exit 3
```

Nothing on stdout in either.

### Always-applied names

`pinfold box exec hl-three --harness H -- sh -c 'echo
"$NODE_USE_ENV_PROXY|$HTTPS_PROXY|$http_proxy"'` printed
`1|http://127.0.0.1:3128|http://127.0.0.1:3128` for claude, pi and codex.

The CLI gives no way to hand an exec a variable of its own: the per-exec
env is what `box up` wrote to the box's `harnesses.json`. That file held
only `ANTHROPIC_BASE_URL` and `CLAUDE_CODE_OAUTH_TOKEN` for claude,
`ANTHROPIC_API_KEY`, `PI_SKIP_VERSION_CHECK` and `PI_TELEMETRY` for pi, and
nothing for codex. No pinned default uses an always-applied name, so the
drop at write time (`box.rs:818`) had nothing to drop in this run. The
override case was therefore not exercised through the CLI.

One more observation, outside the CLI. With the box up, the host file
`boxes/a443d4cb73283f72/harnesses.json` was edited by hand to give codex
`NODE_USE_ENV_PROXY=0`, `HTTPS_PROXY=http://tampered`,
`http_proxy=http://tampered` and `HL_CONTROL=control`:

```
$ pinfold box exec hl-three --harness codex -- sh -c 'echo "$NODE_USE_ENV_PROXY|$HTTPS_PROXY|$http_proxy|$HL_CONTROL"'
0|http://tampered|http://tampered|control                  exit 0
```

So here the exec applied what that file held over the box-wide values.
The only protection seen is the filter when `box up` writes the file. The
macOS run did not try this edit, so this says nothing about Apple. The
file was restored afterwards.

### Other

- `grep -rl` for either dummy value under `~/.local/state/pinfold` and
  `~/.cache/pinfold` printed nothing. It exited 2, because the command
  also named `~/.config/pinfold`, which did not exist.
- `pinfold box inspect RUN`: `harnesses`
  `[{"name":"pi","version":"1.5.2"},{"name":"claude","version":"2.1.322"},{"name":"codex","version":"0.167.1"}]`,
  `env` `["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC","PINFOLD_ALLOW"]`,
  four mounts (the three harness directories and `login`), runtime
  `firecracker` 1.17.0+6.18.57.
- `box down hl-three` exited 0; `up` printed `down` with reason `signal`.
  No `firecracker` or `box up` process was left and `boxes/` was empty.

## B. Start cost

Three specs that differ only in `harnesses`, each with allow
`api.github.com` and no routes, env or disks: `t-none` (no `harnesses`
key), `t-one` (`["pi"]`) and `t-three` (`["pi","claude","codex"]`).
`time.sh 6 t-none t-one t-three`: one warm-up of each, then six rounds,
the three specs in turn within a round. The clock is bash's
`$EPOCHREALTIME` from just before `box up` is spawned to the arrival of
its first stdout line, which was `ready` every time. Each box was brought
down before the next start. Raw lines: `out-time.txt`.

| Spec | Runs (s) | Median | Range |
|---|---|---|---|
| no harness | 2.169, 2.378, 2.317, 2.214, 2.328, 2.225 | 2.27 s | 2.17–2.38 s |
| one (`pi`) | 3.639, 3.656, 3.673, 3.600, 3.623, 3.663 | 3.65 s | 3.60–3.67 s |
| three | 11.187, 11.507, 11.311, 11.142, 11.103, 11.097 | 11.16 s | 11.10–11.51 s |

Warm-ups: 2.298, 3.730 and 11.043 s.

Differences of medians:

- three − one: 7.52 s, so 3.76 s for each of the two added harnesses on
  average.
- one − none: 1.38 s for pi.
- three − none: 8.89 s for all three.

### Where the time goes

One extra start of each spec ran under
`strace -f --seccomp-bpf -ttt -e trace=execve,exit_group`
(`out-strace-*.txt`). `box up` runs `mkfs.ext4 -d` once for the init disk
and once per mount, one after another, before it starts Firecracker
(`vm.rs` `write_init_disk` and `mount_image`). From `execve` to exit, in
the `t-three` trace:

| Image | Source size | Entries | Image size | `mkfs.ext4 -d` |
|---|---|---|---|---|
| init disk | the pinfold binary, 7.2 MiB | 4 | 24476k | 0.06 s |
| pi | 125 MiB | 234 | 157193k | 1.43 s |
| claude | 326 MiB | 2 | 383682k | 3.99 s |
| codex | 381 MiB | 3 | 445441k | 3.44 s |

- The three harness images sum to 8.86 s, against 8.89 s between the
  no-harness and three-harness medians.
- The `t-one` trace gave 1.43 s for pi; one − none was 1.38 s.
- The same `mkfs.ext4` options run by hand on each cached harness
  directory, three times each: pi 1.43, 1.49, 1.45 s; claude 4.03, 4.05,
  4.00 s; codex 3.49, 3.75, 3.48 s.
- In all three traces the first `mkfs.ext4` started 0.77–0.81 s after
  `pinfold`'s own `execve`, and Firecracker started 11–12 ms after the
  last `mkfs.ext4` exited. The rest of the 2.27 s no-harness start is
  after that, by subtraction about 1.4 s.
- Tracing slows the VMM: the traced starts took 4.65, 6.16 and 13.83 s to
  `ready`. The `mkfs.ext4` spans did not move with tracing. An earlier
  trace without `--seccomp-bpf` slowed `mkfs.ext4` itself several times
  over and is not used.

The time does not track the entry count: claude has 2 entries and pi
234. The rates were 87, 82 and 111 MiB/s for pi, claude and codex, so
it does not track the bytes exactly either. The traces' SIGCHLD lines
give user plus system time against wall time of 1.34 of 1.43 s for pi,
3.78 of 3.99 s for claude and 3.23 of 3.44 s for codex.

The macOS record measured 0.78–1.16 s for both one and three harnesses.

## Not covered

- Spec refusals and login route rules ((b) and (c) of the macOS record).
  The task did not list them.
- `pinfold pi` on Linux, and any harness reaching its provider.
- A cold start. Every timed start had the harness files in the page
  cache.
- A second machine. The numbers are from one 2-vCPU VM.
- `scripts/setup-linux-runner.sh` failed once on a held dpkg lock right
  after the VM came up. Its udev rule had applied; the two packages were
  installed by hand afterwards.
