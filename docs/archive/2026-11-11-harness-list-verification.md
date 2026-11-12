# Harness list verification

Date: 2026-11-11. Evidence for step 1 of
`2026-11-11-stage1-implementation-plan.md` (issue #278, core half): a box
spec lists its harnesses, and harness env is set per exec. The end-to-end
suite is suspended, so this records what the real binary did. No test was
added. Specs and scripts: `2026-11-11-harness-list-verification/`. The
scripts expect `P` (the binary) and `D` (a directory holding `specs/` and
`up.sh`) in the environment.

## Environment

- macOS 27.2.1, build 26B131, arm64. Apple `container` CLI 1.7.1. Its
  service was stopped before the run, started with
  `container system start --disable-kernel-install` and stopped again
  after.
- `cargo build -p pinfold` of the commit this file lands in.
- Image `localhost/pinfold/profile-default:latest`, already present. Nothing
  was pulled, built or deleted.
- A private state root: `XDG_STATE_HOME`, `XDG_CACHE_HOME` and
  `XDG_CONFIG_HOME` under `/tmp/pfv.1JAe`. A scratchpad path was too long
  for the proxy socket, so a short `/tmp` directory was used. The harness
  cache started empty there.
- Login values were dummies set for the run: `HL_DUMMY_CLAUDE` and
  `HL_DUMMY_ANTHROPIC`. No real credential was used.
- `box up` was fed the spec on stdin, then held open by a `sleep`; each box
  was ended with `pinfold box down NAME`.

## (a) Three harnesses

Spec `hl-three`: `"harnesses": ["pi","claude","codex"]`, `env`
`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = "spec-wins"`, allow
`api.github.com`, a claude login route `claude` (`from`
`HL_DUMMY_CLAUDE`) and a pi `anthropic` route `anthropic` (`from`
`HL_DUMMY_ANTHROPIC`).

- `up` printed `ready`. The first start, which downloaded all three
  harnesses into the empty cache, took 10.05 s to `ready`.
- `pinfold box exec hl-three -- /opt/pinfold/<n>/<n> --version`:

  | n | Output | Exit |
  |---|---|---|
  | pi | `1.5.2` | 0 |
  | claude | `2.1.322 (Claude Code)` | 0 |
  | codex | `codex-cli 0.167.1`, after `WARNING: proceeding, even though we could not create PATH aliases: Read-only file system (os error 30)` | 0 |

- `pinfold artifacts` pins pi 1.5.2, claude 2.1.322 and codex 0.167.1.
- `/opt/pinfold` listed `claude codex login pi`; `/opt/pinfold/login` held
  `pi.ts`.

## (b) Spec refusals

Each spec went to `box up` on stdin, which then closed. Each exited 1 with
one `refused` line, reason `spec`:

| Spec | `detail` |
|---|---|
| `"harnesses": ["pi","pi"]` | `harness "pi" is listed twice` |
| `"harnesses": ["pi","gemini"]` | `harness "gemini" is not supported; the harnesses are ["pi", "claude", "codex"]` |
| `"harness": "pi"` | ``unknown field `harness`, expected one of …`` |

After all of (b) and (c), `box list --label dev.pinfold.owner` printed
nothing, `container list --all` listed no `hl-` box, the state root's
`boxes/` was empty, and `runs/` held only the run from (a).

## (c) Login route rules

| Spec | Result |
|---|---|
| harnesses `["pi"]`, a claude login route | `spec`: `login route "c" names "claude", but the spec's harnesses are ["pi"]` |
| harnesses `["claude"]`, two claude login routes | `spec`: `login route "c2" is a second claude login route` |
| harnesses `["pi","claude"]`, two claude routes and a pi `anthropic` route | `spec`: `login route "c2" is a second claude login route` |
| `hl-three` from (a): one claude route and one pi `anthropic` route | `ready` |

## (d) Per-exec env

`pinfold box exec hl-three [--harness H] -- env`, filtered to
`ANTHROPIC|CLAUDE|PI_|PINFOLD|PROXY|proxy|OPENROUTER|OPENAI`:

| Exec | Variables seen |
|---|---|
| `--harness claude` | `ANTHROPIC_BASE_URL=http://claude`, `CLAUDE_CODE_OAUTH_TOKEN=pinfold-placeholder`, `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=spec-wins` |
| `--harness pi` | `ANTHROPIC_API_KEY=pinfold-placeholder`, `PI_SKIP_VERSION_CHECK=1`, `PI_TELEMETRY=0`, `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=spec-wins` |
| `--harness codex` | `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=spec-wins` |
| no `--harness` | `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=spec-wins` |

Every exec also showed `PINFOLD_ALLOW=api.github.com`,
`HTTPS_PROXY=http://127.0.0.1:3128`, `http_proxy=http://127.0.0.1:3128` and
`NODE_USE_ENV_PROXY=1`.

- claude's pinned default for `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` is
  `1`. With `--harness claude` the spec's `spec-wins` stayed.
- `--harness nope` printed `pinfold box exec: box "hl-three" has no harness
  "nope"` on stderr and exited 3.
- No file under the state or config root held either dummy value
  (`grep -rl` found none). The box's state dir held `harnesses.json` with
  only pinned defaults, placeholders and `http://claude`.

## (e) Launch record

`pinfold box inspect` of `hl-three`'s run:

- `harnesses`: `[{"name":"pi","version":"1.5.2"},{"name":"claude","version":"2.1.322"},{"name":"codex","version":"0.167.1"}]`
- `env`: `["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC","PINFOLD_ALLOW"]`.
  Harness defaults and placeholders are no longer listed.
- routes `anthropic` and `claude`, both `login` to
  `https://api.anthropic.com`.
- After `box down`: `down` reason `signal`.

## (f) pi layer

From an empty directory outside any Git repository, under the same state
root, `pinfold pi --version < /dev/null`:

- stderr: an `image-outdated` notice for
  `localhost/pinfold/profile-full:latest` (the default profile here), the
  scratch-disk line, then `pinfold pi: empty scratch disks removed`.
- stdout `1.5.2`, exit 0.
- The run's launch record has `harnesses` `[{"name":"pi","version":"1.5.2"}]`,
  `env` `["HERDR_AGENT","HOME","PINFOLD_ALLOW"]`, and terminal
  `harness-exited`.
- The pi layer then failed its exec if the box recorded no env for pi.
  After that change the same command gave the same output and exit 0, so
  the entry was read.

## Start time

Warm cache, `box up` to the `ready` line, then `box down`. `hl-one` lists
`["pi"]` with allow `api.github.com` and no routes; `hl-three` is (a)'s spec.

| Spec | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| one harness | 0.97 s | 0.78 s | 0.79 s |
| three harnesses | 0.78 s | 0.90 s | 1.16 s |

Apple binds each mount at start, so these numbers say nothing about Linux.
Linux re-images every mount at each start, and that cost was not measured
here.

## Not verified

- A `codex` route and pi's `openai-codex` route in one box sharing one
  token and one helper ask. A run would need the host's real Codex login,
  so none was attempted.
- Linux. The Linux code was linted with
  `cargo clippy --target x86_64-unknown-linux-musl`, but not run.
- pi's own environment under `pinfold pi`. The pi layer reads it through
  the same function `box exec --harness pi` uses, which (d) shows.
- Any harness reaching its provider through a login route. The values were
  dummies.
