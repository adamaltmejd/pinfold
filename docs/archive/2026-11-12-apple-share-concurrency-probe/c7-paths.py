#!/usr/bin/env python3
"""Case 7 precondition: what a box that mounts the share at /work makes of a
host-created linked worktree, with absolute paths and with --relative-paths,
and what a guest `git worktree prune` does to the host's worktree.

Usage: c7-paths.py (PFS_CT is the /work box)
"""
from lib import *

for args in ["", "--relative-paths"]:
    r = new_repo("c7p")
    print(f"== host: git worktree add {args} ./wt -b wtb ->", h(r, f"git worktree add -q {args} ./wt -b wtb"))
    print("host wt/.git:", h(r, "cat wt/.git")[1])
    print("host .git/worktrees/wt/gitdir:", h(r, "cat .git/worktrees/wt/gitdir")[1])
    print("host extensions.relativeWorktrees:", h(r, "git config --get extensions.relativeWorktrees")[1] or "(unset)")
    print("guest in wt: git status ->", g(r + "/wt", "git status 2>&1"))
    print("guest in root: git status --short; git worktree list ->", g(r, "git status --short 2>&1; git worktree list 2>&1"))

r = new_repo("c7p")
h(r, "git worktree add -q ./wt -b wtb")
print("== guest git worktree prune on absolute-path worktree")
print("guest prune -n -v:", g(r, "git worktree prune -n -v 2>&1"))
print("guest prune:", g(r, "git worktree prune -v 2>&1; echo rc=$?"))
print("host worktree list:", h(r, "git worktree list"))
print("host ls .git/worktrees:", h(r, "ls .git/worktrees 2>&1"))
print("host in wt: git status ->", h(r + "/wt", "git status 2>&1"))
