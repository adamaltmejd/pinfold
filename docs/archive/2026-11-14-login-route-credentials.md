# Login-route credential policy, PR 1 of #278

Implementation: `2fbad3d`, on `login-route-credentials-287`. This is PR 1
only. It implements #287 without changing runtime or agent configuration.
No dependency or test was added. Nothing was pushed or released.

`Upstream::Inject` retains whether it is a login route. Static logins and
Codex apply the same bounded policy. Validation runs before credential
refresh, DNS or an upstream connection. Guest fields are filtered before
trusted authentication is appended. Provider-specific strip tables and
the three-element resolved-login alias were deleted. The architecture and
README describe the resulting contract and its reflection limit.

## Reproduction

Build and toolchain commands, outputs and source hashes are in
[checks.txt](2026-11-14-login-route-credentials/checks.txt).
The probe uses the real binary, actual login routes and an HTTP fixture
on the host. Guest requests use existing `sh` and `curl`; Python runs only
on the host. Ordered header occurrences and credential values are compared
in memory. Output contains counts and comparisons, never credential values.

```sh
RUSTC_WRAPPER= cargo build --locked
RUSTC_WRAPPER= cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
python3 docs/archive/2026-11-14-login-route-credentials/probe.py target/debug/pinfold
```

On the existing leased Linux VM, transfer the source patch and the two
probe scripts as base64 through `labvm exec -- bash -lc '...'`. Apply the
patch with `git apply --check` and `git apply`. No Git push is used.
Run the long job detached with `setsid nohup`, then poll its output through
the same `labvm exec` interface:

```sh
cargo build --locked --target x86_64-unknown-linux-musl
cargo clippy --all-targets --locked --target x86_64-unknown-linux-musl -- -D warnings
python3 /tmp/pfc-probe/probe.py target/x86_64-unknown-linux-musl/debug/pinfold localhost/pinfold/profile-probe-linux294:latest
readelf -l target/x86_64-unknown-linux-musl/debug/pinfold
```

The driver creates a short `/tmp/pfc.*` scratch root, isolated config and
state, a fresh maintenance stamp before its first pinfold command, and a
synthetic file-backed `CODEX_HOME`. It retains the real artifact cache.
Boxes use two CPUs and 1 GiB. Their home disks and names belong to this
probe. The real pinned Codex helper is never replaced. No real credential
store is read.

## Cases

- Every login form receives mixed-case duplicates of all eleven named
  credential headers and a `Proxy-` field, including comma-separated
  values. Repeated `Connection` fields nominate authentication, `Host`
  and ordinary metadata. An independent case omits nominations. The
  upstream must receive only the ordered trusted authentication fields.
- The hardest POST carries a binary nonempty body, an arbitrary non-model
  path, repeated ordinary metadata, `anthropic-version` and
  `anthropic-beta`. The fixture compares their exact values and order.
  A separate nomination removes beta while retaining version.
- Each login form makes 512 query requests: sixteen listed names, four
  spellings, two separators and four field forms. They cover literal,
  case, underscore/hyphen and percent-encoded names; values, empty values,
  absent `=` and duplicates.
  Each refusal requires its audit reason and no upstream request.
- Invalid percent escapes in names and values, including after a forbidden
  name, malformed Connection options, and nominated Content-Length require
  their specified 400 reasons. Each has a successful corrected request in
  the same box. Harmless query bytes are preserved. Double encoding is
  decoded once and `+` in a name means a space.
- Ordinary injecting and host-service routes accept credential queries,
  malformed query escapes and Connection options as before. The injecting
  route replaces its configured Authorization field; the host service
  receives the guest authentication unchanged.
- Codex and Pi share the real helper and local OAuth fixture. A near-expiry
  token enters the production refresh window after the 60-second ask
  interval. Invalid requests must refuse before asking the helper. A
  corrected request asks the helper; both routes receive the changed token.
  The second route must cause no additional OAuth exchange.
  Valid controls precede expiry; after expiry, both routes must refuse
  with `login lapsed` and no upstream request.
- Guest environment, home and generated login files are captured in host
  memory and scanned for synthetic host markers. Live generated state,
  diagnostics and audit are scanned too. Guest disk images remain opaque;
  their contents are inspected from the guest. A separate reflecting
  fixture response returns the host fields to the guest and compares its
  digest, demonstrating the documented limit.

