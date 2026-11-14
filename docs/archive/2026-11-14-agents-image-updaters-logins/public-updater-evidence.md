# Public updater evidence

Read on 2026-11-14. No credential or login file was read. No upload was
attempted. This is documentation and public client-source evidence, not a
runtime measurement. The probe image resolved Pi 1.5.2, Claude Code 2.1.325 and Codex 0.169.0.
Pi source links match its image version. Codex source links below match
0.169.0. Claude's updater implementation is closed; its evidence is public
documentation plus the separate binary observations.

## Existing pinfold defaults

`crates/pinfold/harnesses.toml` sets pi's `PI_TELEMETRY=0` and
`PI_SKIP_VERSION_CHECK=1`, Claude's
`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, and no Codex defaults.
`core/box.rs::resolve_harnesses` records these per harness; they enter an
exec through `--harness`, not every image-only box automatically.

Pi's environment reference says `PI_SKIP_VERSION_CHECK` disables the
latest-version request, whereas telemetry is controlled separately.
[Pi environment variables](https://docs.pi.dev/latest/environment-variables).

Claude's nonessential-traffic variable disables background auto-updates.
It is any-nonempty-value, so even `0` disables them. Unset it to restore
them. This does not establish whether manual `claude update` is disabled.
[Claude environment variables](https://code.claude.com/docs/en/env-vars).

## Pi

Public README recommends `curl -fsSL https://pi.dev/install.sh | sh`,
which manages a pinned dependency tree, or the alternative
`npm install -g --ignore-scripts @earendil-works/pi-coding-agent`.
Node 22.19 or newer is required.
[Pi README](https://github.com/earendil-works/pi).

`pi update` updates pi; `--extensions` updates packages; `--force` reinstalls
an npm installation even when current. Its managed installation path
rejects `--force`. It can use an explicit `PI_SELF_UPDATE_COMMAND`.
[Pi CLI](https://docs.pi.dev/latest/cli).

At 1.5.2 the update checker GETs `https://pi.dev/api/latest-version`.
`PI_SKIP_VERSION_CHECK` guards `checkForNewPiVersion`, but the manual
updater calls `getLatestPiRelease` separately. Thus disabling the
background check does not disable an explicit update.
[version-check.ts](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/utils/version-check.ts).

For npm installs, pi infers the original prefix. It requires write access
to its package directory and parent before choosing a self-update
command. The command is `npm --prefix PREFIX install -g --ignore-scripts
--min-release-age=0 PACKAGE@VERSION`. A root-owned image install therefore
fails the writability check on a read-only root; it does not silently
switch to the box user's home prefix. Managed installations instead write
a lock, `staging/update-*`, `releases/VERSION`, and atomic `current-version`
under their managed root. They GET package and lock manifests under
`https://pi.dev/api/installer/releases/VERSION/` then run `npm ci` with
scripts, audit and funding disabled.
[config.ts](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/config.ts),
[package-manager-cli.ts](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/package-manager-cli.ts).

## Claude Code

Native installations check at startup and periodically. They install
version files under `/home/user/.local/share/claude/versions/`, with
`/home/user/.local/bin/claude` a launcher symlink. If a custom launcher
exists, the updater preserves it, so a successful home download need not
change which binary runs. npm global installs need their global install
directory writable. Manual update is `claude update`. Native release
manifests carry checksums and signatures.
[Claude setup](https://code.claude.com/docs/en/setup).

Current native installers, version checks and auto-updaters use
`downloads.claude.ai`. `storage.googleapis.com` was used before 2.1.116.
npm/bun installations use `registry.npmjs.org`.
[Claude network requirements](https://code.claude.com/docs/en/network-config).

## Codex

Official docs currently show the standalone installer for initial install
and updates: `curl -fsSL https://chatgpt.com/codex/install.sh | sh`.
[Official OpenAI documentation](https://learn.chatgpt.com/docs/codex/cli).

At 0.169.0 the npm TUI update check GETs
`https://api.github.com/repos/openai/codex/releases/latest`, then npm
package metadata to confirm that version is published. The npm update
action runs `npm install -g @openai/codex`. Standalone installs use the
same GitHub release GET and rerun the standalone installer. The update
cache writes `version.json` under Codex's configured home. There is a
`check_for_update_on_startup` configuration switch; pinfold sets no Codex
updater env default.
[updates.rs](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/tui/src/updates.rs),
[update_action.rs](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/tui/src/update_action.rs).

## Caches and unchanged builds

npm caches HTTP/package data by content under `_cacache` in its configured
cache, default `/home/user/.npm`. This is a download cache; it is distinct
from an image builder's cached RUN layer. A cached RUN does not invoke
npm or re-resolve latest. npm documents integrity checks on insertion
and extraction. A cache is not a permanent artifact store.
[npm cache](https://docs.npmjs.com/cli/v11/commands/npm-cache/).

No native-installer cache was verified in this research. Attempts to read
the pi install script through the web tool encountered unsupported MIME;
Claude's installer fetch was restricted. Do not infer native cache use
from npm's cache or from image layer reuse.

## Hosts and upload capability

These are candidates from docs/source. Use runtime egress logs to separate
actually contacted hosts from paths not reached. No absence claim follows
from a GET-only updater.

| Host | Client use | Public upload/write evidence |
|---|---|---|
| `pi.dev` | Pi latest-version and managed-install manifests, GET | Unknown from sources checked. Docs mention anonymous install/update reporting, but do not establish its endpoint or method. Do not call this host read-only. |
| `registry.npmjs.org` | npm metadata and package downloads, GET | Yes. Registry has publishing and account-write APIs; npm publish sends package metadata and attached tarball in PUT. npm audit POSTs dependency JSON. |
| `downloads.claude.ai` | Native Claude manifests, version checks and binary downloads | Unknown. Public Claude network docs establish distribution use, not an upload API or its absence. |
| `api.github.com` | Codex latest-release GET | Yes. Gist API accepts authenticated POST containing arbitrary file contents. |
| `chatgpt.com` | Standalone Codex installer GET | It is also a service receiving prompts; the installer-only GET does not make the host download-only. Not needed for the npm install action itself. |
| `github.com` | Release download redirect if standalone release artifacts use it | GitHub supports HTTPS Git pushes. Not an npm-action requirement established by this source. |
| `release-assets.githubusercontent.com` | Possible GitHub binary redirect | Unknown; download usage does not establish writable service. Observe actual redirects before listing it as contacted. |
| `storage.googleapis.com` | Historical Claude updater before 2.1.116 | General Cloud Storage API supports uploads, but that does not establish access to Anthropic's distribution bucket. Current Claude docs do not require it for current native updates. |

Primary upload evidence:
[npm Registry](https://docs.npmjs.com/cli/v11/using-npm/registry/) explicitly
lists write APIs and the public registry default;
[npm publish client](https://github.com/npm/cli/blob/latest/workspaces/libnpmpublish/lib/publish.js)
uses PUT with a base64 tarball attachment;
[npm audit](https://docs.npmjs.com/cli/v11/commands/npm-audit/) documents
POST `/-/npm/v1/security/advisories/bulk`;
[GitHub Gists API](https://docs.github.com/en/rest/gists/gists#create-a-gist)
documents authenticated POST with filename/content pairs.
[Codex update-cache source](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/tui/src/updates_cache.rs)
confirms `config.codex_home/version.json`.
[GitHub remote repositories](https://docs.github.com/en/get-started/git-basics/about-remote-repositories)
documents pushes to `https://github.com/user/repo.git`.
[Cloud Storage uploads](https://docs.cloud.google.com/storage/docs/uploading-objects)
documents uploads and required bucket permissions.

## Version follow-up

The public `update_action.rs`, `updates.rs`, and `updates_cache.rs` files
at Codex 0.169.0 are byte-for-byte identical to their 0.167.1 counterparts.
The command, hosts, update-check switch and version-cache conclusions
therefore apply unchanged to the image's 0.169.0 version. This comparison
used the tagged upstream files, not the Mac's installed Codex.

`nodejs.org` and `deb.debian.org` are build-only candidates for the recipe's
Node runtime and Debian package steps. Nothing in the updater client paths
read here requires either host. Confirm actual build contact in the build
evidence; do not add them to an agent updater allowlist on this basis.
