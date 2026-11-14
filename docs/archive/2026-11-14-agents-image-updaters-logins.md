# Agents in the image, updaters and login reruns on macOS

2026-11-14, ticket #298, pinfold source `2a9bf28`. ARCHITECTURE.md is binding.
This is a compact record of the existing probes, not a new run or policy change.
No product code, dependency, test or release changed. No real login was used.
Evidence: [scripts, recipes and excerpts](2026-11-14-agents-image-updaters-logins/).
`results.txt` labels B/U/L/H/C identify selected original output and source lines.
`headers.txt` retains every request's ordered header names and credential-like pairs.
Generated identifiers and other noncredential header values are omitted.

## Decisions for the owner

| Decision | Options | Evidence line |
|---|---|---|
| Image installation layout (decision 24) | Writable user installation with read-only root, or rebuild images instead of self-updating | [U1](2026-11-14-agents-image-updaters-logins/results.txt#L97): `faccessat(...pi-coding-agent..., W_OK) = -1 EACCES`; U2/U4: npm rename `EROFS`. Hosts alone cannot fix this. |
| pi executable/transport | Use the pinned standalone supply, or resolve npm/Node transport upstream; preserve CONNECT-to-route refusal | [L1](2026-11-14-agents-image-updaters-logins/results.txt#L156): `Proxy response (403) ... HTTP Tunneling`; L3: `reason: route`; native capture: one pi POST. |
| Repeated version resolution (decision 23) | Explicitly rerun resolution with a chosen metadata freshness policy, or retain today's cached RUN behavior | [B11](2026-11-14-agents-image-updaters-logins/results.txt#L83): install-time `2026-11-14T18:37:43Z` after unchanged rebuild; B6: npm metadata `cache hit`. |
| Updater allowlist | Retain npm paths, or approve named native/check hosts with upload exposure below | [U5](2026-11-14-agents-image-updaters-logins/results.txt#L141): Codex npm action adds only `registry.npmjs.org`; P3: interactive check GETs `api.github.com`; P5: Gists accepts content POST. |
| Build-host evidence | Run Linux's audited builder before choosing a complete list, or leave completeness unresolved | [ARCHITECTURE.md, Threat model](../ARCHITECTURE.md#threat-model): `Apple builds have unrestricted egress`; recipe explicitly GETs `nodejs.org`, absent from the Linux build list. |
| Home parent ownership | Prepare the immediate parent for a single home disk, or explicitly use a separate agent disk | [L2](2026-11-14-agents-image-updaters-logins/results.txt#L162): `.pi` uid 0, `.pi/agent` uid 501 before pi; `sessions` uid 501 after pi. This control does not fix ordinary one-disk homes. |

B/U/L/H/C lines: [results.txt](2026-11-14-agents-image-updaters-logins/results.txt).
P references: [primary public sources](2026-11-14-agents-image-updaters-logins/public-updater-evidence.md).
Header evidence: [headers.txt](2026-11-14-agents-image-updaters-logins/headers.txt).
These are decisions, not authorization to change the spec or allowlists.
The updater options do not cover Claude's traffic default, which turns its
automatic updates off (see below). The owner's decisions of 2026-11-14 are on
ticket #278: rebuild to update, in-box updaters off.

## Updater and installer hosts

Every host identified by this recipe or the updater/installer paths considered
here is below, including unmeasured alternatives. Apple build redirects and
native installer chains were not captured; this is not an exhaustive observed
network allowlist. A client GET never proves the host cannot accept uploads.
Methods distinguish captured HTTP from public source and inference.

| Host | Agent | Purpose and evidence | Methods the client uses | Known to accept uploads; public basis |
|---|---|---|---|---|
| `pi.dev` | pi | Observed manual checks U1/U5; managed manifests P1 | GET from tagged source | Unknown; P1 establishes reads only. |
| `registry.npmjs.org` | All three | npm metadata/packages B6/B7; Claude/Codex actions U5 | GET captured in install logs and fixture; npm may POST audit data (P4), not captured here | Yes; publish PUT includes tarball, audit POST (P4). |
| `api.github.com` | Codex | Interactive latest-release check P3, not needed by explicit npm action | GET from tagged source | Yes; authenticated Gists POST accepts file content (P5). |
| `downloads.claude.ai` | Claude | Current native installer/updater/checks P2; unmeasured | Download GET inferred; complete native method set unknown | Unknown; P2 documents distribution, no upload basis. |
| `chatgpt.com` | Codex | Standalone installer P3; unmeasured | GET via curl | ChatGPT service accepts messages/files (P7); exact upload endpoint/host is not established. |
| `github.com` | Codex/helper | Standalone release assets candidate P3; helper-download.sh names release URL | GET via curl; redirects unmeasured | Yes; HTTPS Git pushes (P5). |
| `release-assets.githubusercontent.com` | Codex/helper | Possible GitHub artifact redirect P3; not observed | GET if followed; inference | Unknown; no upload basis in checked sources. |
| `storage.googleapis.com` | Claude | Historical native host before 2.1.116, P2; unmeasured | Download GET inferred; full historical flow unknown | Yes; Cloud Storage POST/PUT with permissions (P6); vendor bucket access unknown. |
| `formulae.brew.sh` | Claude/Codex | Public Homebrew version-check alternative P2/P3; unmeasured | Codex metadata GET from source; Claude method not specified | Unknown; checked sources establish metadata use only. |
| `nodejs.org` | All three via Node | Recipe runtime/checksum downloads; not runtime updater traffic | GET via curl in recipes | Unknown; no upload basis established here. |
| `deb.debian.org` | All three via base | Debian package steps; recipe input, not audited build contact | apt HTTP downloads; exact methods not captured | Unknown; no upload basis established here. |
| `api.anthropic.com` | Claude | Auxiliary traffic only without traffic default, U5/P2 | TLS tunnel observed; inner method unknown | Yes; Messages POST accepts prompt content (P10); auxiliary contact does not prove package need (P2). |

The current default agent list already contains pi.dev and registry.npmjs.org;
api.github.com and downloads.claude.ai are absent. No host was added.
Native updater completeness and native installer caches remain unestablished.

## Commands and environment

Original environment: macOS 27.2.1 (26B131), arm64, Apple container 1.7.1,
pinfold 0.5.5. `probe.py` contains the actual commands, fixture handlers,
all three settings files, box specs, readiness and teardown. Settings are
generated by the script; separate copies are unnecessary.
The helpers use only fresh scratch homes; network and `/Users` reads are denied.
Loopback addresses are fixture listeners/proxies. Fixture keys are named
`fixture-*` or `pinfold-placeholder`; the JWT is unsigned and ends `.fixture`.
The `.invalid` emails and `fixture-host` are synthetic.

```sh
RUSTC_WRAPPER= cargo build --locked
# Run from the repository root; original scratch root shown for provenance.
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py MODE
# MODE: build, baseline, refresh, inventory, updates, claude-fixture,
#       logins, logins-native, logins-injected, cleanup
sh docs/archive/2026-11-14-agents-image-updaters-logins/helper-download.sh
```

This is reproduction guidance, not a request to run boxes during condensation.
The script isolates XDG config/state, stamps maintenance, disables update checks,
checks 12 GiB free before builds, and uses random probe box/disk names with
preflight disk inventory. The real cache and shared builder were retained.
Recipe variants must be copied to `Containerfile` before the corresponding build.
The injecting mode replays the committed fixture token file; it does not read a new helper run's output.
`PROBE_LOG` selects a distinct log filename for repeated modes.

## Image and cache observations

| Recipe / excerpt | First changed build | Unchanged rebuild | Cache condition |
|---|---:|---:|---|
| Containerfile.initial / B1 | 70.044 s | 4.423 s | New RUNs, empty named npm cache, shared base store |
| Containerfile.postinstall / B2 | 31.850 s | 4.140 s | Changed npm RUN, warm cache |
| Containerfile.procps / B3 | 52.095 s | 5.154 s | Changed runtime RUN, fresh final npm cache |
| Containerfile.procps, --no-cache / B4 | 50.611 s | Not run | All RUNs rerun, warm npm download cache |
| Containerfile / B5 | 27.523 s | 3.555 s | Explicit Claude allow-scripts, warm download cache |

Single observations, not medians or globally cold-store benchmarks.
Final packages: pi 1.5.2, Claude 2.1.325, Codex 0.169.0; Node 24.22.3,
npm 11.19.1 (B6/B9). Final runtime-reported variant size: 503641218 bytes
(480.31 MiB), not unique host disk cost (B11).
The initial ignore-scripts recipe left Claude's native binary missing (B8).
Separating its installation made it start; the final recipe explicitly allows
Claude's named script after npm's script-policy warning (B10). Codex needed `procps` for
its normal start: initial `ps` invocation failed (B8); final chooser appears (B9).
Pi reports no key; Claude reaches onboarding/connectivity failure under empty
allowlist; Codex reaches sign-in choices. No login choice was submitted (B9).

The retained `/root/.npm` cache grew 4 to 456996 KiB on the fresh-cache build.
Final invocation grew 457000 to 457104 KiB and hit all three tarballs (B6/B7).
The unchanged build reuses the install RUN instead of resolving latest again;
B11 retains its old install-time. npm also caches metadata (B6), so rerunning
the RUN alone does not prove fresh dist-tag resolution. No native cache measured.
Apple builds have no pinfold build-egress log (ARCHITECTURE.md and
`core/runtime/apple.rs::build`). Node's host is a known recipe input missing
from Linux's build list; Debian/npm are listed. Base pulls/redirects remain unknown.

## Updater observations

Both with and without the pi/Claude harness defaults copied box-wide:
pi's explicit check says current (exit 0), but force fails the original npm
prefix permission check (exit 1, U1/U3). Claude's check says current (exit 0).
Codex's explicit npm action fails renaming its global package (EROFS, exit 226,
U2/U3). The roots remain read-only; no writable replacement was selected.
Defaults ordinarily apply per `box exec --harness`, not image-only boxes.

Claude's local fixture advertises 999.0.0, with the nonessential-traffic default
set. It runs npm view --prefer-online and global npm install, fails launcher
and package renames with EROFS, exits 1 and remains 2.1.325 (U4).
Only two metadata GETs reached the fixture; no tarball was fetched/executed.
The home update-result write succeeds. Pi creates state/trust lock (U1); no exhaustive write inventory is claimed after condensation.
PI_SKIP_VERSION_CHECK disables pi's background check, not manual update (P1/U1).
Claude's traffic default disables automatic updates per P2, but not this manual
attempt. pinfold sets no Codex updater-disabling env in harnesses.toml.

## Login and helper observations

All three settings coexist, plus box-wide ANTHROPIC_API_KEY=pinfold-placeholder.
Claude's managed file blanks that key and sets its own OAuth placeholder/route.
Codex selects Responses with requires_openai_auth=false. Pi auto-loads its extension.
Fresh home/work/agent disks require no preparatory boot: the separate agent disk
makes `.pi/agent` user-owned; mounts remain read-only and pi creates sessions
(L1/L2, probe.py). This does not resolve single-home-disk parent preparation.

npm pi creates state but gets four CONNECT route refusals; its fixture sees
zero requests (L1/L3, npm capture). P8 explains Node's forced tunnelling.
The pinned standalone pi control, mounted with harnesses:["pi"] and invoked
by absolute path, reaches its own HTTP route. Claude/Codex remain image binaries.
No per-exec harness env or weakening of CONNECT refusal was used.

| Plain-route client | Requests | Every observed credential-like field |
|---|---|---|
| Standalone pi 1.5.2 | 1 POST /v1/messages?beta=true | x-api-key: pinfold-placeholder |
| Claude 2.1.325 | 6 POST /v1/messages?beta=true | Authorization: Bearer pinfold-placeholder; no API-key header despite box env |
| Codex 0.169.0 | 1 POST /backend-api/codex/responses | None |

`headers.txt` supports absence of Authorization/API-key/Cookie/account/organisation
fields beyond those listed, for every captured request. It preserves names/order,
not all noncredential values. npm pi has no HTTP header result to inherit.
Injected-route captures show fixture pi key, Claude bearer, Codex bearer/account;
the Codex bearer equals the offline helper's fixture JWT. Fixtures return 400:
arrival/header forwarding is established, successful inference is not.

H1 and helper-download.sh retain the 0.167.1 artifact checksum.
helper-fixture.py and helper-fixture-output-enoent.txt show empty home -> null
auth, fixture ChatGPT home -> exact unsigned fixture token, helpers reaped.
The host protocol is initialize/initialized/getAuthStatus(includeToken:true).
Source P9 establishes expiry/account checks and no helper/image version handshake.
The 0.167.1 helper/0.169.0 image exchange is fixture-only. Compatibility depends
on auth-store/protocol/token claims and upstream acceptance, not version equality.
No real login, refresh, subscription, backend compatibility or future-version
compatibility was established. The earlier helper run without id-1 response and
unretained diagnostic are dropped; the successful ENOENT-denial case remains.

## Retained state and limits

Original cleanup reported empty probe disk inventory and running shared service
(C1). C2 names retained probe-agents refs latest, 18e7fd48409e6cc8-96704 and
18e7fd4e9d7d2d98-97194, all final id 7b57a746b152c8907a70469f9fa2c7e8eba2783bf6fc983bcd587d1420f23fb3.
Original scratch roots: /tmp/pfa.07ncy64h and /tmp/pfh.298. Named npm caches remain.
No box or runtime operation was rerun for this edit.
Not run: Linux builds/starts, native installer/updater variants, cold shared store,
writable self-replacement, real inference/login/refresh, upload experiments.
Dropped: full header values/identifiers, bulk traces/logs, ANSI screens, guest du
breakdown, initial helper-failure detail, and validation/setup claims without
retained command output. Scripts and four recipe iterations remain.

Condensation checks: text/privacy scan, exact header projection comparison, Python
parse, shell syntax and git diff --check passed. No boxes, builds or tests rerun.
