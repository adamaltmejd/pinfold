#!/usr/bin/env python3
"""Print the tables in the probe record from logs/c1-*.jsonl and logs/c4-*.jsonl."""
import glob, json, os
from collections import defaultdict

os.chdir(os.path.join(os.path.dirname(os.path.abspath(__file__)), "logs"))
print("c1: variant delay n history_loss content_loss guest_rc0 clean polls(max) seen_after(min-max)")
for f in sorted(glob.glob("c1-*.jsonl")):
    t = defaultdict(list)
    for l in open(f):
        r = json.loads(l)
        t[r["delay"]].append(r)
    for d, rs in sorted(t.items()):
        se = [r["seen_after"] for r in rs]
        print(rs[0]["variant"], d, len(rs), sum(r["loss"] for r in rs),
              sum(r["content_loss"] for r in rs), sum(r["guest_commit_rc"] == "0" for r in rs),
              sum(r["clean"] for r in rs), max(r["polls"] for r in rs),
              f"{min(se)}-{max(se)}")
print()
print("c4: mode bad/iters refusals first_bad commits trees replacements(changes runs) fsck")
for f in sorted(glob.glob("c4-*.jsonl")):
    rs = [json.loads(l) for l in open(f)]
    s = [r for r in rs if r.get("summary")]
    print(s[0]["mode"], f"{sum(r['bad_iters'] for r in s)}/{sum(r['iters'] for r in s)}",
          sum(r["refusals"] for r in s), [r["first_bad"] for r in s],
          f"{sum(r['commits_on_main'] - 2 for r in s)}/{sum(r['iters'] for r in s)}",
          [" ".join(r["tree"]) for r in s], [r["host_loop_replacements_runs"] for r in s],
          [r["fsck_rc"] for r in s])
