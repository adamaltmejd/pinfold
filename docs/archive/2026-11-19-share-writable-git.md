# Share with a writable .git: host commits against a committing box

Date: 2026-11-19. Issue #334, "Before the code". Follows
`2026-11-19-shared-git-probe.md`. Scripts and raw logs:
`2026-11-19-share-writable-git/`.

## Probe: host commits with GIT_OPTIONAL_LOCKS=0, box commits in a loop

The question: does a host that commits with optional locks off damage
commits while a box commits in the same shared repository.

Environment (`logs/env.log`): macOS 27.2.1 (26B131), arm64, Apple
`container` 1.7.1, host git 2.56.1, guest git 2.47.3, guest kernel 6.18.15,
share `virtiofs /work virtiofs rw,relatime`. Image
`localhost/pinfold/profile-default:latest` `9b5df3f99e34`, the same as the
earlier probe. The box is raw `container run` with a writable share, as
before: `container run -d --name pfs-hc1119 --user 501:20 -v ROOT:/work
--entrypoint sleep IMAGE infinity`. `run.sh` starts it, runs `hc.py 3 30`,
and removes the box and the temp directory. `lib.py` is the 2026-11-12 file.

Shape, as case 4: three fresh repositories (`c0`, `c1` touching `a` and
`b`), 30 host iterations each of
`echo N >> mine; git status --porcelain; git add mine && git commit` with
`GIT_OPTIONAL_LOCKS=0` exported and no global config. The box runs one
`container exec` loop per repository: append to `theirs`, `git status
--porcelain`, `git add theirs && git commit`, sleep 0.1 s, until the host
creates a stop file. The box loop starts 1 s before the host. A status line
is anomalous unless it names `mine` or `theirs` with a code other than `D`.

Results (`logs/hc.jsonl`, summary lines):

| Repo | Host anomalous | Host refusals | Host commits exit 0 | Host commits on `main` | Box iterations | Box anomalous | Box commits exit 0 | Box commits on `main` | Commits on `main` | Final tree on `main` |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 30/30 | 5 | 25 | 1 | 14 | 7 | 13 | 1 | 2 | `mine theirs` |
| 2 | 29/30 | 2 | 28 | 0 | 14 | 7 | 9 | 1 | 1 | `theirs` |
| 3 | 26/30 | 6 | 24 | 1 | 13 | 5 | 12 | 1 | 2 | `mine theirs` |

Host anomalies count a refused commit or an odd status line. Anomalies
carry forward, as in case 4. Host status lines over all iterations:
`?? a` and `?? b` 82 times each, `D  mine` 9, `D  a` and `D  b` 3 each.
Box status lines: `?? a` and `?? b` 19 each, one `D  a`, `D  b`,
`D  theirs`. The host's first anomalous iterations were 0, 1 and 0; the
box's 8, 8 and 9. In repository 2 the host's iteration 1 status was
`D  a`, `D  b`, `D  mine`, `M  theirs`, `?? a`, `?? b`, `?? mine` and its
commit exited 0.

Host refusals: 13 `index.lock: File exists`, and 2 `cannot lock ref 'HEAD':
is at X but expected Y`. One box refusal named `index.lock`; five box
refusals in repository 2 logged no stderr.

At the end, in all three repositories: `main` had one root commit and one or
two commits in total, against 38, 37 and 36 commits that exited 0 plus
the two setup commits. `a` and `b`
were gone from `main` and untracked on the host. In repository 2 `mine` was
gone from `main` too. The reflog of `main` held 33 to 37 entries. Nothing was
staged and `fsck` exited 0 with dangling commits and blobs.

Result: with optional locks off on the host, concurrent host and box
commits still damage commits. Commits exit 0 and are then lost from the
branch, and commits record tracked files as deleted. Only the reflog and
dangling objects keep them.

## Not tested

- Host commits paced the way a person commits (seconds or minutes apart).
- Host commits with optional locks on, against a committing box.
- Box loop paces other than 0.1 s, or a box that commits once.
