#!/bin/bash
# Case 3: host adds and removes a worktree while the box runs. Usage: T=... c3.sh
. "$(dirname "$0")/lib.sh"
CT=wtp3; Y=$R/wt/y
fresh
box $CT
g $CT $R "git worktree list; ls wt"
# A box process that sits in R/wt/y once it exists, to see what removal does to it.
echo "## host adds wt/y"
h $R "git worktree add -q wt/y -b y && git worktree list"
g $CT $R "git worktree list; ls wt; git -C wt/y status --short --branch; git -C wt/y log --oneline -1"
sleep 1.5
echo "## 1.5 s after add"
g $CT $R "git worktree list; git -C wt/y status --short --branch; git -C wt/y log --oneline -1"
container exec -d $CT sh -c "cd $Y && sleep 30" 2>&1
echo "  (box process started with cwd $Y)"
g $CT $Y "echo boxfile > boxfile; ls"
echo "## host removes wt/y (dirty: the box's untracked file)"
h $R "git worktree remove wt/y"
h $R "git worktree remove --force wt/y; git worktree list; ls wt"
g $CT $R "git worktree list; ls wt; git -C wt/y status --short --branch"
sleep 1.5
echo "## 1.5 s after remove"
g $CT $R "git worktree list; ls wt; git -C wt/y status --short --branch"
g $CT $R "cat /proc/\$(pgrep -f 'sleep 30' | head -1)/cwd/boxfile 2>&1; ls -l /proc/\$(pgrep -f 'sleep 30' | head -1)/cwd 2>&1"
h $R "git worktree list; git branch; git fsck --no-progress"
container rm -f $CT >/dev/null
