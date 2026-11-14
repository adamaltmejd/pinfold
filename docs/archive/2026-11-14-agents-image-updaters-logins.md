# Agents in the image, updaters and login reruns on macOS

Date: 2026-11-14. Ticket #298, at `2a9bf28`. Read with
`2026-11-12-project-box-plan.md`, the earlier login probe and its files,
and the Linux harness measurement. ARCHITECTURE.md remains the spec.
No product code, policy, dependency pin or test changed. No release was made.

Evidence and scripts: [2026-11-14-agents-image-updaters-logins/](2026-11-14-agents-image-updaters-logins/).
Each `.log` prints the command, output and exit status. HTTP fixtures return
400 deliberately; that error establishes arrival, not successful inference.

## Established and findings

- A recipe on the default Debian base installs pi 1.5.2, Claude 2.1.325
  and Codex 0.169.0 through npm. All report their versions and start without
  login. Codex's normal start needs `procps`, absent from the default base.
- npm's download cache works for all three. An unchanged `pinfold build`
  reuses the install RUN. It does not resolve `latest` again. Decision 23's
  agent step on every build is not today's build behavior.
- These global image installations cannot replace themselves on the
  read-only root. Pi refuses its installation's permissions. Claude and
  Codex reach npm's rename and get `EROFS`. Enabling hosts alone does not
  make decision 24 work for this recipe.
- Claude's managed blank `ANTHROPIC_API_KEY` works with all three settings
  present and a box-wide placeholder key. Claude sends its bearer placeholder,
  with no API-key header. Codex itself sends no credential header on its
  plain route. Every forwarded header is retained in order.
- On a first boot, pi can write its state and reach its fixture when its
  immediate state-directory parent is user-owned. A separate fresh disk at
  `.pi/agent` establishes that condition without a preparatory boot. It does
  not fix missing parent ownership for one ordinary home disk.
- npm pi under Node uses CONNECT even for HTTP routes. The proxy correctly
  refuses it. The same pi version's pinned standalone binary reaches the
  plain route. The npm image is therefore not ready for pi's login route.
- The real pinned Codex helper 0.167.1 exports a fixture JWT offline.
  There is no helper/image version handshake. Codex 0.169.0 also reaches a
  fixture through the injecting proxy using that exported fixture token.
  This is not a real-login or backend-compatibility test.
- Apple builds have no pinfold build-egress log. A complete measured build
  host list cannot be supplied on this platform. No Linux build was run.

## Environment and reproduction

macOS 27.2.1 (26B131), arm64; Apple `container` 1.7.1; pinfold 0.5.5.
The shared service was already running and remains running. The builder
was left to the runtime; no builder command, prune or service restart ran.
Only profile `probe-agents` was built. Only randomly suffixed probe boxes
and disks in this probe's state were removed.

```
RUSTC_WRAPPER= cargo build --locked
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py build
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py baseline
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py refresh
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py inventory
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py updates
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py claude-fixture
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py logins
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py logins-native
PROBE_ROOT=/tmp/pfa.07ncy64h python3 docs/archive/2026-11-14-agents-image-updaters-logins/probe.py logins-injected
```

These are the modes, not a claim that the final recipe existed at the first
build. The recipe iterations and log names are below. `PROBE_LOG` chooses
a distinct log for a repeated mode. Each box spec and image id are logged.
The script uses its own XDG config/state, the real default cache, and
`PINFOLD_NO_UPDATE_CHECK=1`. It refreshes its maintenance stamp before the
first pinfold command. `df -g /` and an exact 12 GiB guard precede each
image build. Every build passed the guard; no low-space build was attempted.
The script checks disk inventory before startup and only removes disks
absent there that it attached in a successful start.

The default `cargo build --locked` first failed because sandboxed sccache
returned `Operation not permitted`. Clearing `RUSTC_WRAPPER` built the real
binary. Runtime commands needed execution outside the session sandbox.
Automatic approval review rejected the first startup invocation's fixed-name
cleanup. No box started from that invocation. Random names and preflight
inventory checks corrected the script. An initial inventory parser expected
an array; an empty JSON-lines inventory stopped it before box creation.

