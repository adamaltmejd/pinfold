#!/bin/bash
# Host-only control, no box involved. Usage: ctl.sh ROOT.
# For each observer command: a fresh repository, then 20 runs of
# `touch a; CMD`, counting runs after which .git/index has a new inode.
# The user's own environment and global git config, as in c4.py.
set -u
ROOT=$1
i=0
while IFS= read -r cmd; do
  i=$((i+1)); r="$ROOT/ctl$i"; mkdir "$r"; cd "$r" || exit 1
  git init -q -b main .
  for k in 0 1; do echo a$k >> a; echo b$k >> b; git add a b
    git -c commit.gpgsign=false commit -qm c$k; done
  n=0
  for k in $(seq 20); do
    touch a; x=$(stat -f %i .git/index)
    bash -c "$cmd" >/dev/null 2>&1
    y=$(stat -f %i .git/index); [ "$x" != "$y" ] && n=$((n+1))
    sleep 0.05
  done
  echo "$cmd: index replaced $n of 20"
done <<'EOF'
git status --porcelain
starship prompt
git --no-optional-locks status --porcelain
GIT_OPTIONAL_LOCKS=0 git status --porcelain
git log -1
git diff --stat
EOF
# What starship runs: one prompt with trace logging in the first repository.
cd "$ROOT/ctl2" && echo x >> a
echo "--- starship prompt output (ANSI stripped):"
starship prompt 2>/dev/null | sed $'s/\x1b\\[[0-9;]*m//g; s/%{//g; s/%}//g'
echo "--- starship trace, git commands executed:"
STARSHIP_LOG=trace starship prompt 2>&1 >/dev/null | grep 'Executing git command' | sed $'s/\x1b\\[[0-9;]*m//g' | head -20
