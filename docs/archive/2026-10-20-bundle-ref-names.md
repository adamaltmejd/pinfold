# Bundle head names reach git unchecked (2026-10-20)

Found by the Opus review of #152, reproduced with host git 2.56.

- `git bundle verify` and `git bundle list-heads` do not check head
  names. A box that writes its own bundle bytes can name a head
  `refs/heads/x:refs/heads/injected`. Used as a fetch source
  `refs/heads/<name>`, it becomes a refspec with a destination, and
  `git fetch` creates `refs/heads/injected` on the host, exit 0.
- A `refs/replace/<host main>` head fetched the same way makes the host's
  `git log main` show the box's commit.
- `update-ref --stdin` without `-z` reads `update REF NEW` with NEW as a
  revision, so a listed name `y refs/heads/other` set the remote ref to
  the host's `refs/heads/other`.
- The listing had no bound on its line count, and each omitted branch
  cost one host `git rev-list`.

Fix: every name passes `git check-ref-format` as
`refs/remotes/NAME/<branch>` before any git command sees it, ref updates
use `update-ref -z`, and a box with more than 4096 branches fails. Row 10
gained the hostile-bundle case (Adam, 2026-10-20). Dropping the name
check lands `refs/heads/injected` in the test.

Not addressed: a pack under 4 GiB with deep deltas can drive host
`index-pack` memory high. That is denial of service only.
