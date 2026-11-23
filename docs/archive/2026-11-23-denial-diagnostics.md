# Denial diagnostics in real clients (2026-11-23)

Issue #333, first half. The proxy's own refusal responses now carry
`Proxy-Status: pinfold; error=TOKEN; details="REASON"` and a short
`text/plain` body naming the same reason. This records what clients in
the box show for a denied request.

## Setup

A box on macOS (Apple `container`) from the existing local
`localhost/pinfold/profile-default:latest` image, no build, started with
the branch's debug binary in a throwaway state root, allowlist
`api.github.com`. Each client asked for `https://example.com/` (CONNECT,
refused `not allowlisted`) and `http://example.com/` (plain HTTP, same
reason). The image has curl 8.14.1 and git. It has no `node` or
`python3`; `pi` is a Bun 1.4.4 executable, so `BUN_BE_BUN=1 pi -e`
ran Bun's `fetch`, which is what a Bun-built agent tool uses.

## What each client shows

| Client | CONNECT (HTTPS) denial | Plain HTTP denial |
|---|---|---|
| `curl -sS` | `curl: (56) CONNECT tunnel failed, response 403`. Header and body not shown. | Prints the body: "The pinfold sandbox refused this request: not allowlisted." plus the host-side line. Exit 0. |
| `curl -v` | Shows the CONNECT response head, including `< Proxy-Status: pinfold; error=http_request_denied; details="not allowlisted"`, then `* CONNECT tunnel failed, response 403`. The 173-byte body is not shown. | Shows the head with `Proxy-Status`, then the body. |
| `curl -D -` | Dumps the CONNECT response head, including `Proxy-Status`. | Same. |
| `git ls-remote` | `fatal: unable to access '…': CONNECT tunnel failed, response 403`. `GIT_TRACE_CURL=1` shows the `Proxy-Status` header. | `remote: The pinfold sandbox refused this request: not allowlisted.` and the host-side line, then `fatal: … The requested URL returned error: 403`. |
| Bun `fetch` | Throws `ERR_PROXY_TUNNEL: Proxy refused to open a tunnel: 403 Forbidden`. Header and body not reachable. | Status 403; `headers.get("proxy-status")` and `text()` return the header and body. |

Other reasons seen through `curl -D -`: a CONNECT to `1.1.1.1` gave
`error=destination_ip_prohibited; details="ip literal"`, plain HTTP to
port 8080 `details="port not allowed"`, `Expect: foo` a 417 with
`error=http_request_error; details="expectation unsupported"`. Each
`details` equalled the egress log line's `reason`. A plain request to
`api.github.com` came back with no `Proxy-Status`.

## Clients that discard proxy diagnostics

Every client above drops the body of a refused CONNECT, and only a
verbose or header-dumping mode shows its `Proxy-Status`. The default
output of curl, git and Bun for HTTPS still reads only "CONNECT tunnel
failed, response 403" or its equivalent. Most agent traffic is HTTPS, so
the reason reaches an agent by default only for plain HTTP; for HTTPS it
needs `curl -v`/`-D -` or the egress log on the host.

Not run: Node's `fetch` with `NODE_USE_ENV_PROXY=1` and Python `urllib`,
which the default image lacks. Read from their current source on
GitHub (main branches, 2026-11-23):

- undici's `ProxyAgent` throws `RequestAbortedError` "Proxy response
  (403) !== 200 when HTTP Tunneling". It attaches neither headers nor
  body and destroys the socket.
- CPython's `http.client` raises `ProxyTunnelError`, an `OSError`
  "Tunnel connection failed: 403 Forbidden", which `urllib` wraps in a
  `URLError`. On main the error keeps the CONNECT response headers as
  `.headers`, so `Proxy-Status` is reachable by code that looks; the
  printed message omits it. Not checked which release added that.
