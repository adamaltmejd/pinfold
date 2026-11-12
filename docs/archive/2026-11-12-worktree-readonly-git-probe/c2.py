#!/usr/bin/env python3
"""Case 2: box edits in the worktree, host commits there, box reads.

Usage: T=... c2.py CT TRIALS LOG. Expects R/wt/x and a running box CT;
c2.sh makes both with fresh() and box(). Each trial: the box appends a
line to a; the host checks it sees the line and commits; the box then
runs `git log -1` and `git status --porcelain` at once and again 1.5 s
after the host commit.
A read is right when HEAD equals the host's new commit and status is empty.
"""
import json, os, subprocess, sys, time

T = os.environ["T"]
CT, N, LOG = sys.argv[1], int(sys.argv[2]), sys.argv[3]
W = f"{T}/R/wt/x"
HENV = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1",
            GIT_AUTHOR_NAME="host", GIT_AUTHOR_EMAIL="h@x",
            GIT_COMMITTER_NAME="host", GIT_COMMITTER_EMAIL="h@x")
GPRE = ("export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_COUNT=1 "
        "GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0='*'; ")


def h(cmd):
    r = subprocess.run(["bash", "-c", cmd], cwd=W, env=HENV, capture_output=True, text=True)
    return r.returncode, (r.stdout + r.stderr).strip()


def g(cmd):
    r = subprocess.run(["container", "exec", CT, "sh", "-c", GPRE + f"cd {W} && {{ {cmd} ; }}"],
                       capture_output=True, text=True)
    return r.returncode, (r.stdout + r.stderr).strip()


READ = "git log -1 --format=%H; echo --; git status --porcelain"


def judge(out, head):
    top, _, st = out.partition("--")
    return top.strip() == head and st.strip() == "", out[-300:]


with open(LOG, "a") as f:
    for t in range(N):
        g(f"echo t{t} >> a")
        seen = h(f"grep -qx t{t} a")[0] == 0
        hrc, hout = h(f"git commit -qam h{t}")
        tc = time.monotonic()
        head = h("git rev-parse HEAD")[1]
        ok0, o0 = judge(g(READ)[1], head)
        d0 = time.monotonic() - tc
        time.sleep(max(0, 1.5 - (time.monotonic() - tc)))
        d1 = time.monotonic() - tc
        ok1, o1 = judge(g(READ)[1], head)
        rec = dict(trial=t, host_saw_edit=seen, host_commit_rc=hrc, host_out=hout,
                   head=head[:7], immediate_ok=ok0, immediate_done_s=round(d0, 3),
                   later_ok=ok1, later_start_s=round(d1, 3),
                   immediate_out=None if ok0 else o0, later_out=None if ok1 else o1)
        line = json.dumps(rec)
        print(line, flush=True)
        f.write(line + "\n")
