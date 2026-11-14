# Codex host helper and image version

The helper and box agent have no direct protocol or version handshake.
The pin is Codex 0.167.1. The host artifact is
`codex-app-server-aarch64-apple-darwin.tar.gz`, SHA256
`d8ccd88f64b776bbf8d330a46a0d2d80eae7580dcd981e0f97070eae58a4bf63`.
The downloaded fixture artifact matched that checksum.

`crates/pinfold/src/core/login.rs:96-115` installs that host artifact and
launches `codex-app-server -c model_provider="openai"`. Lines 141-218
send JSON lines on stdio:

```json
{"method":"initialize","id":0,"params":{"clientInfo":{"name":"pinfold","title":"pinfold","version":"<pinfold version>"}}}
{"method":"initialized"}
{"method":"getAuthStatus","id":1,"params":{"includeToken":true}}
```

pinfold ignores responses and notifications other than id 1. It reads
`result.authToken` and `result.authMethod`. It bounds one line to 64 KiB,
all output to 256 KiB, and the helper/lock wait to 30 seconds. It keeps
stdin open until the response and reaps the helper on every exit.
`getAuthStatus` is deprecated upstream; the source comment explicitly
names removal from the pin as the revisit trigger.

`login.rs:222-249` requires a header-safe JWT, future `exp`, and a nonempty
ASCII-graphic `https://api.openai.com/auth.chatgpt_account_id` claim.
pinfold does not check the JWT signature; the upstream service must.
`Codex::headers` emits `Authorization: Bearer <token>` and
`ChatGPT-Account-ID: <account>` to the upstream. Refresh asks start within
five minutes of expiry, at most once per minute per box. A host-wide lock
serializes asks across state roots.

The image agent is configured separately: `login.rs:280-294` writes a
custom provider whose `base_url` is
`http://<route>/backend-api/codex`, `wire_api="responses"`, and
`requires_openai_auth=false`. The image agent never talks to the helper.
Thus equality of the image's Codex version and the helper's pin is not a
requirement in pinfold. Compatibility is coupled indirectly to (1) the
helper reading the host's current authentication store, (2) the pinned
helper retaining the JSON method and field names, (3) the token retaining
the claims pinfold requires, and (4) the image agent's requests and the
upstream continuing to work with the route's two injected headers and
Responses path. This probe does not prove arbitrary future versions work.

## Offline fixture

Files in this evidence directory: `helper-fixture.py`,
`helper-fixture-output-enoent.txt`, and the first blocked run's
`helper-fixture-output.txt`. The retained fixture scratch is `/tmp/pfh.298`. No real login was used. The only authentication file
read by this fixture is one it creates under its fresh scratch home.

Command: `python3 /tmp/pfh.298/helper-fixture.py`.
The script launches the checksum-verified helper with a fresh HOME and
CODEX_HOME, an explicit file-only credential store, and macOS
`sandbox-exec` rules denying all network and all reads below `/Users`.
The latter deny returns ENOENT. The environment is supplied explicitly,
so no credential-bearing host variables are inherited. Each helper runs
with a scratch cwd and is terminated and reaped after id 1.

The first run denied /Users reads with EPERM and produced no id-1 response.
Its retained stdout records that outcome but not the underlying stderr.
During the probe the failure was diagnosed as a denied system-config
symlink target; that diagnostic was not retained and is not independently
established by the stdout file. The final fixture makes denied host-home
files appear absent with sandbox errno 2. It changes no host configuration.

Observed with an empty fresh home:

```json
{"id":1,"result":{"authMethod":null,"authToken":null,"requiresOpenaiAuth":true}}
```

Observed with an unsigned fixture ChatGPT token, future expiry, a fixture
account claim and fresh `last_refresh`:

```json
{"id":1,"result":{"authMethod":"chatgpt","authToken":"<the exact fixture JWT>","requiresOpenaiAuth":true}}
```

The script printed `matches fixture token=True` and `helper reaped`.
Initialize identified helper 0.167.1. A remote-control disabled
notification was ignored as pinfold does. The fixture output replaces
that notification's machine name with `fixture-host`.

This establishes that the pinned real helper still implements the
protocol pinfold expects and exports the fixture token without a model
request or guest binary. Network access was denied at the OS boundary.
It does not establish token refresh, real subscription authentication,
keychain behavior, backend acceptance, or compatibility with a guest
version other than the ones actually exercised by the main probe.

## Primary upstream sources

- Pinned v1 JSON fields:
  https://github.com/openai/codex/blob/rust-v0.167.1/codex-rs/app-server-protocol/src/protocol/v1.rs
  (`InitializeParams`, `GetAuthStatusParams`, `GetAuthStatusResponse`).
- Pinned handler:
  https://github.com/openai/codex/blob/rust-v0.167.1/codex-rs/app-server/src/request_processors/account_processor.rs
  (`get_auth_status_response`, lines 1097 onward).
- Fixture file schema:
  https://github.com/openai/codex/blob/rust-v0.167.1/codex-rs/login/src/auth/storage.rs
  (`AuthDotJson`) and
  https://github.com/openai/codex/blob/rust-v0.167.1/codex-rs/login/src/token_data.rs
  (`TokenData`).

No changes to product code, tests, host configuration, images or shared
runtime were made.

The current [official app-server documentation](https://learn.chatgpt.com/docs/app-server)
was also read. It describes the JSON-lines initialization and account APIs.
The deprecated method used here is established by the pinned source and
real helper fixture, rather than inferred from current documentation.
