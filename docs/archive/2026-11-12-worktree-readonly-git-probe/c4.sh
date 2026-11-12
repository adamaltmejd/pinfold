#!/bin/bash
# Case 4: the box repoints the worktree's .git file at a gitdir it made.
# Every script the box plants only appends one line to $T/marker.log,
# naming itself and the kernel it ran under. Usage: T=... c4.sh
. "$(dirname "$0")/lib.sh"
CT=wtp4; W=$R/wt/x; M=$T/marker.log
fresh
box $CT
mark() { printf '#!/bin/sh\necho "%s ran on $(uname -s) args=$*" >> %s\nexit %s\n' "$1" "$M" "$2"; }

echo "## variant A: own gitdir with commondir -> R/.git and its own config"
E=$R/wt/evilA
g $CT $W "mkdir $E && cp $R/.git/worktrees/x/HEAD $R/.git/worktrees/x/index $E/ && echo $R/.git > $E/commondir && echo $W/.git > $E/gitdir \
  && printf '$(mark fsmonitor-A 1)' > $E/fsm.sh && chmod +x $E/fsm.sh \
  && printf '[core]\n\tfsmonitor = $E/fsm.sh\n' > $E/config && printf 'gitdir: $E\n' > .git && cat .git"
h $W "git rev-parse --git-dir --git-common-dir; git status --short; git config --show-origin core.fsmonitor"
h $T "cat marker.log"

echo "## variant B: a full copy of R/.git with its own config and hooks"
E=$R/wt/evilB
g $CT $W "cp -r $R/.git $E && echo 'ref: refs/heads/x' > $E/HEAD && cp $R/.git/worktrees/x/index $E/index \
  && mkdir $E/h && printf '$(mark fsmonitor-B 1)' > $E/fsm.sh \
  && printf '$(mark pre-commit-B 0)' > $E/h/pre-commit && printf '$(mark post-commit-B 0)' > $E/h/post-commit \
  && chmod +x $E/fsm.sh $E/h/pre-commit $E/h/post-commit \
  && git config -f $E/config core.fsmonitor $E/fsm.sh && git config -f $E/config core.hooksPath $E/h \
  && printf 'gitdir: $E\n' > .git && cat .git && cat $E/config"
echo "## the box's own Git through the new gitdir"
g $CT $W "git rev-parse --git-dir --git-common-dir; git status --short; echo boxline >> a; git commit -qam box-in-evil; echo commit_rc=\$?; git log --oneline"
h $T "cat marker.log"
echo "## host Git in the worktree"
h $W "git rev-parse --git-dir --git-common-dir; git status --short"
h $T "cat marker.log"
h $W "echo hostline >> a; git commit -qam host-in-evil; echo commit_rc=\$?; git log --oneline"
h $T "cat marker.log"
echo "## where the host commit went"
h $R "git log --oneline --all; git -C $W log --oneline -1"
echo "## host worktree list and repair"
h $R "git worktree list; git worktree list --porcelain"
h $R "git worktree prune --dry-run -v"
h $R "git worktree repair; echo repair_rc=\$?; cat $W/.git"
h $W "git worktree repair; echo repair_rc=\$?; cat $W/.git"
h $T "cat marker.log"
container rm -f $CT >/dev/null
