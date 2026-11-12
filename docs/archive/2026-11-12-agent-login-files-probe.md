# Agent login files probe

Date: 2026-11-12. Probe 1 of "Probes before tickets" in
`2026-11-12-project-box-plan.md`, which is on PR #288 and not on main. The
question was whether each agent can take its login route from a file that
only it reads, mounted read-only, with one box-wide environment and no
per-exec env. No product code was changed. No test was added.

Scripts and raw logs are in `2026-11-12-agent-login-files-probe/`:

- `driver.sh` sets the order.
- `cases.sh` holds one case per box.
- `spec.py` writes each box spec.
- `env.sh` holds the helpers.
- `fake.py` is a copy of the file in `2026-11-12-agent-verbs-verification/`.
  That directory is on the closed PR #286 (branch `feat/agent-verbs`), not
  on main.
- `pty_keys.py` and `screen.py` drive the interactive starts. `pty_run.py`
  is in the directory and no script uses it.
- `files/` holds every login file exactly as mounted.
- `logs/00-service-start.log` and `logs/99-service-stop.log` are produced
  by no script in the directory. One line of the stop log, `rm -rf <T>`,
  was edited by hand to remove the path.

Quoted lines below are from `logs/`, with the home path scrubbed to
`/Users/user`. Two kinds of statement have no log line and are marked
"not logged". The claude start-up messages in section 5 are reconstructed
from a screen capture, as that section says.

## Environment

- macOS 27.2.1, build 26B131, arm64. Apple `container` CLI 1.7.1. Only
  Apple `container` on macOS was run.
  - The service was stopped at the start (not logged). It was started with
    `container system start --disable-kernel-install`
    (`logs/00-service-start.log`) and stopped at the end
    (`logs/99-service-stop.log`).
- `cargo build --locked -p pinfold` at `8dab5f8`, said to be origin/main
  (the build command and that claim are not logged). The only logged
  record is `git rev-parse HEAD` of this checkout, `8dab5f88…`, in
  `logs/01-versions-reset.log`.
- Image `localhost/pinfold/profile-default:latest`, already present. It
  was named in each spec as `image`. Nothing was pulled, built or deleted.
- Agents: pi `1.5.2`, claude `2.1.322 (Claude Code)`, codex
  `codex-cli 0.167.1` (`--version` in `logs/11-baseline.log`).
  - They were already in the cache of the temporary root from trial runs.
    `pinfold artifacts` (`logs/01-versions-reset.log`, line 33) printed
    JSON for pi 1.5.2, claude 2.1.322 and codex 0.167.1, each with
    `"cached":true`, its sha256 and its URL. No recorded log shows a
    download.
