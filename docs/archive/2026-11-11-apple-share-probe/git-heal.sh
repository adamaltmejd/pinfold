#!/bin/bash
# Host commit after a guest git read, then guest git log/status after delay D.
# Usage: git-heal.sh CONTAINER HOSTDIR NAME DELAY...
N=$1; H=$2/$3; G=/work/$3; shift 3
rm -rf $H; mkdir -p $H; cd $H
git init -q -b main; git config user.email p@x; git config user.name p
echo 0 > f; git add f; git commit -qm c0
GG="git -c safe.directory=* -C $G"
for d in "$@"; do
  container exec $N sh -c "$GG status --porcelain >/dev/null; $GG log -1 --format=%H >/dev/null" 2>/dev/null
  echo $d >> f; git commit -qam "c$d"; want=$(git rev-parse HEAD)
  t0=$(perl -MTime::HiRes=time -e 'printf "%.3f",time')
  sleep $d
  out=$(container exec $N sh -c "$GG log -1 --format=%H 2>&1; echo ST; $GG status --porcelain 2>&1")
  t1=$(perl -MTime::HiRes=time -e 'printf "%.3f",time')
  got=$(echo "$out" | head -1); st=$(echo "$out" | sed '1,/^ST$/d' | tr '\n' '|')
  [ "$got" = "$want" ] && l=ok || l="STALE:$got"
  printf 'delay=%s elapsed=%.2f log=%s status=[%s]\n' $d $(echo "$t1-$t0"|bc) "$l" "$st"
done
