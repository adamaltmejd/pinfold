# Spec cleanup for 0.3.2

Rationale for text cut from ARCHITECTURE.md in the 0.3.2 cleanup. The
spec states the contract only.

## Profiles

- The selected profile's live package chooses its extensions and skills,
  so switching profiles does not rewrite saved settings. That is why a
  built-in seeds only the package path and no extension list.
- Metadata lookup and fingerprints write nothing, and embedded shared
  files are materialized only after a box claims its name. This is the
  general rule that nothing is created before the claim (Lifecycle step 1).

## pi login route

- The extension's `baseUrl` sits above a `models.json` one for the same
  provider, so a stored model catalog does not bypass the route.
- pi's WebSocket attempt for `openai-codex` fails at the route (no
  `Upgrade`), and pi falls back to SSE for the session.
- `ANTHROPIC_OAUTH_TOKEN` is set to a value pi accepts as a subscription
  token. The `openai-codex` placeholder is an unsigned JWT held in the
  extension as that provider's `apiKey`.

## Removed with the durable prototype

`examples/pi-durable` was an opt-in Bun program: one sequential
`boxed_bash` tool through `box exec`, host-owned SQLite checkpoints,
recovery under an exclusive writer lock that removed the previous box and
created a fresh generation, and SIGINT or SIGTERM removing the whole box
(exit 130). Exactly-once execution, per-command cancellation and a
scheduler were out of scope. Its dependencies were
`@earendil-works/pi-durable`, `pi-ai` and `chord`.

## Removed flags

`box push`, `--remote` on `box push` and `box fetch`, `box exec --workdir`
and `profile new --builtin` had no e2e test and no caller. `pinfold push`
and `pinfold fetch` use the same core operations.
