#!/usr/bin/env python3
"""Case 3: host and guest commit different files at about the same time.

Usage: c3.py REPS LOG OFFSET...
OFFSET is the host start minus the guest exec launch, in seconds; a guest
exec takes about 40 ms to start, so offsets sweep the overlap.
"""
import subprocess, sys, time
from lib import *

N, LOG = int(sys.argv[1]), sys.argv[2]
GC = GPRE + "cd {g} && echo g > gfile && git add gfile && git commit -qm guest; echo rc=$?"
HC = "echo h > hfile && git add hfile && git commit -qm host; echo rc=$?"

for off in [float(x) for x in sys.argv[3:]]:
    for t in range(N):
        repo = new_repo("c3")
        g(repo, "git status --porcelain >/dev/null; git log -1 >/dev/null")
        gargs = ["container", "exec", CT, "sh", "-c", GC.format(g=f"{GROOT}/{repo}")]
        hargs = ["bash", "-c", HC]
        kw = dict(stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if off >= 0:
            gp = subprocess.Popen(gargs, **kw); time.sleep(off)
            hp = subprocess.Popen(hargs, cwd=f"{ROOT}/{repo}", env=HENV, **kw)
        else:
            hp = subprocess.Popen(hargs, cwd=f"{ROOT}/{repo}", env=HENV, **kw); time.sleep(-off)
            gp = subprocess.Popen(gargs, **kw)
        go, ho = gp.communicate()[0].strip(), hp.communicate()[0].strip()
        s = host_state(repo)
        msgs = h(repo, "git log --format=%s main")[1].split()
        grc, hrc = go.rsplit("rc=", 1)[-1], ho.rsplit("rc=", 1)[-1]
        rec = dict(offset=off, repo=repo, guest_rc=grc, guest=go[-300:], host_rc=hrc, host=ho[-300:],
                   guest_in_main="guest" in msgs, host_in_main="host" in msgs, root_commits=
                   len(h(repo, "git rev-list --max-parents=0 main")[1].split()),
                   lock_left=h(repo, "ls .git/index.lock .git/refs/heads/main.lock 2>/dev/null")[1],
                   tree=s["tree"], fsck_rc=s["fsck_rc"], fsck=s["fsck"][-300:],
                   log=s["log"], reflog=s["reflog"], status=s["status"])
        # A commit that exited 0 must be on main; history must stay one root.
        rec["loss"] = ((grc == "0" and not rec["guest_in_main"]) or (hrc == "0" and not rec["host_in_main"])
                       or rec["root_commits"] != 1 or "a" not in s["tree"] or "b" not in s["tree"])
        out(LOG, rec)
