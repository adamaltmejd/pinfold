# TLS interception build evidence (#475)

Evidence gathered while building #475 on the Mac (Apple `container`
1.7.1, image `profile-adam`). The design is in
`2026-11-26-tls-interception-design.md`; Adam's decisions on PR #476 are
binding where they differ from it.

## Certificates

The CA and leaf are hand-written DER signed with ring (P-256). A scratch
harness issued a pair and checked:

- `openssl verify -x509_strict -CAfile ca.pem leaf.pem`: OK.
- Python 3.15's default context, which sets `VERIFY_X509_STRICT`,
  completed a handshake against a server using the leaf.

Rules the strict checks need: positive serial (top bit clear), long-form
lengths past 127 bytes, `basicConstraints` critical on both, CA with
path length 0, the leaf with an empty `basicConstraints` (CA false),
`keyUsage` critical, SKI on both and AKI on the leaf.

## Trust delivery

- Apple `container` mounts a single file only through
  `-v SRC:DST:ro`; `--mount` takes directories. Mounting the whole
  `/etc/ssl/certs` was ruled out by the decisions.
- The image's own bundle is read by a helper box (`--network none`,
  read-only, no capabilities, host uid) running `init bundle`. The host's
  roots are not exported; the image's bundle is used, so no host-roots
  export is needed on macOS.
- On Linux PID 1 writes the bundle into the overlay root before the
  read-only remount. The CA travels in the BOOT frame, so no attachment is
  added.

## Startup cost

`box up` to `ready` on the Mac, profile-adam, three runs each:

| Box | Seconds |
|---|---|
| No routed host | 0.97, 0.94, 1.01 |
| A github login route | 1.41, 1.38, 1.50 |

The helper costs about 0.45 s, paid only by boxes with a routed host. A
cache keyed by image digest was considered and declined (operator,
2026-11-26): a box start is paid once per project session, since
commands join a running box, and the cache would add a new host path and
a cleanup rule.

## GitHub redirect hosts

Enumerated by running gh 2.x through the proxy and logging CONNECTs:

| Command | Redirect host |
|---|---|
| `gh release download` (public repo) | `release-assets.githubusercontent.com` |
| `gh release download --archive` | `codeload.github.com` |
| `gh run download` | `productionresultssa<N>.blob.core.windows.net`, varying |

No private repository with releases was available, so private-repo
redirects were not observed.

## Mac CLI checks

In a box with only a github login route and `pi.dev` allowed:

- `gh api /user`, `gh api --paginate` (479 items), `git ls-remote`.
- Draft release create, 3 MB asset upload, `--clobber` re-upload,
  download, delete. All went through `uploads.github.com` as `route`.
- A 3 MB `git push` of a probe branch, fetch, branch delete.
- `gh run download`: refused at the blob host, as expected.

Clients in a probe image built from profile-adam:

| Client | `api.github.com` (routed) | `pi.dev` (passthrough) |
|---|---|---|
| Python `urllib` | 200 | 200 |
| Python `requests` | 200 | 200 |
| Ruby `Net::HTTP` | 200 | 200 |
| Java `HttpClient` | PKIX failure | 200 |

Java's failure was logged `closed`, reason `tls handshake`.

Agents at their real hosts with no base-URL override, on the host's
Codex login:

- `codex exec`: answered; requests logged at `chatgpt.com` as `route`.
- `pi --provider openai-codex --model gpt-5.6-luna`: answered.

## Linux checks

Scope, set by the operator: no credential goes to the dev VM. The auth
and route logic is shared between runtimes; Linux differs only in how
trust reaches the box. So Linux was checked for transport only, and
Linux CI's row 26 covers the login routes through fixtures at the real
host names.

On the dev VM (Debian 13, Firecracker), a probe image (Debian trixie
with curl, Python, `requests`, Ruby and a JDK) built with `pinfold image
build`, and one injecting route to `https://api.github.com` carrying a
non-secret header, `pi.dev` allowed:

- The three variables name `/etc/ssl/certs/ca-certificates.crt`: the
  image's 148 roots plus the CA, root-owned, not writable by box work.
  The rest of `/etc/ssl/certs` is the image's.
- `PINFOLD_ALLOW` was `pi.dev,api.github.com`.
- curl, Python `urllib`, `requests` and Ruby: 200 at `api.github.com`
  (logged `route`) and at `pi.dev` (passthrough).
- Plain HTTP to `api.github.com`: 403 `route`.
- Java: 200 at `pi.dev`; handshake failure at `api.github.com`, logged
  `closed`, reason `tls handshake`.
