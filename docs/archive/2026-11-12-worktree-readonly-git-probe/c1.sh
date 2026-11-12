#!/bin/bash
# Case 1: Git in a linked worktree R/wt/x with R/.git read-only. Usage: T=... c1.sh
. "$(dirname "$0")/lib.sh"
CT=wtp1; W=$R/wt/x
fresh
box $CT
g $CT $W "grep -E ' $R' /proc/self/mountinfo | cut -d' ' -f5,6,9-"
g $CT $W "git rev-parse --show-toplevel --git-dir --git-common-dir"
g $CT $W "git status; git log --oneline; git worktree list"
# The box changes a tracked file and touches another, so the index is stale.
g $CT $W "echo boxedit >> a; touch .gitignore; git diff --stat; git diff"
echo "## status with a stale index, default"
g $CT $W "git status --short; echo status_rc=\$?; ls $R/.git/worktrees/x"
g $CT $W "git status --short"
echo "## status with a stale index, GIT_OPTIONAL_LOCKS=0"
g $CT $W "GIT_OPTIONAL_LOCKS=0 git status --short"
g $CT $W "git update-index --refresh"
echo "## writes"
g $CT $W "git add a"
g $CT $W "git commit -qam box"
g $CT $W "git stash"
g $CT $W "git checkout -q -b other"
g $CT $W "touch $R/.git/worktrees/x/y"
g $CT $W "echo x > $R/.git/worktrees/x/HEAD"
echo "## host afterwards"
h $W "git log --oneline --all; git status --short; git fsck --no-progress; ls $R/.git/worktrees/x"
container rm -f $CT >/dev/null
