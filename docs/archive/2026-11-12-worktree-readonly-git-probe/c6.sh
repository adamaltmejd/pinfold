#!/bin/bash
# Case 6: a worktree outside the main root, T/side, shared writable, with
# only R/.git shared read-only. Usage: T=... c6.sh
. "$(dirname "$0")/lib.sh"
CT=wtp6; S=$T/side
fresh
h $R "git worktree add -q $S -b side && cat $S/.git"
echo "host\$ container run -d --name $CT --user 501:20 -v $S:$S -v $R/.git:$R/.git:ro --entrypoint sleep IMAGE infinity"
container run -d --name $CT --user 501:20 -v "$S:$S" -v "$R/.git:$R/.git:ro" --entrypoint sleep "$IMAGE" infinity >/dev/null 2>&1
echo "  rc=$?"
g $CT $S "grep -E ' $T' /proc/self/mountinfo | cut -d' ' -f5,6,9-; ls $R"
g $CT $S "git rev-parse --show-toplevel --git-dir --git-common-dir; git status --short --branch; git log --oneline; git worktree list"
g $CT $S "echo boxedit >> a; git diff --stat; git status --short"
g $CT $S "git add a"
g $CT $S "git commit -qam box"
h $S "git status --short; git log --oneline"
container rm -f $CT >/dev/null
