#!/bin/bash
# Case 5b: after renaming the worktree directory away, can the box put a
# .git file of its own at the path the host expects? Scripts only append
# a line to $T/marker.log. Usage: T=... c5b.sh
. "$(dirname "$0")/lib.sh"
CT=wtp5b; W=$R/wt/x; E=$R/wt/evil; M=$T/marker.log
fresh
box $CT -v $W/.git:$W/.git:ro
g $CT $R "mv wt/x wt/x2 && grep -E ' $R' /proc/self/mountinfo | cut -d' ' -f5,6,9-"
g $CT $R "cat wt/x2/.git; printf 'gitdir: /nowhere\n' > wt/x2/.git"
h $R "ls wt; cat wt/x2/.git; git worktree list"
echo "## box builds a gitdir and a new wt/x"
g $CT $R "cp -r .git $E && echo 'ref: refs/heads/x' > $E/HEAD && cp .git/worktrees/x/index $E/index \
  && printf '#!/bin/sh\necho \"fsmonitor ran on \$(uname -s) args=\$*\" >> $M\nexit 1\n' > $E/fsm.sh && chmod +x $E/fsm.sh \
  && git config -f $E/config core.fsmonitor $E/fsm.sh \
  && mkdir wt/x && cp wt/x2/a wt/x2/.gitignore wt/x/ && printf 'gitdir: $E\n' > wt/x/.git && cat wt/x/.git"
echo "## host in wt/x"
h $W "cat .git; git rev-parse --git-dir; git status --short"
h $T "cat marker.log"
h $R "git worktree list; git worktree repair; echo repair_rc=\$?; cat wt/x/.git"
container rm -f $CT >/dev/null
