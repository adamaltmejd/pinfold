# Live subscription logins through login routes, macOS (#298)

2026-11-15. The owner ran these himself, with his real logins, on macOS
with Apple `container`. Binary: a debug build of `76e17b7`, the same
content as main `2b7364a` (#314). Image: `profile-default` with pi 1.5.2,
Claude Code 2.1.311 and Codex 0.169.0, built earlier the same day.

Until now every login route case used synthetic logins and host fixtures.
This is the one real run.

## Setup

Two owner-run scripts, not committed. Each used isolated XDG roots under a
temporary directory, a scratch Git project, one box from a raw box spec
(`pinfold box up`), and agents launched through PATH with
`pinfold box exec`. The host's `HOME` and `CODEX_HOME` were unchanged, so
the host Codex login was found. The Claude token came from
`claude setup-token`, typed at a hidden prompt into an environment
variable; it was never in argv or a file. The scripts printed no raw
diagnostics, so their output could be shared.

Routes in the project's `pinfold.toml`:

```toml
[routes.codex]
login = "codex"
[routes.pi]
login = "pi"
provider = "openai-codex"
[routes.claude]
login = "claude"
from = "CLAUDE_SETUP_TOKEN"
```

Each agent got one prompt asking for exactly one tool call that creates a
named file with given content. PASS means exit 0 and the file exists with
exactly that content.

## Run 1: all three agents in one box

```
PASS claude exit 0
PASS codex exit 0
FAIL pi exit 1
Environment: literal token not found; JWT prefixes not found
Generated files: literal token not found; JWT prefixes found
Home disk: literal token not found; JWT prefixes found
Egress log: literal token not found; JWT prefixes not found
HOST STATUS
claude 200
claude 200
OTHER -
OTHER -
chatgpt.com -
codex 200
codex 200
OTHER -
```

pi exited 1 with no request to its route. Cause: the script ran
`pi -p --provider openai-codex PROMPT`, and pi requires `--model` with
`--provider`. The builder reproduced exit 1 with zero requests against the
synthetic fixture, and the same command with a model passed. A script
error; no product change.

The first script's JWT scan could not tell pinfold's public pi placeholder
from another JWT. Run 2's scan can.

## Run 2: pi alone, `--provider openai-codex --model gpt-6.1-sol`

```
PASS pi exit 0
Generated files: only placeholder
Home disk: only placeholder
HOST STATUS REASON
pi - route
pi 200 route
pi 200 route
```

## What this establishes

- The scripts reported PASS and exit 0 for Claude Code and Codex in run 1
  and for pi in run 2: each agent exited 0 and its requested file existed
  in the box with the expected content. Each route shows HTTP 200 rows.
- Run 1's scans reported no literal Claude token in the inspected
  environment, generated files, home disk or egress log.
- Run 2's scans reported only pinfold's public placeholder as a JWT-shaped
  string in the generated files and on the home disk.

## What it does not establish

- The output does not identify the credentials used or show tool-call
  traces. The boxes held no other login, and the routes were the only
  configured path to the providers.
- The scripts and their scan coverage are not included here. The results
  do not exclude encoded credentials, unsearched locations or exposure
  before the scan ran. Run 2 has no environment or egress-log scan.
- Run 1's home disk held Claude's and Codex's state as well. Its JWT hit
  was not classified, so "only placeholder" is shown for a pi-only home.
- pi and Codex were not run at the same time on the shared host login.
- Linux was not run. pi's `anthropic-subscription` form was not run.
- Token refresh near expiry was not exercised; the runs were short.
- The rows `OTHER -` and `chatgpt.com -` in run 1 carry no reason column
  and were not examined.
