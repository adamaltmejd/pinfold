#!/usr/bin/env python3
"""Tally a JSONL log: count records per group and per boolean outcome.

Usage: summ.py LOG GROUPKEY[,KEY...] FIELD...
"""
import json, sys
from collections import defaultdict

log, keys, fields = sys.argv[1], sys.argv[2].split(","), sys.argv[3:]
t = defaultdict(lambda: defaultdict(int))
for line in open(log):
    r = json.loads(line)
    if "summary" in r and not r.get("summary"):
        continue
    k = tuple(str(r.get(x)) for x in keys)
    t[k]["n"] += 1
    for f in fields:
        if r.get(f) in (True, "0", 0) if f.endswith("_rc") else r.get(f):
            t[k][f] += 1
for k in sorted(t):
    print(" ".join(k), " ".join(f"{f}={t[k][f]}" for f in ["n"] + fields))
