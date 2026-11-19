#!/usr/bin/env python3
"""Case 4: one side observes the repository while the other commits.

Usage: c4.py MODE REPOS ITERS LOG
Host observer modes (the guest commits):
  hostpoll      plain `git status --porcelain` every 0.1 s, no global config
                (the 2026-11-12 mode, unchanged).
  starship      `starship prompt` every 0.1 s.
  nolocks       `git --no-optional-locks status --porcelain` every 0.1 s.
  ol0           `GIT_OPTIONAL_LOCKS=0 git status --porcelain` every 0.1 s.
  status1       `git status --porcelain` every 1 s.
  status5       `git status --porcelain` every 5 s.
  log           `git log -1` every 0.1 s.
  diffstat      `git diff --stat` every 0.1 s.
  none          no observer (control).
The modes after hostpoll run with the user's own environment and global git
config, as a shell prompt or editor would. Each observer counts how often
.git/index had a new inode after its own command (`../rw-REPO`: changes, runs).
Guest observer modes (the host commits):
  guestloop     guest loops `touch a; git status` (2026-11-12 mode).
  guestpoll     guest loops plain `git status --porcelain` every 0.1 s.
Each iteration of the committing side appends to `mine`, runs status, and
commits; anything but `mine` in that status, or a failed commit, is anomalous.
The summary records the first anomalous iteration and the failed commits.
"""
import os, subprocess, sys
from lib import *

MODE, R, IT, LOG = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
OBS = {
    "hostpoll": ("git status --porcelain", 0.1),
    "starship": ("starship prompt", 0.1),
    "nolocks": ("git --no-optional-locks status --porcelain", 0.1),
    "ol0": ("GIT_OPTIONAL_LOCKS=0 git status --porcelain", 0.1),
    "status1": ("git status --porcelain", 1),
    "status5": ("git status --porcelain", 5),
    "log": ("git log -1", 0.1),
    "diffstat": ("git diff --stat", 0.1),
}
# The slow observers would run only once or twice in an unpaced repo, so the
# guest waits between iterations in those modes.
PACE = {"status1": 0.7, "status5": 0.7}
POLL = ("n=0; k=0; while :; do x=$(stat -f %i .git/index); {cmd} >/dev/null 2>&1; k=$((k+1)); "
        "y=$(stat -f %i .git/index); [ \"$x\" != \"$y\" ] && n=$((n+1)); echo $n $k > ../rw-{repo}; "
        "sleep {s}; done")
GLOOP = {"guestloop": "touch a; git status --porcelain >/dev/null 2>&1",
         "guestpoll": "git status --porcelain >/dev/null 2>&1"}
# The user's environment without the probe's git overrides.
UENV = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}

for r in range(R):
    repo = new_repo("c4")
    p = f"{ROOT}/{repo}"
    ino0 = os.stat(f"{p}/.git/index").st_ino
    loop = None
    if MODE in OBS:
        cmd, s = OBS[MODE]
        loop = subprocess.Popen(["bash", "-c", POLL.format(cmd=cmd, s=s, repo=repo)], cwd=p,
                                env=HENV if MODE == "hostpoll" else UENV, start_new_session=True)
    elif MODE in GLOOP:
        # The guest has no pkill: the loop stops when the host creates a stop
        # file outside the repository (a new name reaches the guest at once).
        gl = (f"i=0; while [ ! -e ../stop-{repo} ] && [ $i -lt 900 ]; do i=$((i+1)); "
              f"{GLOOP[MODE]}; sleep 0.1; done")
        loop = subprocess.Popen(["container", "exec", CT, "sh", "-c", GPRE + f"cd {GROOT}/{repo}; " + gl])
    inos, bad, first, refused = set(), 0, None, []
    for i in range(IT):
        if i and MODE in PACE:
            time.sleep(PACE[MODE])
        step = (f"echo {i} >> mine; git status --porcelain; echo --; git add mine && "
                f"git commit -qm m{i}; echo rc=$?")
        rc, o = (h(repo, step) if MODE in GLOOP else g(repo, step))
        st, _, rest = o.partition("--")
        odd = [l for l in st.strip().splitlines() if l.strip() not in ("M mine", "?? mine")]
        ok = rest.strip().endswith("rc=0") and not odd
        try:
            inos.add(os.stat(f"{p}/.git/index").st_ino)
        except FileNotFoundError:
            pass
        if not rest.strip().endswith("rc=0"):
            refused.append(rest.strip()[-160:])
        if not ok:
            bad += 1
            first = i if first is None else first
            out(LOG, dict(mode=MODE, repo=repo, iter=i, odd_status=odd, out=o[-400:]))
    if MODE in GLOOP:
        open(f"{ROOT}/stop-{repo}", "w").close()
    elif loop:
        os.killpg(loop.pid, 15)
    if loop:
        loop.wait()
    s = host_state(repo)
    n = len(s["log"])
    rw = f"{ROOT}/rw-{repo}"
    rec = dict(mode=MODE, repo=repo, summary=True, iters=IT, bad_iters=bad, first_bad=first,
               refusals=len(refused), refused=refused, commits_on_main=n,
               expected=IT + 2, roots=len(h(repo, "git rev-list --max-parents=0 main")[1].split()),
               index_inodes_seen=len(inos | {ino0}), tree=s["tree"],
               host_loop_replacements_runs=open(rw).read().strip() if os.path.exists(rw) else None,
               mine_lines=int(h(repo, "git show main:mine | wc -l")[1] or 0),
               cached=h(repo, "git diff --cached --name-status")[1], status=s["status"],
               fsck_rc=s["fsck_rc"], fsck=s["fsck"][-300:])
    rec["clean"] = (n == IT + 2 and rec["roots"] == 1 and rec["mine_lines"] == IT and not rec["cached"]
                    and sorted(s["tree"]) == ["a", "b", "mine"] and s["fsck_rc"] == 0)
    out(LOG, rec)
