# TLS interception for routed hosts: design, 2026-11-26

Proposal for #475. The ARCHITECTURE.md diff in the same PR is the
contract; this file holds the evidence and the reasons.

Guarantee served: a login or injecting route keeps working when a client
follows an absolute URL its upstream returns, and the credential never
enters the box.

## Problem

A login route is served at a name of its own (`api.github.localhost`,
`http://<route>` for Claude, Codex and Pi). Clients reach it only because
pinfold rewrites their configuration: `GH_HOST`, Git `insteadOf`,
`ANTHROPIC_BASE_URL`, Codex's provider `base_url`, Pi's `baseUrl`. An
upstream that returns an absolute URL sends the client to the real host
instead. There the client sends its placeholder over a CONNECT tunnel the
proxy cannot read, and the request fails.

The project box plan (2026-11-12-project-box-plan.md, "Considered and
rejected") deferred interception as not needed "for the allowlist or for
login routes". Probe (a) below shows login routes do need it.

## Probes

### (a) Which `gh` commands follow absolute URLs

Host `gh` 2.106.0, `GH_HOST=github.localhost`, placeholder tokens, and
`HTTPS_PROXY`/`HTTP_PROXY` at a local fake proxy. The fake answered plain
HTTP for `api.github.localhost` with canned GitHub JSON carrying real
`https://api.github.com/…` and `https://uploads.github.com/…` URLs, and
answered every CONNECT 403. It logged each request line.

| Command | Requests on the route | Then |
|---|---|---|
| `release upload v1 b.txt` | `GET /repos/o/r/releases/tags/v1`, `POST /graphql` | `CONNECT uploads.github.com:443`, four tries (`upload_url`) |
| `release upload --clobber` (asset exists) | same | `CONNECT api.github.com:443` (asset `url`, DELETE) |
| `release download -p a.txt` | same | `CONNECT api.github.com:443` (asset `url`) |
| `release download --archive tar.gz` | same | `CONNECT api.github.com:443` (`tarball_url`) |
| `run download 7` | `GET …/actions/runs/7/artifacts` | `CONNECT api.github.com:443` (`archive_download_url`) |
| `api --paginate /repos/o/r/issues` | first page | `CONNECT api.github.com:443` (`Link: rel="next"`) |

All six fail today. Commands that stay on GraphQL or on relative REST
paths (`pr list`, `issue view`, the 2026-11-24 spike) work.

Real redirects, host `gh` with `GH_DEBUG=api` against `cli/cli`
v2.106.0:

- An asset: `api.github.com/…/releases/assets/ID` answers 302 to a signed
  `release-assets.githubusercontent.com` URL; `gh` follows it without
  `Authorization`.
- `--archive`: `api.github.com/…/tarball/TAG` answers 302 to
  `codeload.github.com` (signed for a private repository).
- Upstream GitHub negotiates HTTP/2 with `gh`; under interception it
  gets HTTP/1.1, which Go's client accepts.

So a github login route must serve `api.github.com`, `github.com` and
`uploads.github.com`. The redirect targets carry their own signature, so
they need only the allowlist (passthrough), not a credential.

### (b) Which CA trust each TLS stack in the shipped images honours

Shipped images: `profile/default` (Debian trixie, `ca-certificates`,
`curl`, `git`, native Claude, standalone Pi, Codex), `documents` (adds
Bun), `adam` (adds `gh`). No Node and no Python ship; a user who adds
either gets the system store like any other tool.

Native Claude and standalone Pi are Bun-compiled executables (Pi:
`scripts/build-binaries.sh`, `bun build --compile`). Codex is Rust. `gh`
is Go. Debian's `git-remote-https` links `libcurl-gnutls` (GnuTLS);
Debian's `curl` uses OpenSSL.

Box probe, 2026-11-26: two `pinfold box up` boxes on
`profile-default:latest`, no egress. A read-only mount held a test CA, a
leaf for `localhost` and two probe binaries cross-built on the host: a Go
`net/http` GET (`gh`'s stack) and a `bun build --compile` fetch
(Claude's and Pi's runtime). `openssl s_server` served the leaf on
loopback inside the box; proxy variables were unset for the probe.
"ok" means TLS verified (git's "not valid: is this a git repository?"
is past TLS).

Box 1, image's own `/etc/ssl/certs`; `/tmp/bundle.pem` is the image
bundle plus the test CA:

| Variable | curl | git | Go | Bun |
|---|---|---|---|---|
| none | fail | fail | fail | fail |
| `SSL_CERT_FILE=/tmp/bundle.pem` | ok | fail | ok | ok |
| `SSL_CERT_DIR=/etc/ssl/certs:<dir>` | fail* | fail | ok | fail |
| `CURL_CA_BUNDLE=/tmp/bundle.pem` | ok | fail | fail | fail |
| `GIT_SSL_CAINFO=/tmp/bundle.pem` | fail | ok | fail | fail |
| `NODE_EXTRA_CA_CERTS=<ca.pem>` | fail | fail | fail | ok |

\* The directory had no OpenSSL subject-hash link, so this curl failure
is an artefact of the probe, not a finding.

Box 2, a directory mounted read-only over `/etc/ssl/certs` holding only
`ca-certificates.crt` (the host's roots plus the test CA):

| Variable | curl | git | Go | Bun |
|---|---|---|---|---|
| none | ok | ok | ok | fail (`SELF_SIGNED_CERT_IN_CHAIN`) |
| `NODE_USE_SYSTEM_CA=1` | ok | ok | ok | ok |
| `NODE_EXTRA_CA_CERTS=<ca.pem>` | ok | ok | ok | ok |

From documentation and source only, not run in a box:

- Claude Code (docs, "Enterprise network configuration"): trusts its
  bundled Mozilla set and the OS store by default
  (`CLAUDE_CODE_CERT_STORE=bundled,system`), and `NODE_EXTRA_CA_CERTS`.
- Codex (`codex-rs/http-client/src/custom_ca.rs`): starts from native
  roots (`native_roots::load`) and adds `CODEX_CA_CERTIFICATE`, else
  `SSL_CERT_FILE`.
- Go's `crypto/x509` on Linux reads `/etc/ssl/certs/ca-certificates.crt`
  first, or `SSL_CERT_FILE`/`SSL_CERT_DIR` (confirmed by the Go probe).
- On the macOS host, a Bun executable added `SSL_CERT_FILE` to its
  bundled roots, while Node 26 replaced them; neither matters in a
  shipped image.

Conclusion: the system bundle path plus `NODE_EXTRA_CA_CERTS` covers
every stack shipped. `SSL_CERT_FILE` alone misses Git; the variable set
(`SSL_CERT_FILE`, `GIT_SSL_CAINFO`, `NODE_EXTRA_CA_CERTS`) covers them
too but is three names a caller can collide with and still misses a
tool that reads the bundle path without a variable.

## Design

### Scope

The proxy terminates TLS only for routed hosts: each host a login route
or an `https` injecting route serves, by its real name. Every other
allowlisted host keeps today's SNI-checked CONNECT passthrough. A routed
host needs no allowlist entry.

| Route | Served at | Upstream |
|---|---|---|
| host service, `name = "host:port"` | `http://name`, plain HTTP, as today | `host:port` |
| injecting, `http` `to` | `http://name`, plain HTTP, as today | `to` |
| injecting, `https` `to` | `to`'s host, intercepted | `to` |
| `claude` login | `api.anthropic.com` | `to`, else that origin |
| `codex` login, pi `openai-codex` | `chatgpt.com` | same |
| pi `openrouter`, `openai`, `anthropic`, `anthropic-subscription` | `openrouter.ai`, `api.openai.com`, `api.anthropic.com` | same |
| `github` login | `github.com` (Basic), `api.github.com` (Bearer), `uploads.github.com` (Bearer) | `TO/`, `TO/api/v3/`, `TO/api/uploads/` with `to` |

The route's key in `routes` stays as its label (errors, `config show`);
for an intercepted route it is no longer a name the box can reach. An
`https` injecting route is not also served at its label: no
compatibility path.

`to` keeps its meaning for login routes: the box still talks to the
agent's real host; the proxy dials `to`. That keeps e2e row 26 on plain
HTTP host fixtures, with the box side fully intercepted.

Plain HTTP to a routed host is refused 403 `route`, as CONNECT to a
route name is today. CONNECT to a routed host on a port other than 443
is refused `port not allowed`.

Two routes that serve one host must carry the same credential: the same
header names, `from` sources and prefixes, or both the host's Codex
login (already shared today). Otherwise `spec`. So Claude's login and
Pi's `anthropic-subscription` with one `from` share `api.anthropic.com`;
Claude's login and Pi's `anthropic` (API key) do not. Each still
generates its agent's configuration. See open question 1.

### What leaves the box's environment

| Today | After |
|---|---|
| `GH_HOST=github.localhost` | gone |
| `GH_TOKEN` placeholder | stays: `gh` will not run without a token |
| `GH_ENTERPRISE_TOKEN` placeholder | gone: only needed for a non-github.com `GH_HOST` |
| `insteadOf` `https://github.com/` → route | gone |
| `insteadOf` `git@github.com:` → route | becomes `git@github.com:` → `https://github.com/`; the box has no SSH egress |
| `http.<route>.postBuffer` | gone: chunked bodies are accepted (below) |
| Claude `ANTHROPIC_BASE_URL=http://<route>` | gone; placeholder `CLAUDE_CODE_OAUTH_TOKEN` and blank `ANTHROPIC_API_KEY` stay |
| Codex provider `base_url = "http://<route>/backend-api/codex"` | `https://chatgpt.com/backend-api/codex`; the file stays, since Codex needs a provider that does not require its own login |
| Pi `baseUrl` per provider | gone; placeholder `apiKey` stays |

So `GIT_CONFIG_COUNT` drops from 3 to 1. Claude without
`ANTHROPIC_BASE_URL` also behaves as on the direct API (its first-byte
watchdog and fast-mode check treat a base URL as a gateway), and its
other `api.anthropic.com` calls get the real token instead of failing
on the placeholder.

Added when the box has a routed host: the trust mount and
`NODE_EXTRA_CA_CERTS` (below). A caller env entry naming
`NODE_EXTRA_CA_CERTS` is refused as `spec`, as the github variables are
today.

### Per-box CA

- **Generation.** At start, after routes resolve: an ECDSA P-256 key
  pair from ring (`EcdsaKeyPair::generate_pkcs8`,
  `ECDSA_P256_SHA256_ASN1_SIGNING`). A self-signed CA certificate:
  subject `CN=pinfold box CA <run id prefix>`, random 128-bit serial,
  `basicConstraints` CA with path length 0, `keyUsage` keyCertSign.
- **One leaf, issued at start.** The routed host set is fixed at start,
  so on-demand issuance per SNI and a cache are not needed. One leaf
  with every routed host as a `dNSName` SAN, its own P-256 key,
  `extendedKeyUsage` serverAuth, signed by the CA. The CA key is then
  dropped. For the rest of the box's life pinfold holds only the leaf
  key, which can sign for exactly the routed names.
- **Validity.** `notBefore` one day before start, so a guest clock behind
  the host's still accepts it. `notAfter` 397 days after start: a
  detached project box has no bounded life, and a longer leaf is refused
  by some clients. A box that outlives it fails closed; a restart issues
  new ones.
- **Where keys live.** Owner memory only. Never on disk, in a run
  record, a log, argv, or the box. The CA and leaf certificates are
  public and are written like other generated configuration.
- **Encoding.** ring generates and signs; it does not encode X.509.
  Two options, open question 3:
  - New code: a fixed-template DER writer for these two certificates,
    about 150 lines. Every field is constant except names, serial,
    validity and keys. Every TLS client in the box verifies the output,
    so a mistake fails closed and the first CLI run shows it.
  - `rcgen` 0.14 with `default-features = false, features = ["ring"]`.
    New crates: `rcgen`, `yasna`, `time`, `time-core`, `deranged`,
    `powerfmt`, `num-conv` (cargo tree, 2026-11-26).
  Recommendation: new code. CLAUDE.md orders new code before a new
  dependency, and a fixed template does not need a general generator.
- **Box-side TLS server.** rustls `ServerConfig` (ring provider, already
  a dependency) with the leaf and its key. ALPN offers `http/1.1` only.
  A client that requires HTTP/2 (gRPC) fails the handshake: fail
  closed. No shipped agent requires it.

### Automatic trust in the box

No user step. When a box has a routed host:

- A generated read-only mount at `/etc/ssl/certs` holds
  `ca-certificates.crt`, the host's roots (the same `rustls-native-certs`
  set the proxy verifies upstreams with) followed by the box CA, and
  `pinfold-box-ca.pem`, the box CA alone.
- `NODE_EXTRA_CA_CERTS=/etc/ssl/certs/pinfold-box-ca.pem`.

Probe (b) shows this covers OpenSSL (curl), GnuTLS (git), Go (`gh`),
rustls with native roots (Codex) and Bun (Claude, Pi, `documents`' Bun).
Claude also reads the system store itself.

Consequences:

- The image's own `/etc/ssl/certs` is hidden for that box: its hash
  links and any CA a recipe added. Passthrough hosts are then verified
  against the host's roots, not the image's. The two are both Mozilla's
  set in practice; on macOS the host set also holds admin-trusted roots,
  which is what a host behind a corporate TLS proxy needs anyway.
- An image that keeps its bundle elsewhere (Fedora's `/etc/pki`) gets no
  trust and fails closed for routed hosts; its recipe or `env` can set
  `SSL_CERT_FILE` to the mounted bundle.
- The mount follows the generated-mount rules: a caller mount at or over
  `/etc/ssl/certs` is refused.

Trust is not a security control. Security lives in the binary: the
proxy decides what leaves and what credential it carries. A box process
that does not trust the CA, or that pins certificates, gets a failed
handshake and reaches nothing.

Alternative considered: init concatenates the image's bundle and the CA
into a tmpfs file and pinfold sets `SSL_CERT_FILE` and `GIT_SSL_CAINFO`
to it. It keeps the image's roots but needs guest-side code, a writable
path and two more reserved variable names, and still misses tools that
read the bundle path. Open question 2.

### Upstream side

Unchanged from today's `https` routes: the upstream is resolved and
checked like an allowlisted host, dialed over rustls with SNI and the
certificate checked against the host's roots from
`rustls-native-certs`. No `webpki-roots`. The client offers `http/1.1`
by ALPN.

### Requests inside TLS

After the handshake the proxy reads one request in origin form and
applies today's route rules: the parser, `Expect`, hop-by-hop removal,
the login credential policy, header injection, `Upgrade` dropped. An
absolute-form target inside TLS is 400 `malformed request`.

- **Host.** Regenerated from the route's target, as for a route today.
  The box cannot choose the backend behind a routed host's CDN name:
  this closes Host-header fronting for routed hosts.
- **One request per connection.** The proxy sends `Connection: close`
  upstream and closes the box's TLS connection after the response, as
  routes do today. Each upstream request already costs a TLS handshake
  today; the box-side handshake is local. Keep-alive waits for a
  measurement.
- **Responses** stream as received until the upstream closes, as today;
  SSE works unchanged.
- **Chunked request bodies.** Git chunks a request body larger than
  `http.postBuffer` (1 MiB by default): a large push. Today the proxy
  refuses chunked framing and pinfold raises Git's buffer. Proposed rule,
  for every request the proxy parses (plain HTTP and intercepted): a
  body is framed by one `Content-Length`, or by `Transfer-Encoding`
  whose only coding is `chunked`. Both, two of either, another coding,
  or a malformed chunk is `ambiguous framing` (400, or a closed
  connection once the head is forwarded). The proxy decodes the chunks
  and re-encodes them upstream as chunked, drops trailers, and never
  forwards the box's framing bytes. So the `postBuffer` setting goes.
  Open question 4.

### Log

An intercepted request's line names the method, the path without its
query and the upstream status, as a route's does today, and its reason
is `route`. The host is the real name. Handshake failures from the box
(no trust, no SNI) are logged `sni missing`/`malformed clienthello` as
today; a handshake the box aborts after the ClientHello gets no line.

### Builds

Linux image preparation uses the production proxy without routes, so
nothing is intercepted and no CA exists during a build.

## Threat model delta

Protects, newly:

- **Host-header fronting on routed hosts.** The proxy sets `Host`.
- **Foreign credentials on routed hosts.** Today `api.anthropic.com`
  is in the default allowlist, so the box can CONNECT to it with an
  attacker's API key and send project contents to the attacker's
  account; likewise `github.com` with an attacker's token when allowed.
  Under interception every request to a routed host has the box's
  credential headers, cookies and credential query names removed and the
  user's credential added. Writes at a routed host go only to what the
  user's credential reaches (or to an upstream's anonymous endpoints).
- **Audit.** Every request to a routed host is a log line with method
  and path, where passthrough logged only the host.
- **Followed absolute URLs** carry the user's credential through the
  proxy, never a box-held one.

New risk:

- **Leaf key.** In owner memory for the box's life; it signs only for
  the routed names, and only this box trusts its CA. The CA key exists
  only during start. A box that obtained either could only impersonate
  routed hosts to itself.
- **Parser surface.** rustls as a server on box-controlled bytes, and the
  chunked decoder. The request parser is today's.
- **Plaintext in pinfold.** Request and response bodies to routed hosts
  pass through pinfold's memory in clear. Nothing logs or keeps them.
- **The CA must stay box-only.** pinfold never adds it to any host trust
  store, and each box has its own.

"Not protected" lines that change:

- "CDN fronting beyond the SNI check" becomes limited to hosts that are
  not routed.
- "A login route gives every process in the box the host login's
  authority over every path at the selected origin" now names every
  host the route serves, and every request to them, including those a
  client makes on its own.
- "Allowlisted services that accept writes" stays; a routed host now
  accepts writes only as the user.

## Guarantees and tests (row level)

- **Row 2.** Unchanged. Its route control stays a host service. Plain
  HTTP to a routed host refused `route` earns no assertion: had it
  passed, the request would carry no credential and fail visibly.
- **Row 26.** Rewritten around real names. The box reaches the login
  fixtures through `api.anthropic.com`, `github.com`, `api.github.com`,
  `uploads.github.com` and `openrouter.ai` with `to` pointing at host
  fixtures, trusting the CA with no flags. Hardest case: the fixture
  returns an absolute `https://api.github.com/…` `Link` and an
  `https://uploads.github.com/…` `upload_url`; a client following them
  delivers only the host's Bearer on both (the e2e image has no `gh`). A Git
  push large enough to be chunked arrives with only the host's Basic.
  A request with `Host: example.com` on a routed host arrives at the
  fixture with the route's own Host. The existing header, query and
  `Connection` assertions move onto intercepted requests. Claude and Pi
  each complete a fixture exchange through the generated
  configuration with no base URL. Sabotages: skip credential removal on
  intercepted requests; pass the box's `Host` through.
- **Row 38.** The launch record's `routes` keys become real host names;
  no new assertion.
- No new row. Box trust breaking is visible at once (a TLS error), so it
  earns none. The keys' absence from disk is not observable from
  outside without the key, so it is a code-review property, not a test.

## Open questions for Adam

1. **Two routes for one host.** Allow when both carry the same
   credential, refuse otherwise as `spec` (Claude login plus Pi
   `anthropic-subscription` with one `from` works; Claude login plus Pi
   `anthropic` API key does not). Recommendation: yes, as written.
2. **Where the box's public roots come from.** Host roots in a mount over
   `/etc/ssl/certs` (written above), or the image's roots assembled by
   init with `SSL_CERT_FILE`/`GIT_SSL_CAINFO`. Recommendation: host
   roots; no guest code, one reserved variable.
3. **X.509 encoding.** A fixed-template DER writer, or `rcgen`.
   Recommendation: the writer, no new dependency.
4. **Chunked request bodies.** Accept everywhere the proxy parses a
   request, or only inside interception. Recommendation: everywhere, one
   parser and one rule; it removes `postBuffer`.
5. **GitHub redirect hosts.** `release-assets.githubusercontent.com`
   (assets) and `codeload.github.com` (archives) need passthrough.
   Recommendation: add them to the `adam` profile's allowlist, not to
   what a github login route implies; the binary stays a list of served
   hosts. The default allowlist is unchanged.
6. **Interception for `http` injecting routes.** They keep their plain
   name, since there is no TLS to terminate. Recommendation: keep.

## Not proposed

- Per-path or per-repository route rules. Interception makes them
  possible; no guarantee asks for them yet.
- Keep-alive and HTTP/2 to the box.
- Interception of hosts that carry no credential.

## Follow-up tickets (after a decision)

1. Proxy: box-side TLS server, CA and leaf at start, intercepted request
   path reusing the route code.
2. Proxy: chunked request framing.
3. Login routes: real-name serving, shared-host rule, generated
   configuration without base URLs, trust mount and variable.
4. e2e row 26 rewrite.
5. Profile `adam`: GitHub redirect hosts in the allowlist.