## Probe 6: image and caches

`Containerfile` is the final recipe. It copies the default base's Debian
and tools steps and hardening. It adds `xz-utils`, `strace`, `procps`,
Node 24.22.3 (verified against its published SHA256), npm 11.19.1 and the
three packages at `@latest`. `strace` is probe instrumentation. There is no
project context, host login file or credential in the build.

The npm step mounts a named BuildKit cache at `/root/.npm`. That directory
is a download cache, not an image layer. Its name is probe-specific.
The image retains package/cache summaries and npm's HTTP log under
`/usr/local/share/probe-agents/` so pinfold's successful-build log suppression
does not hide installer behavior.

| Recipe and log | First build | Unchanged rebuild | What was cold |
|---|---:|---:|---|
| `Containerfile.initial`, `build.log` | 70.044 s | 4.423 s | New RUNs and empty named npm cache; Debian image store shared |
| `Containerfile.postinstall`, `build-corrected.log` | 31.850 s | 4.140 s | Changed npm RUN; warm download cache |
| `Containerfile.procps`, `build-final.log` | 52.095 s | 5.154 s | Changed runtime RUN and fresh final npm cache; default tools layer warm |
| `Containerfile.procps`, `refresh.log`, `--no-cache` | 50.611 s | not run | All RUNs rerun; npm download cache warm |
| `Containerfile`, `build-script-allowed.log` | 27.523 s | 3.555 s | Changed npm RUN; download cache warm |

No globally cold store was measured. Emptying the shared builder/image
store would violate the rules. These are single wall-clock observations,
not medians or an uncontended-machine benchmark.

The initial recipe applied `--ignore-scripts` to all three installations.
Claude's `--version` then failed with `native binary not installed`, and
its direct interactive exec failed `Exec format error`. Installing Claude
in a separate npm invocation made it start. npm 11.19.1 also warned that
its postinstall was not covered by `allowScripts`; the final recipe
explicitly allows only Claude's named script. These are recipe corrections,
not pinfold fixes. The initial Codex normal start failed because `ps` was
missing; the procps variant reached its sign-in chooser.

Cold final-cache evidence in `baseline-final.log`: `/root/.npm` grew from
4 KiB to 456996 KiB; package downloads report cache misses. Final
`inventory.log`: 457000 to 457104 KiB, and the same pi, Claude and Codex
package tarballs report cache hits. Thus all three selected installers use
the retained npm cache. Native/managed vendor installers were not measured.

The unchanged build's image history retains the install timestamp from its
first build. The RUN is reused, so `@latest` is not queried. Rerunning the
RUN invokes npm, but npm also caches registry metadata: the final invocation
reports metadata cache hits. This does not prove immediate upstream
revalidation of every dist-tag. The source check and cache policy must be
chosen when decision 23 is implemented.

Final image: `localhost/pinfold/profile-probe-agents:latest`, runtime id
`7b57a746b152c8907a70469f9fa2c7e8eba2783bf6fc983bcd587d1420f23fb3`.
Apple reports its arm64 variant size as 503641218 bytes (480.31 MiB).
This is the runtime-reported image size, not a measured unique host-disk
cost. The guest's readable `/usr` is 1363028 KiB; `du` cannot read some
root-only `/var` and `/etc` subdirectories. Package directories occupy
356408 KiB (Claude), 162804 KiB (pi), 412252 KiB (Codex), plus 120556 KiB
for Node. Cache sizes are separate from these image directories.

`baseline-final.log` runs `--version` and noninteractive starts, then normal
TTY starts. Versions: pi `1.5.2`, Claude `2.1.325 (Claude Code)`, Codex
`codex-cli 0.169.0`. Pi prints its missing-login/model warning; Claude
reaches onboarding and then fails its public connectivity check under an
empty allowlist; Codex reaches its sign-in chooser. No login choice or token
was submitted. Observation clients were bounded and terminated; their
boxes were then torn down. The final recipe has the same versions, also
exercised by the updater and login cases.

### Build hosts: limit of the observation

