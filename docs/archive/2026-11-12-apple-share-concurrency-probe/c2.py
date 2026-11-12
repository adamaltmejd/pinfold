#!/usr/bin/env python3
"""Case 2: guest commit, then host commit after a delay.

Usage: c2.py TRIALS LOG DELAY...
"""
import sys, time
from lib import *

N, LOG = int(sys.argv[1]), sys.argv[2]
for d in [float(x) for x in sys.argv[3:]]:
    for t in range(N):
        repo = new_repo("c2")
        g(repo, "git status --porcelain >/dev/null; git log -1 >/dev/null")
        grc, gout = g(repo, "echo g > gfile; git add gfile; git commit -qm guest")
        G = rev(repo)
        time.sleep(d)
        hrc, hout = h(repo, "echo host >> a; echo h > hfile; git add a hfile; git commit -qm host")
        H = rev(repo)
        s = host_state(repo)
        rec = dict(delay=d, repo=repo, guest_rc=grc, guest=gout[-300:], host_rc=hrc, host=hout[-300:],
                   guest_kept=grc == 0 and is_anc(repo, G, "main") and G != "",
                   host_kept=hrc == 0 and is_anc(repo, H, "main"),
                   tree=s["tree"], fsck_rc=s["fsck_rc"], fsck=s["fsck"][-300:],
                   log=s["log"], status=s["status"])
        rec["clean"] = (rec["guest_kept"] and rec["host_kept"] and len(s["log"]) == 4
                        and sorted(s["tree"]) == ["a", "b", "gfile", "hfile"]
                        and not s["status"] and s["fsck_rc"] == 0)
        out(LOG, rec)
