# Public source basis

Sources checked on 2026-11-14. Documentation/source, not extra box runs.
Unknown means no upload capability established by these sources; never read-only.
Host capability does not establish write permission to a vendor's bucket/account.

- P1: [pi 1.5.2 version check](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/utils/version-check.ts):
  GET `pi.dev/api/latest-version`; manual update is separate from background check.
  [Managed/npm updater](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/package-manager-cli.ts):
  managed manifests under `pi.dev/api/installer/releases/`; npm replacement uses original prefix.
  [Environment](https://docs.pi.dev/latest/environment-variables): version-check and telemetry switches differ.
- P2: [Claude network requirements](https://code.claude.com/docs/en/network-config):
  npm uses `registry.npmjs.org`; native installer, updater and checks use `downloads.claude.ai`;
  before 2.1.116 they used `storage.googleapis.com`. Native HTTP methods not specified.
  Homebrew checks use `formulae.brew.sh`; `api.anthropic.com` is API/telemetry traffic.
  [Environment](https://code.claude.com/docs/en/env-vars): nonessential-traffic switch disables auto-updates.
  [Setup](https://code.claude.com/docs/en/setup): native and npm installation layouts differ.
- P3: [Codex 0.169.0 checks](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/tui/src/updates.rs):
  GET GitHub latest release, then npm metadata for npm installs; Homebrew uses formula metadata.
  [Actions](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/tui/src/update_action.rs):
  npm global installation or `curl -fsSL https://chatgpt.com/codex/install.sh | sh` (GET).
  [Official CLI installation](https://learn.chatgpt.com/docs/codex/cli) also gives that installer.
  GitHub release artifact redirects may use `release-assets.githubusercontent.com`; not captured here.
- P4: [npm publish implementation](https://github.com/npm/cli/blob/latest/workspaces/libnpmpublish/lib/publish.js):
  PUT package metadata containing a base64 tarball attachment.
  [npm audit](https://docs.npmjs.com/cli/v11/commands/npm-audit/): POST dependency JSON to registry.
  [Registry](https://docs.npmjs.com/cli/v11/using-npm/registry/): public default and write APIs.
  [Cache](https://docs.npmjs.com/cli/v11/commands/npm-cache/): HTTP/package cache, not a RUN layer.
- P5: [Create a Gist](https://docs.github.com/en/rest/gists/gists#create-a-gist):
  authenticated POST `https://api.github.com/gists` accepts filename/content pairs.
  [HTTPS repositories](https://docs.github.com/en/get-started/git-basics/about-remote-repositories):
  pushes to `https://github.com/user/repo.git` are supported.
- P6: [Cloud Storage uploads](https://docs.cloud.google.com/storage/docs/uploading-objects):
  POST/PUT to `storage.googleapis.com` with bucket permissions; vendor bucket access unknown.
- P7: [ChatGPT web](https://learn.chatgpt.com/docs/web) and
  [file attachments](https://learn.chatgpt.com/docs/use-chatgpt): ChatGPT accepts messages/files.
  Exact upload host/path/method is not established by these user-facing instructions.
- P8: [pi dispatcher](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/core/http-dispatcher.ts)
  sets `proxyTunnel: true`; [Undici ProxyAgent](https://undici.nodejs.org/api/ProxyAgent)
  documents CONNECT even for HTTP with that option.
- P9: [Helper protocol](https://github.com/openai/codex/blob/rust-v0.167.1/codex-rs/app-server-protocol/src/protocol/v1.rs)
  defines `getAuthStatus` fields; [handler](https://github.com/openai/codex/blob/rust-v0.167.1/codex-rs/app-server/src/request_processors/account_processor.rs)
  returns the token. Local `crates/pinfold/src/core/login.rs` checks expiry/account claim,
  injects bearer/account headers, and has no image/helper version handshake.
- P10: [Messages API](https://platform.claude.com/docs/en/api/messages/create):
  POST `https://api.anthropic.com/v1/messages` accepts text/image input messages.
