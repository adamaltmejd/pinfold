#!/bin/bash
# Q6: prime in container A (and optionally B), mutate on host, observe in both at once. Usage: two-guests.sh A B HOSTDIR TAG primeB(0|1)
A=$1; B=$2; H=$3/$5$4; G=/work/$5$4; mkdir -p $H; echo old > $H/atom; echo d > $H/del
P="cat $G/atom >/dev/null; stat $G/del >/dev/null"
container exec $A sh -c "$P"; [ $5 = 1 ] && container exec $B sh -c "$P"
echo new > $H/atom.tmp; mv -f $H/atom.tmp $H/atom; rm $H/del
O="echo atom=\$(cat $G/atom 2>&1) del=\$(stat -c %i $G/del 2>&1)"
echo "primeB=$5 A: $(container exec $A sh -c "$O")"
echo "primeB=$5 B: $(container exec $B sh -c "$O")"
