#!/bin/bash
# Case 5: the worktree's .git file mounted read-only on its own. Usage: T=... c5.sh
. "$(dirname "$0")/lib.sh"
CT=wtp5; W=$R/wt/x
fresh
box $CT -v $W/.git:$W/.git:ro
container list --all 2>&1 | grep wtp5 | sed 's/^/  /'
g $CT $W "grep -E ' $R' /proc/self/mountinfo | cut -d' ' -f5,6,9-"
echo "## reads"
g $CT $W "git rev-parse --show-toplevel --git-dir; git status --short --branch; git log --oneline; echo boxedit >> a; git diff --stat"
echo "## replace the .git file"
g $CT $W "printf 'gitdir: /nowhere\n' > .git"
g $CT $W "echo x >> .git"
g $CT $W "mv .git .git.old"
g $CT $W "mv a .git"
g $CT $W "ln -s /nowhere .git2 && mv -T .git2 .git"
g $CT $W "cat .git"
echo "## rename the worktree directory or its parent"
g $CT $R "mv wt/x wt/x2"
g $CT $R "mv wt wt2"
g $CT $R "mv $R ${R}2"
echo "## a sibling path with its own .git file"
g $CT $R "mkdir wt/z && cp -r wt/x/. wt/z/ 2>&1 | head -3; printf 'gitdir: $R/wt/zgit\n' > wt/z/.git && cat wt/z/.git"
h $W "cat .git; git status --short"
container rm -f $CT >/dev/null
