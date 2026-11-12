#!/usr/bin/env python3
"""Case 7: a linked worktree at ./wt inside the repository root.

Usage: c7.py TRIALS LOG
Needs a box that mounts ROOT at its host path (PFS_GROOT=ROOT), because a
host `git worktree add` records absolute host paths. Each trial: the guest
primes both checkouts, one side commits, and the other side commits at once.
  main-then-wt: host commits in main, guest commits in wt (branch wtb).
  wt-then-main: host commits in wt, guest commits in main.
"""
import sys
from lib import *

N, LOG = int(sys.argv[1]), sys.argv[2]
PRIME = "git status --porcelain >/dev/null; git log -1 >/dev/null"
C = "echo {w} >> {f} && git add {f} && git commit -qm {w}; echo rc=$?"

for order in ("main-then-wt", "wt-then-main"):
    for t in range(N):
        repo = new_repo("c7")
        rc, o = h(repo, "git worktree add -q ./wt -b wtb && echo wt/ >> .git/info/exclude")
        assert rc == 0, o
        g(repo, PRIME); g(repo + "/wt", PRIME)
        hdir, gdir = (repo, repo + "/wt") if order == "main-then-wt" else (repo + "/wt", repo)
        hrc, ho = h(hdir, C.format(w="host", f="hfile"))
        grc, go = g(gdir, C.format(w="guest", f="gfile"))
        main = h(repo, "git log --format=%s main")[1].split()
        wtb = h(repo, "git log --format=%s wtb")[1].split()
        hb, gb = (main, wtb) if order == "main-then-wt" else (wtb, main)
        s = host_state(repo)
        rec = dict(order=order, repo=repo, host=ho[-200:], guest=go[-300:], main=main, wtb=wtb,
                   status_main=s["status"], status_wt=h(repo + "/wt", "git status --porcelain")[1],
                   fsck_rc=s["fsck_rc"], fsck=s["fsck"][-300:])
        rec["clean"] = (ho.endswith("rc=0") and go.endswith("rc=0") and hb == ["host", "c1", "c0"]
                        and gb == ["guest", "c1", "c0"] and not s["status"] and not rec["status_wt"]
                        and s["fsck_rc"] == 0)
        out(LOG, rec)
