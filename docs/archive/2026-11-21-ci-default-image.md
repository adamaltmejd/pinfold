# Default image without api.github.com (#385), 2026-11-21

## The failure

The CI run for da25738 (run 44862496801, attempt 1) failed 12 of 15
tests. Each panicked in `default_image` with `curl: (22) The requested
URL returned error: 403` from the `agent-resolve` stage. The rerun
(attempt 2) passed, with the suite finishing in 190.88 s.

The panic leaves `default_image`'s `OnceLock` unset, so every later test
rebuilt the image and asked the API again. One rate-limit 403 cost more
than a dozen extra API requests from the same address.

## What the resolver fetched

Before (main at 087b7c0), per build:

| Request | Host | Used for |
|---|---|---|
| `repos/earendil-works/pi/releases/latest` | api.github.com | pi tag, asset URL, asset `digest` |
| `claude-code-releases/stable`, `<version>/manifest.json` | downloads.claude.ai | Claude version, checksum |
| `repos/openai/codex/releases/latest` | api.github.com | codex tag, asset URL, asset `digest` |

These were unauthenticated, so GitHub allows 60 per hour per address.
A shared runner address can use that up before the build starts.

After, per build:

| Request | Host | Used for |
|---|---|---|
| `earendil-works/pi/releases/latest`, read for its 302 `Location` only | github.com | pi tag |
| `earendil-works/pi/releases/download/<tag>/SHA256SUMS` | github.com, then release-assets.githubusercontent.com | pi checksum |
| Claude, as before | downloads.claude.ai | Claude version, checksum |
| `openai/codex/releases/latest`, read for its 302 `Location` only | github.com | codex tag |
| `openai/codex/releases/download/<tag>/codex-package_SHA256SUMS` | github.com, then release-assets.githubusercontent.com | codex checksum |

The install stage already downloaded from github.com and
release-assets.githubusercontent.com. No host was added, and the binary's
build allowlist is unchanged. `api.github.com` stays on it as guarantee
44's control.

## Equivalence

The resolver step was run on the Mac outside any build, under dash and
under `/bin/sh`, for amd64 and arm64. It produced the same tags as the
API (pi `v1.6.0`, codex `rust-v0.171.0`), and the same SHA-256 as the
API's `digest` field for each of the four assets. `/resolved.json` has
the same shape, so the install stage is unchanged.

Both checksum files exist on every recent stable release checked: pi
v1.1.0 to v1.6.0, codex rust-v0.169.1 to rust-v0.171.0.

The step fails before writing `/resolved.json` in these cases:

| Case | Result |
|---|---|
| asset name not in the checksum file | jq exit 4 |
| tag not matching the version pattern | grep exit 1 |
| repository with no release (404) | curl exit 56 |

GitHub's `/releases/latest` names the same release as the API's
`releases/latest`: the newest one that is neither a draft nor a
prerelease. The tag pattern check is kept.

## CI

| Run | Suite | Result |
|---|---|---|
| da25738 attempt 2 (before) | 190.88 s | 15 passed |
| PR #386, 0055d7e attempt 1 | 173.42 s | 15 passed |
| PR #386, 0055d7e attempt 2 | 176.84 s | 15 passed |

The budget is 300 s. CI's build output is not printed when a build
succeeds, so the request lists above come from the recipe and the local
run, not from a CI egress log.

## What remains

- github.com web pages and release asset downloads can still be throttled
  for an address. They have no 60-per-hour limit, and the install stage
  already depended on them.
- The `FROM debian:trixie-slim` pull from Docker Hub is anonymous, and
  Docker Hub limits anonymous pulls per address.
- Checksums now come from each publisher's own checksum file, not from
  the digest GitHub computes. Both are fetched over TLS from GitHub, and
  neither is signed. If a publisher drops or renames its checksum file,
  the build fails closed.
- A failed default image build is retried by every later test, because
  the panic leaves the `OnceLock` unset.
