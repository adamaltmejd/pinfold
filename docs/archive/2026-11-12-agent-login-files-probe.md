# Agent login files probe

Date: 2026-11-12. Probe 1 of "Probes before tickets" in
`2026-11-12-project-box-plan.md`. The question was whether each agent can
take its login route from a file that only it reads, mounted read-only,
with one box-wide environment and no per-exec env. No product code was
changed. No test was added.

Scripts and raw logs are in `2026-11-12-agent-login-files-probe/`:

- `driver.sh` sets the order.
- `cases.sh` holds one case per box.
- `spec.py` writes each box spec.
- `env.sh` holds the helpers.
- `fake.py` is copied from `2026-11-12-agent-verbs-verification/`.
- `pty_keys.py` and `screen.py` drive the interactive starts.
- `files/` holds every login file exactly as mounted.

Every quoted line below is from `logs/`. The home path is scrubbed to
`/Users/user`.

## Environment

- macOS 27.2.1, build 26B131, arm64. Apple `container` CLI 1.7.1.
  - The service was stopped at the start. It was started with
    `container system start --disable-kernel-install`
    (`logs/00-service-start.log`) and stopped at the end
    (`logs/99-service-stop.log`).
- `cargo build --locked -p pinfold` at `8dab5f8` (origin/main).
- Image `localhost/pinfold/profile-default:latest`, already present. It
  was named in each spec as `image`. Nothing was pulled, built or deleted.
- Agents: pi `1.5.2`, claude `2.1.322 (Claude Code)`, codex
  `codex-cli 0.167.1` (`--version` in `logs/11-baseline.log`).
  - They were downloaded at their pins into a fresh temporary cache by
    pinfold's own harness path on the first `box up`.
  - `pinfold artifacts` in `01-versions-reset.log` printed nothing, so the
    versions come from `--version`.
- Temporary root `T=/tmp/plp.1Kel` held the XDG state, cache and config.
  It was deleted at the end.
- Trial runs came before the recorded run. `driver.sh` started the
  recorded run with `pinfold disk rm --all` and removed the run records.
  It ended with `disk rm --all`, `disk list`, `box list` and
  `container list --all` (`logs/99-cleanup.log`). Only `buildkit` was
  left, stopped and untouched.

## Method

Each case is one `pinfold box up` with a spec from `spec.py`. Every box
had the following:

- `harnesses: ["pi","claude","codex"]`, only so the pinned binaries are
  mounted at `/opt/pinfold/<name>/`. No exec passed `--harness`, so no
  per-exec env was set.
- Fresh disks at `/home/pi` (`HOME`) and `/work`.
- One box-wide `env`: `HOME`, plus the harnesses' pinned defaults
  `PI_TELEMETRY=0`, `PI_SKIP_VERSION_CHECK=1` and
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`.
  - Each case printed `env` from `box exec`. It held only those, the
    proxy variables, `PATH`, `PINFOLD_ALLOW=` and `SSH_AUTH_SOCK`.
- Injecting routes with today's login header sets. Each goes to its own
  host fake:

  | Route | Fake port | Header the proxy sets |
  |---|---|---|
  | `claude` | 18771 | `Authorization: Bearer $FAKE_CLAUDE_TOKEN` |
  | `anthropic` (pi) | 18772 | `x-api-key: $FAKE_PI_KEY` |
  | `codex` | 18773 | `Authorization: Bearer $FAKE_CODEX_TOKEN`, `ChatGPT-Account-ID: $FAKE_CODEX_ACCOUNT` |
  | `raw` | 18774 | none: a plain host route, so the fake sees the box's own headers |

  No `login` route was configured. A `codex` login route would ask the
  host's real Codex login.
- The login files from `files/`, as directory mounts (pinfold mounts
  only directories, all read-only).

The fixture values were `FAKE-claude-token`, `FAKE-pi-key`,
`FAKE-codex-token` and `FAKE-codex-account`. They were passed by `from`
and never put in argv. The box files hold only `pinfold-placeholder`.

The pass signal is a request on the agent's own port that carries the
host's `FAKE-…` value. The fake answers every request 400. So pi exits 1
after one request, claude exits 1 after six, and codex exits 1 after
one. Each agent ran non-interactively with the prompt `say hi`:

- `pi -p "say hi" --provider anthropic --model sonnet`
- `claude -p "say hi"`
- `codex exec --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox "say hi"`

## Files

```
files/pi-ext/pinfold-login.ts          (pi-ext-raw: baseUrl "http://raw")
export default function (pi) {
  pi.registerProvider("anthropic", {"baseUrl":"http://anthropic","apiKey":"pinfold-placeholder"});
}