ARCHITECTURE.md's threat model and Linux image-preparation section explicitly
say Apple builds have unrestricted egress. `core/runtime/apple.rs::build`
uses the native builder without the production proxy/audit. No build run
record or build-egress path is produced. The caller also rejects `--allow`
on Apple. This is an observation limit, not a missing Linux log.

The recipe names `nodejs.org` and Debian repositories; npm's retained HTTP
log names `registry.npmjs.org`. Comparing named inputs against the Linux
build list: `nodejs.org` is absent; `deb.debian.org` and
`registry.npmjs.org` are present. Docker registry/base-pull contacts and
redirects are not enumerated by these logs. This is not a complete observed
host list. It must not be used as authority for an exhaustive build-allowlist
change. Linux audit remains needed for that decision.

## Probe 5: updater behavior

`updates.log` uses two fresh boxes with the current default agent allowlist.
One sets the current pi and Claude harness defaults box-wide; one omits
them. These defaults are normally per `box exec --harness`, not automatic
env in an image-only caller box. This probe does not change that behavior.
`strace -f -e trace=file` observes the real command and its children.
Egress snapshots are cumulative within each box; compare adjacent snapshots
for each command, not every host in the last snapshot with every updater.

| Agent and command | Writes / outcome on this image | Defaults today |
|---|---|---|
| `pi update --self` | GET latest and says current, exit 0. Creates `.pi/agent` and its trust lock; Node also caches compiled code under `/tmp`. | `PI_SKIP_VERSION_CHECK=1` skips the background version check; manual update still contacts `pi.dev`. `PI_TELEMETRY=0` is separate. |
| `pi update --self --force` | `faccessat` on `/usr/local/lib/node_modules/@earendil-works/pi-coding-agent` returns `EACCES`; exit 1, no package replacement. Prints the npm command for that original prefix. | Same refusal with and without defaults. |
| `claude update` | Checks npm and says current, exit 0. Writes `.claude.json`, backups and npm cache/logs in the home. | `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` disables automatic updates per vendor docs, but does not prevent this manual check or the fixture replacement attempt below. |
| Codex's npm update action, `npm install -g @openai/codex@latest` | npm metadata/cache/logs in `/home/user/.npm`; renaming `/usr/local/lib/node_modules/@openai/codex` to its hidden replacement returns `EROFS`, exit 226. | No Codex updater-disabling env is set by pinfold. |

All three retain their image versions. No writable replacement was silently
selected. Pi's expected prefix and Codex's failed rename are recorded, not
inferred from file ownership alone.

Claude's public check cannot exercise replacement while it is current.
`claude-fixture.log` therefore uses a local, plain HTTP npm-registry route,
no external allowlist, and metadata advertising fixture version `999.0.0`.
The real `claude update` runs `npm view ... --prefer-online` and then
`npm install -g @anthropic-ai/claude-code@999.0.0`. Renames of both
`/usr/local/bin/claude` and its package directory return `EROFS`. It exits
1 and remains 2.1.325. The fixture saw only two metadata GETs; its tarball
was never downloaded or executed. This also ran with the nonessential
traffic default set. Writes to `.claude/.last-update-result.json` succeed.

### Updater hosts and upload decision

No upload experiment was performed. The classification below uses public
vendor docs, public client source and the client methods actually observed.
Exact source links are in `public-updater-evidence.md`. The Codex 0.169.0
updater files were compared with 0.167.1 and are identical.

| Host | Evidence / need | Already in default agent allowlist? | Accepts uploads or writes? |
|---|---|---|---|
| `pi.dev` | Both explicit pi checks allowed in egress. Tagged pi source GETs `/api/latest-version`. | Yes | Unknown from checked public docs. GET usage does not prove a read-only host. |
| `registry.npmjs.org` | Claude's check and Codex's npm action allowed. All three npm installers/updaters use it. Fixture Claude shows metadata GETs and global npm replacement. | Yes | Yes: npm publish's PUT includes tarball data; npm also documents write APIs and audit POSTs. |
| `api.github.com` | Codex startup egress refuses it; tagged updater source GETs release latest before npm metadata. The explicit npm action itself only contacts npm. | No | Yes: the documented Gists POST API accepts file contents with authentication. |
| `downloads.claude.ai` | Vendor's current native updater distribution host. This npm installation did not contact it. | No | Unknown from checked public docs. Distribution GETs do not establish absence of an upload API. |

