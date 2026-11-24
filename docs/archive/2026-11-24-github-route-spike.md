# GitHub through an injecting route: spike, 2026-11-24

Question: can the box use `gh` and `git` against GitHub with the token
held at the proxy, as model logins are, and not in `GH_TOKEN`? Prompted
by OpenShell (ai-openshell/openshell), which keeps credentials out of
the sandbox. It does this with placeholder values and TLS interception.

## Setup

pinfold 0.7.0 on macOS, the `adam-local` profile image (`gh` 2.105.0).
Two boxes from `pinfold box up`, each with injecting routes reading the
profile's existing `op` GitHub reference (the `aa-agent` fine-grained
token). Box env held only `GH_TOKEN=placeholder` and
`GH_ENTERPRISE_TOKEN=placeholder`. No allowlist.

Routes:

- `api.github.localhost` → `https://api.github.com`, `Authorization: Bearer`.
- `github.localhost` → `https://github.com`, first `Bearer`, then
  `Basic` with a host-computed `base64("x-access-token:" + token)`
  passed by `from`.

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

The egress log for the routes, without queries:

| Method | Host | Path | Status |
|---|---|---|---|
| GET | api.github.localhost | /user | 200 |
| POST | api.github.localhost | /graphql | 200 |
| GET | github.localhost | /adamaltmejd/pinfold.git/info/refs | 401 Bearer, 200 Basic |
| POST | github.localhost | /adamaltmejd/pinfold.git/git-upload-pack | 200 |

## Findings

- `gh` sends plain HTTP for the host `github.localhost`, REST to
  `http://api.github.localhost/` and GraphQL to `/graphql`. This is a
  built-in special case for local development that GitHub does not
  document. It lets `gh` use a route with no TLS interception.
- GitHub's git endpoint needs Basic auth. An injecting header's
  `prefix` plus value cannot produce it. Hence #457.
- Path rules can separate git operations: each repo has its own path
  prefix, a push is `POST …/git-receive-pack` and a fetch is
  `…/git-upload-pack`. Every `gh` pr and issue command is one
  `POST /graphql`. Restricting those by repo needs the GraphQL body,
  and objects are named by opaque global IDs. A fine-grained token
  already enforces read-only and per-repo limits on GitHub's side, so
  method and path rules were not filed.

Not tested: `gh` resolving the repo from a remote rewritten by
`insteadOf` without `-R`, and release downloads that redirect to other
hosts.
