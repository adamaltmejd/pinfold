#!/usr/bin/env python3
"""Case 6: atomic editor saves against edits on the other side.

Usage: c6.py h2g|g2h TRIALS LOG DELAY...
h2g: the guest reads f, the host saves f atomically (temp, then rename) as
"l1 host", and after DELAY the guest edits f by each METHOD to add "guest".
g2h: the same with the sides swapped. Sequential, so last-writer-wins
predicts "l1 host guest" (h2g) or "l1 guest host" (g2h) on the host.
"""
import os, sys, time
from lib import *

DIR, N, LOG = sys.argv[1], int(sys.argv[2]), sys.argv[3]
DELAYS = [float(x) for x in sys.argv[4:]]
METHODS = {
    "append": "echo {w} >> f",
    "truncwrite": "c=$(cat f); printf '%s\\n{w}\\n' \"$c\" > f",
    "tmpmv": "c=$(cat f); printf '%s\\n{w}\\n' \"$c\" > f.tmp && mv f.tmp f",
    "sedi": "sed -i '$a {w}' f",
}
SAVE = "printf 'l1\\n{w}\\n' > f.tmp && mv -f f.tmp f"

for d in DELAYS:
    for m, cmd in METHODS.items():
        if DIR == "g2h" and m == "sedi":
            m, cmd = "perli", "perl -0pi -e 's/\\z/{w}\\n/' f"  # BSD sed -i differs; perl -i also renames
        for t in range(N):
            repo = f"c6{DIR}{os.getpid() % 10000}_{m}_{d}_{t}"
            os.makedirs(f"{ROOT}/{repo}")
            h(repo, "printf 'l1\\n' > f")
            if DIR == "h2g":
                first, second, fw, sw = h, g, "host", "guest"
            else:
                first, second, fw, sw = g, h, "guest", "host"
            g(repo, "cat f >/dev/null; stat f >/dev/null")
            r1 = first(repo, SAVE.format(w=fw))
            time.sleep(d)
            seen = second(repo, "cat f 2>&1")
            r2 = second(repo, cmd.format(w=sw) + "; echo rc=$?")
            host_now = h(repo, "cat f 2>&1")[1]
            time.sleep(1.5)
            host_later = h(repo, "cat f 2>&1")[1]
            guest_later = g(repo, "cat f 2>&1")[1]
            want = f"l1\n{fw}\n{sw}"
            rec = dict(dir=DIR, method=m, delay=d, repo=repo, first_rc=r1[0], second_saw=seen[1],
                       second_out=r2[1][-200:], host_now=host_now, host_later=host_later,
                       guest_later=guest_later, want=want)
            rec["first_lost"] = fw not in host_later
            rec["second_lost"] = sw not in host_later
            rec["clean"] = host_later == want and guest_later == want
            out(LOG, rec)
