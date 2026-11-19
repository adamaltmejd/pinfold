#!/bin/bash
# Usage: ctl.sh. Host only, no box: as the earlier probe's ctl.sh, a fresh
# repository per command, 20 runs of `touch a; CMD`, counting runs after
# which .git/index had a new inode.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=host GIT_AUTHOR_EMAIL=h@x GIT_COMMITTER_NAME=host GIT_COMMITTER_EMAIL=h@x
T=$(mktemp -d /tmp/pfc.XXXX)
for cmd in "git diff --stat" "GIT_OPTIONAL_LOCKS=0 git diff --stat" "git --no-optional-locks diff --stat" \
           "git diff" "GIT_OPTIONAL_LOCKS=0 git diff" \
           "git status --porcelain" "GIT_OPTIONAL_LOCKS=0 git status --porcelain"; do
  d=$(mktemp -d "$T/r.XXXX"); cd "$d" || exit 1
  git init -q -b main; echo a > a; echo b > b; git add a b; git commit -qm c0
  n=0
  for _ in $(seq 20); do
    x=$(stat -f %i .git/index); sleep 1.1; touch a; bash -c "$cmd" >/dev/null 2>&1
    y=$(stat -f %i .git/index); [ "$x" != "$y" ] && n=$((n+1))
  done
  echo "$cmd: $n/20"
done
rm -rf "$T"
