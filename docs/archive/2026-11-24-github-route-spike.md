# GitHub through an injecting route: spike, 2026-11-24

Question: can the box use `gh` and `git` against GitHub with the token
held at the proxy, as model logins are, and not in `GH_TOKEN`? Prompted
by OpenShell (ai-openshell/openshell), which keeps credentials out of
the sandbox. It does this with placeholder values and TLS interception.

## Setup

pinfold 0.7.0 on macOS, the `adam-local` profile image (`gh` 2.105.0).
Two boxes from `pinfold box up`, with injecting routes carrying the
`aa-agent` fine-grained token. In both boxes the two GitHub token
variables held placeholders: `GH_TOKEN=placeholder` and
`GH_ENTERPRISE_TOKEN=placeholder`. No allowlist.

- Box 1: `api.github.localhost` → `https://api.github.com` and
  `github.localhost` → `https://github.com`, both `Authorization: Bearer`
  with the profile's `op` reference.
- Box 2: `github.localhost` → `https://github.com` with `prefix = "Basic "`
  and a `from` variable holding `base64("x-access-token:" + token)`,
  computed on the host from the same `op` reference.

## Results

| Check | Result |
|---|---|
| `curl http://api.github.localhost/user` | 200, login `aa-agent` |
| `GH_HOST=github.localhost gh api user` | `aa-agent` |
| `gh pr list -R github.localhost/adamaltmejd/pinfold` | ran, no error |
| `gh issue view 1 -R …` (GraphQL) | title returned; `GH_DEBUG=api` shows `http://api.github.localhost/graphql` |
| `git ls-remote` over `github.localhost`, Bearer | 401; git asks for a username |
| Same, Basic | HEAD returned |
| `info/refs?service=git-receive-pack`, Basic | 200 |
| `git clone --depth 1`, Basic | cloned |
| `git push --dry-run` to a new branch, Basic | would create the branch |
| Token in the box's environment | absent |

A summary of selected egress log entries from both boxes, without
queries:

| Method | Host | Path | Status |
|---|---|---|---|
| GET | api.github.localhost | /user | 200 |
| POST | api.github.localhost | /graphql | 200 |
| GET | github.localhost | /adamaltmejd/pinfold.git/info/refs | 401 in box 1 (Bearer), 200 in box 2 (Basic) |
| POST | github.localhost | /adamaltmejd/pinfold.git/git-upload-pack | 200 |

## Findings

- With `GH_HOST=github.localhost`, `gh` sent plain HTTP to the route:
  `gh api user` appears in the log as `GET /user` on
  `api.github.localhost`, and `GH_DEBUG=api` showed GraphQL going to
  `http://api.github.localhost/graphql`. This is a special case in
  `gh`'s client for that host name. It lets `gh` use a route with no
  TLS interception.
- For the tested git request, `Bearer` got 401 and `Basic` worked.
  pinfold can send `Basic ` before a value encoded on the host, as box 2
  did, but `prefix` cannot derive that value from the raw token. Hence
  #457.
- Git requests carry the repository in the path. The fetch transfer was
  `POST …/git-upload-pack`; a push transfer is `POST …/git-receive-pack`
  (Git's HTTP protocol, gitprotocol-http). Fetch and push discovery share
  `GET …/info/refs` and differ only by the `service` query. The spike
  observed the fetch POST and exercised push discovery and a dry run,
  not a push transfer.
- `gh`'s GraphQL requests share one path, `POST /graphql`. Their bodies
  name repositories by owner and name or refer to objects by node ID, so
  restricting them by repository means reading and resolving the body.
  A fine-grained token can enforce read-only and per-repository limits on
  GitHub's side when it is configured that way; the spike recorded
  neither the token's settings nor a denial testing them. Method and path
  rules were not filed.

Not tested: `gh` resolving the repo from a remote rewritten by
`insteadOf` without `-R`, and release downloads that redirect to other
hosts.
