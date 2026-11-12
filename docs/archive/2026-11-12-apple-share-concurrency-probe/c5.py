#!/usr/bin/env python3
"""Case 5: host ref updates other than commit.

Usage: c5.py observe|damage TRIALS LOG OP...
OP is checkout-b, switch, rebase or stash. Each trial sets up the repo on
the host, the guest primes HEAD, refs, index and worktree, then the host
runs OP. observe: the guest reports branch, HEAD, status and stash count at
0, 0.3, 0.6, 1.0 and 1.5 s. damage: at once, the guest runs status, adds
gfile and commits; the host then checks that only its current branch moved,
by one commit on top of its own tip, with its own tree plus gfile.
"""
import sys, time
from lib import *

MODE, N, LOG, OPS = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4:]

SETUP = {
    "checkout-b": "",
    "switch": "git checkout -q -b feat && echo f >> a && echo f > ffile && git add a ffile && git commit -qm feat",
    "rebase": ("git checkout -q -b feat HEAD~1 && echo f > ffile && git add ffile && git commit -qm feat"),
    "stash": "echo dirty >> a",
}
OP = {"checkout-b": "git checkout -q -b feat", "switch": "git switch -q main",
      "rebase": "git rebase -q main", "stash": "git stash -q"}
PRIME = ("git status --porcelain >/dev/null; git log -1 >/dev/null; git branch >/dev/null; "
         "git stash list >/dev/null; cat a b >/dev/null; ls -la . .git .git/refs/heads >/dev/null")
OBS = ("echo br=$(git branch --show-current 2>&1); echo head=$(git rev-parse --short HEAD 2>&1); "
       "echo st=$(git status --porcelain 2>&1 | tr '\\n' '|'); echo stash=$(git stash list 2>&1 | wc -l)")


def refs(repo):
    return dict(l.split() for l in h(repo, "git for-each-ref --format='%(refname) %(objectname:short)'")[1].splitlines())


def tree(repo, r):
    return h(repo, f"git ls-tree -r {r}")[1].splitlines()


for op in OPS:
    for t in range(N):
        repo = new_repo("c5")
        if SETUP[op]:
            rc, o = h(repo, SETUP[op])
            assert rc == 0, o
        g(repo, PRIME)
        hrc, ho = h(repo, OP[op])
        t0 = now()
        truth = dict(br=h(repo, "git branch --show-current")[1], head=rev(repo)[:7],
                     st=h(repo, "git status --porcelain")[1], stash=h(repo, "git stash list | wc -l")[1].strip())
        rec = dict(mode=MODE, op=op, repo=repo, host_rc=hrc, host=ho[-200:], truth=truth)
        if MODE == "observe":
            obs = []
            for d in (0, 0.3, 0.6, 1.0, 1.5):
                time.sleep(max(0, d - (now() - t0)))
                ts = round(now() - t0, 2)
                rc, o = g(repo, OBS)
                obs.append(dict(at=ts, rc=rc, out=o))
            rec["obs"] = obs
            # Fresh: every field matches the host's truth.
            for o in obs:
                kv = dict(l.split("=", 1) for l in o["out"].splitlines() if "=" in l)
                o["fresh"] = all(kv.get(k, "").strip() == truth[k] for k in ("br", "head", "stash"))
            rec["stale_at"] = [o["at"] for o in obs if not o["fresh"]]
        else:
            before = refs(repo)
            br = truth["br"]
            tip = rev(repo)
            rc, o = g(repo, "git status --porcelain; echo g > gfile; git add gfile; git commit -qm guest; echo rc=$?")
            after = refs(repo)
            moved = {k for k in set(before) | set(after) if before.get(k) != after.get(k)}
            new = rev(repo, f"refs/heads/{br}")
            parent = rev(repo, f"refs/heads/{br}^") if new else ""
            got = sorted(tree(repo, new)) if new else []
            s = host_state(repo, br)
            rec.update(guest=o[-500:], moved=sorted(moved), host_branch_after=h(repo, "git branch --show-current")[1],
                       parent_is_tip=parent == tip,
                       tree_ok=sorted(l.split("\t")[1] for l in got) == sorted([l.split("\t")[1] for l in tree(repo, tip)] + ["gfile"])
                       and all(l in got for l in tree(repo, tip)),
                       fsck_rc=s["fsck_rc"], fsck=s["fsck"][-300:], log=s["log"][:4], status=s["status"],
                       stash_after=h(repo, "git stash list | wc -l")[1].strip())
            rec["clean"] = (rec["moved"] == [f"refs/heads/{br}"] and rec["parent_is_tip"] and rec["tree_ok"]
                            and rec["host_branch_after"] == br and not s["status"] and s["fsck_rc"] == 0
                            and rec["stash_after"] == truth["stash"])
        out(LOG, rec)