- Temporary root `T=/tmp/plp.1Kel` held the XDG state, cache and config.
  It was deleted at the end (`rm -rf <T>`, `logs/99-service-stop.log`).
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
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`. Cases 19 and 21 also set
  `ANTHROPIC_API_KEY=pinfold-placeholder` box-wide.
  - Eight of the 14 cases printed `env` from `box exec`: baseline, pi-ext,
    pi-models, claude-managed, claude-key-boxwide, claude-key-blanked,
    codex and all. It held only the variables above, the proxy variables,
    `PATH`, `PINFOLD_ALLOW=` and `SSH_AUTH_SOCK`.
  - The other six did not print it: pi-ext-predir, pi-settings,
    pi-ext-raw, claude-managed-raw, claude-key-boxwide-raw and
    claude-tui-env. For them the spec line in the log is the only evidence
    of the box-wide env.
- Injecting routes with the header names and prefixes of today's login
  routes. Each goes to its own host fake:

  | Route | Fake port | Header the proxy sets |
  |---|---|---|
  | `claude` | 18771 | `Authorization: Bearer $FAKE_CLAUDE_TOKEN` |
  | `anthropic` (pi) | 18772 | `x-api-key: $FAKE_PI_KEY` |
  | `codex` | 18773 | `Authorization: Bearer $FAKE_CODEX_TOKEN`, `ChatGPT-Account-ID: $FAKE_CODEX_ACCOUNT` |
  | `raw` | 18774 | none: a plain host route, so the fake sees the box's own headers |

  Today's pi `anthropic` login route also removes a box-sent
  `Authorization` (ARCHITECTURE.md, pi login route table). The probe's
  `anthropic` route did not.

  No `login` route was configured. A `codex` login route would ask the
  host's real Codex login.
- The login files from `files/`, as directory mounts (pinfold mounts
  only directories, all read-only).

The fixture values were `FAKE-claude-token`, `FAKE-pi-key`,
`FAKE-codex-token` and `FAKE-codex-account`. They were passed by `from`
and never put in argv. The box files hold only `pinfold-placeholder`. The
`FAKE-` values exist in the host environment (`env.sh`, lines 7-8) and in
what the fakes recorded. They appear in no box env print.

The pass signal is a request on the agent's own port that carries the
host's `FAKE-…` value. It shows that the route was reached and set its
header. The fake answers every request 400. So pi exits 1 after one
request, claude exits 1 after six, and codex exits 1 after one. Each agent
ran non-interactively with the prompt `say hi`:

- `pi -p "say hi" --provider anthropic --model sonnet`
- `claude -p "say hi"`
- `codex exec --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox "say hi"`

What the fakes can show is limited in two ways.

- An injecting route replaces any header the box sent under a name it sets
  (ARCHITECTURE.md, injecting routes). So a box-sent `Authorization` is
  invisible at the claude and codex fakes, a box-sent `ChatGPT-Account-ID`
  at the codex fake, and a box-sent `x-api-key` at the pi fake.
- `fake.py` records only `authorization`, `x-api-key`,
  `chatgpt-account-id`, `host` and `user-agent`.

What an agent itself sends was seen only where a raw-route control ran:
pi (case 15) and claude (cases 18 and 20). There was no codex raw-route
control. What codex sends by itself was not observed.

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

The paths come from the installed agents (not logged):

- pi's bundled `docs/configuration.md` lists `<agent-dir>/models.json`
  and `<agent-dir>/extensions/`.
- pi's `docs/settings.md` lists an `extensions` resource array that
  takes absolute paths.
- The claude binary's strings name `/etc/claude-code`,
  `managed-settings.json` and `managed-settings.d`.

The `touch` into each mount printed `Read-only file system` in the seven
cases whose script runs the `common` helper against a mount: pi-ext,
pi-models, claude-managed, claude-key-boxwide, claude-key-blanked, codex
and all (all three mounts, at the start and the end). It was not run in
pi-ext-predir, pi-settings, pi-ext-raw, claude-managed-raw and
claude-key-boxwide-raw. Baseline and claude-tui-env have no mounts.

## Baseline: no login file (`11-baseline.log`)

- pi: `No API key found for anthropic.`, exit 1.
- claude: `Not logged in · Please run /login`, exit 1.
- codex: retried, then `Proxy connection failed: HTTP CONNECT failed with
  status 403`. Exit 124 from `timeout 60`. The egress log shows
  `chatgpt.com`, `api.openai.com`, `github.com` and `api.github.com`
  refused `not allowlisted` (`api.github.com` at line 57).
- No fake saw a request.

## 1. pi without `-e` and without a box-wide `ANTHROPIC_API_KEY`

Observed:

- **Extension mounted at `~/.pi/agent/extensions/` on a fresh home disk
  fails** (`12-pi-ext.log`).
  - pi printed `EACCES: permission denied, mkdir
    '/home/pi/.pi/agent/sessions'`, exit 1. With `--no-session` it hit
    `EACCES` in `acquireTrustLockSync`, exit 1.
  - `ls -lan` showed `/home/pi/.pi` and `/home/pi/.pi/agent` owned by
    `0 0` (lines 44-60).
  - No fake saw a request.
- **The same mount works when `~/.pi/agent` already exists**
  (`13-pi-ext-predir.log`).
  - A first box on the same disks ran `mkdir -p /home/pi/.pi/agent`. A
    second box mounted the extension. `.pi` and `agent` stayed
    `501 20`.
  - The control was also a second boot of the disk, so it differs from
    case 12 in two ways: the directory existed, and the disk had been
    booted before.
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
- **pi's own headers** (`15-pi-ext-raw.log`): the same extension, named by
  `settings.json` as in case 14 and pointed at `http://raw`, sent
  `"x-api-key": "pinfold-placeholder"` and no `authorization`. The spec set
  no `ANTHROPIC_API_KEY`. `env` was not printed in this case.

