# Harnesses behind injecting routes, 2026-09-26

The Switchyard builder tested these on pinfold 0.0.5 with the host holding
the credential (GitHub #51). Versions: claude 2.1.283, codex 0.157.0. They are
evidence of setups that worked at those pins, not a contract.

## claude: works

- Route: to `https://api.anthropic.com`, injecting `Authorization: Bearer`
  from the host's token.
- Box env: `ANTHROPIC_BASE_URL=http://<route>` and a placeholder
  `CLAUDE_CODE_OAUTH_TOKEN` (or `ANTHROPIC_AUTH_TOKEN`).
- A placeholder `ANTHROPIC_API_KEY` fails. claude sends that key in
  `x-api-key`, which the route does not set.

## codex: native ChatGPT login fails, a custom provider works

- `chatgpt_base_url` pointing at a route is refused by codex: "workspace
  backend must use an HTTPS origin without credentials". codex also tries a
  WebSocket first, which a route does not carry.
- Works: a custom model provider with
  `base_url = "http://<route>/backend-api/codex"`, `wire_api = "responses"`,
  `requires_openai_auth = false`, and no `auth.json` in the box. The route
  goes to `https://chatgpt.com` and injects `Authorization: Bearer <access
  token>` and `ChatGPT-Account-ID`.
- codex's shell tool needs `sandbox_mode = "danger-full-access"` (GitHub
  #50). With its own sandbox, the first shell call panics and `codex exec`
  still exits 0.