The npm recipe needs no new download host for its replacement action, but
Codex's interactive version check needs `api.github.com`. A future native
Claude recipe would need its named distribution host. Neither host was added
to policy. Native pi/Claude/Codex updater host completeness and native
installer caching are not established by the npm measurements.

Without Claude's traffic default its updater case also contacted
`api.anthropic.com`; it is already allowed. The corresponding case with the
default did not. This is auxiliary traffic, not evidence that this host is
needed to download a package. Initial unauthenticated startup also attempts
`github.com`, `raw.githubusercontent.com`, `ab.chatgpt.com` and provider
hosts. They are startup traffic, not all proven updater dependencies.

## Login reruns and every credential-like header

Each case has all three settings files and a box-wide
`ANTHROPIC_API_KEY=pinfold-placeholder`. The managed Claude file blanks that
key and supplies only its own base URL and OAuth placeholder. Codex's system
file selects `requires_openai_auth=false` and the Responses route. Pi's
extension auto-loads from `.pi/agent/extensions`, without `-e` or per-exec
harness env. Files are retained in `files-pi/`, `files-claude/`, `files-codex/`.
Touches to all three mounts fail `Read-only file system`.

Every case starts fresh home, work and `.pi/agent` disks. The `.pi/agent`
disk root is uid 501:20 on first readiness, before the first pi process.
Its nested extensions mount is read-only. `.pi` itself remains root-owned
and traversable. No root exec, chmod/chown correction or earlier boot was
used. Pi creates a uid-501 sessions directory. This is an ownership control
using a separate disk, not evidence that pinfold now prepares missing nested
parents correctly on an ordinary one-disk home.

### npm pi transport failure, then the standalone control

`logins.log` first uses only the image's packages. Pi creates its state,
then fails after four CONNECT refusals naming `anthropic`, reason `route`.
Its fixture receives no request. Claude receives six fixture responses and
Codex one, both through their own routes. The original capture is
`login-headers.json`.

