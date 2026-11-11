#!/bin/bash
# Prime, wait W seconds, mutate on host, observe immediately. Usage: expired-prime.sh CONTAINER HOSTDIR W
N=$1; H=$2; W=$3; T=$(basename $H)$W; mkdir -p $H/$T; G=/work/$T
echo old > $H/$T/atom; echo d > $H/$T/del
container exec $N sh -c "cat $G/atom; stat $G/del >/dev/null" >/dev/null
sleep $W
echo new > $H/$T/atom.tmp; mv -f $H/$T/atom.tmp $H/$T/atom; rm $H/$T/del
container exec $N sh -c "echo wait=$W atom=\$(cat $G/atom 2>&1) del=\$(stat -c ok $G/del 2>&1 >/dev/null && echo STALE || echo gone)"