The supported helper fixture endpoint is
`CODEX_REFRESH_TOKEN_URL_OVERRIDE`. Its production implementation and JSON
refresh flow are in the pinned
[Codex auth manager](https://github.com/openai/codex/blob/rust-v0.167.1/codex-rs/login/src/auth/manager.rs#L1872-L1879).
The helper's
[getAuthStatus handler](https://github.com/openai/codex/blob/rust-v0.167.1/codex-rs/app-server/src/request_processors/account_processor.rs#L1097-L1124)
requests authentication through that manager. These links explain the
fixture setup; runtime outcomes below are the evidence.

## Outcomes and limits

The full matrix passed on both platforms. Each login form produced 14
accepted requests, eight 400 refusals and 512 credential-query 403s.
All 520 refusals per form had their required audit reason and zero
upstream requests. Both ordinary routes accepted their two controls.
The shared box recorded 3,208 decisions and 88 upstream requests; the
subscription box recorded 534 decisions and 14 upstream requests.

| Login | Ordered upstream authentication occurrences |
|---|---|
| Claude | Authorization: 1 |
| Codex | Authorization: 1; ChatGPT-Account-ID: 1 |
| Pi openai-codex | Authorization: 1; ChatGPT-Account-ID: 1 |
| Pi openai | Authorization: 1 |
| Pi openrouter | Authorization: 1 |
| Pi anthropic | X-Api-Key: 1 |
| Pi anthropic-subscription | Authorization: 1 |

The initial full runs failed two probe assertions that assumed exactly
one OAuth POST per helper lookup. Request behavior, shared token
comparisons, early validation and expiry refusal reasons passed.
[initial-refresh.txt](2026-11-14-login-route-credentials/initial-refresh.txt)
retains the failure excerpts. That count assumption is stronger than the
plan. The corrected probe measures the helper's OAuth count and requires
that Pi's shared request and refused requests add no exchanges. It also
checks the fixture's refresh-token chain. No production change followed
from this probe failure.

The final full runs passed with zero failures:

| Platform | Final run | OAuth fixture exchanges | Extra exchanges from Pi or refusals |
|---|---|---|---|
| macOS, Apple container 1.7.1 | 101.022 s, exit 0 | 5, every rotating grant matched | 0 |
| Linux x64, Firecracker 1.17.0 / guest kernel 6.18.57 | 156.598 s, exit 0 | 3, every rotating grant matched | 0 |

Raw bounded output is in [macos.txt](2026-11-14-login-route-credentials/macos.txt)
and [linux.txt](2026-11-14-login-route-credentials/linux.txt).
Each platform exercised 3,752 proxy requests in its final run. The real
helper refreshed through the local fixture. Both routes received the same
final host-issued token and account. Four valid-token controls succeeded;
the two expired-token refusals and four early-validation refusals reached
neither the model fixture nor another refresh. All base and harness exec
environment, home, generated-file, live-state, diagnostic and audit marker
checks passed. Reflection returned host fields as the spec permits.

Pinfold was 0.5.5. Pins were Pi 1.5.2, Claude 2.1.322 and Codex/helper
0.167.1. Rust and Cargo were 1.99.1. Build, formatting and Clippy passed on
both platforms. Linux used the required musl target and had no ELF
interpreter. The Mac build disabled the sandbox-incompatible sccache wrapper;
this did not change the build target or source.

No plan departure or missing PR 1 acceptance case remains. These probes
establish HTTP credential policy and synthetic OAuth behavior. They do not
establish live-provider inference or real-account refresh. TLS behavior,
helper contention and cancellation were unchanged and were not re-exercised.

## Cleanup

No image was built, rebuilt, removed or pruned. Used existing images:
`localhost/pinfold/profile-probe-agents:latest` on Mac and
`localhost/pinfold/profile-probe-linux294:latest` on Linux.
Both first and final runs stopped their boxes and removed their home disks.
Final inventory found zero own boxes on either host and zero own Apple
volumes. The first Linux inventory command omitted the required `--label`
filter and refused; the corrected inventory succeeded. Owned scratch roots,
temporary probe scripts, patch transfer and logs were removed after copying
the evidence. Verified artifact downloads remain in the normal cache.
The VM clone retains the applied source patch and its musl build. The lease
was neither released nor replaced; Linux finished before 20:00 UTC, well
before the 00:28:50 UTC stop boundary. The shared Apple service was already
running and remains running.

## Caller changes

Login routes remove additional guest credential headers and all `Proxy-`
fields. They remove Connection-nominated guest fields before injecting
trusted authentication. Named credential queries get 403; malformed login
queries and Connection options get 400; nominated Content-Length gets 400.
There is no CLI, box-spec or JSON-line schema change.