Inferred:

- The `x-api-key` value in case 15 is the extension's `apiKey`.
- The extension's `apiKey` replaces today's per-exec
  `ANTHROPIC_API_KEY` placeholder, and its auto-loaded location replaces
  `-e`.
- In case 12 the runtime created the nested mount's missing parents as
  root on the user's home disk.
- Every working arrangement needs something user-owned on the home disk
  first: `~/.pi/agent` itself, a `settings.json` entry, or a symlink.
- Mounting into `$HOME` on a fresh disk needs those parents to be created
  as the box user. The fix was not tested. It could belong to pinfold, to
  the Apple `container` runtime's mount preparation, or to both.

## 2. claude's two variables from its managed settings

All arrangements used `/etc/claude-code/managed-settings.json`, mounted
read-only. Nothing claude-specific was in the box-wide env except where
stated.

Observed:

- **Managed settings alone work** (`17-claude-managed.log`). claude
  printed `API Error: 400 fixture`, exit 1. Fake 18771 saw six
  `POST /v1/messages`, each `"authorization": "Bearer FAKE-claude-token"`
  and no `x-api-key`.
- **What claude itself sends** (`18-claude-managed-raw.log`): the raw
  route saw six requests with `"authorization": "Bearer
  pinfold-placeholder"`.
- **With `ANTHROPIC_API_KEY=pinfold-placeholder` present**, in claude's
  process env (`17`, second run) or box-wide (`19-claude-key-boxwide.log`):
  - Fake 18771 saw six requests, each with `"x-api-key":
    "pinfold-placeholder"` beside the route's `authorization`.
  - On the raw route (`18` second run, `20-claude-key-boxwide-raw.log`),
    claude sent `"x-api-key": "pinfold-placeholder"` and no
    `authorization`.
  - The requests reached the routes, so the managed file's
    `ANTHROPIC_BASE_URL` still applied.
- **The managed file can blank a box-wide key** (`21-claude-key-blanked.log`).
  Cases 19 and 21 have the same box-wide `ANTHROPIC_API_KEY` and the same
  route. Only the file differs. With `"ANTHROPIC_API_KEY":""` in the file,
  fake 18771 saw six requests with only `"authorization": "Bearer
  FAKE-claude-token"`. The `x-api-key` header was gone.

Inferred:

- In case 18 claude took both `ANTHROPIC_BASE_URL` and
  `CLAUDE_CODE_OAUTH_TOKEN` from the file. The box env prints in the
  other managed-file cases hold neither variable. Case 18 itself did not
  print `env`.
- claude prefers `ANTHROPIC_API_KEY` over the OAuth token when both are
  present.
- In case 21 the file's env value overrode the box-wide variable.
- A managed-settings `env` block scopes both variables to claude and needs
  no wrapper. Adding `"ANTHROPIC_API_KEY": ""` to it stopped a box-wide
  `ANTHROPIC_API_KEY` from reaching the claude route in case 21.

Not tried:

- The `managed-settings.d/` drop-in directory.
- Whether claude passes the file's env on to its tool subprocesses. The
  fake's 400 means claude never runs a tool.
- `ANTHROPIC_API_KEY` in claude's own process env (the `CLK` command)
  against the blanked file.
- A raw-route run with the blanked file, to see what claude sends then.
- The all-three box with the blanked file. Case 23 mounted
  `claude-managed`, not `claude-managed-key` (`cases.sh`, lines 136-137).
  The combination the Summary recommends was not run.

## 3. codex with `/etc/codex/config.toml` alone (`22-codex.log`)

Observed: codex printed `provider: pinfold` and `model: gpt-6.1-sol`,
then the fixture error twice, exit 1. Fake 18773 saw one
`POST /backend-api/codex/responses` with `"authorization": "Bearer
FAKE-codex-token"` and `"chatgpt-account-id": "FAKE-codex-account"`. No
per-exec env was set.

