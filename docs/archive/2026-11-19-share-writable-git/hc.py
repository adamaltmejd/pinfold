#!/usr/bin/env python3
"""Both sides commit: the host with GIT_OPTIONAL_LOCKS=0, the guest in a loop.

Usage: hc.py REPOS ITERS LOG
The guest loop is one `container exec` per repository. Each pass appends to
`theirs`, runs `git status --porcelain`, commits `theirs`, logs the status and
exit code to ../glog-REPO, and sleeps 0.1 s; it stops when the host creates
../stop-REPO. The host runs ITERS unpaced iterations of the case 4 step on
`mine` with GIT_OPTIONAL_LOCKS=0 exported. A status line is anomalous unless
it names `mine` or `theirs` with a code other than D, so a deletion or an
untracked `a` or `b` is anomalous. lib.py is the 2026-11-12 file.
"""
import os, subprocess, sys
from lib import *

R, IT, LOG = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
OL0 = dict(HENV, GIT_OPTIONAL_LOCKS="0")


def odd(status):
    # The first line arrives stripped, so the code is read without its spaces.
    bad = []
    for l in status.splitlines():
        code, _, path = l.strip().rpartition(" ")
        if l.strip() and (path not in ("mine", "theirs") or "D" in code):
            bad.append(l)
    return bad


for r in range(R):
    repo = new_repo("hc")
    gl = (f"i=0; while [ ! -e ../stop-{repo} ] && [ $i -lt 900 ]; do i=$((i+1)); "
          f"echo $i >> theirs; s=$(git status --porcelain | tr '\\n' ';'); "
          f"git add theirs && git commit -qm g$i >/dev/null 2>../gerr-{repo}; rc=$?; "
          f"echo \"$i rc=$rc st=$s err=$(tr '\\n' ' ' < ../gerr-{repo})\" >> ../glog-{repo}; "
          f"sleep 0.1; done")
    loop = subprocess.Popen(["container", "exec", CT, "sh", "-c", GPRE + f"cd {GROOT}/{repo}; " + gl])
    time.sleep(1)  # the guest loop is committing before the host starts
    bad, first, refused = 0, None, []
    for i in range(IT):
        step = (f"echo {i} >> mine; git status --porcelain; echo --; git add mine && "
                f"git commit -qm h{i}; echo rc=$?")
        rc, o = h(repo, step, env=OL0)
        st, _, rest = o.partition("--")
        o_ = odd(st)
        if not rest.strip().endswith("rc=0"):
            refused.append(rest.strip()[-160:])
        if o_ or not rest.strip().endswith("rc=0"):
            bad += 1
            first = i if first is None else first
            out(LOG, dict(side="host", repo=repo, iter=i, odd_status=o_, out=o[-400:]))
    open(f"{ROOT}/stop-{repo}", "w").close()
    loop.wait()
    glines = open(f"{ROOT}/glog-{repo}").read().splitlines()
    g_ok, g_bad, g_first, g_ref = 0, 0, None, []
    for line in glines:
        n, rcs, rest = line.split(" ", 2)
        st = rest.split(" err=")[0][3:].replace(";", "\n")
        if rcs == "rc=0":
            g_ok += 1
        else:
            g_ref.append(rest.split(" err=")[1][-160:])
        if rcs != "rc=0" or odd(st):
            g_bad += 1
            g_first = int(n) if g_first is None else g_first
            out(LOG, dict(side="guest", repo=repo, iter=int(n), odd_status=odd(st), line=line[-400:]))
    s = host_state(repo)
    subjects = h(repo, "git log --format=%s main")[1].split()
    reflog = h(repo, "git reflog --format=%gs main")[1].splitlines()
    h_landed = IT - len(refused)
    rec = dict(summary=True, repo=repo, host_iters=IT, host_bad=bad, host_first_bad=first,
               host_refusals=len(refused), host_refused=refused, host_committed=h_landed,
               host_on_main=sum(1 for x in subjects if x.startswith("h")),
               guest_iters=len(glines), guest_bad=g_bad, guest_first_bad=g_first,
               guest_refusals=len(g_ref), guest_refused=g_ref[:5], guest_committed=g_ok,
               guest_on_main=sum(1 for x in subjects if x.startswith("g")),
               commits_on_main=len(subjects),
               roots=len(h(repo, "git rev-list --max-parents=0 main")[1].split()),
               reflog_entries=len(reflog), tree=s["tree"],
               mine_lines=int(h(repo, "git show main:mine 2>/dev/null | wc -l")[1] or 0),
               theirs_lines=int(h(repo, "git show main:theirs 2>/dev/null | wc -l")[1] or 0),
               cached=h(repo, "git diff --cached --name-status")[1], status=s["status"],
               fsck_rc=s["fsck_rc"], fsck=s["fsck"][-300:])
    rec["clean"] = (rec["host_on_main"] == h_landed and rec["guest_on_main"] == g_ok
                    and rec["roots"] == 1 and not rec["cached"] and not s["status"]
                    and sorted(s["tree"]) == ["a", "b", "mine", "theirs"] and s["fsck_rc"] == 0)
    out(LOG, rec)