files/pi-models/models.json
{"providers":{"anthropic":{"baseUrl":"http://anthropic","apiKey":"pinfold-placeholder"}}}

files/claude-managed/managed-settings.json   (claude-managed-raw: "http://raw")
{"env":{"ANTHROPIC_BASE_URL":"http://claude","CLAUDE_CODE_OAUTH_TOKEN":"pinfold-placeholder"}}

files/claude-managed-key/managed-settings.json
{"env":{"ANTHROPIC_BASE_URL":"http://claude","CLAUDE_CODE_OAUTH_TOKEN":"pinfold-placeholder","ANTHROPIC_API_KEY":""}}

files/codex/config.toml                 (pinfold's codex_config("codex") text)
model_provider = "pinfold"

[model_providers.pinfold]
name = "pinfold"
base_url = "http://codex/backend-api/codex"
wire_api = "responses"
requires_openai_auth = false
```

The paths come from the installed agents:

- pi's bundled `docs/configuration.md` lists `<agent-dir>/models.json`
  and `<agent-dir>/extensions/`.
- pi's `docs/settings.md` lists an `extensions` resource array that
  takes absolute paths.
- The claude binary's strings name `/etc/claude-code`,
  `managed-settings.json` and `managed-settings.d`.

In every case, `touch` into each mount printed `Read-only file system`.

## Baseline: no login file (`11-baseline.log`)

- pi: `No API key found for anthropic.`, exit 1.
- claude: `Not logged in · Please run /login`, exit 1.
- codex: retried, then `Proxy connection failed: HTTP CONNECT failed with
  status 403`. Exit 124 from `timeout 60`. The egress log shows
  `chatgpt.com`, `api.openai.com` and `github.com` refused
  `not allowlisted`.
- No fake saw a request.

## 1. pi without `-e` and without a box-wide `ANTHROPIC_API_KEY`

Observed:

- **Extension mounted at `~/.pi/agent/extensions/` on a fresh home disk
  fails** (`12-pi-ext.log`).
  - pi printed `EACCES: permission denied, mkdir
    '/home/pi/.pi/agent/sessions'`, exit 1. With `--no-session` it hit
    `EACCES` in `acquireTrustLockSync`, exit 1.
  - `ls -lan` showed `/home/pi/.pi` and `/home/pi/.pi/agent` owned by
    `0 0`. The runtime created the nested mount's missing parents as
    root on the user's home disk.
  - No fake saw a request.
- **The same mount works when `~/.pi/agent` already exists**
  (`13-pi-ext-predir.log`).
  - A first box on the same disks ran `mkdir -p /home/pi/.pi/agent`. A
    second box mounted the extension. `.pi` and `agent` stayed
    `501 20`.
  - pi exited 1 on the fixture 400. Fake 18772 saw one
    `POST /v1/messages` with `"x-api-key": "FAKE-pi-key"`.
- **`settings.json` naming an extension in a read-only mount works**
  (`14-pi-settings.log`).
  - The setup was `~/.pi/agent/settings.json` =
    `{"extensions":["/opt/plp/pinfold-login.ts"]}` on the home disk, with
    `files/pi-ext` mounted at `/opt/plp`.
  - Fake 18772 saw `"x-api-key": "FAKE-pi-key"`. pi left
    `settings.json` unchanged.
- **`models.json` symlinked to a read-only mount works**
  (`16-pi-models.log`).
  - The setup was `~/.pi/agent/models.json ->
    /opt/plp/models.json`, a symlink on the home disk.
  - Fake 18772 saw `"x-api-key": "FAKE-pi-key"`. The symlink was still
    there after the run.
- **pi's own headers** (`15-pi-ext-raw.log`): the same extension pointed
  at `http://raw` sent `"x-api-key": "pinfold-placeholder"` and no
  `authorization`. The key is the extension's `apiKey`. No
  `ANTHROPIC_API_KEY` was set anywhere.

Inferred:

- The extension's `apiKey` replaces today's per-exec
  `ANTHROPIC_API_KEY` placeholder, and its auto-loaded location replaces
  `-e`.
- Every working arrangement needs something user-owned on the home disk
  first: `~/.pi/agent` itself, a `settings.json` entry, or a symlink.
  Mounting into `$HOME` on a fresh disk also needs pinfold to create the
  mount point's parents as the box user. That is a change in the Apple
  runtime's mount preparation, not in pi.

## 2. claude's two variables from its managed settings

All arrangements used `/etc/claude-code/managed-settings.json`, mounted
read-only. Nothing claude-specific was in the box-wide env.

Observed:

- **Managed settings alone work** (`17-claude-managed.log`). claude
  printed `API Error: 400 fixture`, exit 1. Fake 18771 saw six
  `POST /v1/messages`, each `"authorization": "Bearer FAKE-claude-token"`
  and no `x-api-key`.
- **What claude itself sends** (`18-claude-managed-raw.log`): the raw
  route saw six requests with `"authorization": "Bearer
  pinfold-placeholder"`. So both `ANTHROPIC_BASE_URL` and
  `CLAUDE_CODE_OAUTH_TOKEN` were taken from the file.
- **With `ANTHROPIC_API_KEY=pinfold-placeholder` present**, in claude's
  process env (`17`, second run) or box-wide (`19-claude-key-boxwide.log`):
  - Fake 18771 saw six requests, each with `"x-api-key":
    "pinfold-placeholder"` beside the route's `authorization`.
  - On the raw route (`18` second run, `20-claude-key-boxwide-raw.log`),
    claude sent `"x-api-key": "pinfold-placeholder"` and no
    `authorization`.
  - So claude prefers the key, as known. The managed file's
    `ANTHROPIC_BASE_URL` still applied, because the requests reached
    the routes.
- **The managed file can blank it** (`21-claude-key-blanked.log`). With
  `"ANTHROPIC_API_KEY":""` in the file and the placeholder box-wide, fake
  18771 saw six requests with only `"authorization": "Bearer
  FAKE-claude-token"`. The file's env overrode the inherited variable.

Inferred: a managed-settings `env` block scopes both variables to claude
and needs no wrapper. Adding `"ANTHROPIC_API_KEY": ""` to it also makes
claude immune to a box-wide or inherited `ANTHROPIC_API_KEY`.

Not tried:

- The `managed-settings.d/` drop-in directory.
- Whether claude passes the file's env on to its tool subprocesses. The
  fake's 400 means claude never runs a tool.

## 3. codex with `/etc/codex/config.toml` alone (`22-codex.log`)

Observed: codex printed `provider: pinfold` and `model: gpt-6.1-sol`,
then the fixture error twice, exit 1. Fake 18773 saw one
`POST /backend-api/codex/responses` with `"authorization": "Bearer
FAKE-codex-token"` and `"chatgpt-account-id": "FAKE-codex-account"`. No
per-exec env was set.

Not tried: the host-helper half of a real `codex` login route. It reads
the host's real Codex login.

## 4. All three in one box (`23-all.log`)

The box had three mounts: the extension at `~/.pi/agent/extensions/`
(with `~/.pi/agent` created by a first box on the same disks), the
managed settings at `/etc/claude-code`, and the codex config at
`/etc/codex`. The env was the same box-wide env as above.

Observed:

| Run | Fake that saw it | Credential headers |
|---|---|---|
| pi | 18772, 1 request | `x-api-key: FAKE-pi-key` |
| claude | 18771, 6 requests | `authorization: Bearer FAKE-claude-token` |
| codex | 18773, 1 request | `authorization: Bearer FAKE-codex-token`, `chatgpt-account-id: FAKE-codex-account` |

- No request reached another agent's port.
- No request carried a header that another agent's file sets. For
  example, claude sent no `x-api-key` and pi sent no `authorization`.
- pi and codex were also run with `ANTHROPIC_BASE_URL=http://claude
  CLAUDE_CODE_OAUTH_TOKEN=pinfold-placeholder` in their env, as a child
  of claude might inherit. Each still reached only its own fake with the
  same headers.
- Other commands in that box:
  - `pi auth check --provider anthropic` printed `not_ready`, though
    the extension's key works.
  - `codex login status` printed `Not logged in`.

Inferred: nothing collides once pi's key lives in pi's extension. The
collision that remains is a box-wide `ANTHROPIC_API_KEY` reaching claude
(case 2), and the managed file's blank entry closes it.

## 5. Writes to the read-only files

Observed:

- Each agent's `-p` or `exec` runs wrote only under its own home
  directory: `~/.pi/agent/{auth.json,models-store.json,sessions/}`,
  `~/.claude.json` and `~/.claude/`, and `~/.codex/`. None printed an
  error about its login file.
- The mounts' files were unchanged after the runs (`common` at the end
  of `23-all.log`). They could not change, because the mounts are
  read-only.
