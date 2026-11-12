#!/usr/bin/env python3
"""Case 4 and mitigation (f): a side that only refreshes the index.

Usage: c4.py MODE REPOS ITERS LOG
MODE:
  hostloop       host loops `touch a; git status`; guest edits, adds, commits.
  hostloop-ol0   the same with GIT_OPTIONAL_LOCKS=0 on the host loop.
  hostloop-wrap  host loop as hostloop; the guest runs git through gitwrap.sh.
  guestloop      guest loops `touch a; git status`; host edits, adds, commits.
The touch makes each status refresh stat data and rewrite .git/index, unless
optional locks are off. Each iteration of the committing side appends to its
own file, checks status for anything but that file, and commits.
"""
import os, subprocess, sys
from lib import *

MODE, R, IT, LOG = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
LOOP = "while :; do touch a; git status --porcelain >/dev/null 2>&1; sleep 0.1; done"
GIT = "sh /work/gitwrap.sh" if MODE.endswith("wrap") else "git"

for r in range(R):
    repo = new_repo("c4")
    p = f"{ROOT}/{repo}"
    ino0 = os.stat(f"{p}/.git/index").st_ino
    if MODE.startswith("hostloop"):
        env = dict(HENV, GIT_OPTIONAL_LOCKS="0") if MODE == "hostloop-ol0" else HENV
        loop = subprocess.Popen(["bash", "-c", LOOP], cwd=p, env=env)
    else:
        # The guest has no pkill: the loop stops when the host creates a stop
        # file outside the repository (a new name reaches the guest at once).
        gl = LOOP.replace("while :; do", f"i=0; while [ ! -e ../stop-{repo} ] && [ $i -lt 900 ]; do i=$((i+1));")
        loop = subprocess.Popen(["container", "exec", CT, "sh", "-c", GPRE + f"cd {GROOT}/{repo}; " + gl])
    inos, bad = set(), 0
    for i in range(IT):
        step = (f"echo {i} >> mine; {GIT} status --porcelain; echo --; {GIT} add mine && "
                f"{GIT} commit -qm m{i}; echo rc=$?")
        rc, o = (h(repo, step) if MODE == "guestloop" else g(repo, step))
        st, _, rest = o.partition("--")
        # Expected status: only `mine` (modified, or untracked on the first pass).
        odd = [l for l in st.strip().splitlines() if l.strip() not in ("M mine", "?? mine")]
        ok = rest.strip().endswith("rc=0") and not odd
        try:
            inos.add(os.stat(f"{p}/.git/index").st_ino)
        except FileNotFoundError:
            pass
        if not ok:
            bad += 1
            out(LOG, dict(mode=MODE, repo=repo, iter=i, odd_status=odd, out=o[-400:]))
    if MODE.startswith("hostloop"):
        loop.terminate()
    else:
        open(f"{ROOT}/stop-{repo}", "w").close()
    loop.wait()
    s = host_state(repo)
    n = len(s["log"])
    rec = dict(mode=MODE, repo=repo, summary=True, iters=IT, bad_iters=bad, commits_on_main=n,
               expected=IT + 2, roots=len(h(repo, "git rev-list --max-parents=0 main")[1].split()),
               index_inodes_seen=len(inos | {ino0}), tree=s["tree"],
               mine_lines=int(h(repo, "git show main:mine | wc -l")[1] or 0),
               cached=h(repo, "git diff --cached --name-status")[1], status=s["status"],
               fsck_rc=s["fsck_rc"], fsck=s["fsck"][-300:])
    rec["clean"] = (n == IT + 2 and rec["roots"] == 1 and rec["mine_lines"] == IT and not rec["cached"]
                    and sorted(s["tree"]) == ["a", "b", "mine"] and s["fsck_rc"] == 0)
    out(LOG, rec)