The [tagged pi dispatcher](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/core/http-dispatcher.ts)
explicitly sets `proxyTunnel: true`; [Undici's documentation](https://undici.nodejs.org/api/ProxyAgent)
says that forces CONNECT even for HTTP. No documented pi setting to change
it was found. The agent uses the configured route name, so this is not a
missing extension or key. The proxy's CONNECT-to-route refusal is required
and was preserved.

`logins-native.log` changes only pi's executable supply: `harnesses:["pi"]`
mounts pinned standalone pi 1.5.2 read-only, and the command names
`/opt/pinfold/pi/pi`. There is no `--harness`. Claude and Codex remain the
image installations. On entirely new first-boot disks, standalone pi reaches
its fixture once. The runtime difference explains why the earlier standalone
probe did not find the npm/Node failure. The exact bundled Bun revision was
not identified; its internals are not established by this probe.

### Ordered plain-route captures

`logins-native-headers.json` stores every request's headers as ordered
`[name,value]` pairs, preserving names and duplicates. Fixtures read
`headers.raw_items()`. They do not select a few credential fields as the
old probe did. These are plain routes: no credential header is replaced.
The proxy still rewrites Host and connection framing; this is the complete
forwarded header list, not a byte capture before those proxy edits.
All requested credential-like names pass through that observation point.

| Agent | Requests / path | Every observed credential-like header |
|---|---|---|
| Standalone pi 1.5.2 | 1 POST `/v1/messages?beta=true` | `x-api-key: pinfold-placeholder`. No Authorization, Cookie, account or organisation header. |
| Claude 2.1.325 | 6 POST `/v1/messages?beta=true` | `Authorization: Bearer pinfold-placeholder`. No X-Api-Key, Cookie, account or organisation header, despite the box-wide API-key placeholder. |
| Codex 0.169.0 | 1 POST `/backend-api/codex/responses` | None: no Authorization, X-Api-Key, Cookie, account or organisation header. |

Codex does send installation/session/thread/window/turn identifiers in
`x-codex-turn-metadata`, plus separate identifier headers. Their complete
values and order are retained, not described as credentials. The full ordered
header-name list is in the adjacent capture, including User-Agent, originator
and content headers. No request reaches another agent's fixture.

The npm pi case establishes no HTTP credential-header list: it never gets
past CONNECT. It must not inherit the standalone result by version number.

### Injecting-route control and helper coupling

`logins-injected.log` repeats the first-boot, all-settings case with injecting
routes. Pi is the standalone control. The upstream fixtures receive their
own fixture pi key, Claude bearer, and Codex bearer/account. Codex's token is
exactly the JWT exported by the offline pinned helper fixture. The ordered
capture is `logins-injected-headers.json`. These injected captures do not
substitute for the plain-route observation of what each box client sent.

`helper-download.sh`, `helper-fixture.py`, `helper-checksum.log` and
`helper-fixture-output-enoent.txt` record the separate real-helper fixture.
The artifact checksum matches pinfold's 0.167.1 pin. macOS `sandbox-exec`
denies all network and all reads under `/Users`; fresh HOME and CODEX_HOME
hold only fixture data. The initial EPERM-denial variant returned no id-1
answer (`helper-fixture-output.txt`); its stderr was not retained, so the
underlying config-read diagnostic is not independently established by that
file. The final script returns ENOENT for denied home files and initializes
without reading those files. Empty fixture home
returns no token; fixture ChatGPT home returns the exact unsigned fixture
JWT. Both helpers were reaped.

`core/login.rs` sends `initialize`, `initialized`, and
`getAuthStatus {includeToken:true}` over JSON lines. Only result id 1's
`authToken`/`authMethod` matters. pinfold checks future expiry and the account
claim, then supplies the two upstream headers. The guest talks HTTP Responses
to the proxy; it never talks to this helper or exchanges a version with it.
The protocol and pinned-source analysis is in `helper-analysis.md`.

The coupling is the helper's ability to read the current host auth store,
the deprecated JSON method/fields, required JWT claims, and upstream acceptance
of the image agent's requests plus the proxy's bearer/account headers.
No image/helper version equality is checked. This fixture establishes the
current method and a 0.167.1-helper/0.169.0-client proxy exchange, not arbitrary
future compatibility. No helper was run against the real login, no real
subscription was used, and no token refresh was attempted.

## Owner decisions and remaining limits

1. Choose an image installation/runtime layout that permits the intended
   self-updates while preserving the read-only root. The measured global npm
   prefix cannot do so. Pi's Node transport also needs a packaging or upstream
   decision; weakening CONNECT-to-route refusal is not a probe fix.
2. Make decision 23's repeated version resolution explicit. Today's cached
   RUN skips it; rerunning npm also has its own metadata freshness policy.
3. Decide the named updater hosts with the upload table. In particular,
   `api.github.com` accepts authenticated writes. Native Claude's distribution
   host has unknown upload capability from the checked sources. No allowlist
   change was made.
4. A complete measured build-host list requires Linux's build audit. This
   Mac cannot establish one. `nodejs.org` is a known recipe input missing
   from today's Linux build list, not an exhaustive proposed addition.
5. The one-home-disk parent-ownership preparation is still open. The fresh
   separate-disk control proves what works once the immediate parent is owned;
   no code changed to make that condition automatic.

Not run: native vendor installer/updater variants, a cold shared image store,
Linux builds or starts, real provider inference/login/refresh, successful
self-replacement on a writable installation, or future Codex version
compatibility. No upload endpoint was probed.

Validation: `cargo fmt --check` and
`RUSTC_WRAPPER= cargo clippy --all-targets --locked -- -D warnings` passed.
The suite is suspended and no test was added.

The probe's boxes and disks were removed. The shared service remains running.
Retained images are listed in `cleanup.log`: `latest`,
`18e7fd48409e6cc8-96704` and `18e7fd4e9d7d2d98-97194` under
`localhost/pinfold/profile-probe-agents`. All three refs name the final id.
The probe's scratch state/config and fixture files remain under
`/tmp/pfa.07ncy64h`; helper scratch is `/tmp/pfh.298`. No live probe process
uses them. The named BuildKit npm caches remain with the shared builder.