- pi with the extension mount on a fresh disk failed on its own state
  directories, not on the login file (question 1).
- pi interactive (`23`): the screen listed `[Extensions]` and
  `pinfold-login.ts`. `/login` opened `Select authentication method:`
  with `Sign in with an account`, `Sign in with an API key` and
  `Sign in with Radius`. Escape and Ctrl-C ended it, exit 241. No
  method was picked.
- claude interactive: both with the managed file (`23`) and with
  today's variables in its process env and no file
  (`24-claude-tui-env.log`), the first start stopped at
  `Unable to connect to Anthropic services`, `Failed to connect to
  api.anthropic.com: ERR_PROXY_TUNNEL`. The egress log shows
  `api.anthropic.com` and `platform.claude.com` refused
  `not allowlisted`. So `/login` was never reached, and the managed
  file did not cause the failure.
- codex interactive: without `--no-daemon` it printed `Error: this CLI
  has no complete local package; install a packaged Codex CLI or use the
  standalone installer`, exit 1. With `--no-daemon` the capture held
  only `Session ID: …`, exit 0. Whether `/login` ran is unknown.
  - After these starts `~/.codex/config.toml` existed on the home disk:
    codex writes its user layer, not `/etc/codex`. Its content was not
    read.

Inferred: none of the three keeps its login file anywhere it writes.
pi's `/login` writes `auth.json`, and a stored credential wins over the
extension's `apiKey` (pi's docs). claude's writes go to `~/.claude*`.
codex's go to `~/.codex/`.