The egress log shows codex also dialled `chatgpt.com` (twice),
`github.com` and `api.github.com`, all refused `not allowlisted`
(`22-codex.log`, lines 51-54). The all-three run shows `chatgpt.com` and
`github.com` refused again for each of its two codex runs
(`23-all.log`, lines 132-137).

Not tried:

- The host-helper half of a real `codex` login route. It reads the host's
  real Codex login.
- A codex raw-route control, to see the headers codex sends itself.

## 4. All three in one box (`23-all.log`)

The box had three mounts: the extension at `~/.pi/agent/extensions/`
(with `~/.pi/agent` created by a first box on the same disks), the
managed settings at `/etc/claude-code`, and the codex config at
`/etc/codex`. The managed settings were the non-blanked file. The env was
the same box-wide env as above.

Observed:

| Run | Fake that saw it | Credential headers |
|---|---|---|
| pi | 18772, 1 request | `x-api-key: FAKE-pi-key` |
| claude | 18771, 6 requests | `authorization: Bearer FAKE-claude-token` |
| codex | 18773, 1 request | `authorization: Bearer FAKE-codex-token`, `chatgpt-account-id: FAKE-codex-account` |

- No request reached another agent's port. The fakes' lines were printed
  after every run (`show_fakes`), and each run's lines were on its own
  agent's port only.
- No request carried another agent's header, within two limits. Only the
  three header names `fake.py` records were checked. And the check is
  masked where the route sets the name: `Authorization` at the claude and
  codex fakes, `ChatGPT-Account-ID` at the codex fake, `x-api-key` at the
  pi fake. Unmasked examples: claude sent no `x-api-key` and pi sent no
  `authorization`.
- pi and codex were also run with `ANTHROPIC_BASE_URL=http://claude
  CLAUDE_CODE_OAUTH_TOKEN=pinfold-placeholder` in their env, as a child
  of claude might inherit (`cases.sh`, lines 142-145). Each still reached
  only its own fake, with the same headers as without them.
  - For codex this cannot show whether codex sent the placeholder as a
    bearer. The route replaces `Authorization`, and there was no codex
    raw-route control.
  - The simulation did not include `ANTHROPIC_API_KEY`, blank or not.
- Other commands in that box:
  - `pi auth check --provider anthropic` printed `not_ready`, though
    the extension's key works.
  - `codex login status` printed `Not logged in`.
  - The `[exit 0]` logged for both is `head`'s, not the agent's
    (`cases.sh`, lines 146-147 pipe through `head`).

Inferred: in these runs nothing collided once pi's key lives in pi's
extension. The collision seen is a box-wide `ANTHROPIC_API_KEY` reaching
claude (section 2). Case 21 shows the file's blank entry stopped it in
one box. The blank entry was not run together with the other two files.

## 5. Writes to the read-only files

Observed:

- A `find` listing of `/home/pi` and `/work`, sorted and cut at 60 lines,
  was printed in pi-ext, pi-ext-predir, pi-settings, pi-models,
  claude-managed, codex and, twice, all. In those listings:
  - pi wrote `~/.pi/agent/{auth.json,models-store.json,sessions/}`
    (cases 13, 14 and 16).
  - claude wrote `~/.claude.json` and `~/.claude/` (cases 17 and 23).
  - codex wrote `~/.codex/` (cases 22 and 23).
  - The listings of cases 22 and 23 are cut at 60 lines. Neither case 23
    listing has a `~/.pi` entry.
  - Nothing outside `/home/pi` and `/work` was listed, `/tmp` included.
  - None of the agents printed an error about its login file.
- At the end of case 23 `common` ran again. The mounted files' contents,
  sizes and times were the same as at the start (`23-all.log`, lines
  28-52 and 374-398). The mount-top directories changed owner: `root root`
  at lines 28, 36 and 42, `501 dialout` at lines 374, 382 and 388. In
  case 12 the extensions directory is `root root` at line 19 and `501 20`
  at line 60 of the same box. In case 13 it is `0 0` (line 29). The logs
  do not say why. So "unchanged" holds for content only.
- pi with the extension mount on a fresh disk failed on its own state
  directories, not on the login file (section 1).
