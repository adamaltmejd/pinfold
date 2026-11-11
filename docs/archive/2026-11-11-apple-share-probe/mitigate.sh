#!/bin/bash
# Q3: prime, host-mutate, run MITIGATION in the guest, observe; prints elapsed guest time.
# Usage: mitigate.sh CONTAINER HOSTDIR TAG 'MITIGATION'
N=$1; H=$2/$3; G=/work/$3; M=$4; mkdir -p $H
echo old > $H/atom; echo d > $H/del; echo r > $H/ren; echo x > $H/chm; chmod 644 $H/chm
cat > $2/obs-$3.sh <<EOF
O() { echo "\$1: atom=\$(cat $G/atom 2>&1|cut -c1-12) del=\$(stat -c %i $G/del 2>&1|cut -c1-12) ren=\$(cat $G/ren 2>&1|cut -c1-12) chm=\$(stat -c %a $G/chm)"; }
t0=\$(date +%s.%N); O before
$M
rc=\$?; t1=\$(date +%s.%N)
echo "mitigation rc=\$rc took=\$(awk -v a=\$t0 -v b=\$t1 'BEGIN{printf "%.3f", b-a}')s"; O after
EOF
container exec $N sh -c "cat /work/obs-$3.sh >/dev/null; cat $G/atom $G/del $G/ren >/dev/null; stat $G/chm >/dev/null"
echo new > $H/atom.tmp; mv -f $H/atom.tmp $H/atom; rm $H/del; mv $H/ren $H/ren2; chmod 755 $H/chm
container exec $N sh /work/obs-$3.sh