Not tried:

- Completing any `/login`.
- A settings change in any agent. `claude config set theme dark` is no
  subcommand in 2.1.322: it ran as a prompt, so the run was dropped.
- Agent self-update (probe 5).

## 6. Wrapper on `PATH`

Not needed, not tried: claude's managed settings scope its variables
(question 2).

## Summary

| Agent | File that carries its login | Mounted where | Works alone | With the other two |
|---|---|---|---|---|
| pi | `pinfold-login.ts` (extension: `baseUrl` and placeholder `apiKey`) | `~/.pi/agent/extensions/`, read-only | Yes, if `~/.pi/agent` already exists as the user; on a fresh disk pi fails `EACCES` | Yes (same precondition) |
| pi, alternative | the same extension, named by `~/.pi/agent/settings.json`, or `models.json` by symlink | any read-only dir (`/opt/plp`), plus a file on the home disk | Yes | Not run |
| claude | `managed-settings.json` `env`: `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_OAUTH_TOKEN` (and `ANTHROPIC_API_KEY: ""` to block a stray key) | `/etc/claude-code/`, read-only | Yes; a box-wide `ANTHROPIC_API_KEY` is sent as `x-api-key` unless the file blanks it | Yes |
| codex | `config.toml` (custom provider at the route) | `/etc/codex/`, read-only | Yes | Yes |