- pi interactive (`23`, lines 200-239): the screen listed `[Extensions]`
  and `pinfold-login.ts`. `/login` opened `Select authentication
  method:` with `Sign in with an account`, `Sign in with an API key` and
  `Sign in with Radius`. The keys after `/login` were Escape, Ctrl-C and
  Ctrl-C. Then `pty_keys.py` sent SIGTERM (line 39) and exited with the
  wait status. The logged exit 241 is -15 mod 256, so the probe's SIGTERM
  ended the `box exec` client. Whether Escape or Ctrl-C ended pi is not
  known. No method was picked.
- claude interactive: both with the managed file (`23`) and with today's
  variables in its process env and no file (`24-claude-tui-env.log`), the
  first start stopped on a connection error naming Anthropic services and
  `api.anthropic.com` with `ERR_PROXY_TUNNEL`.
  - The quoted messages are reconstructed. `screen.py` drops repeated
    lines and the capture has one word per line. The reconstructed
    messages are "Unable to connect to Anthropic services" and "Failed to
    connect to api.anthropic.com: ERR_PROXY_TUNNEL". They do not appear as
    log lines. They come from the lines `Unable`, `connect`, `Anthropic`,
    `services` and `Failed`, `api.anthropic.com:`, `ERR_PROXY_TUNNEL`
    (`23-all.log`, lines 248-254; `24-claude-tui-env.log`, lines 14-20).
  - The egress refusals of `api.anthropic.com` and `platform.claude.com`,
    `not allowlisted`, are only in `24-claude-tui-env.log`, lines 58-59.
    Case 23 printed its egress log before the interactive runs
    (`cases.sh`, lines 148-152), so it has no egress record of claude's
    interactive start.
  - Inference: `/login` was never reached in either box, and the managed
    file did not cause the failure, because the box without the file
    failed the same way.
- codex interactive: without `--no-daemon` it printed `Error: this CLI
  has no complete local package; install a packaged Codex CLI or use the
  standalone installer`, exit 1. With `--no-daemon` the capture held
  only `Session ID: …`, exit 0. Whether `/login` ran is unknown.
  - `~/.codex/config.toml` is in the second listing of case 23 (line 311)
    and not in the first. Its content was not read. Inference: codex
    wrote it, so it writes its user layer and not `/etc/codex`.

Inferred, from the listings above and not exhaustive: none of the three
keeps its login file anywhere it writes. The mounts could not change
because they are read-only. pi's `/login` writes `auth.json`, and a stored
credential wins over the extension's `apiKey` (pi's docs, not logged).
`/login` was not completed in any agent, so this is unverified. claude's
writes went to `~/.claude*` and codex's to `~/.codex/`.

Not tried:

- Completing any `/login`.
- A settings change in any agent. `claude config set theme dark` is no
  subcommand in 2.1.322: it ran as a prompt, so the run was dropped (a
  trial run, not logged).
- Agent self-update (probe 5).
- Writes outside `/home/pi` and `/work`, such as `/tmp`.
- pi's writes in the all-three box. Its listings had no `~/.pi` entry.

## 6. Wrapper on `PATH`

Not tried. Section 2 infers that claude's managed settings scope its
variables, so a wrapper would not be needed.

## Summary

Only Apple `container` on macOS was tested.

| Agent | File that carries its login | Mounted where | Works alone | With the other two |
|---|---|---|---|---|
| pi | `pinfold-login.ts` (extension: `baseUrl` and placeholder `apiKey`) | `~/.pi/agent/extensions/`, read-only | Yes, if `~/.pi/agent` already exists as the user; on a fresh disk pi fails `EACCES` | Yes (same precondition) |
| pi, alternative | the same extension, named by `~/.pi/agent/settings.json`, or `models.json` by symlink | any read-only dir (`/opt/plp`), plus a file on the home disk | Yes | Not run |
| claude | `managed-settings.json` `env`: `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_OAUTH_TOKEN` (and `ANTHROPIC_API_KEY: ""`, which stopped a box-wide key in case 21) | `/etc/claude-code/`, read-only | Yes; a box-wide `ANTHROPIC_API_KEY` is sent as `x-api-key` unless the file blanks it | Yes with the non-blanked file (case 23). The blanked file with the other two was not run |
| codex | `config.toml` (custom provider at the route) | `/etc/codex/`, read-only | Yes | Yes |
