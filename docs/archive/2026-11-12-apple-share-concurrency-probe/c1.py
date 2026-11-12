#!/usr/bin/env python3
"""Cases 1 and 8: host commit, then guest commit after a delay.

Usage: c1.py VARIANT TRIALS LOG DELAY...
Each trial: fresh repo with 2 commits; guest primes (status, log -1); host
commits (appends to a, adds hfile); after DELAY the guest adds gfile and
commits. VARIANT picks what the guest or host does differently (see PRE);
"ls-packhook" combines the ls pre-command with a host pack-refs after commit.
"""
import re, subprocess, sys, time
from lib import *

V, N, LOG = sys.argv[1], int(sys.argv[2]), sys.argv[3]
DELAYS = [float(x) for x in sys.argv[4:]]

# Guest commands run before the guest commit, per variant.
PRE = {
    "base": "",
    "observe": "git status --porcelain; echo '--log'; git log --oneline -3; echo '--end';",
    "refresh": "git update-index --refresh; echo refresh_rc=$?;",
    "status": "git status --porcelain >/dev/null 2>&1;",
    "ls": "ls -la .git .git/refs/heads >/dev/null 2>&1;",
    "sleep11": "sleep 1.1;",
}
GIT = "sh /work/gitwrap.sh" if V == "wrap" else "git"
INIT = "--ref-format=reftable" if V == "reftable" else ""
CFG = [("core.trustctime", "false"), ("core.checkStat", "minimal"),
       ("core.fsmonitor", "false"), ("core.untrackedCache", "false")] if V == "cfg" else []

for d in DELAYS:
    for t in range(N):
        repo = new_repo(f"c1{V}", INIT, cfg=CFG)
        if V == "packed":
            h(repo, "git pack-refs --all")
        loop = None
        if V in ("optlocks", "hostloop"):
            env = dict(HENV, GIT_OPTIONAL_LOCKS="0") if V == "optlocks" else HENV
            loop = subprocess.Popen(["bash", "-c", "while :; do git status --porcelain >/dev/null; sleep 0.05; done"],
                                    cwd=f"{ROOT}/{repo}", env=env)
        prime = f"{GIT} status --porcelain >/dev/null; {GIT} log -1 >/dev/null"
        if V == "packed":
            # Steady state: the first host commit after pack-refs creates a
            # loose ref; the second replaces it.
            g(repo, prime)
            h(repo, "echo p >> b; git commit -qam pre")
        g(repo, prime)
        tp = now()
        if V == "primegap":
            time.sleep(1.2)
        hc = "echo host >> a; echo h > hfile; git add a hfile; git commit -qm host"
        if V.endswith("packhook"):
            hc += " && git pack-refs --all"
        hrc, hout = h(repo, hc)
        gap = now() - tp
        H = rev(repo)
        t0 = now()
        time.sleep(d)
        ts = now() - t0
        rc, gout = g(repo, PRE.get(V.split("-")[0], "") + f"echo g > gfile; {GIT} add gfile; {GIT} commit -qm guest; echo commit_rc=$?")
        if loop:
            loop.terminate(); loop.wait()
        s = host_state(repo)
        a_ok = "host" in h(repo, "git show main:a")[1]
        rec = dict(variant=V, delay=d, started=round(ts, 3), prime_to_host_done=round(gap, 3),
                   host_rc=hrc, host_out=hout[-200:], repo=repo,
                   guest_commit_rc=(re.findall(r"commit_rc=(\d+)", gout) or [None])[-1],
                   history_kept=is_anc(repo, H, "main"),
                   tree=s["tree"], a_has_host=a_ok,
                   fsck_rc=s["fsck_rc"], fsck=s["fsck"][-300:],
                   log=s["log"], reflog=s["reflog"], status=s["status"], guest=gout[-600:])
        rec["loss"] = not rec["history_kept"]
        rec["content_loss"] = (not a_ok) or ("hfile" not in s["tree"]) or ("b" not in s["tree"])
        rec["clean"] = (rec["history_kept"] and not rec["content_loss"] and "gfile" in s["tree"]
                        and not s["status"] and s["fsck_rc"] == 0)
        rec["setup_fail"] = hrc != 0
        out(LOG, rec)
