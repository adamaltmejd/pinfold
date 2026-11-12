# Agent verbs verification

Date: 2026-11-12. Evidence for step 2 of
`2026-11-11-stage1-implementation-plan.md` (issue #278, launch half):
`pinfold claude` and `pinfold codex` beside `pinfold pi` on one launch
path. The end-to-end suite is suspended, so this records what the real
binary did. No test was added. Scripts: `2026-11-12-agent-verbs-verification/`.
Each expects `T` (a short private directory holding the projects) and `P`
(the binary); `env.sh` points the XDG state, cache and config roots into
`T` and enters `$T/${PROJ:-proj}`.

## Environment

- macOS 27.2.1, build 26B131, arm64. Apple `container` CLI 1.7.1. Its
  service was stopped before the run, started with
  `container system start --disable-kernel-install` and stopped after.
- `cargo build -p pinfold` of the commit before this file.
- Image `localhost/pinfold/profile-full:latest`, already present (the
  default profile under an empty config root). Nothing was pulled, built
  or deleted. Each run printed an `image-outdated` notice for it; it is
  left out below.
- State root `/tmp/pfv.dFlv`, deleted after. Its harness cache started
  empty; the first start downloaded pi, claude and codex at their pins
  through pinfold's own path.
- `proj`: a Git repository, branch `main`, one commit `582d86d`.
  `proj2`: a Git repository, branch `trunk`, one commit `b73369a`, never
  started before case 3. `scr`: an empty directory outside Git.
- Credentials were fixture values only: `fixture-claude-token`,
  `fixture-pi-key`, `fixture-api-key`, `fixture-oauth-token`. Login routes
  pointed at `fake.py`, a host-run endpoint on `127.0.0.1:18765` that logs
  each request's credential headers and answers 400.

## 1. Each verb runs its agent at its pin (`versions.sh`)

`pinfold artifacts`: pi 1.5.2, claude 2.1.322, codex 0.167.1.

| Command, in `proj` | stdout | Exit |
|---|---|---|
| `pinfold pi --version` | `1.5.2` | 0 |
| `pinfold claude --version` | `2.1.322 (Claude Code)` | 0 |
| `pinfold codex --version` | `codex-cli 0.167.1` | 0 |

`help.sh`: `pinfold claude --help` printed claude's `Usage: claude
[options] [command] [prompt]` and `pinfold codex --help` printed `Codex
CLI`: `--help` reaches the agent. `pinfold --help` lists the three verbs.
The launch record of the `pinfold codex` run had `harnesses`
`[{"name":"codex","version":"0.167.1"}]`, `env`
`["HERDR_AGENT","HOME","PINFOLD_ALLOW"]` and terminal `harness-exited`.

## 2. pi alone gets its login extension and exit summary

Extension (`login.sh`, with `pinfold.toml` as `.pinfold.toml`: a host
route `fake`, a claude login route, a pi `anthropic` login route and a
`codex` login route, every `to` the fake):

- `pinfold pi -p hello --provider anthropic --model sonnet` reached the
  fake: one `POST /v1/messages`, `x-api-key: fixture-pi-key`,
  user-agent `pi/1.5.2`. pi reaches `http://anthropic` only through the
  extension; without it pi dials `api.anthropic.com` (probe 2).
- `pinfold claude -p hello` reached its own route and exited 1 with
  `API Error: 400 fixture`. That claude's argv lacks `-e` was not
  observed from outside; the code gates it on the agent being pi.

Summary (`summary.sh`, in `proj2`): with `untracked.txt` written on the
project disk through `pinfold attach`,

- `pinfold pi --version` printed `pinfold pi: summary {…"kind":"untracked","path":"untracked.txt"…}`
  and the `pinfold fetch` hint on stderr.
- `pinfold claude --version` and `pinfold codex --version` printed nothing
  on stderr.

Scratch (`scratch.sh`, in `scr`):

- `pinfold pi --version`: the scratch line ending `empty work is removed
  on clean exit; retained disks …`, then `pinfold pi: empty scratch disks
  removed`.
- `pinfold claude --version` and `pinfold codex --version`: the same
  scratch line, the version, then `pinfold <agent>: empty scratch disks
  removed`. `pinfold disk list` showed no disk afterwards.

The first build kept claude's and codex's disks, because removal ran only
inside pi's summary. Review found the leak; the lines above are from the
rerun after the fix. A claude or codex scratch run that leaves work on the
disk was not exercised.

## 3. `pinfold claude` on a fresh project gets the start sync (`sync.sh`)

In `proj2`, before any start, `pinfold disk list` showed only `proj`'s
disks. `pinfold claude --version` printed `2.1.322 (Claude Code)`, exit 0.
`pinfold fetch` then printed
`{"box":"pi-proj2-061074cb8df2-65172","event":"fetched","refs":{"refs/remotes/pinfold/trunk":"b73369a…"},"remote":"pinfold"}`;
host `HEAD` and `refs/remotes/pinfold/trunk` were both `b73369a5…`. In
case 4, `pinfold attach` in `proj2`'s claude box listed `.git file.txt`,
`b73369a fresh` and branch `trunk`: the checkout was fast-forwarded.

## 4. A second agent's start is refused (`held.sh`)

The held agent ran interactively on a terminal from `pty_run.py`, with no
input. While it ran, each verb's `--version` in the same project:

| Held | Second start | stderr | Exit |
|---|---|---|---|
| `pinfold pi` in `proj` | pi, claude, codex | `pinfold <verb>: disk-in-use: disk "pi-proj-43720b5b83c5-home" is attached by box "pi-proj-43720b5b83c5-68437"` | 1 |
| `pinfold claude` in `proj2` | pi, claude, codex | `pinfold <verb>: disk-in-use: disk "pi-proj2-061074cb8df2-home" is attached by box "pi-proj2-061074cb8df2-69242"` | 1 |

The held claude drew its first-run theme picker. SIGTERM to each held
launcher ended it, and `box list --label dev.pinfold.owner` printed
nothing after.

## 5. `pinfold attach` gets pi's harness env (`held.sh`)

`pinfold attach env`, filtered to `ANTHROPIC|CLAUDE|PI_|OPENAI|OPENROUTER|HERDR|HOME=`:

- pi's box: `HERDR_AGENT=pi`, `HOME=/home/pi`, `PI_SKIP_VERSION_CHECK=1`,
  `PI_TELEMETRY=0`. The 283f052 lookup is read.
- claude's box: `HERDR_AGENT=claude`, `HOME=/home/pi`, and no harness
  variables: the box lists no pi, and attach adds none.

## 6. Login routes per agent (`login.sh`)

The same configuration named a login route for every agent.

- `pinfold claude -p hello`: the fake got six `POST /v1/messages`, each
  `authorization: Bearer fixture-claude-token`, no `x-api-key`. Launch
  record: `harnesses` claude only; routes `claude` (login) and `fake`.
- `pinfold pi -p …`: as in case 2. Launch record: `harnesses` pi only;
  routes `anthropic` (login) and `fake`.
- Neither start was refused for the other agents' routes, and the
  `codex` route, which asks the host's Codex login, was left out of both.
- No file under the state, config or cache root held any fixture value
  (`grep -rl` found none).

`misroute.sh`, in a second fresh state root `/tmp/pfv.WzmB` (deleted
after), with the binary of the amended commit:

| `.pinfold.toml` route | Command | Result |
|---|---|---|
| `x = { login = "nope", from = … }` | `pinfold claude --version` | `spec: invalid box spec: login route "x" names "nope", but the spec's harnesses are ["claude"]`, exit 1 |
| `x = { login = "pi", provider = "openrouter", from = … }` | `pinfold claude --version` | `2.1.322 (Claude Code)`, exit 0 |
| the same | `pinfold pi --version` | `1.5.2`, exit 0 |

Only another agent's route is left out; a route naming no agent still
refuses the start. Before this change a pi launch refused a claude or
codex route the same way.

## 7. Probes

**Does claude prefer `ANTHROPIC_API_KEY` over its route variables?** Yes.

- With `PINFOLD_ENV_ANTHROPIC_BASE_URL=http://fake`,
  `PINFOLD_ENV_ANTHROPIC_API_KEY=fixture-api-key` and
  `PINFOLD_ENV_CLAUDE_CODE_OAUTH_TOKEN=fixture-oauth-token` through a plain
  host route, claude sent `x-api-key: fixture-api-key` and no
  `Authorization`.
- Through the claude login route with only
  `PINFOLD_ENV_ANTHROPIC_API_KEY=fixture-api-key` added, the fake got both
  `x-api-key: fixture-api-key` (the box's) and `authorization: Bearer
  fixture-claude-token` (the route's). The claude route does not remove a
  box-sent `x-api-key` (`Login::resolve` gives claude an empty strip
  list). Guarantee 26 says a login route's fixture receives none of the
  box's credentials, so this conflicts with it. Not changed here.
- The launch path sets no `ANTHROPIC_API_KEY` for claude: pi's
  placeholder is in pi's execs only. Only a user's
  `PINFOLD_ENV_ANTHROPIC_API_KEY` reaches claude. Nothing was changed.

**Does pi read `ANTHROPIC_BASE_URL`?** No. The pi binary holds the string
only in its bundled Anthropic SDK's defaults. With
`PINFOLD_ENV_ANTHROPIC_BASE_URL=http://fake`, a fixture
`ANTHROPIC_API_KEY` and no pi login route, `pinfold pi -p hello
--provider anthropic --model sonnet` failed with `Proxy refused to open a
tunnel: 403`; the egress log showed four `api.anthropic.com` refusals,
`not allowlisted`, and the fake got nothing. claude's
`ANTHROPIC_BASE_URL` cannot redirect pi. Nothing was changed.

**Does codex take `--sandbox` at top level?** Yes. `pinfold codex --help`
lists `-s, --sandbox <SANDBOX_MODE>`, `-a, --ask-for-approval` and
`--dangerously-bypass-approvals-and-sandbox`; `pinfold codex --sandbox
danger-full-access --version` printed `codex-cli 0.167.1`, exit 0.
Observed: `pinfold codex sandbox linux -- sh -c …` panicked:
`bubblewrap is unavailable: no system bwrap was found on PATH and no
bundled codex-resources/bwrap binary was found`, exit 101. Inferred, not
run: a codex tool call under its default sandbox fails the same way. The
launch path passes arguments through and adds no `--sandbox`. Not
decided here.

## Cleanup

`cleanup.sh`: `pinfold disk rm --all` removed the six disks this root
created; `disk list` and `box list` were then empty. `container list
--all` showed only `buildkit`, untouched. The state root was deleted.
Volumes were not listed separately from `disk list`.

## Not verified

- `pinfold codex` through a `codex` login route: it needs the host's real
  Codex login.
- Linux. The Linux code was linted with
  `cargo clippy --all-targets --target x86_64-unknown-linux-musl`, zig as
  ring's C compiler, but not run.
- An interactive codex session, and any codex tool call.
- The update notice for the new verbs: development builds never check.
