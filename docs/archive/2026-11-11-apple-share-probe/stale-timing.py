#!/usr/bin/env python3
"""Measure how long a guest sees a stale view after a host mutation.

Usage: stale-timing.py CONTAINER REPO_HOST_DIR TAG [mode]
mode "all" (default): per op, one "poll" file observed at every delay, plus one
"quiet" file per delay observed only once (no guest lookup between priming and
that observation). All four ops share one timeline.
"""
import subprocess, sys, time, os, json

N, R, TAG = sys.argv[1], sys.argv[2], sys.argv[3]
DELAYS = [float(x) for x in os.environ.get("DELAYS", "0 0.5 1 2 5 10 30 60 120").split()]
H = f"{R}/{TAG}"; G = f"/work/{TAG}"
os.makedirs(H, exist_ok=True)
labels = ["poll"] + [f"q{i}" for i in range(len(DELAYS))]

def w(p, s):
    with open(p, "w") as f: f.write(s)

for l in labels:
    w(f"{H}/atom_{l}", "old\n")
    w(f"{H}/del_{l}", "del\n")
    w(f"{H}/ren_{l}", "ren\n")
    w(f"{H}/chm_{l}", "x\n"); os.chmod(f"{H}/chm_{l}", 0o644)

def gexec(script):
    r = subprocess.run(["container", "exec", N, "sh", "-c", script],
                       capture_output=True, text=True)
    return r.stdout

# Guest check for one label: prints op=ok|stale per op.
def check(l):
    return (f'a=$(cat {G}/atom_{l} 2>&1); [ "$a" = new ] && echo atom_{l}=ok || echo "atom_{l}=stale:$a";'
            f'stat {G}/del_{l} >/dev/null 2>&1 && echo del_{l}=stale || echo del_{l}=ok;'
            f'o=0; cat {G}/ren_{l} >/dev/null 2>&1 && o=1; n=0; cat {G}/ren2_{l} >/dev/null 2>&1 && n=1;'
            f' [ $o$n = 01 ] && echo ren_{l}=ok || echo ren_{l}=stale:old$o.new$n;'
            f'm=$(stat -c %a {G}/chm_{l} 2>&1); [ "$m" = 755 ] && echo chm_{l}=ok || echo "chm_{l}=stale:$m";')

def prime(l):
    return (f'cat {G}/atom_{l} >/dev/null; stat {G}/atom_{l} >/dev/null; cat {G}/del_{l} >/dev/null;'
            f'cat {G}/ren_{l} >/dev/null; cat {G}/ren2_{l} >/dev/null 2>&1; stat {G}/chm_{l} >/dev/null;')

tp0 = time.monotonic()
gexec("".join(prime(l) for l in labels))
tp1 = time.monotonic()

t0 = time.monotonic()
for l in labels:
    w(f"{H}/atom_{l}.tmp", "new\n"); os.replace(f"{H}/atom_{l}.tmp", f"{H}/atom_{l}")
    os.unlink(f"{H}/del_{l}")
    os.rename(f"{H}/ren_{l}", f"{H}/ren2_{l}")
    os.chmod(f"{H}/chm_{l}", 0o755)
tmut = time.monotonic() - t0

rows = []
for i, d in enumerate(DELAYS):
    now = time.monotonic() - t0
    if d > now: time.sleep(d - now)
    ts = time.monotonic() - t0
    out = gexec(check("poll") + check(f"q{i}"))
    te = time.monotonic() - t0
    res = dict(x.split("=", 1) for x in out.splitlines() if "=" in x)
    rows.append({"target": d, "start": round(ts, 2), "end": round(te, 2), **res})
    print(json.dumps(rows[-1]), flush=True)
print(json.dumps({"mutation_took": round(tmut, 3), "prime_start": round(tp0 - t0, 3), "prime_end": round(tp1 - t0, 3)}))
